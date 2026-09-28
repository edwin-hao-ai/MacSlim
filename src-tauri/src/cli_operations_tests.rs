use super::{
    clean_cache_in_session, clean_processes_in_session, clean_processes_with, clean_snapshot_with,
    CacheScope, CliHistory, CliWhitelist, UnreadableStorage, CLI_OWNER,
};
use crate::cache_cleaner::{CleanReport, CleanSummary};
use crate::cache_scanner::{CacheCategory, CacheItem, CacheScanResult, Safety};
use crate::operation_commands::{HistorySink, OperationHistoryEntry, ProcessWhitelist};
use crate::operation_executor::CacheCleaner;
use crate::operations::{CacheAction, OperationStore, ProcessIdentity, ProcessMode, ProcessTarget};
use crate::process_ops::{KillOutcome, LiveProcess, ProcessObserver, ProcessSignaller};
use crate::scanner::{ProcessInfo, ProcessKind, Risk, ScanResult, SystemHealth};
use crate::user_error::UserError;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

fn cache_item(id: &str, category: CacheCategory, default_select: bool) -> CacheItem {
    CacheItem {
        id: id.to_owned(),
        category,
        label_key: "cache.item.npmCache".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.npmCache".into(),
        description_params: Vec::new(),
        path: Some(format!("/tmp/{id}")),
        size_bytes: 128,
        safety: Safety::Safe,
        default_select,
        action: CacheAction::Npm,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: String::new(),
    }
}

fn cache_scan() -> CacheScanResult {
    CacheScanResult {
        items: vec![
            cache_item("npm", CacheCategory::Npm, true),
            cache_item("yarn", CacheCategory::Yarn, true),
            cache_item("xcode", CacheCategory::Xcode, false),
            cache_item("brew", CacheCategory::Homebrew, false),
        ],
        total_bytes: 512,
        scanned_at_ms: 1,
    }
}

fn health() -> SystemHealth {
    SystemHealth {
        cpu_percent: 1.0,
        memory_used_mb: 1.0,
        memory_total_mb: 2.0,
        memory_percent: 50.0,
        disk_used_gb: 1.0,
        disk_total_gb: 2.0,
        disk_percent: 50.0,
    }
}

fn clean_summary(cleaned: usize) -> CleanSummary {
    CleanSummary {
        reports: (0..cleaned)
            .map(|index| CleanReport {
                id: format!("item-{index}"),
                label_key: "cache.item.npmCache".into(),
                label_params: Vec::new(),
                success: true,
                freed_bytes: 128,
                duration_ms: 1,
                error: None,
            })
            .collect(),
        total_freed_bytes: 128 * cleaned as u64,
        success_count: cleaned,
        fail_count: 0,
    }
}

#[derive(Default)]
struct RecordingCleaner {
    batches: Mutex<Vec<Vec<String>>>,
}

impl CacheCleaner for RecordingCleaner {
    fn clean<'a>(
        &'a self,
        items: Vec<CacheItem>,
    ) -> Pin<Box<dyn Future<Output = CleanSummary> + Send + 'a>> {
        let summary = clean_summary(items.len());
        self.batches
            .lock()
            .unwrap()
            .push(items.iter().map(|item| item.id.clone()).collect());
        Box::pin(async move { summary })
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

struct NamedWhitelist;

impl ProcessWhitelist for NamedWhitelist {
    fn is_process_whitelisted(&self, name: &str) -> bool {
        name == "whitelisted"
    }
}

struct FixedObserver {
    live: Vec<LiveProcess>,
    observed: Mutex<usize>,
}

impl ProcessObserver for FixedObserver {
    fn observe(&mut self, pids: &[u32]) -> Result<Vec<LiveProcess>, UserError> {
        *self.observed.lock().unwrap() += 1;
        Ok(self
            .live
            .iter()
            .filter(|process| pids.contains(&process.identity.pid))
            .cloned()
            .collect())
    }
}

#[derive(Default)]
struct RecordingSignaller {
    terminated: Mutex<Vec<(u32, ProcessMode)>>,
}

impl ProcessSignaller for RecordingSignaller {
    fn terminate(&mut self, target: &ProcessTarget, mode: ProcessMode) -> KillOutcome {
        self.terminated
            .lock()
            .unwrap()
            .push((target.identity.pid, mode));
        KillOutcome::Success
    }
}

fn process_info(pid: u32, name: &str, default_select: bool, whitelisted: bool) -> ProcessInfo {
    ProcessInfo {
        pid,
        name: name.to_owned(),
        exe: format!("/usr/bin/{name}"),
        start_time: 100 + u64::from(pid),
        cpu_percent: 1.0,
        memory_mb: 10.0,
        kind: ProcessKind::Zombie,
        risk: Risk::Safe,
        default_select,
        reason_key: "process.reason.idle".to_owned(),
        reason_params: Vec::new(),
        ports: Vec::new(),
        icon_base64: None,
        selection_key: String::new(),
        protected: whitelisted,
        protected_reason_key: whitelisted.then(|| "process.protect.whitelisted".to_owned()),
        protected_reason_params: Vec::new(),
        whitelisted,
    }
}

fn process_scan() -> ScanResult {
    ScanResult {
        health: health(),
        processes: vec![
            process_info(11, "zombie", true, false),
            process_info(12, "hog", true, false),
            process_info(13, "dev", false, false),
            process_info(14, "whitelisted", false, true),
        ],
        scanned_at_ms: 1,
    }
}

fn live_for(result: &ScanResult) -> Vec<LiveProcess> {
    result
        .processes
        .iter()
        .map(|info| LiveProcess {
            identity: ProcessIdentity {
                pid: info.pid,
                name: info.name.clone(),
                exe: info.exe.clone(),
                start_time: info.start_time,
            },
            protected: info.protected,
            whitelisted: info.whitelisted,
        })
        .collect()
}

#[tokio::test]
async fn cache_clean_registers_a_snapshot_and_cleans_only_default_safe_items() {
    let cleaner = RecordingCleaner::default();
    let history = RecordingHistory::default();

    let outcome = clean_snapshot_with(cache_scan(), CacheScope::All, &cleaner, &history)
        .await
        .expect("默认安全项清理");

    assert_eq!(outcome.default_safe_count, 2);
    assert_eq!(
        cleaner.batches.lock().unwrap().as_slice(),
        [vec!["npm".to_owned(), "yarn".to_owned()]]
    );
    assert_eq!(outcome.scan.items.len(), 4);
    assert_eq!(outcome.scan.total_bytes, 512);
    assert_eq!(outcome.summary.success_count, 2);
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].operation, "cache");
    assert_eq!(entries[0].target, "2 项缓存");
}

#[tokio::test]
async fn cache_clean_scope_narrows_the_snapshot_instead_of_the_command_line() {
    let cleaner = RecordingCleaner::default();
    let history = RecordingHistory::default();

    let outcome = clean_snapshot_with(cache_scan(), CacheScope::NodeFamily, &cleaner, &history)
        .await
        .expect("Node 生态清理");

    assert_eq!(outcome.default_safe_count, 2);
    assert_eq!(
        cleaner.batches.lock().unwrap().as_slice(),
        [vec!["npm".to_owned(), "yarn".to_owned()]]
    );
    assert_eq!(outcome.scan.items.len(), 2);
}

#[tokio::test]
async fn cache_clean_skips_scopes_without_default_safe_items() {
    let cleaner = RecordingCleaner::default();
    let history = RecordingHistory::default();

    let outcome = clean_snapshot_with(cache_scan(), CacheScope::Xcode, &cleaner, &history)
        .await
        .expect("Xcode 清理");

    assert_eq!(outcome.default_safe_count, 0);
    assert_eq!(outcome.summary.success_count, 0);
    assert_eq!(outcome.summary.total_freed_bytes, 0);
    assert!(outcome.summary.reports.is_empty());
    assert!(cleaner.batches.lock().unwrap().is_empty());
    assert!(history.entries.lock().unwrap().is_empty());
    assert_eq!(outcome.scan.items.len(), 1);
}

#[tokio::test]
async fn process_clean_terminates_only_default_safe_rows_through_the_broker() {
    let scan = process_scan();
    let mut observer = FixedObserver {
        live: live_for(&scan),
        observed: Mutex::new(0),
    };
    let mut signaller = RecordingSignaller::default();
    let history = RecordingHistory::default();

    let outcome = clean_processes_with(
        scan,
        Arc::new(NamedWhitelist),
        &mut observer,
        &mut signaller,
        &history,
    )
    .await
    .expect("默认安全进程终止");

    assert_eq!(outcome.default_safe_count, 2);
    assert_eq!(outcome.scan.items.len(), 3);
    assert_eq!(*observer.observed.lock().unwrap(), 1);
    let mut terminated = signaller.terminated.lock().unwrap().clone();
    terminated.sort_by_key(|(pid, _)| *pid);
    assert_eq!(
        terminated,
        vec![(11, ProcessMode::Graceful), (12, ProcessMode::Graceful)]
    );
    assert_eq!(outcome.report.killed, vec![11, 12]);
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].operation, "process");
    assert_eq!(entries[0].target, "2 个进程");
}

#[tokio::test]
async fn process_clean_refuses_targets_that_vanished_after_the_scan() {
    let scan = process_scan();
    let mut observer = FixedObserver {
        live: Vec::new(),
        observed: Mutex::new(0),
    };
    let mut signaller = RecordingSignaller::default();
    let history = RecordingHistory::default();

    let error = clean_processes_with(
        scan,
        Arc::new(NamedWhitelist),
        &mut observer,
        &mut signaller,
        &history,
    )
    .await
    .expect_err("目标消失必须拒绝");

    assert!(error.contains("请重新扫描"), "{error}");
    assert!(signaller.terminated.lock().unwrap().is_empty());
    let entries = history.entries.lock().unwrap().clone();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].success);
}

#[tokio::test]
async fn process_clean_drops_whitelisted_rows_before_they_can_be_planned() {
    let scan = process_scan();
    let mut observer = FixedObserver {
        live: live_for(&scan),
        observed: Mutex::new(0),
    };
    let mut signaller = RecordingSignaller::default();
    let history = RecordingHistory::default();

    let outcome = clean_processes_with(
        scan,
        Arc::new(NamedWhitelist),
        &mut observer,
        &mut signaller,
        &history,
    )
    .await
    .expect("白名单行被过滤");

    assert_eq!(outcome.scan.items.len(), 3);
    assert!(outcome.scan.items.iter().all(|item| !item.whitelisted));
    let mut terminated = signaller.terminated.lock().unwrap().clone();
    terminated.sort_by_key(|(pid, _)| *pid);
    assert_eq!(
        terminated,
        vec![(11, ProcessMode::Graceful), (12, ProcessMode::Graceful)]
    );
}

#[test]
fn cli_owner_is_fixed_and_enforced_by_the_store() {
    assert_eq!(CLI_OWNER, "cli");

    let mut store = OperationStore::new();
    let registration = store
        .register_cache_snapshot(vec![cache_item("npm", CacheCategory::Npm, true)])
        .expect("注册缓存快照");
    let prepared = store
        .prepare_cache(
            &registration.snapshot_id,
            &registration.selection_keys,
            CLI_OWNER,
        )
        .expect("以 cli 身份准备");

    let error = store
        .consume(&prepared.operation_id, "somebody-else")
        .expect_err("其他 owner 不能消费");
    assert_eq!(error, "操作所有者不匹配");

    let plan = store
        .consume(&prepared.operation_id, CLI_OWNER)
        .expect("cli 可以消费");
    assert_eq!(plan.kind().label(), "cache");
}

#[test]
fn prepared_summary_never_leaks_the_operation_identifier() {
    let mut store = OperationStore::new();
    let registration = store
        .register_cache_snapshot(vec![cache_item("npm", CacheCategory::Npm, true)])
        .expect("注册缓存快照");
    let prepared = store
        .prepare_cache(
            &registration.snapshot_id,
            &registration.selection_keys,
            CLI_OWNER,
        )
        .expect("准备操作");

    assert_eq!(prepared.kind, "cache");
    assert_eq!(prepared.item_count, 1);
    assert_eq!(prepared.summary_key, "opSummary.cache");
    // 摘要现在是 key + 参数：operation_id 根本装不进去，这条断言恒成立，
    // 但它守的是「别把 operation_id 塞进 summary_params」这个新漏法。
    assert!(!prepared.summary_key.contains(&prepared.operation_id));
    assert!(
        prepared
            .summary_params
            .iter()
            .all(|(name, value)| !name.contains(&prepared.operation_id)
                && !value.contains(&prepared.operation_id)),
        "摘要参数里泄漏了 operation_id：{:?}",
        prepared.summary_params
    );
}

#[tokio::test]
async fn empty_scan_produces_no_destructive_work() {
    let cleaner = RecordingCleaner::default();
    let history = RecordingHistory::default();
    let scan = CacheScanResult {
        items: Vec::new(),
        total_bytes: 0,
        scanned_at_ms: 1,
    };

    let outcome = clean_snapshot_with(scan, CacheScope::All, &cleaner, &history)
        .await
        .expect("空扫描不报错");

    assert_eq!(outcome.default_safe_count, 0);
    assert!(cleaner.batches.lock().unwrap().is_empty());
    assert!(history.entries.lock().unwrap().is_empty());
}

#[test]
fn cache_scope_covers_every_documented_cli_subcommand() {
    let all = vec![
        cache_item("npm", CacheCategory::Npm, true),
        cache_item("yarn", CacheCategory::Yarn, true),
        cache_item("xcode", CacheCategory::Xcode, true),
        cache_item("brew", CacheCategory::Homebrew, true),
        cache_item("docker", CacheCategory::Docker, true),
    ];
    for (scope, expected) in [
        (CacheScope::All, 5),
        (CacheScope::NodeFamily, 2),
        (CacheScope::Xcode, 1),
        (CacheScope::Category(CacheCategory::Docker), 1),
    ] {
        let mut scan = CacheScanResult {
            items: all.clone(),
            total_bytes: 0,
            scanned_at_ms: 1,
        };
        let matched = scan.items.iter().filter(|item| scope.matches(item)).count();
        assert_eq!(matched, expected, "{scope:?}");
        scan.items.retain(|item| scope.matches(item));
        assert_eq!(scan.items.len(), expected);
    }
}

fn history_entry(operation: &str) -> OperationHistoryEntry {
    OperationHistoryEntry {
        operation: operation.to_owned(),
        target: "1 项".to_owned(),
        freed_bytes: 0,
        success: true,
        detail: String::new(),
    }
}

#[test]
fn missing_history_refuses_to_record_instead_of_pretending_success() {
    let history = CliHistory::without_storage();
    let error = history
        .record(&history_entry("cache"))
        .expect_err("没有存储时不能假装写入成功");

    assert!(error.contains("历史"), "{error}");
}

#[test]
fn missing_whitelist_denies_every_process_on_the_cleaning_path() {
    let whitelist = CliWhitelist::unreadable(UnreadableStorage::Deny);
    for name in ["Chrome", "Finder", "任意进程"] {
        assert!(
            whitelist.is_process_whitelisted(name),
            "{name} 必须被拒绝而不是放行"
        );
    }
}

#[test]
fn missing_whitelist_degrades_to_builtin_rules_on_the_scan_only_path() {
    let whitelist = CliWhitelist::unreadable(UnreadableStorage::Degrade);
    assert!(!whitelist.is_process_whitelisted("Chrome"));
}

#[tokio::test]
async fn storage_failure_stops_cache_clean_before_any_destructive_call() {
    let cleaner = RecordingCleaner::default();
    let error = clean_cache_in_session(
        cache_scan(),
        CacheScope::All,
        Err(UserError::from("数据库被锁定")),
        &cleaner,
    )
    .await
    .expect_err("存储不可用时必须拒绝清理");

    assert!(error.contains("无法打开本地存储"), "{error}");
    assert!(error.contains("数据库被锁定"), "{error}");
    assert!(cleaner.batches.lock().unwrap().is_empty());
}

#[tokio::test]
async fn storage_failure_stops_process_clean_before_any_signal() {
    let scan = process_scan();
    let observer = FixedObserver {
        live: live_for(&scan),
        observed: Mutex::new(0),
    };
    let signaller = RecordingSignaller::default();

    let built = Mutex::new(false);
    let observed_calls = Arc::new(Mutex::new(0usize));
    let mut signaller_guard = signaller;
    let error = clean_processes_in_session(
        scan,
        Err(UserError::from("数据库被锁定")),
        |_| {
            *built.lock().unwrap() = true;
            *observed_calls.lock().unwrap() += 1;
            observer
        },
        &mut signaller_guard,
    )
    .await
    .expect_err("存储不可用时必须拒绝终止");

    assert!(error.contains("无法打开本地存储"), "{error}");
    assert!(!*built.lock().unwrap(), "存储不可用时不得构造进程观测器");
    assert_eq!(*observed_calls.lock().unwrap(), 0);
    assert!(signaller_guard.terminated.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cli_history_refuses_to_record_when_storage_is_missing() {
    let history = CliHistory::without_storage();
    let error = history
        .record(&history_entry("process"))
        .expect_err("缺少存储必须显式失败");

    assert!(error.contains("历史"), "{error}");
}
