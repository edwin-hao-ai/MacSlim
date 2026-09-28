use super::types::{
    AppIdentity, ApplicationChildBinding, DockerAction, DockerInventoryFingerprint, DockerTarget,
    InstalledAppIdentity, OperationKind, OperationPlan, ProcessMode, ProcessTarget,
    ResidueIdentity, SnapshotKind, SnapshotPayload, UninstallPlanTarget,
};
use crate::cache_scanner::CacheItem;
use crate::i18n_text::I18nText;
use crate::user_error::{ErrorCode, UserError};
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) enum SnapshotItems {
    Cache(HashMap<String, CacheItem>),
    Process(HashMap<String, ProcessTarget>),
    Application(HashMap<String, AppIdentity>),
    InstalledApps(HashMap<String, InstalledAppIdentity>),
    Residue(HashMap<String, ResidueIdentity>),
    Docker(HashMap<String, DockerTarget>),
}

pub(super) struct SnapshotEntry {
    pub(super) id: super::SnapshotId,
    pub(super) items: SnapshotItems,
    pub(super) expires_at_ms: u64,
}

pub(super) fn payload_kind(payload: &SnapshotPayload) -> SnapshotKind {
    match payload {
        SnapshotPayload::Cache(_) => SnapshotKind::Cache,
        SnapshotPayload::Process(_) => SnapshotKind::Process,
        SnapshotPayload::InstalledApps(_) => SnapshotKind::InstalledApps,
        SnapshotPayload::Residue(_) => SnapshotKind::Residue,
        SnapshotPayload::Docker(_) => SnapshotKind::Docker,
    }
}

pub(super) fn dedicated_entry_error(payload: &SnapshotPayload) -> Option<&'static str> {
    match payload {
        SnapshotPayload::Residue(_) => Some("残留快照必须使用专用注册接口"),
        SnapshotPayload::InstalledApps(_) => Some("已安装应用快照必须使用专用注册接口"),
        SnapshotPayload::Docker(_) => Some("Docker 快照必须使用专用注册接口"),
        SnapshotPayload::Cache(_) | SnapshotPayload::Process(_) => None,
    }
}

pub(super) fn keyed_payload(
    payload: SnapshotPayload,
) -> Result<(Vec<String>, SnapshotItems), UserError> {
    match payload {
        SnapshotPayload::Cache(items) => {
            keyed_items(items).map(|(keys, items)| (keys, SnapshotItems::Cache(items)))
        }
        SnapshotPayload::Process(items) => {
            keyed_items(items).map(|(keys, items)| (keys, SnapshotItems::Process(items)))
        }
        SnapshotPayload::InstalledApps(items) => {
            keyed_items(items).map(|(keys, items)| (keys, SnapshotItems::InstalledApps(items)))
        }
        SnapshotPayload::Residue(items) => {
            keyed_items(items).map(|(keys, items)| (keys, SnapshotItems::Residue(items)))
        }
        SnapshotPayload::Docker(items) => {
            keyed_items(items).map(|(keys, items)| (keys, SnapshotItems::Docker(items)))
        }
    }
}

pub(super) fn keyed_items<T>(
    items: Vec<T>,
) -> Result<(Vec<String>, HashMap<String, T>), UserError> {
    let mut selection_keys = Vec::with_capacity(items.len());
    let mut keyed_items = HashMap::with_capacity(items.len());
    for item in items {
        let key = unique_key(&keyed_items)?;
        selection_keys.push(key.clone());
        keyed_items.insert(key, item);
    }
    Ok((selection_keys, keyed_items))
}

type ApplicationKeys = (
    Vec<String>,
    Vec<ApplicationChildBinding>,
    HashMap<String, AppIdentity>,
);

pub(super) fn keyed_applications(apps: Vec<AppIdentity>) -> Result<ApplicationKeys, UserError> {
    let mut app_keys = Vec::with_capacity(apps.len());
    let mut reserved: HashSet<String> = HashSet::new();
    for _ in 0..apps.len() {
        let key = fresh_key(&reserved)?;
        reserved.insert(key.clone());
        app_keys.push(key);
    }
    let mut child_bindings = Vec::new();
    let mut items: HashMap<String, AppIdentity> = HashMap::with_capacity(apps.len());
    for (app, app_key) in apps.into_iter().zip(app_keys.iter()) {
        let child_keys = child_selection_keys(app.processes.len(), &mut reserved)?;
        for (pid, child_key) in app
            .processes
            .iter()
            .map(|target| target.identity.pid)
            .zip(&child_keys)
        {
            child_bindings.push(ApplicationChildBinding {
                app_key: app_key.clone(),
                child_key: child_key.clone(),
                pid,
            });
        }
        items.insert(app_key.clone(), app);
    }
    Ok((app_keys, child_bindings, items))
}

fn child_selection_keys(
    count: usize,
    reserved: &mut HashSet<String>,
) -> Result<Vec<String>, UserError> {
    let mut keys = Vec::with_capacity(count);
    for _ in 0..count {
        let key = fresh_key(reserved)?;
        reserved.insert(key.clone());
        keys.push(key);
    }
    Ok(keys)
}

fn fresh_key(reserved: &HashSet<String>) -> Result<String, UserError> {
    for _ in 0..8 {
        let key = random_id()?;
        if !reserved.contains(&key) {
            return Ok(key);
        }
    }
    Err(UserError::new(
        ErrorCode::SELECTION_KEY_GENERATION_FAILED,
        "生成选择项随机 key 失败",
    ))
}

fn unique_key<T>(items: &HashMap<String, T>) -> Result<String, UserError> {
    for _ in 0..8 {
        let key = random_id()?;
        if !items.contains_key(&key) {
            return Ok(key);
        }
    }
    Err(UserError::new(
        ErrorCode::SELECTION_KEY_GENERATION_FAILED,
        "生成选择项随机 key 失败",
    ))
}

pub(super) fn plan_metadata(plan: &OperationPlan) -> (OperationKind, usize, u64, I18nText) {
    let (kind, count, bytes) = match plan {
        OperationPlan::Cache { items, .. } => {
            let count = items.len();
            let bytes = items
                .iter()
                .fold(0_u64, |total, item| total.saturating_add(item.size_bytes));
            (OperationKind::Cache, count, bytes)
        }
        OperationPlan::Process { targets, .. } => (OperationKind::Process, targets.len(), 0),
        OperationPlan::AppTerminate { targets, .. } => {
            (OperationKind::AppTerminate, targets.len(), 0)
        }
        OperationPlan::AppGracefulQuit { targets } => {
            (OperationKind::AppGracefulQuit, targets.len(), 0)
        }
        OperationPlan::Uninstall { targets, .. } => uninstall_metadata(targets),
        OperationPlan::Docker {
            action,
            targets,
            inventory,
        } => docker_metadata(*action, targets, inventory),
    };
    (kind, count, bytes, summary_for(plan, count, bytes))
}

fn docker_metadata(
    action: DockerAction,
    targets: &[DockerTarget],
    inventory: &DockerInventoryFingerprint,
) -> (OperationKind, usize, u64) {
    if action == DockerAction::Prune {
        let count = inventory
            .resources
            .iter()
            .filter(|target| target.reclaimable)
            .count();
        return (OperationKind::Docker, count, inventory.reclaimable_bytes);
    }
    let bytes = targets.iter().fold(0_u64, |total, target| {
        total.saturating_add(target.size_bytes)
    });
    (OperationKind::Docker, targets.len(), bytes)
}

fn uninstall_metadata(targets: &[UninstallPlanTarget]) -> (OperationKind, usize, u64) {
    let count = targets.iter().fold(0_usize, |total, target| {
        total.saturating_add(1 + target.residues.len())
    });
    let bytes = targets.iter().fold(0_u64, |total, target| {
        total
            .saturating_add(target.app.bundle_size_bytes)
            .saturating_add(residue_bytes(&target.residues))
    });
    (OperationKind::Uninstall, count, bytes)
}

fn residue_bytes(residues: &[ResidueIdentity]) -> u64 {
    residues.iter().fold(0_u64, |total, residue| {
        total.saturating_add(residue.size_bytes)
    })
}

/// 操作计划的展示摘要 —— 返回 **i18n key + 插值参数**，不是文案。
///
/// 铁律：摘要是纯展示字段。
/// - `operation_id` 校验（`operations.rs` 的 owner / 过期 / 单次消费）只看
///   `operation_id` 字段本身，与本函数毫无关系；
/// - 两条测试只断言「summary 里不能泄漏 operation_id」—— key 位天然装不下
///   16 字节随机 hex，这条断言会一直成立。
///
/// 为什么不用「一个模板 + 条件片段」：卸载摘要有「并先退出运行中的应用」这个
/// 可选尾巴，Docker 摘要有 3 种不同动作。塞进模板就得让模板里出现「可能为空」
/// 的参数，那在英文语序下会直接翻车。所以**每个变体一条 key**，条件走 key 选择。
fn summary_for(plan: &OperationPlan, count: usize, bytes: u64) -> I18nText {
    let size = human_size(bytes);
    let counted = |key: &str, items: usize| {
        I18nText::with(key, vec![param("count", items), param("size", &size)])
    };
    match plan {
        OperationPlan::Cache { .. } => counted("opSummary.cache", count),
        OperationPlan::Process { targets, mode } => {
            let key = if *mode == ProcessMode::Force {
                "opSummary.processForce"
            } else {
                "opSummary.processGraceful"
            };
            I18nText::with(key, vec![param("count", targets.len())])
        }
        OperationPlan::AppTerminate { targets, .. } => {
            app_summary("opSummary.appTerminate", targets)
        }
        OperationPlan::AppGracefulQuit { targets } => {
            app_summary("opSummary.appGracefulQuit", targets)
        }
        OperationPlan::Uninstall {
            targets,
            quit_running,
        } => uninstall_summary(targets, *quit_running, size),
        OperationPlan::Docker {
            action, targets, ..
        } => docker_summary(*action, targets, count, &size),
    }
}

/// 「终止/退出 N 个应用、共 M 个进程」两条摘要共用的参数集。
///
/// 两条只有 key 不同 —— 合并成一个函数是为了让「应用数 + 进程数」这两个参数的
/// 口径只有一处，界面上两个数字对不上同一个应用集合这类 bug 就不可能发生。
fn app_summary(key: &str, targets: &[AppIdentity]) -> I18nText {
    I18nText::with(
        key,
        vec![
            param("apps", targets.len()),
            param("processes", process_total(targets)),
        ],
    )
}

/// Docker 摘要。Prune 的口径是「可回收资源数」，其余是「选中项数」，不能混。
fn docker_summary(
    action: DockerAction,
    targets: &[DockerTarget],
    reclaimable: usize,
    size: &str,
) -> I18nText {
    if action == DockerAction::Prune {
        return I18nText::with(
            "opSummary.dockerPrune",
            vec![param("count", reclaimable), param("size", size)],
        );
    }
    let key = match action {
        DockerAction::RemoveImage => "opSummary.dockerRemoveImage",
        DockerAction::RemoveContainer => "opSummary.dockerRemoveContainer",
        DockerAction::RemoveVolume => "opSummary.dockerRemoveVolume",
        DockerAction::Prune => unreachable!("Prune 分支已在上面返回"),
    };
    I18nText::with(
        key,
        vec![param("count", targets.len()), param("size", size)],
    )
}

fn uninstall_summary(
    targets: &[UninstallPlanTarget],
    quit_running: bool,
    size: String,
) -> I18nText {
    let residues = targets
        .iter()
        .fold(0_usize, |total, target| total + target.residues.len());
    // 条件尾巴走**不同的 key**，不在模板里留空位 —— 见 `summary_for` 的说明。
    let key = if quit_running {
        "opSummary.uninstallQuitFirst"
    } else {
        "opSummary.uninstall"
    };
    I18nText::with(
        key,
        vec![
            param("apps", targets.len()),
            param("residues", residues),
            param("size", &size),
        ],
    )
}

/// 插值参数。参数名必须是纯 ASCII —— 值是要显示的内容本身（数字 / 体积），
/// **不是文案**：文案由前端词典按 key 决定。
fn param(name: &str, value: impl ToString) -> (String, String) {
    (name.to_owned(), value.to_string())
}

fn process_total(apps: &[AppIdentity]) -> usize {
    apps.iter()
        .fold(0_usize, |total, app| total + app.processes.len())
}

fn human_size(bytes: u64) -> String {
    const UNITS: [(&str, u64); 4] = [
        ("TB", 1024 * 1024 * 1024 * 1024),
        ("GB", 1024 * 1024 * 1024),
        ("MB", 1024 * 1024),
        ("KB", 1024),
    ];
    for (unit, factor) in UNITS {
        if bytes >= factor {
            let scaled = bytes as f64 / factor as f64;
            return format!("{scaled:.1} {unit}");
        }
    }
    format!("{bytes} B")
}

pub(super) fn normalize_keys<K>(keys: K) -> Result<Vec<String>, UserError>
where
    K: IntoIterator,
    K::Item: AsRef<str>,
{
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for key in keys {
        let key = key.as_ref();
        if key.trim().is_empty() {
            return Err(UserError::new(ErrorCode::SELECTION_EMPTY, "选择不能为空"));
        }
        if !seen.insert(key.to_owned()) {
            return Err(UserError::new(
                ErrorCode::SELECTION_DUPLICATED,
                "选择项不能重复",
            ));
        }
        normalized.push(key.to_owned());
    }
    Ok(normalized)
}

pub(super) fn require_non_empty(keys: Vec<String>) -> Result<Vec<String>, UserError> {
    if keys.is_empty() {
        return Err(UserError::new(ErrorCode::SELECTION_EMPTY, "选择不能为空"));
    }
    Ok(keys)
}

pub(crate) fn ensure_targets_terminable(
    targets: &[ProcessTarget],
    mode: ProcessMode,
) -> Result<(), UserError> {
    for target in targets {
        if target.whitelisted {
            return Err(UserError::new(
                ErrorCode::WHITELISTED_PROCESS_CANNOT_TERMINATE,
                "白名单进程不能终止",
            ));
        }
        if target.protected && mode != ProcessMode::Force {
            return Err(UserError::new(
                ErrorCode::PROTECTED_FORCE_ONLY,
                "受保护进程只能强制终止",
            ));
        }
    }
    Ok(())
}

fn missing_selection() -> UserError {
    UserError::new(ErrorCode::SELECTION_MISSING, "选择项不存在")
}

fn random_id_failed() -> UserError {
    UserError::new(ErrorCode::RANDOM_ID_FAILED, "生成随机 ID 失败")
}

pub(super) fn ensure_app_quit_allowed(targets: &[AppIdentity]) -> Result<(), UserError> {
    for app in targets {
        if app.processes.is_empty() {
            return Err(UserError::new(
                ErrorCode::APP_NO_QUITTABLE_PROCESS,
                "目标应用没有可退出的进程，请重新扫描",
            ));
        }
        if app.processes.iter().any(|target| target.whitelisted) {
            return Err(UserError::new(
                ErrorCode::WHITELISTED_APP_NOT_QUIT,
                "白名单进程不会被退出",
            ));
        }
    }
    Ok(())
}

pub(super) fn select_items<T: Clone>(
    items: &HashMap<String, T>,
    keys: &[String],
) -> Result<Vec<T>, UserError> {
    keys.iter()
        .map(|key| items.get(key).cloned().ok_or_else(missing_selection))
        .collect()
}

pub(super) fn expiry(now: u64) -> u64 {
    now.saturating_add(super::SNAPSHOT_TTL_MS)
}

pub(super) fn random_id() -> Result<String, UserError> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| random_id_failed())?;
    let mut id = String::with_capacity(64);
    for byte in bytes {
        write!(&mut id, "{byte:02x}").map_err(|_| random_id_failed())?;
    }
    Ok(id)
}

pub(super) fn system_time_ms() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    u64::try_from(millis).unwrap_or(u64::MAX)
}
