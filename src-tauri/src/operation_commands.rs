use crate::app_scanner::{self, InstalledApp};
use crate::applications::{self, AppInfo};
use crate::cache_cleaner::CleanSummary;
use crate::cache_scanner::{CacheItem, CacheScanResult};
use crate::docker::{self, DockerExecutionReport, DockerInventory};
use crate::operation_executor::OperationOutcome;
use crate::operations::ProcessKillReport;
use crate::operations::{
    ConsumedPlan, DockerAction, InstalledAppIdentity, OperationKind, OperationStore,
    PreparedOperation, ProcessMode, SnapshotKind,
};
use crate::residue_scanner::{self, AppResidue};
use crate::scanner::{self, ProcessInfo, ProcessRow, ScanResult};
use crate::storage::Storage;
use crate::uninstaller::UninstallReport;
use crate::user_error::{ErrorCode, UserError};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::sync::{Arc, Mutex};

#[derive(Serialize, Clone, Debug)]
pub struct SnapshotResult<T> {
    pub snapshot_id: String,
    pub expires_at_ms: u64,
    pub value: T,
}

#[derive(Serialize, Clone, Debug)]
pub struct Keyed<T: Serialize> {
    pub selection_key: String,
    #[serde(flatten)]
    pub item: T,
}

#[derive(Serialize, Clone, Debug)]
pub struct CacheSnapshotView {
    pub items: Vec<Keyed<CacheItem>>,
    pub total_bytes: u64,
    pub scanned_at_ms: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct ResidueAppGroup {
    pub app_key: String,
    pub residue: AppResidue,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrepareOperationRequest {
    Cache {
        snapshot_id: String,
        item_keys: Vec<String>,
    },
    Process {
        snapshot_id: String,
        process_keys: Vec<String>,
        mode: ProcessMode,
    },
    AppTerminate {
        snapshot_id: String,
        app_keys: Vec<String>,
        mode: ProcessMode,
    },
    AppGracefulQuit {
        snapshot_id: String,
        app_keys: Vec<String>,
    },
    Uninstall {
        app_snapshot_id: String,
        residue_snapshot_id: String,
        app_keys: Vec<String>,
        residue_keys: Vec<String>,
        quit_running: bool,
    },
    Docker {
        snapshot_id: String,
        action: DockerAction,
        target_keys: Vec<String>,
    },
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum OperationResult {
    Cache(CleanSummary),
    Process(ProcessKillReport),
    AppTerminate(ProcessKillReport),
    AppGracefulQuit(Vec<crate::applications::AppGracefulQuitReport>),
    Uninstall(Vec<UninstallReport>),
    Docker(DockerExecutionReport),
}

impl From<OperationOutcome> for OperationResult {
    fn from(outcome: OperationOutcome) -> Self {
        match outcome {
            OperationOutcome::Cache(summary) => Self::Cache(summary),
            OperationOutcome::Process(report) => Self::Process(report),
            OperationOutcome::AppTerminate(report) => Self::AppTerminate(report),
            OperationOutcome::AppGracefulQuit(reports) => Self::AppGracefulQuit(reports),
            OperationOutcome::Uninstall(reports) => Self::Uninstall(reports),
            OperationOutcome::Docker(report) => Self::Docker(report),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationHistoryEntry {
    pub operation: String,
    pub target: String,
    pub freed_bytes: u64,
    pub success: bool,
    pub detail: String,
}

pub trait HistorySink: Send + Sync {
    fn record(&self, entry: &OperationHistoryEntry) -> Result<(), UserError>;
}

pub struct StorageHistory<'a> {
    storage: &'a Storage,
}

impl HistorySink for StorageHistory<'_> {
    fn record(&self, entry: &OperationHistoryEntry) -> Result<(), UserError> {
        self.storage.log_history(
            &entry.operation,
            &entry.target,
            entry.freed_bytes,
            entry.success,
            &entry.detail,
        )?;
        Ok(())
    }
}

pub trait ProcessWhitelist {
    fn is_process_whitelisted(&self, name: &str) -> bool;
}

impl ProcessWhitelist for Storage {
    fn is_process_whitelisted(&self, name: &str) -> bool {
        self.is_whitelisted("process", name)
    }
}

pub trait OperationSource: Sync {
    fn consume_plan(&self, operation_id: &str, owner: &str) -> Result<ConsumedPlan, UserError>;
}

impl OperationSource for Mutex<OperationStore> {
    fn consume_plan(&self, operation_id: &str, owner: &str) -> Result<ConsumedPlan, UserError> {
        let mut store = self
            .lock()
            .map_err(|_| UserError::new(ErrorCode::OPERATION_LOCK_BROKEN, "操作锁已失效"))?;
        store.consume(operation_id, owner)
    }
}

pub fn process_whitelist_policy<S: ProcessWhitelist + Send + Sync + 'static>(
    source: Arc<S>,
) -> impl Fn(&str) -> bool + Send + Sync + 'static {
    move |name: &str| crate::whitelist::is_whitelisted(name) || source.is_process_whitelisted(name)
}

pub(crate) fn snapshot_process_scan<P>(
    store: &mut OperationStore,
    mut result: ScanResult,
    policy: &P,
) -> Result<SnapshotResult<ScanResult>, UserError>
where
    P: Fn(&str) -> bool + ?Sized,
{
    result
        .processes
        .retain(|info: &ProcessInfo| !policy(&info.name));
    let registration = scanner::register_scan_processes(store, &mut result.processes)?;
    wrap(
        store,
        SnapshotKind::OverviewProcess,
        registration.snapshot_id,
        result,
    )
}

pub(crate) fn snapshot_process_rows<P>(
    store: &mut OperationStore,
    mut rows: Vec<ProcessRow>,
    policy: &P,
) -> Result<SnapshotResult<Vec<ProcessRow>>, UserError>
where
    P: Fn(&str) -> bool + ?Sized,
{
    for row in &mut rows {
        apply_policy(
            &mut row.protected,
            &mut row.protected_reason_key,
            &mut row.whitelisted,
            // 必须用原始名：执行期 `SystemProcessObserver` 拿到的是现场
            // `proc.name()`，两边看到不同名字会让 `revalidate_targets` 报
            // 「进程保护状态已变化」，用户白名单里的进程永远终止不掉。
            policy(&row.full_name),
        );
    }
    let registration = scanner::register_process_rows(store, &mut rows)?;
    wrap(store, SnapshotKind::Process, registration.snapshot_id, rows)
}

pub(crate) fn snapshot_applications<P>(
    store: &mut OperationStore,
    mut apps: Vec<AppInfo>,
    policy: &P,
) -> Result<SnapshotResult<Vec<AppInfo>>, UserError>
where
    P: Fn(&str) -> bool + ?Sized,
{
    for app in &mut apps {
        for child in &mut app.children {
            apply_policy(
                &mut child.protected,
                &mut child.protected_reason_key,
                &mut child.whitelisted,
                policy(&child.name),
            );
        }
        app.protected_process_count = app.children.iter().filter(|c| c.protected).count();
        app.whitelisted_process_count = app.children.iter().filter(|c| c.whitelisted).count();
    }
    let registration = applications::register_applications(store, &mut apps)?;
    wrap(
        store,
        SnapshotKind::Application,
        registration.snapshot_id,
        apps,
    )
}

pub(crate) fn snapshot_cache(
    store: &mut OperationStore,
    mut result: CacheScanResult,
) -> Result<SnapshotResult<CacheSnapshotView>, UserError> {
    let registration = store.register_cache_snapshot(result.items.clone())?;
    if registration.selection_keys.len() != result.items.len() {
        return Err(UserError::new(
            ErrorCode::CACHE_SELECTION_COUNT_MISMATCH,
            "缓存选择 key 数量与快照不一致",
        ));
    }
    let items = result
        .items
        .drain(..)
        .zip(registration.selection_keys)
        .map(|(item, selection_key)| Keyed {
            selection_key,
            item,
        })
        .collect();
    let value = CacheSnapshotView {
        items,
        total_bytes: result.total_bytes,
        scanned_at_ms: result.scanned_at_ms,
    };
    wrap(store, SnapshotKind::Cache, registration.snapshot_id, value)
}

pub(crate) fn snapshot_docker_inventory(
    store: &mut OperationStore,
    mut inventory: DockerInventory,
) -> Result<SnapshotResult<DockerInventory>, UserError> {
    let registration = docker::register_docker_inventory(store, &mut inventory)?;
    wrap(
        store,
        SnapshotKind::Docker,
        registration.snapshot_id,
        inventory,
    )
}

pub(crate) fn snapshot_installed_apps(
    store: &mut OperationStore,
    mut apps: Vec<InstalledApp>,
) -> Result<SnapshotResult<Vec<InstalledApp>>, UserError> {
    let registration = app_scanner::register_installed_apps(store, &mut apps)?;
    wrap(
        store,
        SnapshotKind::InstalledApps,
        registration.snapshot_id,
        apps,
    )
}

pub(crate) fn resolve_installed_apps(
    store: &OperationStore,
    app_snapshot_id: &str,
    app_keys: &[String],
) -> Result<Vec<(String, InstalledAppIdentity)>, UserError> {
    store.installed_app_selections(app_snapshot_id, app_keys)
}

pub(crate) fn register_residue_groups(
    store: &mut OperationStore,
    groups: Vec<(String, AppResidue)>,
) -> Result<SnapshotResult<Vec<ResidueAppGroup>>, UserError> {
    if groups.is_empty() {
        return Err(UserError::new(
            ErrorCode::RESIDUE_BATCH_EMPTY,
            "残留批次不能为空",
        ));
    }
    let mut residues = groups;
    let batches: Vec<(String, &mut AppResidue)> = residues
        .iter_mut()
        .map(|(app_key, residue)| (app_key.clone(), residue))
        .collect();
    let registration = residue_scanner::register_app_residue_batch(store, batches)?;
    let value = residues
        .into_iter()
        .map(|(app_key, residue)| ResidueAppGroup { app_key, residue })
        .collect();
    wrap(
        store,
        SnapshotKind::Residue,
        registration.snapshot_id,
        value,
    )
}

pub(crate) fn prepare_operation_with(
    store: &mut OperationStore,
    owner: &str,
    request: PrepareOperationRequest,
) -> Result<PreparedOperation, UserError> {
    match request {
        PrepareOperationRequest::Cache {
            snapshot_id,
            item_keys,
        } => store.prepare_cache(&snapshot_id, item_keys, owner),
        PrepareOperationRequest::Process {
            snapshot_id,
            process_keys,
            mode,
        } => store.prepare_process(&snapshot_id, process_keys, mode, owner),
        PrepareOperationRequest::AppTerminate {
            snapshot_id,
            app_keys,
            mode,
        } => store.prepare_app_termination(&snapshot_id, app_keys, mode, owner),
        PrepareOperationRequest::AppGracefulQuit {
            snapshot_id,
            app_keys,
        } => store.prepare_app_graceful_quit(&snapshot_id, app_keys, owner),
        PrepareOperationRequest::Uninstall {
            app_snapshot_id,
            residue_snapshot_id,
            app_keys,
            residue_keys,
            quit_running,
        } => store.prepare_uninstall(
            &app_snapshot_id,
            &residue_snapshot_id,
            app_keys,
            residue_keys,
            quit_running,
            owner,
        ),
        PrepareOperationRequest::Docker {
            snapshot_id,
            action,
            target_keys,
        } => store.prepare_docker(&snapshot_id, action, target_keys, owner),
    }
}

pub(crate) async fn execute_operation_with<S, F, Fut>(
    source: &S,
    history: &dyn HistorySink,
    owner: &str,
    operation_id: &str,
    run: F,
) -> Result<OperationResult, UserError>
where
    S: OperationSource,
    F: FnOnce(ConsumedPlan) -> Fut + Send,
    Fut: Future<Output = Result<OperationOutcome, UserError>> + Send,
{
    let plan = source.consume_plan(operation_id, owner)?;
    if plan.operation_id() != operation_id {
        return Err(UserError::new(
            ErrorCode::OPERATION_ID_MISMATCH,
            "操作计划与请求的 operation_id 不匹配",
        ));
    }
    let kind = plan.kind();
    let outcome = match run(plan).await {
        Ok(outcome) => outcome,
        Err(error) => {
            let entry = rejection_entry(kind, &error);
            if let Err(failure) = history.record(&entry) {
                return Err(history_write_failed(&error, &entry, &failure));
            }
            return Err(error);
        }
    };
    record_execution(history, outcome)
}

pub(crate) fn rejection_entry(kind: OperationKind, error: &str) -> OperationHistoryEntry {
    OperationHistoryEntry {
        operation: kind.label().to_owned(),
        target: "未执行".to_owned(),
        freed_bytes: 0,
        success: false,
        detail: format!("执行前复核未通过：{error}"),
    }
}

pub(crate) fn record_execution(
    history: &dyn HistorySink,
    outcome: OperationOutcome,
) -> Result<OperationResult, UserError> {
    let entry = history_entry(&outcome);
    history
        .record(&entry)
        .map_err(|failure| history_failure("操作已执行", &entry, &failure))?;
    Ok(OperationResult::from(outcome))
}

fn history_failure(executed: &str, entry: &OperationHistoryEntry, failure: &str) -> String {
    format!(
        "{executed}，但写入历史记录失败（{} · {}）：{failure}，请到历史页核对",
        entry.operation, entry.target
    )
}

/// 「操作本身失败了，但历史记录也没写成」的双重失败。
///
/// 改造前这条消息是 `format!("{error}；{…}")` 拼出来的裸串，前端靠
/// `message.includes("写入历史记录失败")` 认出它。现在 `code` 独立承载这个语义：
/// **`code` 不再由文案决定**，所以哪怕中文兜底以后改写，分类也不会失效。
fn history_write_failed(
    error: &UserError,
    entry: &OperationHistoryEntry,
    failure: &UserError,
) -> UserError {
    UserError::with(
        ErrorCode::HISTORY_WRITE_FAILED,
        format!("{error}；{}", history_failure("操作未执行", entry, failure)),
        vec![
            ("operation".to_owned(), entry.operation.clone()),
            ("target".to_owned(), entry.target.clone()),
            ("failure".to_owned(), failure.message.clone()),
        ],
    )
}

pub(crate) fn history_entry(outcome: &OperationOutcome) -> OperationHistoryEntry {
    let operation = outcome.kind().label().to_owned();
    match outcome {
        OperationOutcome::Cache(summary) => OperationHistoryEntry {
            operation,
            target: format!("{} 项缓存", summary.reports.len()),
            freed_bytes: summary.total_freed_bytes,
            success: summary.fail_count == 0,
            detail: format!(
                "成功 {} 项，失败 {} 项，释放 {}",
                summary.success_count, summary.fail_count, summary.total_freed_bytes
            ),
        },
        OperationOutcome::Process(report) => process_entry(operation, report),
        OperationOutcome::AppTerminate(report) => process_entry(operation, report),
        OperationOutcome::AppGracefulQuit(reports) => quit_entry(operation, reports),
        OperationOutcome::Uninstall(reports) => OperationHistoryEntry {
            operation,
            target: format!("{} 个应用", reports.len()),
            freed_bytes: reports.iter().fold(0_u64, |total, report| {
                total.saturating_add(report.total_freed_bytes)
            }),
            success: reports.iter().all(|report| report.failed_count == 0),
            detail: format!(
                "移动 {} 项，失败 {} 项",
                reports
                    .iter()
                    .map(|report| report.moved_count)
                    .sum::<usize>(),
                reports
                    .iter()
                    .map(|report| report.failed_count)
                    .sum::<usize>()
            ),
        },
        OperationOutcome::Docker(report) => OperationHistoryEntry {
            operation,
            target: format!("Docker {}", report.action),
            freed_bytes: 0,
            success: report.failed.is_empty(),
            detail: format!(
                "成功 {} 项，失败 {} 项",
                report.succeeded.len(),
                report.failed.len()
            ),
        },
    }
}

fn quit_entry(
    operation: String,
    reports: &[crate::applications::AppGracefulQuitReport],
) -> OperationHistoryEntry {
    let failed = reports
        .iter()
        .filter(|report| report.quit_error.is_some())
        .count();
    OperationHistoryEntry {
        operation,
        target: format!("{} 个应用", reports.len()),
        freed_bytes: 0,
        success: failed == 0,
        detail: format!("成功 {} 个，失败 {} 个", reports.len() - failed, failed),
    }
}

fn process_entry(operation: String, report: &ProcessKillReport) -> OperationHistoryEntry {
    OperationHistoryEntry {
        operation,
        target: format!("{} 个进程", report.details.len().max(report.killed.len())),
        freed_bytes: 0,
        success: report.failed.is_empty(),
        detail: format!(
            "成功 {} 个，失败 {} 个",
            report.killed.len(),
            report.failed.len()
        ),
    }
}

fn wrap<T>(
    store: &OperationStore,
    kind: SnapshotKind,
    snapshot_id: String,
    value: T,
) -> Result<SnapshotResult<T>, UserError> {
    Ok(SnapshotResult {
        expires_at_ms: store.snapshot_expiry_ms(kind)?,
        snapshot_id,
        value,
    })
}

/// 命中用户白名单时把行标记成受保护，并给出**受保护原因**。
///
/// 原因存 i18n key（`process.protect.whitelisted`）而不是文案：译文只在前端
/// 词典里。`reason.is_none()` 的语义 —— 「已经有更具体的原因就别覆盖」——
/// 与改造前逐字一致。
fn apply_policy(
    protected: &mut bool,
    reason_key: &mut Option<String>,
    whitelisted: &mut bool,
    policy_hit: bool,
) {
    if !policy_hit {
        return;
    }
    *whitelisted = true;
    *protected = true;
    if reason_key.is_none() {
        *reason_key = Some("process.protect.whitelisted".to_owned());
    }
}

pub(crate) fn system_history<'a>(storage: &'a Storage) -> StorageHistory<'a> {
    StorageHistory { storage }
}

#[cfg(test)]
#[path = "operation_commands_tests.rs"]
mod tauri_commands;
