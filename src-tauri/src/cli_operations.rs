use crate::applications::SystemAppQuitter;
use crate::cache_cleaner::CleanSummary;
use crate::cache_scanner::{self, CacheCategory, CacheItem, CacheScanResult};
use crate::operation_commands::{
    execute_operation_with, prepare_operation_with, process_whitelist_policy, snapshot_cache,
    snapshot_process_scan, system_history, HistorySink, OperationHistoryEntry, OperationResult,
    PrepareOperationRequest, ProcessWhitelist,
};
use crate::operation_executor::{
    execute_domain_plan, execute_termination_plan, system_domain_services, CacheCleaner,
    ProcessServices, SystemCacheCleaner,
};
use crate::operations::{OperationStore, ProcessKillReport, ProcessMode};
use crate::process_ops::{
    ProcessObserver, ProcessSignaller, SystemProcessObserver, SystemProcessSignaller,
};
use crate::scanner::{ProcessInfo, ScanResult};
use crate::storage::Storage;
use crate::uninstaller::SystemUninstaller;
use crate::user_error::{ErrorCode, UserError};
use std::sync::{Arc, Mutex};

pub const CLI_OWNER: &str = "cli";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CacheScope {
    All,
    NodeFamily,
    Xcode,
    Category(CacheCategory),
}

impl CacheScope {
    pub fn matches(&self, item: &CacheItem) -> bool {
        match self {
            Self::All => true,
            Self::NodeFamily => {
                matches!(
                    item.category,
                    CacheCategory::Npm | CacheCategory::Pnpm | CacheCategory::Yarn
                ) || item.id == "stale-node-modules"
            }
            Self::Xcode => item.category == CacheCategory::Xcode,
            Self::Category(category) => item.category == *category,
        }
    }
}

#[derive(Debug)]
pub struct CacheScanView {
    pub items: Vec<CacheItem>,
    pub total_bytes: u64,
}

#[derive(Debug)]
pub struct CacheCleanOutcome {
    pub scan: CacheScanView,
    pub default_safe_count: usize,
    pub summary: CleanSummary,
}

#[derive(Debug)]
pub struct ProcessScanView {
    pub items: Vec<ProcessInfo>,
}

#[derive(Debug)]
pub struct ProcessCleanOutcome {
    pub scan: ProcessScanView,
    pub default_safe_count: usize,
    pub report: ProcessKillReport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnreadableStorage {
    Deny,
    Degrade,
}

struct CliWhitelist {
    storage: Option<Arc<Storage>>,
    unreadable: UnreadableStorage,
}

impl CliWhitelist {
    pub(crate) fn unreadable(unreadable: UnreadableStorage) -> Self {
        Self {
            storage: None,
            unreadable,
        }
    }

    fn with_storage(storage: Arc<Storage>) -> Self {
        Self {
            storage: Some(storage),
            unreadable: UnreadableStorage::Deny,
        }
    }
}

impl ProcessWhitelist for CliWhitelist {
    fn is_process_whitelisted(&self, name: &str) -> bool {
        match &self.storage {
            Some(storage) => storage.is_whitelisted("process", name),
            None => self.unreadable == UnreadableStorage::Deny,
        }
    }
}

struct CliHistory(Option<Arc<Storage>>);

impl CliHistory {
    #[cfg(test)]
    pub(crate) fn without_storage() -> Self {
        Self(None)
    }
}

impl HistorySink for CliHistory {
    fn record(&self, entry: &OperationHistoryEntry) -> Result<(), UserError> {
        let Some(storage) = &self.0 else {
            return Err(UserError::with(
                ErrorCode::HISTORY_STORAGE_UNAVAILABLE,
                format!(
                    "本地存储不可用，无法写入历史记录（{} · {}）：操作结果未被审计",
                    entry.operation, entry.target
                ),
                vec![
                    ("operation".to_owned(), entry.operation.clone()),
                    ("target".to_owned(), entry.target.clone()),
                ],
            ));
        };
        system_history(storage).record(entry)
    }
}

pub(crate) struct CliSession {
    storage: Arc<Storage>,
}

impl CliSession {
    pub(crate) fn open(storage: Result<Storage, UserError>) -> Result<Self, UserError> {
        let storage = storage
            .map_err(|error| format!("无法打开本地存储，CLI 拒绝执行破坏性操作: {error}"))?;
        Ok(Self {
            storage: Arc::new(storage),
        })
    }

    fn whitelist(&self) -> Arc<CliWhitelist> {
        Arc::new(CliWhitelist::with_storage(Arc::clone(&self.storage)))
    }

    fn history(&self) -> CliHistory {
        CliHistory(Some(Arc::clone(&self.storage)))
    }
}

pub async fn scan_cache(scope: CacheScope) -> Result<CacheScanView, UserError> {
    Ok(cache_view(scope_scan(
        cache_scanner::scan(None).await,
        scope,
    )))
}

pub async fn run_default_cache_clean(scope: CacheScope) -> Result<CacheCleanOutcome, UserError> {
    clean_cache_in_session(
        cache_scanner::scan(None).await,
        scope,
        Storage::open(),
        &SystemCacheCleaner,
    )
    .await
}

pub(crate) async fn clean_cache_in_session<C: CacheCleaner + ?Sized>(
    scan: CacheScanResult,
    scope: CacheScope,
    storage: Result<Storage, UserError>,
    cleaner: &C,
) -> Result<CacheCleanOutcome, UserError> {
    let session = CliSession::open(storage)?;
    clean_snapshot_with(scan, scope, cleaner, &session.history()).await
}

pub async fn scan_processes() -> Result<ProcessScanView, UserError> {
    let mut system = sysinfo::System::new_all();
    let result = crate::scanner::scan(&mut system);
    let whitelist = Arc::new(match Storage::open() {
        Ok(storage) => CliWhitelist::with_storage(Arc::new(storage)),
        Err(_) => CliWhitelist::unreadable(UnreadableStorage::Degrade),
    });
    Ok(process_view(result, &process_whitelist_policy(whitelist)))
}

pub async fn run_default_process_clean() -> Result<ProcessCleanOutcome, UserError> {
    let mut system = sysinfo::System::new_all();
    let result = crate::scanner::scan(&mut system);
    clean_processes_in_session(
        result,
        Storage::open(),
        system_observer,
        &mut SystemProcessSignaller,
    )
    .await
}

fn system_observer(storage: Arc<Storage>) -> SystemProcessObserver {
    SystemProcessObserver::with_policy(process_whitelist_policy(Arc::new(
        CliWhitelist::with_storage(storage),
    )))
}

pub(crate) async fn clean_processes_in_session<O, S>(
    scan: ScanResult,
    storage: Result<Storage, UserError>,
    build_observer: impl FnOnce(Arc<Storage>) -> O,
    signaller: &mut S,
) -> Result<ProcessCleanOutcome, UserError>
where
    O: ProcessObserver + Send,
    S: ProcessSignaller + Send,
{
    let session = CliSession::open(storage)?;
    let whitelist = session.whitelist();
    let mut observer = build_observer(Arc::clone(&session.storage));
    clean_processes_with(
        scan,
        whitelist,
        &mut observer,
        signaller,
        &session.history(),
    )
    .await
}

fn scope_scan(result: CacheScanResult, scope: CacheScope) -> CacheScanResult {
    let mut result = result;
    result.items.retain(|item| scope.matches(item));
    result.total_bytes = result.items.iter().map(|item| item.size_bytes).sum();
    result
}

fn cache_view(result: CacheScanResult) -> CacheScanView {
    CacheScanView {
        total_bytes: result.total_bytes,
        items: result.items,
    }
}

fn process_view<P>(mut result: ScanResult, policy: &P) -> ProcessScanView
where
    P: Fn(&str) -> bool + ?Sized,
{
    result.processes.retain(|info| !policy(&info.name));
    ProcessScanView {
        items: result.processes,
    }
}

pub(crate) async fn clean_snapshot_with<C, H>(
    scan: CacheScanResult,
    scope: CacheScope,
    cleaner: &C,
    history: &H,
) -> Result<CacheCleanOutcome, UserError>
where
    C: CacheCleaner + ?Sized,
    H: HistorySink,
{
    let scoped = scope_scan(scan, scope);
    let view = cache_view(scoped.clone());
    let mut store = OperationStore::new();
    let snapshot = snapshot_cache(&mut store, scoped)?;
    let default_keys: Vec<String> = snapshot
        .value
        .items
        .iter()
        .filter(|item| item.item.default_select)
        .map(|item| item.selection_key.clone())
        .collect();
    if default_keys.is_empty() {
        return Ok(CacheCleanOutcome {
            scan: view,
            default_safe_count: 0,
            summary: empty_summary(),
        });
    }
    let count = default_keys.len();
    let prepared = prepare_cache_plan(&mut store, &snapshot.snapshot_id, default_keys)?;
    let store = Mutex::new(store);
    let domains = system_domain_services(cleaner, &SystemUninstaller, &SystemAppQuitter);
    let outcome =
        execute_operation_with(&store, history, CLI_OWNER, &prepared, |plan| async move {
            execute_domain_plan(plan, &domains).await
        })
        .await?;
    let OperationResult::Cache(summary) = outcome else {
        return Err(UserError::new(
            ErrorCode::RESULT_NOT_CACHE_SUMMARY,
            "操作结果类型与缓存清理不一致",
        ));
    };
    Ok(CacheCleanOutcome {
        scan: view,
        default_safe_count: count,
        summary,
    })
}

pub(crate) async fn clean_processes_with<W, O, S, H>(
    scan: ScanResult,
    whitelist: Arc<W>,
    observer: &mut O,
    signaller: &mut S,
    history: &H,
) -> Result<ProcessCleanOutcome, UserError>
where
    W: ProcessWhitelist + Send + Sync + 'static,
    O: ProcessObserver + Send,
    S: ProcessSignaller + Send,
    H: HistorySink,
{
    let mut store = OperationStore::new();
    let policy = process_whitelist_policy(whitelist);
    let snapshot = snapshot_process_scan(&mut store, scan, &policy)?;
    let view = ProcessScanView {
        items: snapshot.value.processes,
    };
    let default_keys: Vec<String> = view
        .items
        .iter()
        .filter(|info| info.default_select)
        .map(|info| info.selection_key.clone())
        .collect();
    if default_keys.is_empty() {
        return Ok(ProcessCleanOutcome {
            scan: view,
            default_safe_count: 0,
            report: empty_kill_report(),
        });
    }
    let count = default_keys.len();
    let prepared = prepare_process_plan(&mut store, &snapshot.snapshot_id, default_keys)?;
    let store = Mutex::new(store);
    let observer_ref: &mut (dyn ProcessObserver + Send) = observer;
    let signaller_ref: &mut (dyn ProcessSignaller + Send) = signaller;
    let outcome =
        execute_operation_with(&store, history, CLI_OWNER, &prepared, |plan| async move {
            let mut services = ProcessServices {
                observer: observer_ref,
                signaller: signaller_ref,
            };
            execute_termination_plan(plan, &mut services)
        })
        .await?;
    let OperationResult::Process(report) = outcome else {
        return Err(UserError::new(
            ErrorCode::RESULT_NOT_KILL_REPORT,
            "操作结果类型与进程终止不一致",
        ));
    };
    Ok(ProcessCleanOutcome {
        scan: view,
        default_safe_count: count,
        report,
    })
}

fn prepare_cache_plan(
    store: &mut OperationStore,
    snapshot_id: &str,
    item_keys: Vec<String>,
) -> Result<String, UserError> {
    prepare_operation_with(
        store,
        CLI_OWNER,
        PrepareOperationRequest::Cache {
            snapshot_id: snapshot_id.to_owned(),
            item_keys,
        },
    )
    .map(|prepared| prepared.operation_id)
}

fn prepare_process_plan(
    store: &mut OperationStore,
    snapshot_id: &str,
    process_keys: Vec<String>,
) -> Result<String, UserError> {
    prepare_operation_with(
        store,
        CLI_OWNER,
        PrepareOperationRequest::Process {
            snapshot_id: snapshot_id.to_owned(),
            process_keys,
            mode: ProcessMode::Graceful,
        },
    )
    .map(|prepared| prepared.operation_id)
}

fn empty_summary() -> CleanSummary {
    CleanSummary {
        reports: Vec::new(),
        total_freed_bytes: 0,
        success_count: 0,
        fail_count: 0,
    }
}

fn empty_kill_report() -> ProcessKillReport {
    ProcessKillReport::default()
}

#[cfg(test)]
#[path = "cli_operations_tests.rs"]
mod tests;
