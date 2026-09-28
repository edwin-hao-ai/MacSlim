use super::registry::{
    ensure_app_quit_allowed, ensure_targets_terminable, SnapshotEntry, SnapshotItems,
};
use super::types::{
    DockerAction, DockerInventoryFingerprint, DockerTarget, InstalledAppIdentity, OperationPlan,
    ProcessMode, ProcessTarget, ResidueIdentity, SnapshotKind, UninstallPlanTarget,
};
use super::{
    normalize_keys, require_non_empty, select_items, OperationStore, PreparedOperation,
    PROCESS_SNAPSHOT_KINDS,
};
use crate::user_error::{ErrorCode, UserError};
use std::collections::{HashMap, HashSet};

/// 路由提示消息（CLI / 日志用）。
///
/// 这是「内部不变量」类提示：正常流程点不到 UI，所以用兜底 code 而不进
/// 权威清单（只收前端需要按 code 分类或查译文的那批）。
pub(crate) const GRACEFUL_QUIT_ROUTE: &str =
    "应用优雅退出请使用专用的 app_graceful_quit 操作，不要用 app_terminate 的优雅模式";

fn payload_mismatch() -> UserError {
    UserError::new(ErrorCode::SNAPSHOT_PAYLOAD_MISMATCH, "快照类型与载荷不匹配")
}

fn stale_snapshot() -> UserError {
    UserError::new(ErrorCode::SNAPSHOT_STALE, "快照不存在或已失效")
}

macro_rules! selected {
    ($snapshot:expr, $keys:expr, $variant:ident) => {
        match &$snapshot.items {
            SnapshotItems::$variant(items) => select_items(items, $keys),
            _ => Err(payload_mismatch()),
        }
    };
}

impl OperationStore {
    pub(crate) fn prepare_cache<K>(
        &mut self,
        snapshot_id: &str,
        keys: K,
        owner: &str,
    ) -> Result<PreparedOperation, UserError>
    where
        K: IntoIterator,
        K::Item: AsRef<str>,
    {
        let keys = require_non_empty(normalize_keys(keys)?)?;
        self.ensure_snapshot(snapshot_id, SnapshotKind::Cache)?;
        let items = selected!(
            self.snapshot(snapshot_id, SnapshotKind::Cache)?,
            &keys,
            Cache
        )?;
        let plan = OperationPlan::Cache {
            snapshot_id: snapshot_id.to_owned(),
            items,
        };
        self.insert_operation(&[snapshot_id.to_owned()], owner, plan)
    }

    pub(crate) fn prepare_process<K>(
        &mut self,
        snapshot_id: &str,
        keys: K,
        mode: ProcessMode,
        owner: &str,
    ) -> Result<PreparedOperation, UserError>
    where
        K: IntoIterator,
        K::Item: AsRef<str>,
    {
        let keys = require_non_empty(normalize_keys(keys)?)?;
        self.cleanup_expired();
        let targets = self.select_process_targets(snapshot_id, &keys)?;
        ensure_targets_terminable(&targets, mode)?;
        let plan = OperationPlan::Process { mode, targets };
        self.insert_operation(&[snapshot_id.to_owned()], owner, plan)
    }

    fn select_process_targets(
        &self,
        snapshot_id: &str,
        keys: &[String],
    ) -> Result<Vec<ProcessTarget>, UserError> {
        let entry = PROCESS_SNAPSHOT_KINDS
            .iter()
            .find_map(|kind| {
                self.active_snapshots
                    .get(kind)
                    .filter(|snapshot| snapshot.id == snapshot_id)
            })
            .ok_or_else(stale_snapshot)?;
        match &entry.items {
            SnapshotItems::Process(items) => select_items(items, keys),
            _ => Err(payload_mismatch()),
        }
    }

    pub(crate) fn prepare_app_graceful_quit<K>(
        &mut self,
        snapshot_id: &str,
        keys: K,
        owner: &str,
    ) -> Result<PreparedOperation, UserError>
    where
        K: IntoIterator,
        K::Item: AsRef<str>,
    {
        let keys = require_non_empty(normalize_keys(keys)?)?;
        self.ensure_snapshot(snapshot_id, SnapshotKind::Application)?;
        let targets = selected!(
            self.snapshot(snapshot_id, SnapshotKind::Application)?,
            &keys,
            Application
        )?;
        ensure_app_quit_allowed(&targets)?;
        let plan = OperationPlan::AppGracefulQuit { targets };
        self.insert_operation(&[snapshot_id.to_owned()], owner, plan)
    }

    pub(crate) fn prepare_app_termination<K>(
        &mut self,
        snapshot_id: &str,
        keys: K,
        mode: ProcessMode,
        owner: &str,
    ) -> Result<PreparedOperation, UserError>
    where
        K: IntoIterator,
        K::Item: AsRef<str>,
    {
        let keys = require_non_empty(normalize_keys(keys)?)?;
        if mode == ProcessMode::Graceful {
            return Err(UserError::new(ErrorCode::INTERNAL, GRACEFUL_QUIT_ROUTE));
        }
        self.ensure_snapshot(snapshot_id, SnapshotKind::Application)?;
        let targets = selected!(
            self.snapshot(snapshot_id, SnapshotKind::Application)?,
            &keys,
            Application
        )?;
        let child_targets: Vec<ProcessTarget> = targets
            .iter()
            .flat_map(|app| app.processes.iter().cloned())
            .collect();
        ensure_targets_terminable(&child_targets, mode)?;
        let plan = OperationPlan::AppTerminate { mode, targets };
        self.insert_operation(&[snapshot_id.to_owned()], owner, plan)
    }

    pub(crate) fn prepare_uninstall<A, R>(
        &mut self,
        app_snapshot_id: &str,
        residue_snapshot_id: &str,
        app_keys: A,
        residue_keys: R,
        quit_running: bool,
        owner: &str,
    ) -> Result<PreparedOperation, UserError>
    where
        A: IntoIterator,
        A::Item: AsRef<str>,
        R: IntoIterator,
        R::Item: AsRef<str>,
    {
        let app_keys = require_non_empty(normalize_keys(app_keys)?)?;
        let residue_keys = normalize_keys(residue_keys)?;
        self.ensure_snapshot(app_snapshot_id, SnapshotKind::InstalledApps)?;
        self.ensure_snapshot(residue_snapshot_id, SnapshotKind::Residue)?;
        let targets = resolve_uninstall(
            self.snapshot(app_snapshot_id, SnapshotKind::InstalledApps)?,
            self.snapshot(residue_snapshot_id, SnapshotKind::Residue)?,
            &app_keys,
            &residue_keys,
        )?;
        ensure_unique_uninstall_targets(&targets, quit_running)?;
        let plan = OperationPlan::Uninstall {
            targets,
            quit_running,
        };
        self.insert_operation(
            &[app_snapshot_id.to_owned(), residue_snapshot_id.to_owned()],
            owner,
            plan,
        )
    }

    pub(crate) fn prepare_docker<K>(
        &mut self,
        snapshot_id: &str,
        action: DockerAction,
        keys: K,
        owner: &str,
    ) -> Result<PreparedOperation, UserError>
    where
        K: IntoIterator,
        K::Item: AsRef<str>,
    {
        let keys = normalize_keys(keys)?;
        self.ensure_snapshot(snapshot_id, SnapshotKind::Docker)?;
        let snapshot = self.snapshot(snapshot_id, SnapshotKind::Docker)?;
        let inventory = docker_fingerprint(snapshot)?;
        let targets = resolve_docker_targets(snapshot, action, &keys)?;
        let plan = OperationPlan::Docker {
            action,
            targets,
            inventory,
        };
        self.insert_operation(&[snapshot_id.to_owned()], owner, plan)
    }
}

fn resolve_uninstall(
    apps: &SnapshotEntry,
    residues: &SnapshotEntry,
    app_keys: &[String],
    residue_keys: &[String],
) -> Result<Vec<UninstallPlanTarget>, UserError> {
    let app_values = selected!(apps, app_keys, InstalledApps)?;
    let mut app_map: HashMap<String, InstalledAppIdentity> =
        app_keys.iter().cloned().zip(app_values).collect();
    let residue_values = selected!(residues, residue_keys, Residue)?;
    let mut grouped: HashMap<String, Vec<ResidueIdentity>> = HashMap::new();
    for residue in residue_values {
        if !app_keys.contains(&residue.app_key) {
            return Err(UserError::new(
                ErrorCode::RESIDUE_NOT_IN_SELECTED_APP,
                "残留选择项不属于所选应用",
            ));
        }
        grouped
            .entry(residue.app_key.clone())
            .or_default()
            .push(residue);
    }
    let mut targets = Vec::new();
    let mut planned_paths: HashSet<String> = HashSet::new();
    for key in app_keys {
        let app = app_map
            .remove(key)
            .ok_or_else(|| UserError::new(ErrorCode::SELECTION_MISSING, "应用选择项不存在"))?;
        let app_residues = grouped.remove(key).unwrap_or_default();
        ensure_unique_residue_paths(&app_residues, &mut planned_paths)?;
        targets.push(UninstallPlanTarget {
            app_key: key.clone(),
            app,
            residues: app_residues,
        });
    }
    Ok(targets)
}

fn ensure_unique_residue_paths(
    residues: &[ResidueIdentity],
    planned: &mut HashSet<String>,
) -> Result<(), UserError> {
    for residue in residues {
        if !planned.insert(residue.path.clone()) {
            return Err(UserError::new(
                ErrorCode::RESIDUE_PATH_DUPLICATED,
                "残留路径重复",
            ));
        }
    }
    Ok(())
}

fn ensure_unique_uninstall_targets(
    targets: &[UninstallPlanTarget],
    quit_running: bool,
) -> Result<(), UserError> {
    let mut bundles: HashSet<&str> = HashSet::with_capacity(targets.len());
    for target in targets {
        if target.app.is_system {
            return Err(UserError::new(
                ErrorCode::SYSTEM_APP_CANNOT_UNINSTALL,
                "系统核心应用不能卸载",
            ));
        }
        if !bundles.insert(target.app.bundle_path.as_str()) {
            return Err(UserError::new(
                ErrorCode::APP_SELECTED_TWICE,
                "同一应用被重复选择",
            ));
        }
    }
    if !quit_running {
        return Ok(());
    }
    let mut names: HashSet<&str> = HashSet::with_capacity(targets.len());
    for target in targets {
        if !names.insert(target.app.app_name.as_str()) {
            return Err(UserError::new(
                ErrorCode::APP_AMBIGUOUS_NAME,
                "同名应用无法精确定位，请分开卸载",
            ));
        }
    }
    Ok(())
}

fn docker_fingerprint(snapshot: &SnapshotEntry) -> Result<DockerInventoryFingerprint, UserError> {
    let mut resources = match &snapshot.items {
        SnapshotItems::Docker(items) => items.values().cloned().collect::<Vec<DockerTarget>>(),
        _ => return Err(payload_mismatch()),
    };
    resources.sort_by(|left, right| {
        (left.resource_type, &left.id, &left.name).cmp(&(
            right.resource_type,
            &right.id,
            &right.name,
        ))
    });
    Ok(DockerInventoryFingerprint::from_resources(resources))
}

fn resolve_docker_targets(
    snapshot: &SnapshotEntry,
    action: DockerAction,
    keys: &[String],
) -> Result<Vec<DockerTarget>, UserError> {
    if action == DockerAction::Prune {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        return Err(UserError::new(
            ErrorCode::DOCKER_PRUNE_REJECTS_SELECTION,
            "Docker prune 不接受资源选择项",
        ));
    }
    let keys = require_non_empty(keys.to_vec())?;
    let targets = selected!(snapshot, &keys, Docker)?;
    let expected = action.resource_type().ok_or_else(|| {
        UserError::new(
            ErrorCode::DOCKER_SELECTION_TYPE_MISMATCH,
            "Docker 操作类型错误",
        )
    })?;
    if targets
        .iter()
        .all(|target| target.resource_type == expected)
    {
        Ok(targets)
    } else {
        Err(UserError::new(
            ErrorCode::DOCKER_SELECTION_TYPE_MISMATCH,
            "Docker 选择项类型不匹配",
        ))
    }
}
