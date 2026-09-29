// 应用扫描器：枚举已安装应用，计算大小，读取元数据
use crate::applications::{plist_string, read_plist_map, read_plist_metadata};
use crate::operations::{InstalledAppIdentity, OperationStore, SnapshotRegistration};
use crate::user_error::{ErrorCode, UserError};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use sysinfo::System;

/// 系统核心应用白名单（Bundle ID），默认隐藏不出现在卸载列表
const SYSTEM_CORE_APPS: &[&str] = &[
    "com.apple.finder",
    "com.apple.Safari",
    "com.apple.AppStore",
    "com.apple.systempreferences",
    "com.apple.SystemPreferences",
    "com.apple.Terminal",
    "com.apple.mail",
    "com.apple.iCal",
    "com.apple.AddressBook",
    "com.apple.Photos",
    "com.apple.iWork.Keynote",
    "com.apple.iWork.Pages",
    "com.apple.iWork.Numbers",
    "com.apple.FaceTime",
    "com.apple.MobileSMS",
    "com.apple.Music",
    "com.apple.TV",
    "com.apple.Podcasts",
    "com.apple.Maps",
    "com.apple.Notes",
    "com.apple.reminders",
    "com.apple.stocks",
    "com.apple.weather",
    "com.apple.calculator",
    "com.apple.Preview",
    "com.apple.TextEdit",
    "com.apple.ActivityMonitor",
    "com.apple.DiskUtility",
    "com.apple.Console",
    "com.apple.Automator",
    "com.apple.ScriptEditor2",
    "com.apple.ScreenSharing",
    "com.apple.keychainaccess",
];

/// 已安装应用信息（磁盘上的 .app bundle）
#[derive(Serialize, Clone, Debug)]
pub struct InstalledApp {
    pub bundle_path: String,
    pub name: String,
    pub bundle_id: String,
    pub icon_base64: Option<String>,
    pub bundle_size_bytes: u64,
    pub is_system: bool,
    pub is_running: bool,
    pub estimated_residue_bytes: u64,
    pub selection_key: String,
}

pub fn installed_app_identities(apps: &[InstalledApp]) -> Vec<InstalledAppIdentity> {
    apps.iter()
        .map(|app| InstalledAppIdentity {
            bundle_path: app.bundle_path.clone(),
            app_name: app.name.clone(),
            bundle_id: app.bundle_id.clone(),
            is_system: app.is_system,
            bundle_size_bytes: app.bundle_size_bytes,
        })
        .collect()
}

pub fn register_installed_apps(
    store: &mut OperationStore,
    apps: &mut [InstalledApp],
) -> Result<SnapshotRegistration, UserError> {
    let registration = store.register_installed_apps(installed_app_identities(apps))?;
    if registration.selection_keys.len() != apps.len() {
        return Err(UserError::new(
            ErrorCode::APP_SELECTION_COUNT_MISMATCH,
            "应用选择 key 数量与快照不一致",
        ));
    }
    for (app, key) in apps.iter_mut().zip(registration.selection_keys.iter()) {
        app.selection_key = key.clone();
    }
    Ok(registration)
}

/// 判断 Bundle ID 是否为系统核心应用
pub fn is_system_app(bundle_id: &str) -> bool {
    if bundle_id.is_empty() {
        return false;
    }
    SYSTEM_CORE_APPS.contains(&bundle_id)
}

/// 从 .app bundle 读取图标并转为 base64 PNG
/// 流程：Info.plist → CFBundleIconFile → .icns 路径 → sips 转 PNG → base64
fn read_icon_base64(app_path: &Path, plist_path: &Path, tmp_dir: &Path) -> Option<String> {
    let icon_name = read_icon_name(plist_path)?;
    let resources = app_path.join("Contents/Resources");
    // 图标文件可能带 .icns 后缀也可能不带
    let icns_path = if icon_name.ends_with(".icns") {
        resources.join(&icon_name)
    } else {
        resources.join(format!("{}.icns", icon_name))
    };
    if !icns_path.exists() {
        return None;
    }
    icns_to_base64_png(&icns_path, tmp_dir)
}

/// 供其他模块（如 applications.rs）调用的公共接口
pub fn read_icon_base64_for_bundle(bundle_path: &Path) -> Option<String> {
    let plist_path = bundle_path.join("Contents/Info.plist");
    if !plist_path.exists() {
        return None;
    }
    read_icon_base64(bundle_path, &plist_path, &std::env::temp_dir())
}

/// 从 Info.plist 读取图标文件名。
///
/// **原生解析。** 原先是「bplist 先 plutil 转 XML，再正则抓 key」——
/// App Sandbox 下 `plutil` 不可用（沙箱只能 exec 自带二进制），所以这条在
/// MAS 版上必然失效。改走 `applications::read_plist_map` 一次性拿到字典。
fn read_icon_name(plist_path: &Path) -> Option<String> {
    let map = read_plist_map(plist_path);
    plist_string(&map, "CFBundleIconFile").or_else(|| plist_string(&map, "CFBundleIconName"))
}

/// 用 sips 将 .icns 转为 64x64 PNG 并返回 base64 编码
fn icns_to_base64_png(icns_path: &Path, dir: &Path) -> Option<String> {
    use base64::Engine;
    // 临时文件名必须每次调用唯一：并行扫描时不同 app 会撞名。本机 15 个 app 的
    // CFBundleIconFile 都叫 "AppIcon.icns"，若共用同一个临时路径，并行的 sips
    // 会互相覆盖对方正在写/正在读的文件，导致图标随机损坏或整体丢失。
    static ICON_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = ICON_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(
        "macslim_icon_{}_{}_{}.png",
        std::process::id(),
        seq,
        icns_path.file_stem()?.to_string_lossy()
    ));
    let output = std::process::Command::new("sips")
        .args(["-s", "format", "png", "-z", "64", "64"])
        .arg(icns_path)
        .arg("--out")
        .arg(&tmp)
        .output()
        .ok()?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    let png_bytes = std::fs::read(&tmp).ok();
    let _ = std::fs::remove_file(&tmp);
    Some(base64::engine::general_purpose::STANDARD.encode(&png_bytes?))
}

/// 计算目录总大小（字节）
pub(crate) fn dir_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// 从 .app 路径构建 InstalledApp 的「磁盘派生」部分
///
/// 这里**故意不接收**运行态集合：`is_running` 依赖实时进程状态，不能进缓存。
/// 详见 `apply_running_state`。
fn build_installed_app(app_path: &Path) -> Option<InstalledApp> {
    let plist_path = app_path.join("Contents/Info.plist");
    let (name, bundle_id) = if plist_path.exists() {
        read_plist_metadata(&plist_path)
    } else {
        (None, None)
    };

    // 从文件名推断名称（兜底）
    let display_name = name.unwrap_or_else(|| {
        app_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "未知应用".to_string())
    });

    let bid = bundle_id.unwrap_or_default();
    let icon = read_icon_base64(app_path, &plist_path, &std::env::temp_dir());
    let bundle_size = dir_size(app_path);
    let is_system = is_system_app(&bid);

    Some(InstalledApp {
        bundle_path: app_path.to_string_lossy().to_string(),
        name: display_name,
        bundle_id: bid,
        icon_base64: icon,
        bundle_size_bytes: bundle_size,
        is_system,
        // 占位值：每次调用都由 `apply_running_state` 用实时进程状态覆写
        is_running: false,
        estimated_residue_bytes: 0, // 快速扫描阶段不计算残留
        selection_key: String::new(),
    })
}

/// 用**本次调用现读**的运行态覆写 `is_running`
///
/// 这是正确性边界，不是优化：缓存只保存磁盘派生数据（plist / icon /
/// bundle_size），运行态每次都重新计算，否则「应用卸载」页会显示
/// 最多 10 分钟的过期运行状态。
fn apply_running_state(apps: &mut [InstalledApp], running_bundles: &HashSet<String>) {
    for app in apps {
        app.is_running = !app.bundle_id.is_empty() && running_bundles.contains(&app.bundle_id);
    }
}

/// 列表排序：体积降序，体积相同时按路径升序
///
/// 路径是 bundle 的唯一键，所以这个比较是**全序**。这一点是并行化的前提：
/// rayon 的 `flat_map` 是无序（unindexed）迭代器，collect 出来的顺序取决于
/// 工作线程调度。原来的 `sort_by_key(Reverse(size))` 是稳定排序，遇到同体积
/// 的应用（比如一堆 size=0）会把无序的输入顺序原样带进结果，并行与串行就
/// 不可能逐项等价。补上路径 tiebreaker 后，并行结果与线程数无关。
fn compare_apps(left: &InstalledApp, right: &InstalledApp) -> std::cmp::Ordering {
    right
        .bundle_size_bytes
        .cmp(&left.bundle_size_bytes)
        .then_with(|| left.bundle_path.cmp(&right.bundle_path))
}

/// 收集当前运行中应用的 Bundle ID 集合
fn collect_running_bundle_ids(sys: &mut System) -> std::collections::HashSet<String> {
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let mut ids = std::collections::HashSet::new();
    for proc in sys.processes().values() {
        if let Some(exe) = proc.exe() {
            if let Some(bundle) = crate::applications::find_app_bundle(exe) {
                let plist = bundle.join("Contents/Info.plist");
                if plist.exists() {
                    let (_, bid) = read_plist_metadata(&plist);
                    if let Some(id) = bid {
                        ids.insert(id);
                    }
                }
            }
        }
    }
    ids
}

/// 枚举指定目录下的所有 .app bundle
fn enumerate_apps(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|ext| ext == "app").unwrap_or(false))
        .collect()
}

// ========== 磁盘扫描结果缓存（TTL 10 分钟） ==========

/// 缓存有效期。应用安装/卸载是低频操作，10 分钟内复用磁盘派生数据足够新鲜；
/// 真正需要实时的运行态不走缓存（见 `apply_running_state`）。
const APP_SCAN_CACHE_TTL: Duration = Duration::from_secs(600);

/// 一条缓存记录。注意 `apps` 里存的是 `is_running == false` 的**磁盘派生**快照。
struct AppScanCache {
    /// 扫描目录列表构成的键；目录变了必须重扫
    key: String,
    fetched_at: Instant,
    apps: Vec<InstalledApp>,
}

static APP_SCAN_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static APP_SCAN_CACHE_MISSES: AtomicU64 = AtomicU64::new(0);

fn cache_store() -> &'static Mutex<Option<AppScanCache>> {
    static STORE: OnceLock<Mutex<Option<AppScanCache>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(None))
}

/// 扫描目录列表 → 缓存键（用 ASCII 单位分隔符，避免路径拼接歧义）
fn scan_cache_key(scan_dirs: &[PathBuf]) -> String {
    let mut key = String::new();
    for dir in scan_dirs {
        key.push_str(&dir.to_string_lossy());
        key.push('\u{1f}');
    }
    key
}

/// 取缓存。`now` 显式传入是为了让测试能用可控时钟验证 TTL 过期。
fn cached_apps(scan_dirs: &[PathBuf], now: Instant) -> Option<Vec<InstalledApp>> {
    let guard = cache_store().lock().ok()?;
    let entry = guard.as_ref()?;
    if entry.key != scan_cache_key(scan_dirs) {
        return None;
    }
    // checked_duration_since：单调时钟倒退时按过期处理，不 panic
    let age = now.checked_duration_since(entry.fetched_at)?;
    if age >= APP_SCAN_CACHE_TTL {
        return None;
    }
    Some(entry.apps.clone())
}

fn store_cached_apps(scan_dirs: &[PathBuf], apps: &[InstalledApp]) {
    let Ok(mut guard) = cache_store().lock() else {
        return;
    };
    *guard = Some(AppScanCache {
        key: scan_cache_key(scan_dirs),
        fetched_at: Instant::now(),
        apps: apps.to_vec(),
    });
}

/// 强制失效应用扫描缓存（供测试观测 TTL / 缓存键 / 失效行为）
#[cfg(test)]
pub(crate) fn invalidate_app_scan_cache() {
    if let Ok(mut guard) = cache_store().lock() {
        *guard = None;
    }
}

/// 缓存命中 / 未命中计数，仅供测试观测
#[cfg(test)]
pub(crate) fn app_scan_cache_stats() -> (u64, u64) {
    (
        APP_SCAN_CACHE_HITS.load(Ordering::Relaxed),
        APP_SCAN_CACHE_MISSES.load(Ordering::Relaxed),
    )
}

/// 磁盘派生的应用列表（并行），命中缓存则直接返回
fn apps_from_dirs_cached(scan_dirs: &[PathBuf]) -> AppScanOutcome {
    if let Some(apps) = cached_apps(scan_dirs, Instant::now()) {
        APP_SCAN_CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        return AppScanOutcome {
            apps,
            cache_hit: true,
        };
    }
    APP_SCAN_CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
    let apps = scan_apps_parallel(scan_dirs);
    store_cached_apps(scan_dirs, &apps);
    AppScanOutcome {
        apps,
        cache_hit: false,
    }
}

/// 并行扫描所有目录下的 .app bundle
///
/// 每个 bundle 要做三件重活：读 Info.plist、`sips` 转图标并 base64、整棵目录树
/// 递归算体积。其中 `dir_size` 是纯文件系统遍历、内部无共享状态，`read_icon_base64`
/// 只在临时目录写本次调用独有的文件，两者都可安全并行。
///
/// 本机实测（release，35 个 bundle / 25.3 GB）：串行 21.2s，并行 8 线程 11.3s，
/// 约 1.9x。瓶颈是 I/O 而非 CPU，并行度再高收益也趋零——首次冷扫描要压到
/// 3s 以内必须不再遍历整棵树（即懒加载体积），并行与缓存都做不到。
///
/// 返回顺序不确定（`flat_map` 是无序迭代器），由 `compare_apps` 的全序排序收敛。
fn scan_apps_parallel(scan_dirs: &[PathBuf]) -> Vec<InstalledApp> {
    use rayon::prelude::*;
    scan_dirs
        .par_iter()
        .flat_map(|dir| enumerate_apps(dir))
        .filter_map(|path| build_installed_app(&path))
        .collect()
}

/// 扫描结果 + 本次是否命中缓存
pub(crate) struct AppScanOutcome {
    pub apps: Vec<InstalledApp>,
    pub cache_hit: bool,
}

/// 扫描指定目录下的应用：磁盘部分走缓存，运行态每次现算
fn scan_installed_apps_in_dirs(scan_dirs: &[PathBuf], running: &HashSet<String>) -> AppScanOutcome {
    let AppScanOutcome {
        mut apps,
        cache_hit,
    } = apps_from_dirs_cached(scan_dirs);
    apply_running_state(&mut apps, running);
    apps.sort_by(compare_apps);
    AppScanOutcome { apps, cache_hit }
}

/// 默认扫描目录：/Applications + ~/Applications
fn default_scan_dirs() -> Vec<PathBuf> {
    let mut scan_dirs: Vec<PathBuf> = vec![PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        let user_apps = home.join("Applications");
        if user_apps.exists() {
            scan_dirs.push(user_apps);
        }
    }
    scan_dirs
}

/// 扫描已安装应用列表
/// 扫描 /Applications 和 ~/Applications，排除 /System/Applications
pub fn scan_installed_apps(sys: &mut System) -> Vec<InstalledApp> {
    let running = collect_running_bundle_ids(sys);
    let scan_dirs = default_scan_dirs();
    scan_installed_apps_in_dirs(&scan_dirs, &running).apps
}

#[cfg(test)]
#[path = "app_scanner_tests.rs"]
mod tests;
