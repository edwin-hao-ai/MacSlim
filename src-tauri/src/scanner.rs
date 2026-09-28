use crate::i18n_text::{split_optional, I18nParams, I18nText};
use crate::operations::{OperationStore, ProcessTarget, SnapshotRegistration};
use crate::process_safety::{
    collect_parent_pids, evaluate_protection, is_multiprocess_family, is_same_user, safety_veto,
    ProcessProtection, SafetyVeto,
};
use crate::user_error::UserError;
use crate::whitelist::is_whitelisted;
use serde::Serialize;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::PathBuf;
use sysinfo::{Disks, Process, ProcessStatus, System};

#[derive(Serialize, Clone, Debug)]
pub struct SystemHealth {
    pub cpu_percent: f32,
    pub memory_used_mb: f64,
    pub memory_total_mb: f64,
    pub memory_percent: f32,
    pub disk_used_gb: f64,
    pub disk_total_gb: f64,
    pub disk_percent: f32,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub exe: String,
    pub start_time: u64,
    pub cpu_percent: f32,
    pub memory_mb: f64,
    pub kind: ProcessKind,
    pub risk: Risk,
    pub default_select: bool,
    /// 分类理由的 i18n key（纯 ASCII），译文在前端词典里。
    pub reason_key: String,
    /// 分类理由的插值参数（参数名纯 ASCII，值是要显示的内容本身）。
    pub reason_params: I18nParams,
    pub ports: Vec<u16>,
    pub icon_base64: Option<String>,
    pub selection_key: String,
    pub protected: bool,
    /// 受保护原因（系统核心 / 多进程族父进程 / 有子进程 / 年轻进程 / 白名单）。
    /// 受保护的行前端必须显示出来，并且不能被默认选中。
    ///
    /// 存的是 key 不是文案：`protected` 才是判定输入，本字段纯展示。
    pub protected_reason_key: Option<String>,
    pub protected_reason_params: I18nParams,
    pub whitelisted: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ProcessKind {
    /// 僵尸进程（父进程已退出，内核未回收）
    Zombie,
    /// 长期闲置（低 CPU + 中等内存 + 老进程）
    Idle,
    /// 资源大户（CPU 或内存显著）
    Hog,
    /// 开发工具闲置
    Dev,
    System,
    Foreground,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// 100% 安全 —— 只有僵尸进程能拿到这个等级
    Safe,
    /// 低风险 —— 大概率可清理，但保守起见不默认选中
    Low,
    /// 开发工具 —— 用户需主动判断
    Dev,
    /// 不显示给用户
    Hidden,
}

#[derive(Serialize, Clone, Debug)]
pub struct ScanResult {
    pub health: SystemHealth,
    pub processes: Vec<ProcessInfo>,
    pub scanned_at_ms: u64,
}

/// bundle id 形态的系统进程名 → 用户可读名称的 **i18n key**。
/// 只收已知条目；未命中时调用方取最后一段，不做启发式拼接。
///
/// 第二列是 key，不是文案 —— 译文在 `src/i18n/*.ts`。
const KNOWN_BUNDLE_ID_PROCESSES: &[(&str, &str)] = &[
    (
        "com.apple.WebKit.WebContent",
        "process.name.webkitWebContent",
    ),
    (
        "com.apple.WebKit.Networking",
        "process.name.webkitNetworking",
    ),
    ("com.apple.WebKit.GPU", "process.name.webkitGpu"),
];

/// 展示用进程名。判定逻辑必须继续使用 `proc.name()` 的原始值。
///
/// 返回 `(展示名, 展示名的 i18n key)`：
/// - `key` 为 `Some` 时前端必须用它替换展示名（WebKit 三个固定映射）；
/// - `key` 为 `None` 时展示名就是真实进程名 / bundle 名，原样显示。
///
/// 铁律：**这里返回的名字绝不能被塞进 `ProcessIdentity.name`**。
/// `revalidate_targets` 会把它与现场 `proc.name()` 逐字比较。
pub fn display_name_for_process(
    raw_name: &str,
    bundle_display_name: Option<String>,
) -> (String, Option<String>) {
    if let Some(name) = bundle_display_name {
        if !name.trim().is_empty() {
            // 子进程通常在 bundle 名后带角色后缀（Helper (Renderer) / Helper (GPU)），
            // 直接用 bundle 名会把主进程和所有 helper 显示成同一个名字。
            return if raw_name.len() > name.len() && raw_name.starts_with(&name) {
                (raw_name.to_string(), None)
            } else {
                (name, None)
            };
        }
    }
    if let Some((_, key)) = KNOWN_BUNDLE_ID_PROCESSES
        .iter()
        .find(|(raw, _)| *raw == raw_name)
    {
        // `name` 里放**原始进程名**而不是 key：万一前端漏翻，用户看到的是
        // 「com.apple.WebKit.WebContent」这种不好看但无害的东西，而不是一个
        // 裸 i18n key。翻译与否由 `name_key` 决定。
        return (raw_name.to_string(), Some((*key).to_string()));
    }
    if raw_name.contains('.') && !raw_name.starts_with('.') && !raw_name.ends_with('.') {
        if let Some(last) = raw_name.rsplit('.').next() {
            if !last.is_empty() {
                return (last.to_string(), None);
            }
        }
    }
    (raw_name.to_string(), None)
}

/// 进程管理视图用的行数据，信息比 scan 更丰富
#[derive(Serialize, Clone, Debug)]
pub struct ProcessRow {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    /// 给用户看的可读名（bundle 名 / bundle id 末段）。
    ///
    /// 铁律：这个字段是**展示用**的原始名，绝不能被塞进 `ProcessIdentity.name`；
    /// 那个位置必须是 `full_name`。
    pub name: String,
    /// `name` 的 i18n key（目前只有 WebKit 三个固定映射）。`Some` 时前端必须
    /// 用译文替换 `name`；`None` 时 `name` 原样显示。
    pub name_key: Option<String>,
    /// 未经解析的原始进程名，用于长名称 tooltip 与问题排查
    pub full_name: String,
    pub exe: String,
    pub start_time: u64,
    pub cpu_percent: f32,
    pub memory_mb: f64,
    pub uptime_secs: u64,
    /// 进程状态的 i18n key（`process.status.*`），不是中文状态名。
    pub status_key: String,
    pub ports: Vec<u16>,
    pub icon_base64: Option<String>,
    /// 是否受保护（系统核心 / 多进程族父进程 / 有子进程 / 跨用户）
    /// 受保护的进程用户能看到但终止按钮禁用
    pub protected: bool,
    /// 受保护原因的 i18n key（`process.protect.*`）+ 插值参数，纯展示。
    pub protected_reason_key: Option<String>,
    pub protected_reason_params: I18nParams,
    /// 是否在用户白名单（基于名字）
    pub whitelisted: bool,
    pub selection_key: String,
}

pub(crate) fn process_targets_from_rows(rows: &[ProcessRow]) -> Vec<ProcessTarget> {
    rows.iter().map(process_target_from_row).collect()
}

fn process_target_from_row(row: &ProcessRow) -> ProcessTarget {
    ProcessTarget {
        identity: crate::operations::ProcessIdentity {
            pid: row.pid,
            // 必须用原始名：`revalidate_targets` 会在终止前把这里登记的名字
            // 与现场 `proc.name()` 逐字比较，用展示名会让一键清理全部报
            // 「进程身份已变化」。
            name: row.full_name.clone(),
            exe: row.exe.clone(),
            start_time: row.start_time,
        },
        protected: row.protected,
        whitelisted: row.whitelisted,
    }
}

pub fn register_process_rows(
    store: &mut OperationStore,
    rows: &mut [ProcessRow],
) -> Result<SnapshotRegistration, UserError> {
    let registration = store.register_process_snapshot(process_targets_from_rows(rows))?;
    for (row, key) in rows.iter_mut().zip(registration.selection_keys.iter()) {
        row.selection_key = key.clone();
    }
    Ok(registration)
}

/// bundle 路径 → `Info.plist` 里的 CFBundleDisplayName / CFBundleName，带调用内 memo。
///
/// 二进制 plist 会 fork `plutil`，`list_all` 遍历全量进程时同一 bundle 的每个子进程
/// 都重读一次 Info.plist 就是几百次进程创建，所以按 bundle 路径缓存（`None` 也缓存，
/// 同一 bundle 内不会变）。memo 生命周期限定在单次 `list_all` 调用内，不做全局缓存
/// —— 应用名可能随时被用户改。
fn bundle_display_name(
    bundle: PathBuf,
    cache: &mut HashMap<PathBuf, Option<String>>,
) -> Option<String> {
    match cache.entry(bundle) {
        Entry::Occupied(entry) => entry.get().clone(),
        Entry::Vacant(entry) => {
            let plist = entry.key().join("Contents/Info.plist");
            let display = crate::applications::read_plist_metadata(&plist).0;
            entry.insert(display.clone()).clone()
        }
    }
}

pub(crate) fn scan_targets_from_infos(processes: &[ProcessInfo]) -> Vec<ProcessTarget> {
    processes.iter().map(scan_target_from_info).collect()
}

fn scan_target_from_info(info: &ProcessInfo) -> ProcessTarget {
    ProcessTarget {
        identity: crate::operations::ProcessIdentity {
            pid: info.pid,
            name: info.name.clone(),
            exe: info.exe.clone(),
            start_time: info.start_time,
        },
        protected: info.protected,
        whitelisted: info.whitelisted,
    }
}

pub fn register_scan_processes(
    store: &mut OperationStore,
    processes: &mut [ProcessInfo],
) -> Result<SnapshotRegistration, UserError> {
    let registration =
        store.register_overview_process_snapshot(scan_targets_from_infos(processes))?;
    for (info, key) in processes.iter_mut().zip(registration.selection_keys.iter()) {
        info.selection_key = key.clone();
    }
    Ok(registration)
}

pub fn read_health(sys: &mut System) -> SystemHealth {
    sys.refresh_cpu_all();
    sys.refresh_memory();

    let cpu_percent = sys.global_cpu_usage();
    let mem_total = sys.total_memory() as f64 / 1024.0 / 1024.0;

    // macOS 内存优化：用 vm_stat 区分「应用内存」和「系统缓存」
    // sysinfo 的 used_memory 包含了文件缓存，对普通用户有误导
    let (app_mem_mb, mem_pct) = macos_app_memory(mem_total);
    let mem_used = app_mem_mb; // 只显示应用真正占用的内存

    let disks = Disks::new_with_refreshed_list();
    let (disk_total, disk_avail) = disks
        .iter()
        .filter(|d| d.mount_point().to_string_lossy() == "/")
        .map(|d| (d.total_space(), d.available_space()))
        .next()
        .unwrap_or((0, 0));
    let disk_used = disk_total.saturating_sub(disk_avail);
    let gb = 1024u64.pow(3) as f64;
    let disk_pct = if disk_total > 0 {
        (disk_used as f64 / disk_total as f64 * 100.0) as f32
    } else {
        0.0
    };

    SystemHealth {
        cpu_percent,
        memory_used_mb: mem_used,
        memory_total_mb: mem_total,
        memory_percent: mem_pct,
        disk_used_gb: disk_used as f64 / gb,
        disk_total_gb: disk_total as f64 / gb,
        disk_percent: disk_pct,
    }
}

/// 列出当前用户所有进程，不做分类过滤，供进程管理页使用。
pub fn list_all(sys: &mut System) -> Vec<ProcessRow> {
    sys.refresh_all();
    std::thread::sleep(std::time::Duration::from_millis(200));
    sys.refresh_cpu_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let parent_pids = collect_parent_pids(sys);
    let mut bundle_display_names: HashMap<PathBuf, Option<String>> = HashMap::new();
    let mut out: Vec<ProcessRow> = Vec::new();

    for (pid, proc) in sys.processes() {
        // 刻意叫 `raw_name` 而不是 `name`：白名单与保护判定必须拿到 `proc.name()`
        // 的**原始值**，而展示名是另一条独立的派生路径。让这个绑定不可能被
        // 「顺手重绑定」覆盖，是判定输入不变式的结构性保证。
        let raw_name = proc.name().to_string_lossy().to_string();

        // 跨用户（如 root 服务）跳过 —— 用户根本无权终止
        if !is_same_user(proc) {
            continue;
        }
        // 低 PID 系统进程跳过
        if pid.as_u32() < 50 {
            continue;
        }

        let whitelisted = is_whitelisted(&raw_name);
        let protection = evaluate_protection(proc, &raw_name, &parent_pids);
        let protected = protection.protected;
        let (protected_reason_key, protected_reason_params) = split_optional(&protection.reason);

        let exe = proc
            .exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let status_key = status_key_for(proc.status());

        // 判定必须继续使用上面的原始 raw_name；这里只计算展示名。
        let bundle_name = proc
            .exe()
            .and_then(crate::applications::find_app_bundle)
            .and_then(|bundle| bundle_display_name(bundle, &mut bundle_display_names));
        let (name, name_key) = display_name_for_process(&raw_name, bundle_name);

        out.push(ProcessRow {
            pid: pid.as_u32(),
            parent_pid: proc.parent().map(|p| p.as_u32()),
            full_name: raw_name.clone(),
            name,
            name_key,
            exe,
            start_time: proc.start_time(),
            cpu_percent: proc.cpu_usage(),
            memory_mb: proc.memory() as f64 / 1024.0 / 1024.0,
            uptime_secs: crate::process_safety::process_uptime_secs(proc),
            status_key,
            ports: Vec::new(),
            icon_base64: proc.exe().and_then(|exe_path| {
                let bundle = crate::applications::find_app_bundle(exe_path)?;
                crate::app_scanner::read_icon_base64_for_bundle(&bundle)
            }),
            protected,
            protected_reason_key,
            protected_reason_params,
            whitelisted,
            selection_key: String::new(),
        });
    }

    // 附加端口信息
    let pids: Vec<u32> = out.iter().map(|p| p.pid).collect();
    let port_map = crate::ports::ports_by_pid(&pids);
    for p in out.iter_mut() {
        if let Some(ports) = port_map.get(&p.pid) {
            p.ports = ports.clone();
        }
    }

    // 默认按内存降序 —— 让用户一眼看到大户
    out.sort_by(|a, b| {
        b.memory_mb
            .partial_cmp(&a.memory_mb)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// 进程状态 → `process.status.*` 的 i18n key。
///
/// **不是文案**：译文在 `src/i18n/*.ts`。前端拿这个 key 做展示，也拿它做
/// 「僵尸 / 已死才打危险徽章」这类条件判断 —— 用 key 判断而不是中文子串，
/// 是为了让展示语言与判定彻底解耦（英文界面下徽章也照常出现）。
///
/// 映射必须与改造前逐条一致：13 个分支一个不多一个不少，`_` 兜底仍是「未知」。
fn status_key_for(status: ProcessStatus) -> String {
    let key = match status {
        ProcessStatus::Idle => "process.status.idle",
        ProcessStatus::Run => "process.status.run",
        ProcessStatus::Sleep => "process.status.sleep",
        ProcessStatus::Stop => "process.status.stop",
        ProcessStatus::Zombie => "process.status.zombie",
        ProcessStatus::Tracing => "process.status.tracing",
        ProcessStatus::Dead => "process.status.dead",
        ProcessStatus::Wakekill => "process.status.wakekill",
        ProcessStatus::Waking => "process.status.waking",
        ProcessStatus::Parked => "process.status.parked",
        ProcessStatus::LockBlocked => "process.status.lockBlocked",
        ProcessStatus::UninterruptibleDiskSleep => "process.status.diskSleep",
        _ => "process.status.unknown",
    };
    key.to_owned()
}

pub fn scan(sys: &mut System) -> ScanResult {
    sys.refresh_all();
    // sysinfo 在 macOS 上需要两次采样间隔才能算出准确的 CPU 使用率
    // 200ms 有时不够，提高到 400ms 确保数据稳定
    std::thread::sleep(std::time::Duration::from_millis(400));
    sys.refresh_cpu_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let health = read_health(sys);
    let processes = classify_processes(sys);

    let scanned_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    ScanResult {
        health,
        processes,
        scanned_at_ms,
    }
}

/// 进程分类（保守原则）
///
/// 流程：
/// 1. 跳过系统核心、白名单、低 PID、非当前用户的进程
/// 2. 进入安全审计层（process_safety::safety_veto）—— 多进程族 / 父进程 / 年轻进程全部淘汰
/// 3. 剩余进程按状态分类：僵尸 > 开发工具闲置 > 资源大户 > 长期闲置
/// 4. **只有通过安全审计的僵尸进程**能 default_select = true
///
/// 铁律：**受保护或白名单进程永远不能被默认选中**。
/// 任何 `default_select = true` 的分支都必须先过 `default_selectable` ——
/// 主 CTA 会把默认选择整批送进 graceful 终止，一个受保护目标就会让整批失败。
fn default_selectable(protection: &ProcessProtection) -> bool {
    !protection.protected && !protection.whitelisted
}

/// 保护闸门：受保护/白名单目标一律取消默认选中。
///
/// 「受保护（原因）」这一段**不再由后端拼进 reason**：它完全由前端从
/// `protected` + `protected_reason_key` 拼出来（`ProcessList.tsx`）。后端只需要
/// 守住唯一有安全语义的那一步 —— 取消默认选中。
///
/// 铁律：这个函数只许改 `default_select`。任何「顺手把文案也改了」的修改都可能
/// 连带改动别的分支的默认选中行为。
fn apply_protection_gate(classification: &mut Classification, protection: &ProcessProtection) {
    if default_selectable(protection) {
        return;
    }
    classification.default_select = false;
}

/// 僵尸分类：**先看安全审计再决定能不能默认选中**。
///
/// 触发 veto 的僵尸（例如刚启动不足 10 分钟、或本身是别人的父进程）
/// 降级为「展示但不可默认选中」，而不是 100% 可清理。
///
/// 被 veto 的僵尸按**具体否决种类**选 key（而不是运行时拼句），这样
/// 「僵尸进程（<原因>），默认不选中」这句话的每个变体都是一条可被前端门禁
/// 核对的显式 key，不需要任何嵌套插值约定。
fn zombie_classification(vetoed: Option<SafetyVeto>) -> Classification {
    Classification {
        kind: ProcessKind::Zombie,
        risk: if vetoed.is_some() {
            Risk::Low
        } else {
            Risk::Safe
        },
        default_select: vetoed.is_none(),
        reason: match vetoed {
            None => I18nText::plain("process.reason.zombie"),
            Some(SafetyVeto::ParentOfOthers) => {
                I18nText::plain("process.reason.zombieVetoedParentOfOthers")
            }
            Some(SafetyVeto::MultiProcessComponent) => {
                I18nText::plain("process.reason.zombieVetoedMultiProcessComponent")
            }
            Some(SafetyVeto::YoungProcess) => {
                I18nText::plain("process.reason.zombieVetoedYoungProcess")
            }
        },
    }
}

fn classify_processes(sys: &System) -> Vec<ProcessInfo> {
    let parent_pids = collect_parent_pids(sys);
    let total_mem_mb = sys.total_memory() as f64 / 1024.0 / 1024.0;
    let mut out: Vec<ProcessInfo> = Vec::new();

    for (pid, proc) in sys.processes() {
        let name = proc.name().to_string_lossy().to_string();

        if is_whitelisted(&name) {
            continue;
        }
        if pid.as_u32() < 100 {
            continue;
        }
        if !is_same_user(proc) {
            continue;
        }

        let protection = evaluate_protection(proc, &name, &parent_pids);
        let mut classification = classify_one(proc, &name, &parent_pids, total_mem_mb);
        // Hidden 不展示
        if classification.risk == Risk::Hidden {
            continue;
        }
        apply_protection_gate(&mut classification, &protection);

        let exe = proc
            .exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let (protected_reason_key, protected_reason_params) = split_optional(&protection.reason);
        out.push(ProcessInfo {
            pid: pid.as_u32(),
            name,
            exe,
            start_time: proc.start_time(),
            cpu_percent: proc.cpu_usage(),
            memory_mb: proc.memory() as f64 / 1024.0 / 1024.0,
            kind: classification.kind,
            risk: classification.risk,
            default_select: classification.default_select,
            reason_key: classification.reason.key,
            reason_params: classification.reason.params,
            ports: Vec::new(),
            icon_base64: proc.exe().and_then(|exe_path| {
                let bundle = crate::applications::find_app_bundle(exe_path)?;
                crate::app_scanner::read_icon_base64_for_bundle(&bundle)
            }),
            selection_key: String::new(),
            protected: protection.protected,
            protected_reason_key,
            protected_reason_params,
            whitelisted: protection.whitelisted,
        });
    }

    // 按内存降序
    out.sort_by(|a, b| {
        b.memory_mb
            .partial_cmp(&a.memory_mb)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(40);

    // 端口占用
    let pids: Vec<u32> = out.iter().map(|p| p.pid).collect();
    let port_map = crate::ports::ports_by_pid(&pids);
    for p in out.iter_mut() {
        if let Some(ports) = port_map.get(&p.pid) {
            p.ports = ports.clone();
            // 铁律：有监听端口的一律不默认选中（可能是运行中的服务）
            p.default_select = false;
            // 「端口 N（运行中的服务，请确认）」这一段由前端按 `ports` 拼装
            // （`ProcessList.tsx`），后端不再把端口写进 reason 文案。
        }
    }

    out
}

struct Classification {
    kind: ProcessKind,
    risk: Risk,
    default_select: bool,
    /// 分类理由的待翻译文案（i18n key + 参数）。隐藏分支用空 key。
    reason: I18nText,
}

/// 「资源占用类」分类的语义标签。
///
/// 这些 reason **只讲语义、不带 CPU 百分比与内存 MB 数值**：扫描页行尾已经单独
/// 渲染 `{cpu_percent}% CPU` 与 `{memory_mb}MB` 两列（`ProcessList.tsx`），
/// reason 再写一遍就是同一屏出现两遍同一个数。
#[derive(Debug, Clone, Copy)]
enum ResourceNote {
    /// CPU 显著偏高
    HighCpu,
    /// 内存达到 hog 门槛
    HighMemory,
    /// 长期低活动。运行时长**不在扫描页任何列里**（只有进程管理页有 uptime 列），
    /// 属于新增信息，保留；内存是重复，去掉。
    Idle { uptime_min: u64 },
    /// 开发工具进程、内存偏高
    DevToolHighMemory,
    /// 未通过安全审计（父进程 / 多进程族）但占用达门槛，`variant` 说明是哪一类
    VetoedHighUsage { variant: VetoedHighUsageVariant },
}

/// 「未通过安全审计但占用达门槛」的两种进程身份。
///
/// 过去是一个 `hint: &'static str` 存中文片段，再和后半句 `format!` 起来。
/// 那样拼出来的句子没法被任何门禁核对（key 里会含中文），所以拆成两个显式 key。
#[derive(Debug, Clone, Copy)]
enum VetoedHighUsageVariant {
    /// 多进程架构应用的组件
    MultiProcessComponent,
    /// 别的应用的主进程
    MainProcess,
}

impl VetoedHighUsageVariant {
    fn i18n_key(self) -> &'static str {
        match self {
            Self::MultiProcessComponent => "process.reason.vetoedHighUsageMultiProcess",
            Self::MainProcess => "process.reason.vetoedHighUsageMainProcess",
        }
    }
}

/// 把资源占用语义标签映射成 reason 的待翻译文案。
///
/// 刻意做成纯函数、不依赖 `sysinfo::Process`：走 `classify_one` 需要真实进程，
/// 阈值分支在测试机上未必被命中，纯 live-scan 护栏会**空转放行**回退
/// （已实测：内存 hog 分支改回带数值时 live 护栏仍然绿）。
fn resource_reason(note: ResourceNote) -> I18nText {
    match note {
        ResourceNote::HighCpu => I18nText::plain("process.reason.highCpu"),
        ResourceNote::HighMemory => I18nText::plain("process.reason.highMemory"),
        ResourceNote::Idle { uptime_min } => {
            I18nText::one("process.reason.idle", "uptime_min", uptime_min)
        }
        ResourceNote::DevToolHighMemory => I18nText::plain("process.reason.devToolHighMemory"),
        ResourceNote::VetoedHighUsage { variant } => I18nText::plain(variant.i18n_key()),
    }
}

/// 隐藏分支的空 reason：key 为空串，且这类行根本不会被序列化出去
/// （`classify_processes` 在 `Risk::Hidden` 时 `continue`）。
fn hidden_classification() -> Classification {
    Classification {
        kind: ProcessKind::Foreground,
        risk: Risk::Hidden,
        default_select: false,
        reason: I18nText::default(),
    }
}

fn idle_memory_threshold(total_mem_mb: f64) -> f64 {
    (total_mem_mb * 0.01).max(80.0)
}

fn classify_one(
    proc: &Process,
    name: &str,
    parent_pids: &std::collections::HashSet<u32>,
    total_mem_mb: f64,
) -> Classification {
    let cpu = proc.cpu_usage();
    let mem_mb = proc.memory() as f64 / 1024.0 / 1024.0;
    let status = proc.status();

    // 动态门槛：按系统总内存的百分比
    // 8GB 机器上 hog=200MB, idle=80MB，让更多内存大户可见
    let hog_mem_threshold = (total_mem_mb * 0.025).max(200.0);

    // 安全审计必须先于任何 default_select：先算出 veto，再决定能不能默认选中
    let vetoed = safety_veto(proc, name, parent_pids);
    let is_mp_family = is_multiprocess_family(name);

    // —— 第一优先：僵尸进程；只有通过安全审计的僵尸才允许默认选中 ——
    if matches!(status, ProcessStatus::Zombie) {
        return zombie_classification(vetoed);
    }

    if let Some(veto_kind) = vetoed {
        // 年轻进程 —— 不可能是清理目标，隐藏。
        //
        // 过去这里是 `veto_reason.contains("不足") || veto_reason.contains("进程刚启动")`
        // —— **用中文字符串做业务分支判断**。改文案就会悄悄改行为。枚举化之后
        // 判断的是「否决种类」，命中集合与过去逐字等价（当时也只有「进程刚启动
        // 不足 10 分钟…」这一条含「不足」/「进程刚启动」）。
        if veto_kind == SafetyVeto::YoungProcess {
            return hidden_classification();
        }

        // 父进程 / 多进程族 —— 仍然展示（用户需要感知内存占用），但固定 Low 风险 + 不默认选中
        // 内存或 CPU 达到展示门槛，让用户能看到内存大户
        if mem_mb >= 100.0 || cpu >= 10.0 {
            let variant = if is_mp_family {
                VetoedHighUsageVariant::MultiProcessComponent
            } else {
                VetoedHighUsageVariant::MainProcess
            };
            return Classification {
                kind: ProcessKind::Hog,
                risk: Risk::Low,
                default_select: false,
                reason: resource_reason(ResourceNote::VetoedHighUsage { variant }),
            };
        }
        return hidden_classification();
    }

    // —— 以下都通过了安全审计 ——

    let exe = proc
        .exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let is_dev = exe.contains("/node_modules/")
        || matches!(
            name,
            "node" | "python" | "python3" | "ruby" | "java" | "rustc" | "go" | "bun" | "deno"
        );
    // 开发工具进程如果内存占用大，优先标记为 Hog（让用户看到内存大户）
    if is_dev && mem_mb >= 200.0 {
        return Classification {
            kind: ProcessKind::Hog,
            risk: Risk::Low,
            default_select: false,
            reason: resource_reason(ResourceNote::DevToolHighMemory),
        };
    }
    if is_dev {
        return Classification {
            kind: ProcessKind::Dev,
            risk: Risk::Dev,
            default_select: false,
            reason: I18nText::plain("process.reason.devTool"),
        };
    }

    // 资源大户：显著 CPU 或内存占用 —— Low 风险，不默认选中
    if cpu > 20.0 {
        return Classification {
            kind: ProcessKind::Hog,
            risk: Risk::Low,
            default_select: false,
            reason: resource_reason(ResourceNote::HighCpu),
        };
    }
    if mem_mb >= hog_mem_threshold {
        return Classification {
            kind: ProcessKind::Hog,
            risk: Risk::Low,
            default_select: false,
            reason: resource_reason(ResourceNote::HighMemory),
        };
    }

    // 长期闲置：低活动 + 中等内存 + 已运行较久（动态内存门槛 + 20 分钟）
    let uptime_min = crate::process_safety::process_uptime_secs(proc) / 60;
    if cpu < 0.5 && mem_mb >= idle_memory_threshold(total_mem_mb) && uptime_min > 20 {
        return Classification {
            kind: ProcessKind::Idle,
            risk: Risk::Low,
            default_select: false,
            reason: resource_reason(ResourceNote::Idle { uptime_min }),
        };
    }

    // 其他都隐藏
    hidden_classification()
}

/// macOS 内存分类：解析 vm_stat 输出，返回（应用内存 MB, 使用百分比）
/// 应用内存 = wired + (active - purgeable) + compressed
/// 这和活动监视器的「已使用内存」一致，排除了文件缓存
fn macos_app_memory(total_mb: f64) -> (f64, f32) {
    let output = std::process::Command::new("vm_stat")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok());
    let Some(text) = output else {
        return (total_mb * 0.5, 50.0); // 降级
    };

    let page_size = parse_vm_stat_page_size(&text).unwrap_or(16384) as f64;
    let pages = |key: &str| -> f64 { parse_vm_stat_value(&text, key).unwrap_or(0) as f64 };

    let wired = pages("Pages wired down") * page_size;
    let active = pages("Pages active") * page_size;
    let compressed = pages("Pages occupied by compressor") * page_size;
    let purgeable = pages("Pages purgeable") * page_size;

    // 应用内存 = wired + (active - purgeable) + compressed
    let app_bytes = wired + (active - purgeable).max(0.0) + compressed;
    let app_mb = app_bytes / 1024.0 / 1024.0;
    let pct = if total_mb > 0.0 {
        (app_mb / total_mb * 100.0) as f32
    } else {
        0.0
    };
    (app_mb, pct)
}

fn parse_vm_stat_page_size(text: &str) -> Option<u64> {
    // 第一行: "Mach Virtual Memory Statistics: (page size of 16384 bytes)"
    let line = text.lines().next()?;
    let start = line.find("page size of ")? + "page size of ".len();
    let end = line[start..].find(' ')? + start;
    line[start..end].parse().ok()
}

fn parse_vm_stat_value(text: &str, key: &str) -> Option<u64> {
    for line in text.lines() {
        if line.starts_with(key) {
            let val = line.split(':').nth(1)?.trim().trim_end_matches('.');
            return val.parse().ok();
        }
    }
    None
}

#[cfg(test)]
#[path = "scanner_tests.rs"]
mod tests;
