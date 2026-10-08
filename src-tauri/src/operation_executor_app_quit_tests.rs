use super::*;
use crate::operation_commands::{
    execute_operation_with, HistorySink, OperationHistoryEntry, OperationResult,
};
use crate::operations::{OperationStore, ProcessIdentity};
use crate::user_error::UserError;
use std::sync::Mutex;

struct RecordingAppQuitter {
    live: Vec<InstalledAppIdentity>,
    observed: Mutex<usize>,
    quit_names: Mutex<Vec<String>>,
    failures: Vec<(String, UserError)>,
}

impl RecordingAppQuitter {
    fn new(live: Vec<InstalledAppIdentity>, failures: Vec<(&str, UserError)>) -> Self {
        Self {
            live,
            observed: Mutex::new(0),
            quit_names: Mutex::new(Vec::new()),
            failures: failures
                .into_iter()
                .map(|(name, message)| (name.to_owned(), message.to_owned()))
                .collect(),
        }
    }
}

impl AppQuitter for RecordingAppQuitter {
    fn observe_apps(
        &self,
        bundle_paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<InstalledAppIdentity>, UserError>> {
        *self.observed.lock().unwrap() += 1;
        let observed: Vec<InstalledAppIdentity> = self
            .live
            .iter()
            .filter(|app| bundle_paths.contains(&app.bundle_path))
            .cloned()
            .collect();
        Box::pin(async move { Ok(observed) })
    }

    fn quit(&self, app: &InstalledAppIdentity) -> DomainFuture<'_, Result<(), UserError>> {
        self.quit_names.lock().unwrap().push(app.app_name.clone());
        let failure = self
            .failures
            .iter()
            .find(|(name, _)| *name == app.app_name)
            .map(|(_, message)| message.clone());
        Box::pin(async move {
            match failure {
                Some(message) => Err(message),
                None => Ok(()),
            }
        })
    }
}

#[derive(Default)]
struct RecordingHistory {
    entries: Mutex<Vec<OperationHistoryEntry>>,
}

impl HistorySink for RecordingHistory {
    fn record(&self, entry: &OperationHistoryEntry) -> Result<(), UserError> {
        self.entries.lock().unwrap().push(entry.clone());
        Ok(())
    }
}

struct NoopCache;

impl CacheCleaner for NoopCache {
    fn clean<'a>(
        &'a self,
        _items: Vec<CacheItem>,
    ) -> Pin<Box<dyn Future<Output = CleanSummary> + Send + 'a>> {
        Box::pin(async {
            CleanSummary {
                reports: Vec::new(),
                deleted_bytes: 0,
                reclaimed_bytes: None,
                success_count: 0,
                fail_count: 0,
            }
        })
    }
}

struct NoopUninstall;

impl UninstallDomain for NoopUninstall {
    fn observe_apps(
        &self,
        _bundle_paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<InstalledAppIdentity>, UserError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn observe_residues(
        &self,
        _paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<LiveResidue>, UserError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn quit(&self, _app_name: &str) -> DomainFuture<'_, Result<(), UserError>> {
        Box::pin(async { Ok(()) })
    }

    fn remove(
        &self,
        _app: &InstalledAppIdentity,
        _residues: &[ResidueIdentity],
    ) -> DomainFuture<'_, UninstallReport> {
        Box::pin(async {
            UninstallReport {
                app_name: String::new(),
                bundle_id: String::new(),
                total_freed_bytes: 0,
                moved_count: 0,
                failed_count: 0,
                details: Vec::new(),
                quit_error: None,
            }
        })
    }
}

struct NoopDocker;

impl DockerDomain for NoopDocker {
    fn inventory(&self) -> DomainFuture<'_, Result<DockerInventory, UserError>> {
        Box::pin(async { Ok(empty_inventory()) })
    }

    fn remove(
        &self,
        _action: DockerAction,
        _target: &DockerTarget,
    ) -> DomainFuture<'_, Result<(), UserError>> {
        Box::pin(async { Ok(()) })
    }

    fn prune(&self) -> DomainFuture<'_, Result<String, UserError>> {
        Box::pin(async { Ok(String::new()) })
    }
}

fn empty_inventory() -> DockerInventory {
    DockerInventory {
        daemon_running: true,
        images: Vec::new(),
        containers: Vec::new(),
        volumes: Vec::new(),
        builder: crate::docker::DockerBuilderCache {
            total_bytes: 0,
            reclaimable_bytes: 0,
        },
        reclaimable_bytes: 0,
    }
}

struct PanicObserver;

impl ProcessObserver for PanicObserver {
    fn observe(&mut self, _pids: &[u32]) -> Result<Vec<LiveProcess>, UserError> {
        panic!("优雅退出不得触碰进程观测");
    }
}

struct PanicSignaller;

impl ProcessSignaller for PanicSignaller {
    fn terminate(
        &mut self,
        _target: &ProcessTarget,
        _mode: ProcessMode,
    ) -> crate::process_ops::KillOutcome {
        panic!("优雅退出不得触碰进程信号");
    }
}

fn app_process(pid: u32, name: &str, protected: bool, whitelisted: bool) -> ProcessTarget {
    ProcessTarget {
        identity: ProcessIdentity {
            pid,
            name: name.to_owned(),
            exe: format!("/Applications/Demo.app/Contents/MacOS/{name}"),
            start_time: 10 + u64::from(pid),
        },
        protected,
        whitelisted,
    }
}

fn app_identity(label: &str) -> AppIdentity {
    AppIdentity {
        bundle_path: format!("/Applications/{label}.app"),
        bundle_id: format!("com.example.{label}"),
        app_name: label.to_owned(),
        processes: vec![app_process(700, label, true, false)],
    }
}

fn installed_identity(label: &str) -> InstalledAppIdentity {
    InstalledAppIdentity {
        bundle_path: format!("/Applications/{label}.app"),
        app_name: label.to_owned(),
        bundle_id: format!("com.example.{label}"),
        is_system: false,
        bundle_size_bytes: 42,
    }
}

type TestDomains<'a> =
    DomainServices<'a, NoopCache, NoopUninstall, RecordingAppQuitter, NoopDocker>;

struct GracefulHarness {
    store: Mutex<OperationStore>,
    operation_id: String,
}

impl GracefulHarness {
    fn new() -> Self {
        let mut store = OperationStore::new();
        let registration = store
            .register_application_snapshot(vec![app_identity("Demo")])
            .expect("注册应用快照");
        let prepared = store
            .prepare_app_graceful_quit(
                &registration.snapshot_id,
                registration.app_keys.clone(),
                "main",
            )
            .expect("准备优雅退出");
        Self {
            store: Mutex::new(store),
            operation_id: prepared.operation_id,
        }
    }

    async fn run(
        &self,
        quitter: &RecordingAppQuitter,
        history: &RecordingHistory,
    ) -> Result<OperationResult, UserError> {
        let cache = NoopCache;
        let uninstall = NoopUninstall;
        let docker = NoopDocker;
        let domains: TestDomains<'_> = DomainServices {
            cache: &cache,
            uninstall: &uninstall,
            app_quit: quitter,
            docker: &docker,
        };
        execute_operation_with(
            &self.store,
            history,
            "main",
            &self.operation_id,
            |plan| async move { execute_domain_plan(plan, &domains).await },
        )
        .await
    }
}

#[tokio::test]
async fn graceful_quit_plan_routes_to_the_app_quitter_domain() {
    let harness = GracefulHarness::new();
    let quitter = RecordingAppQuitter::new(vec![installed_identity("Demo")], Vec::new());
    let history = RecordingHistory::default();

    let result = harness
        .run(&quitter, &history)
        .await
        .expect("优雅退出必须成功");

    let OperationResult::AppGracefulQuit(reports) = result else {
        panic!("期望优雅退出结果，得到 {result:?}");
    };
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].app_name, "Demo");
    assert_eq!(reports[0].bundle_id, "com.example.Demo");
    assert_eq!(reports[0].quit_error, None);
    assert_eq!(quitter.quit_names.lock().unwrap().as_slice(), ["Demo"]);
    assert_eq!(*quitter.observed.lock().unwrap(), 1);
}

#[tokio::test]
async fn graceful_quit_never_touches_the_process_channel() {
    let harness = GracefulHarness::new();
    let plan = harness
        .store
        .lock()
        .unwrap()
        .consume(&harness.operation_id, "main")
        .expect("消费优雅退出计划");
    let mut observer = PanicObserver;
    let mut signaller = PanicSignaller;

    let error = execute_termination_plan(
        plan,
        &mut ProcessServices {
            observer: &mut observer,
            signaller: &mut signaller,
        },
    )
    .expect_err("优雅退出计划不能进进程通道");

    assert!(error.contains("只接受进程与应用终止计划"), "{error}");
}

#[tokio::test]
async fn graceful_quit_refuses_a_changed_bundle_id() {
    let harness = GracefulHarness::new();
    let mut drifted = installed_identity("Demo");
    drifted.bundle_id = "com.other.Demo".to_owned();
    let quitter = RecordingAppQuitter::new(vec![drifted], Vec::new());

    let error = harness
        .run(&quitter, &RecordingHistory::default())
        .await
        .expect_err("bundle ID 变化必须拒绝");

    assert!(error.contains("bundle ID 已变化"), "{error}");
    assert!(error.contains("请重新扫描"), "{error}");
    assert!(quitter.quit_names.lock().unwrap().is_empty());
}

#[tokio::test]
async fn graceful_quit_refuses_a_changed_app_name() {
    let harness = GracefulHarness::new();
    let mut drifted = installed_identity("Demo");
    drifted.app_name = "Demo Renamed".to_owned();
    let quitter = RecordingAppQuitter::new(vec![drifted], Vec::new());

    let error = harness
        .run(&quitter, &RecordingHistory::default())
        .await
        .expect_err("应用名变化必须拒绝");

    assert!(error.contains("名称已变化"), "{error}");
    assert!(quitter.quit_names.lock().unwrap().is_empty());
}

#[tokio::test]
async fn graceful_quit_refuses_a_missing_app() {
    let harness = GracefulHarness::new();
    let quitter = RecordingAppQuitter::new(Vec::new(), Vec::new());

    let error = harness
        .run(&quitter, &RecordingHistory::default())
        .await
        .expect_err("应用消失必须拒绝");

    assert!(error.contains("已不存在"), "{error}");
    assert!(quitter.quit_names.lock().unwrap().is_empty());
}

#[tokio::test]
async fn graceful_quit_reports_an_applescript_failure_per_app() {
    let harness = GracefulHarness::new();
    let quitter = RecordingAppQuitter::new(
        vec![installed_identity("Demo")],
        vec![("Demo", UserError::from("应用没有响应退出信号"))],
    );

    let result = harness
        .run(&quitter, &RecordingHistory::default())
        .await
        .expect("单个应用失败仍要返回报告");

    let OperationResult::AppGracefulQuit(reports) = result else {
        panic!("期望优雅退出结果");
    };
    assert_eq!(reports.len(), 1);
    assert_eq!(
        reports[0].quit_error.as_deref(),
        Some("应用没有响应退出信号")
    );
    assert_eq!(quitter.quit_names.lock().unwrap().as_slice(), ["Demo"]);
}

#[tokio::test]
async fn graceful_quit_history_marks_failure_and_hides_the_operation_id() {
    let harness = GracefulHarness::new();
    let quitter = RecordingAppQuitter::new(
        vec![installed_identity("Demo")],
        vec![("Demo", UserError::from("应用没有响应退出信号"))],
    );
    let history = RecordingHistory::default();

    harness.run(&quitter, &history).await.expect("返回报告");

    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].operation, "app_graceful_quit");
    assert_eq!(entries[0].target, "1 个应用");
    assert_eq!(entries[0].freed_bytes, 0);
    assert!(!entries[0].success);
    assert!(!entries[0].detail.contains(&harness.operation_id));
    assert!(!entries[0].target.contains(&harness.operation_id));
}

#[tokio::test]
async fn graceful_quit_history_marks_success_when_every_app_quit() {
    let harness = GracefulHarness::new();
    let quitter = RecordingAppQuitter::new(vec![installed_identity("Demo")], Vec::new());
    let history = RecordingHistory::default();

    harness.run(&quitter, &history).await.expect("返回报告");

    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].operation, "app_graceful_quit");
    assert!(entries[0].success);
}
