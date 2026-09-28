use super::*;
use crate::cache_scanner::{CacheCategory, Safety};
use crate::operations::{
    CacheAction, DockerResourceKind, DockerTarget, InstalledAppIdentity, OperationStore,
    ProcessIdentity, ProcessTarget, ResidueIdentity,
};
use crate::process_ops::{KillOutcome, LiveProcess, ProcessObserver, ProcessSignaller};
use crate::user_error::UserError;
use std::sync::{Arc, Mutex};

struct CountingAppQuitter {
    quits: Arc<Mutex<Vec<String>>>,
}

impl crate::operation_executor::AppQuitter for CountingAppQuitter {
    fn observe_apps(
        &self,
        _bundle_paths: &[String],
    ) -> crate::operation_executor::DomainFuture<'_, Result<Vec<InstalledAppIdentity>, UserError>>
    {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn quit(
        &self,
        app: &InstalledAppIdentity,
    ) -> crate::operation_executor::DomainFuture<'_, Result<(), UserError>> {
        self.quits.lock().unwrap().push(app.app_name.clone());
        Box::pin(async { Ok(()) })
    }
}

struct CountingCache {
    calls: Arc<Mutex<usize>>,
}

impl CacheCleaner for CountingCache {
    fn clean<'a>(
        &'a self,
        items: Vec<CacheItem>,
    ) -> Pin<Box<dyn Future<Output = CleanSummary> + Send + 'a>> {
        *self.calls.lock().unwrap() += 1;
        Box::pin(async move {
            CleanSummary {
                reports: Vec::new(),
                total_freed_bytes: 0,
                success_count: items.len(),
                fail_count: 0,
            }
        })
    }
}

struct CountingUninstall {
    calls: Arc<Mutex<usize>>,
}

impl UninstallDomain for CountingUninstall {
    fn observe_apps(
        &self,
        _bundle_paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<InstalledAppIdentity>, UserError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn observe_residues(
        &self,
        paths: &[String],
    ) -> DomainFuture<'_, Result<Vec<LiveResidue>, UserError>> {
        let observed = paths
            .iter()
            .map(|path| LiveResidue {
                path: path.clone(),
                exists: true,
            })
            .collect();
        Box::pin(async move { Ok(observed) })
    }

    fn quit(&self, _app_name: &str) -> DomainFuture<'_, Result<(), UserError>> {
        Box::pin(async { Ok(()) })
    }

    fn remove(
        &self,
        _app: &InstalledAppIdentity,
        _residues: &[ResidueIdentity],
    ) -> DomainFuture<'_, UninstallReport> {
        *self.calls.lock().unwrap() += 1;
        Box::pin(async {
            UninstallReport {
                app_name: "Alpha".to_owned(),
                bundle_id: "com.example.Alpha".to_owned(),
                total_freed_bytes: 0,
                moved_count: 0,
                failed_count: 0,
                details: Vec::new(),
                quit_error: None,
            }
        })
    }
}

struct CountingDocker {
    calls: Arc<Mutex<usize>>,
    inventory: DockerInventory,
}

impl DockerDomain for CountingDocker {
    fn inventory(&self) -> DomainFuture<'_, Result<DockerInventory, UserError>> {
        Box::pin(async move {
            Ok(crate::docker::build_inventory(
                self.inventory.daemon_running,
                self.inventory.images.clone(),
                self.inventory.containers.clone(),
                self.inventory.volumes.clone(),
                self.inventory.builder.clone(),
            ))
        })
    }

    fn remove(
        &self,
        _action: DockerAction,
        _target: &DockerTarget,
    ) -> DomainFuture<'_, Result<(), UserError>> {
        *self.calls.lock().unwrap() += 1;
        Box::pin(async { Ok(()) })
    }

    fn prune(&self) -> DomainFuture<'_, Result<String, UserError>> {
        *self.calls.lock().unwrap() += 1;
        Box::pin(async { Ok(String::new()) })
    }
}

struct SilentObserver {
    live: Vec<LiveProcess>,
}

impl ProcessObserver for SilentObserver {
    fn observe(&mut self, _pids: &[u32]) -> Result<Vec<LiveProcess>, UserError> {
        Ok(self.live.clone())
    }
}

struct SilentSignaller;

impl ProcessSignaller for SilentSignaller {
    fn terminate(&mut self, _target: &ProcessTarget, _mode: ProcessMode) -> KillOutcome {
        KillOutcome::Success
    }
}

fn cache_item() -> CacheItem {
    CacheItem {
        id: "npm".to_owned(),
        category: CacheCategory::Npm,
        label_key: "cache.item.npmCache".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.npmCache".into(),
        description_params: Vec::new(),
        path: None,
        size_bytes: 1,
        safety: Safety::Safe,
        default_select: true,
        action: CacheAction::Npm,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: String::new(),
    }
}

fn cache_plan(store: &mut OperationStore) -> ConsumedPlan {
    let registration = store.register_cache_snapshot(vec![cache_item()]).unwrap();
    let prepared = store
        .prepare_cache(
            &registration.snapshot_id,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap();
    store.consume(&prepared.operation_id, "main").unwrap()
}

fn process_plan(store: &mut OperationStore) -> ConsumedPlan {
    let registration = store
        .register_process_snapshot(vec![ProcessTarget {
            identity: ProcessIdentity {
                pid: 77,
                name: "sleepy".to_owned(),
                exe: "/usr/local/bin/sleepy".to_owned(),
                start_time: 5,
            },
            protected: false,
            whitelisted: false,
        }])
        .unwrap();
    let prepared = store
        .prepare_process(
            &registration.snapshot_id,
            vec![&registration.selection_keys[0]],
            ProcessMode::Graceful,
            "main",
        )
        .unwrap();
    store.consume(&prepared.operation_id, "main").unwrap()
}

fn docker_plan(store: &mut OperationStore) -> ConsumedPlan {
    let registration = store
        .register_docker_resources(vec![DockerTarget {
            resource_type: DockerResourceKind::Image,
            id: "sha256-image".to_owned(),
            name: "nginx:latest".to_owned(),
            size_bytes: 1,
            referenced: false,
            reclaimable: false,
        }])
        .unwrap();
    let prepared = store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::RemoveImage,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap();
    store.consume(&prepared.operation_id, "main").unwrap()
}

#[test]
fn consumed_plan_reports_its_kind_before_dispatch() {
    let mut store = OperationStore::new();

    assert_eq!(cache_plan(&mut store).kind(), OperationKind::Cache);
    assert_eq!(process_plan(&mut store).kind(), OperationKind::Process);
    assert_eq!(docker_plan(&mut store).kind(), OperationKind::Docker);
}

#[tokio::test]
async fn domain_plan_rejects_termination_plans() {
    let mut store = OperationStore::new();
    let plan = process_plan(&mut store);
    let cache = CountingCache {
        calls: Arc::new(Mutex::new(0)),
    };
    let uninstall = CountingUninstall {
        calls: Arc::new(Mutex::new(0)),
    };
    let docker = CountingDocker {
        calls: Arc::new(Mutex::new(0)),
        inventory: crate::docker::build_inventory(
            true,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            crate::docker::DockerBuilderCache {
                total_bytes: 0,
                reclaimable_bytes: 0,
            },
        ),
    };
    let app_quit = CountingAppQuitter {
        quits: Arc::new(Mutex::new(Vec::new())),
    };
    let domains = DomainServices {
        cache: &cache,
        uninstall: &uninstall,
        app_quit: &app_quit,
        docker: &docker,
    };

    let outcome = execute_domain_plan(plan, &domains).await;

    assert!(outcome.is_err());
    assert_eq!(*cache.calls.lock().unwrap(), 0);
    assert_eq!(*uninstall.calls.lock().unwrap(), 0);
    assert_eq!(*docker.calls.lock().unwrap(), 0);
}

#[test]
fn termination_plan_rejects_domain_plans() {
    let mut store = OperationStore::new();
    let plan = cache_plan(&mut store);
    let mut observer = SilentObserver { live: Vec::new() };
    let mut signaller = SilentSignaller;
    let mut processes = ProcessServices {
        observer: &mut observer,
        signaller: &mut signaller,
    };

    let outcome = execute_termination_plan(plan, &mut processes);

    assert!(outcome.is_err());
}

const ALPHA_EXE: &str = "/Applications/Alpha.app/Contents/MacOS/Alpha";

fn alpha_process_target() -> ProcessTarget {
    ProcessTarget {
        identity: ProcessIdentity {
            pid: 88,
            name: "Alpha".to_owned(),
            exe: ALPHA_EXE.to_owned(),
            start_time: 9,
        },
        protected: false,
        whitelisted: false,
    }
}

fn alpha_live_process() -> LiveProcess {
    LiveProcess {
        identity: ProcessIdentity {
            pid: 88,
            name: "Alpha".to_owned(),
            exe: ALPHA_EXE.to_owned(),
            start_time: 9,
        },
        protected: false,
        whitelisted: false,
    }
}

#[test]
fn termination_plan_reports_the_app_terminate_outcome() {
    let mut store = OperationStore::new();
    let registration = store
        .register_application_snapshot(vec![AppIdentity {
            bundle_path: "/Applications/Alpha.app".to_owned(),
            bundle_id: "com.example.Alpha".to_owned(),
            app_name: "Alpha".to_owned(),
            processes: vec![alpha_process_target()],
        }])
        .unwrap();
    let prepared = store
        .prepare_app_termination(
            &registration.snapshot_id,
            vec![&registration.app_keys[0]],
            ProcessMode::Force,
            "main",
        )
        .unwrap();
    let plan = store.consume(&prepared.operation_id, "main").unwrap();
    let mut observer = SilentObserver {
        live: vec![alpha_live_process()],
    };
    let mut signaller = SilentSignaller;
    let mut processes = ProcessServices {
        observer: &mut observer,
        signaller: &mut signaller,
    };

    let outcome = execute_termination_plan(plan, &mut processes).expect("应用终止计划必须可执行");

    assert_eq!(outcome.kind(), OperationKind::AppTerminate);
    match outcome {
        OperationOutcome::AppTerminate(report) => assert_eq!(report.killed, vec![88]),
        other => panic!("期望应用终止结果，得到 {other:?}"),
    }
}

#[tokio::test]
async fn domain_plan_reports_the_docker_outcome() {
    let mut store = OperationStore::new();
    let plan = docker_plan(&mut store);
    let cache = CountingCache {
        calls: Arc::new(Mutex::new(0)),
    };
    let uninstall = CountingUninstall {
        calls: Arc::new(Mutex::new(0)),
    };
    let docker = CountingDocker {
        calls: Arc::new(Mutex::new(0)),
        inventory: crate::docker::build_inventory(
            true,
            vec![crate::docker::DockerImage {
                id: "sha256-image".to_owned(),
                repository: "nginx".to_owned(),
                tag: "latest".to_owned(),
                size_bytes: 1,
                created: "2026-01-15".to_owned(),
                dangling: false,
                in_use: false,
                selection_key: String::new(),
            }],
            Vec::new(),
            Vec::new(),
            crate::docker::DockerBuilderCache {
                total_bytes: 0,
                reclaimable_bytes: 0,
            },
        ),
    };
    let app_quit = CountingAppQuitter {
        quits: Arc::new(Mutex::new(Vec::new())),
    };
    let domains = DomainServices {
        cache: &cache,
        uninstall: &uninstall,
        app_quit: &app_quit,
        docker: &docker,
    };

    let outcome = execute_domain_plan(plan, &domains)
        .await
        .expect("Docker 计划");

    assert_eq!(outcome.kind(), OperationKind::Docker);
    assert_eq!(*docker.calls.lock().unwrap(), 1);
    assert_eq!(*cache.calls.lock().unwrap(), 0);
    assert_eq!(*uninstall.calls.lock().unwrap(), 0);
}
