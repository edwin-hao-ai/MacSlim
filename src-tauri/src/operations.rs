use crate::cache_scanner::CacheItem;
use crate::residue_policy::{self, ResidueRoots};
use crate::user_error::{ErrorCode, UserError};
use registry::{
    dedicated_entry_error, expiry, keyed_applications, keyed_items, keyed_payload, normalize_keys,
    payload_kind, plan_metadata, random_id, require_non_empty, select_items, system_time_ms,
    SnapshotEntry, SnapshotItems,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;

#[path = "operation_types.rs"]
mod types;

#[path = "operation_registry.rs"]
mod registry;

#[path = "operations_prepare.rs"]
mod prepare;

pub(crate) use registry::ensure_targets_terminable;
pub(crate) use types::PROCESS_SNAPSHOT_KINDS;

pub(crate) use types::{AppIdentity, ConsumedPlan, OperationPlan, ProcessTarget, SnapshotPayload};
pub use types::{
    ApplicationChildBinding, ApplicationSnapshotRegistration, ProcessKillDetail, ProcessKillReport,
    UninstallPlanTarget,
};
pub use types::{
    CacheAction, DockerAction, DockerInventoryFingerprint, DockerResourceKind, DockerTarget,
    InstalledAppIdentity, LiveDockerResource, LiveResidue, OperationId, OperationKind,
    PreparedOperation, ProcessIdentity, ProcessMode, ResidueIdentity, SnapshotId, SnapshotKind,
    SnapshotRegistration,
};

pub const SNAPSHOT_TTL_MS: u64 = 600_000;
pub const OPERATION_TTL_MS: u64 = SNAPSHOT_TTL_MS;
pub const MAX_SNAPSHOTS: usize = 128;
pub const MAX_OPERATIONS: usize = 256;

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

struct OperationEntry {
    owner: String,
    snapshot_ids: Vec<SnapshotId>,
    plan: OperationPlan,
    created_at_ms: u64,
    expires_at_ms: u64,
}

pub struct OperationStore {
    active_snapshots: HashMap<SnapshotKind, SnapshotEntry>,
    snapshot_order: VecDeque<SnapshotId>,
    operations: HashMap<OperationId, OperationEntry>,
    operation_order: VecDeque<OperationId>,
    clock: Clock,
}

impl Default for OperationStore {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationStore {
    pub fn new() -> Self {
        Self::with_clock(system_time_ms)
    }

    pub(crate) fn with_clock<F>(clock: F) -> Self
    where
        F: Fn() -> u64 + Send + Sync + 'static,
    {
        Self {
            active_snapshots: HashMap::new(),
            snapshot_order: VecDeque::new(),
            operations: HashMap::new(),
            operation_order: VecDeque::new(),
            clock: Arc::new(clock),
        }
    }

    pub(crate) fn register_snapshot(
        &mut self,
        kind: SnapshotKind,
        payload: SnapshotPayload,
    ) -> Result<SnapshotRegistration, UserError> {
        if let Some(message) = dedicated_entry_error(&payload) {
            return Err(UserError::new(
                ErrorCode::SNAPSHOT_DEDICATED_ENTRY_REQUIRED,
                message,
            ));
        }
        self.register_typed(kind, payload)
    }

    fn register_typed(
        &mut self,
        kind: SnapshotKind,
        payload: SnapshotPayload,
    ) -> Result<SnapshotRegistration, UserError> {
        self.cleanup_expired();
        if payload_kind(&payload) != kind {
            return Err(UserError::new(
                ErrorCode::SNAPSHOT_PAYLOAD_MISMATCH,
                "快照类型与载荷不匹配",
            ));
        }
        let snapshot_id = random_id()?;
        let (selection_keys, items) = keyed_payload(payload)?;
        let expires_at_ms = expiry(self.now_ms());
        self.replace_snapshot(kind, snapshot_id.clone(), items, expires_at_ms);
        Ok(SnapshotRegistration {
            snapshot_id,
            selection_keys,
        })
    }

    fn replace_snapshot(
        &mut self,
        kind: SnapshotKind,
        snapshot_id: SnapshotId,
        items: SnapshotItems,
        expires_at_ms: u64,
    ) {
        if let Some(old_id) = self
            .active_snapshots
            .get(&kind)
            .map(|snapshot| snapshot.id.clone())
        {
            self.remove_snapshot(kind, &old_id);
        }
        self.active_snapshots.insert(
            kind,
            SnapshotEntry {
                id: snapshot_id.clone(),
                items,
                expires_at_ms,
            },
        );
        self.snapshot_order.push_back(snapshot_id);
        self.enforce_limits();
    }

    pub(crate) fn register_cache_snapshot(
        &mut self,
        items: Vec<CacheItem>,
    ) -> Result<SnapshotRegistration, UserError> {
        self.register_snapshot(SnapshotKind::Cache, SnapshotPayload::Cache(items))
    }

    pub(crate) fn register_process_snapshot(
        &mut self,
        targets: Vec<ProcessTarget>,
    ) -> Result<SnapshotRegistration, UserError> {
        self.register_snapshot(SnapshotKind::Process, SnapshotPayload::Process(targets))
    }

    pub(crate) fn register_overview_process_snapshot(
        &mut self,
        targets: Vec<ProcessTarget>,
    ) -> Result<SnapshotRegistration, UserError> {
        self.register_process_targets(SnapshotKind::OverviewProcess, targets)
    }

    fn register_process_targets(
        &mut self,
        kind: SnapshotKind,
        targets: Vec<ProcessTarget>,
    ) -> Result<SnapshotRegistration, UserError> {
        self.cleanup_expired();
        let snapshot_id = random_id()?;
        let (selection_keys, items) = keyed_items(targets)?;
        let expires_at_ms = expiry(self.now_ms());
        self.replace_snapshot(
            kind,
            snapshot_id.clone(),
            SnapshotItems::Process(items),
            expires_at_ms,
        );
        Ok(SnapshotRegistration {
            snapshot_id,
            selection_keys,
        })
    }

    pub(crate) fn register_application_snapshot(
        &mut self,
        apps: Vec<AppIdentity>,
    ) -> Result<ApplicationSnapshotRegistration, UserError> {
        self.cleanup_expired();
        let snapshot_id = random_id()?;
        let (app_keys, child_bindings, items) = keyed_applications(apps)?;
        let expires_at_ms = expiry(self.now_ms());
        self.replace_snapshot(
            SnapshotKind::Application,
            snapshot_id.clone(),
            SnapshotItems::Application(items),
            expires_at_ms,
        );
        Ok(ApplicationSnapshotRegistration {
            snapshot_id,
            app_keys,
            child_bindings,
        })
    }

    pub(crate) fn register_installed_apps(
        &mut self,
        apps: Vec<InstalledAppIdentity>,
    ) -> Result<SnapshotRegistration, UserError> {
        self.cleanup_expired();
        let registration = self.register_typed(
            SnapshotKind::InstalledApps,
            SnapshotPayload::InstalledApps(apps),
        )?;
        self.drop_residue_snapshot();
        Ok(registration)
    }

    pub(crate) fn register_residues(
        &mut self,
        residues: Vec<ResidueIdentity>,
    ) -> Result<SnapshotRegistration, UserError> {
        self.validate_residues(&residues)?;
        self.register_typed(SnapshotKind::Residue, SnapshotPayload::Residue(residues))
    }

    pub(crate) fn register_residues_for_apps(
        &mut self,
        batches: Vec<(String, Vec<ResidueIdentity>)>,
    ) -> Result<SnapshotRegistration, UserError> {
        if batches.is_empty() {
            return Err(UserError::new(
                ErrorCode::RESIDUE_SNAPSHOT_EMPTY,
                "残留快照不能为空",
            ));
        }
        let mut flattened = Vec::new();
        for (app_key, residues) in batches {
            if app_key.trim().is_empty() {
                return Err(UserError::new(
                    ErrorCode::RESIDUE_BATCH_MISSING_APP_KEY,
                    "残留批次缺少应用选择项",
                ));
            }
            if residues.is_empty() {
                continue;
            }
            self.known_app_key(&app_key)?;
            for mut residue in residues {
                residue.app_key = app_key.clone();
                flattened.push(residue);
            }
        }
        self.register_residues(flattened)
    }

    pub(crate) fn register_docker_resources(
        &mut self,
        targets: Vec<DockerTarget>,
    ) -> Result<SnapshotRegistration, UserError> {
        self.register_typed(SnapshotKind::Docker, SnapshotPayload::Docker(targets))
    }

    pub fn snapshot_expiry_ms(&self, kind: SnapshotKind) -> Result<u64, UserError> {
        let entry = self
            .active_snapshots
            .get(&kind)
            .ok_or_else(stale_snapshot)?;
        if entry.expires_at_ms <= self.now_ms() {
            return Err(stale_snapshot());
        }
        Ok(entry.expires_at_ms)
    }

    pub fn installed_app_selections(
        &self,
        snapshot_id: &str,
        keys: &[String],
    ) -> Result<Vec<(String, InstalledAppIdentity)>, UserError> {
        let keys = require_non_empty(normalize_keys(keys)?)?;
        let entry = self.snapshot(snapshot_id, SnapshotKind::InstalledApps)?;
        if entry.expires_at_ms <= self.now_ms() {
            return Err(stale_snapshot());
        }
        let SnapshotItems::InstalledApps(items) = &entry.items else {
            return Err(UserError::new(
                ErrorCode::SNAPSHOT_PAYLOAD_MISMATCH,
                "快照类型与载荷不匹配",
            ));
        };
        let mut values = Vec::with_capacity(keys.len());
        for key in &keys {
            values.push(items.get(key).cloned().ok_or_else(missing_selection)?);
        }
        Ok(keys.into_iter().zip(values).collect())
    }

    fn drop_residue_snapshot(&mut self) {
        if let Some(stale) = self
            .active_snapshots
            .get(&SnapshotKind::Residue)
            .map(|snapshot| snapshot.id.clone())
        {
            self.remove_snapshot(SnapshotKind::Residue, &stale);
        }
    }

    fn known_app_key(&mut self, app_key: &str) -> Result<String, UserError> {
        self.installed_app_bindings()?
            .remove(app_key)
            .ok_or_else(unknown_residue_app_key)
    }

    fn validate_residues(&mut self, residues: &[ResidueIdentity]) -> Result<(), UserError> {
        let apps = self.installed_app_bindings()?;
        let mut roots: HashMap<&str, ResidueRoots> = HashMap::new();
        for residue in residues {
            let bundle_id = apps
                .get(residue.app_key.as_str())
                .ok_or_else(unknown_residue_app_key)?;
            let allowed = roots
                .entry(bundle_id.as_str())
                .or_insert_with(|| residue_policy::allowed_roots(bundle_id));
            residue_policy::ensure_within_roots(Path::new(&residue.path), allowed)?;
        }
        Ok(())
    }

    fn installed_app_bindings(&mut self) -> Result<HashMap<String, String>, UserError> {
        let apps = self
            .active_snapshots
            .get(&SnapshotKind::InstalledApps)
            .ok_or_else(unknown_residue_app_key)?;
        let items = match &apps.items {
            SnapshotItems::InstalledApps(items) => items,
            _ => return Err(unknown_residue_app_key()),
        };
        Ok(items
            .iter()
            .map(|(key, app)| (key.clone(), app.bundle_id.clone()))
            .collect())
    }

    pub(crate) fn consume(
        &mut self,
        operation_id: &str,
        owner: &str,
    ) -> Result<ConsumedPlan, UserError> {
        self.cleanup_expired();
        let now = self.now_ms();
        let entry = self
            .operations
            .get(operation_id)
            .ok_or_else(used_operation)?;
        if entry.expires_at_ms <= now {
            self.remove_operation(operation_id);
            return Err(used_operation());
        }
        if entry.created_at_ms > now {
            return Err(UserError::new(
                ErrorCode::OPERATION_TIME_INVALID,
                "操作时间无效",
            ));
        }
        if entry.owner != owner {
            return Err(UserError::new(
                ErrorCode::OPERATION_OWNER_MISMATCH,
                "操作所有者不匹配",
            ));
        }
        if !self.snapshots_are_active(&entry.snapshot_ids, now) {
            self.remove_operation(operation_id);
            return Err(UserError::new(
                ErrorCode::OPERATION_SNAPSHOT_STALE,
                "操作所属快照已失效",
            ));
        }
        self.operations
            .remove(operation_id)
            .map(|entry| ConsumedPlan::from_plan(operation_id.to_owned(), entry.plan))
            .ok_or_else(used_operation)
    }

    fn now_ms(&self) -> u64 {
        (self.clock)()
    }

    fn snapshot(&self, snapshot_id: &str, kind: SnapshotKind) -> Result<&SnapshotEntry, UserError> {
        self.active_snapshots
            .get(&kind)
            .filter(|snapshot| snapshot.id == snapshot_id)
            .ok_or_else(stale_snapshot)
    }

    fn ensure_snapshot(&mut self, snapshot_id: &str, kind: SnapshotKind) -> Result<(), UserError> {
        self.cleanup_expired();
        let snapshot = self.snapshot(snapshot_id, kind)?;
        if snapshot.expires_at_ms <= self.now_ms() {
            return Err(stale_snapshot());
        }
        Ok(())
    }

    fn snapshots_are_active(&self, snapshot_ids: &[SnapshotId], now: u64) -> bool {
        !snapshot_ids.is_empty()
            && snapshot_ids.iter().all(|snapshot_id| {
                self.active_snapshots
                    .values()
                    .any(|snapshot| snapshot.id == *snapshot_id && snapshot.expires_at_ms > now)
            })
    }

    fn insert_operation(
        &mut self,
        snapshot_ids: &[SnapshotId],
        owner: &str,
        plan: OperationPlan,
    ) -> Result<PreparedOperation, UserError> {
        if owner.is_empty() {
            return Err(UserError::new(
                ErrorCode::OPERATION_OWNER_EMPTY,
                "操作所有者不能为空",
            ));
        }
        if snapshot_ids.is_empty() {
            return Err(UserError::new(
                ErrorCode::OPERATION_MISSING_SNAPSHOT,
                "操作缺少所属快照",
            ));
        }
        self.cleanup_expired();
        let (kind, item_count, estimated_bytes, summary) = plan_metadata(&plan);
        let operation_id = random_id()?;
        let now = self.now_ms();
        let expires_at_ms = expiry(now);
        self.operations.insert(
            operation_id.clone(),
            OperationEntry {
                owner: owner.to_owned(),
                snapshot_ids: snapshot_ids.to_vec(),
                plan,
                created_at_ms: now,
                expires_at_ms,
            },
        );
        self.operation_order.push_back(operation_id.clone());
        self.enforce_limits();
        Ok(PreparedOperation {
            operation_id,
            kind: kind.label().to_owned(),
            expires_at_ms,
            item_count,
            estimated_bytes,
            summary_key: summary.key,
            summary_params: summary.params,
        })
    }

    fn cleanup_expired(&mut self) {
        let now = self.now_ms();
        let expired_snapshots: Vec<(SnapshotKind, SnapshotId)> = self
            .active_snapshots
            .iter()
            .filter_map(|(kind, snapshot)| {
                (snapshot.expires_at_ms <= now).then_some((*kind, snapshot.id.clone()))
            })
            .collect();
        for (kind, snapshot_id) in expired_snapshots {
            self.remove_snapshot(kind, &snapshot_id);
        }
        let expired_operations: Vec<OperationId> = self
            .operations
            .iter()
            .filter(|(_, operation)| operation.expires_at_ms <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for operation_id in expired_operations {
            self.remove_operation(&operation_id);
        }
    }

    fn enforce_limits(&mut self) {
        while self.active_snapshots.len() > MAX_SNAPSHOTS {
            let Some(snapshot_id) = self.snapshot_order.pop_front() else {
                break;
            };
            let kind = self
                .active_snapshots
                .iter()
                .find_map(|(kind, snapshot)| (snapshot.id == snapshot_id).then_some(*kind));
            if let Some(kind) = kind {
                self.remove_snapshot(kind, &snapshot_id);
            }
        }
        while self.operations.len() > MAX_OPERATIONS {
            let Some(operation_id) = self.operation_order.pop_front() else {
                break;
            };
            self.remove_operation(&operation_id);
        }
    }

    fn remove_snapshot(&mut self, kind: SnapshotKind, snapshot_id: &str) {
        if self
            .active_snapshots
            .get(&kind)
            .is_some_and(|snapshot| snapshot.id == snapshot_id)
        {
            self.active_snapshots.remove(&kind);
        }
        self.snapshot_order.retain(|stored| stored != snapshot_id);
        let operation_ids: HashSet<OperationId> = self
            .operations
            .iter()
            .filter(|(_, operation)| {
                operation
                    .snapshot_ids
                    .iter()
                    .any(|bound_id| bound_id == snapshot_id)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for operation_id in &operation_ids {
            self.operations.remove(operation_id);
        }
        self.operation_order
            .retain(|stored| !operation_ids.contains(stored));
    }

    fn remove_operation(&mut self, operation_id: &str) {
        self.operations.remove(operation_id);
        self.operation_order.retain(|stored| stored != operation_id);
    }
}

/// 四条反复出现的 broker 拒绝理由，各自一个 `code`。
///
/// 抽成函数而不是内联 `UserError::new(..)`，是因为同一句话在 6～11 个地方出现：
/// 复制粘贴的代价是「改文案漏改一处」，而 `ErrorCode` 的权威清单只有 `ALL`
/// 一份（见 `user_error_tests.rs` 的逐条比对门禁）。
fn stale_snapshot() -> UserError {
    UserError::new(ErrorCode::SNAPSHOT_STALE, "快照不存在或已失效")
}

fn used_operation() -> UserError {
    UserError::new(ErrorCode::OPERATION_USED_OR_EXPIRED, "操作已使用或已失效")
}

fn missing_selection() -> UserError {
    UserError::new(ErrorCode::SELECTION_MISSING, "选择项不存在")
}

fn unknown_residue_app_key() -> UserError {
    UserError::new(
        ErrorCode::RESIDUE_UNKNOWN_APP_KEY,
        "残留快照引用了未知应用选择项",
    )
}

/// 把 `PreparedOperation` 的摘要渲染成 `key(参数=值, …)` 供测试逐字比对。
///
/// 摘要本身已经是 i18n key（译文在 `src/i18n/*.ts`），所以这里断言的是
/// 「**选了哪枚 key、带了哪些参数、参数值是多少**」——这三样合起来就等价于
/// 改造前那句整串中文断言的信息量。
#[cfg(test)]
pub(crate) fn describe_summary(prepared: &PreparedOperation) -> String {
    if prepared.summary_params.is_empty() {
        return prepared.summary_key.clone();
    }
    let rendered: Vec<String> = prepared
        .summary_params
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    format!("{}({})", prepared.summary_key, rendered.join(", "))
}

#[cfg(test)]
#[path = "operations_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "operations_process_tests.rs"]
mod process_tests;

#[cfg(test)]
#[path = "operations_uninstall_tests.rs"]
mod uninstall_tests;

#[cfg(test)]
#[path = "operations_docker_plan_tests.rs"]
mod docker_plan_tests;

#[cfg(test)]
#[path = "operations_app_quit_tests.rs"]
mod app_quit_tests;

#[cfg(test)]
#[path = "operations_residue_batch_tests.rs"]
mod residue_batch_tests;
