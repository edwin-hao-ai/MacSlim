pub mod app_scanner;
pub mod applications;
pub mod cache_cleaner;
pub mod cache_scanner;
pub mod cli_operations;
pub mod dev_tool_rules;
pub mod docker;
pub mod fda;
pub mod flavor;
pub mod folder_access;
pub mod i18n_text;
pub mod monitor;
pub(crate) mod operation_commands;
#[allow(dead_code)]
pub(crate) mod operation_executor;
pub mod operations;
pub mod ports;
pub mod process_monitor;
pub(crate) mod process_ops;
pub mod process_safety;
pub mod process_snapshot;
pub(crate) mod residue_policy;
pub mod residue_scanner;
pub mod sandbox_probe;
pub mod scan_progress;
pub mod scanner;
pub mod storage;
pub mod tray;
pub mod uninstaller;
pub mod user_error;
pub mod whitelist;

// CLI-friendly re-exports
use crate::user_error::{ErrorCode, UserError};
pub use scanner::read_health as scanner_read_health;
pub fn run_tauri() {
    run();
}

use operation_commands::{
    process_whitelist_policy, CacheSnapshotView, OperationResult, PrepareOperationRequest,
    ResidueAppGroup, SnapshotResult,
};
use operations::OperationStore;
use scanner::SystemHealth;
use std::sync::{Arc, Mutex};
use storage::{HistoryEntry, Storage, WhitelistEntry};
use sysinfo::System;
use tauri::{Manager, State, WebviewWindow, WindowEvent};

pub struct AppState {
    pub sys: Mutex<System>,
    pub storage: Arc<Storage>,
    pub operations: Arc<Mutex<OperationStore>>,
}

fn whitelist_policy(state: &AppState) -> Box<dyn Fn(&str) -> bool + Send + Sync + 'static> {
    // MAS 版一律返回 false，也就是「没有任何进程命中白名单」。
    //
    // 白名单回答的是「这个进程该不该被终止」，而 MAS 版终止不了任何进程 ——
    // 于是这些标记是**纯噪音**，而且很容易被误读：实测抓截屏时，
    // 第一行进程下面挂着「命中白名单，默认不建议终止」，我第一眼读成了
    // 「受保护进程」（那是另一套提示），差点以为 MAS 版的安全判定没生效。
    //
    // 列表里给用户「这个进程不建议动」的暗示、却又不给任何操作入口，
    // 是最难解释的一种界面。
    // `impl Fn` 的返回类型由编译器按第一条 return 推断，所以这里必须让两个
    // 分支返回**同一种闭包形状** —— 用 Box 包一层而不是提前 return，
    // 否则先返回的那个闭包会把类型钉死，另一个分支编译不过。
    let inner = process_whitelist_policy(state.storage.clone());
    Box::new(move |name: &str| {
        if matches!(flavor::CURRENT, flavor::Flavor::Mas) {
            return false;
        }
        inner(name)
    })
}

/// 下发当前构建形态（见 `flavor` 模块）。
///
/// 前端需要它来决定「终止进程」这类入口要不要出现 —— MAS 版在沙箱里终止不了
/// 别的进程，与其让用户点一个注定失败的按钮、再弹一个「权限不足」（会被误
/// 解成系统设置问题），不如直接把入口藏掉。
///
/// 纯元数据读取，无副作用，不碰任何用户数据。
#[tauri::command]
fn get_build_flavor() -> &'static str {
    flavor::CURRENT.as_str()
}

/// 完全磁盘访问权限的探测结果。
///
/// 分开报「用户缓存」与「系统目录」两类，因为它们挡住的**能力不同**：
/// 用户缓存挡的是缓存清理，系统目录挡的是进程枚举。前端要按类给引导文案，
/// 笼统一句「请授予完全磁盘访问权限」会让用户不知道授权后能干什么。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FdaStatus {
    /// 能不能读 `~/Library/Caches` —— 决定缓存清理是否可用
    user_cache: bool,
    /// 能不能读 `/Library/*` —— 决定进程枚举等是否可用
    system_dirs: bool,
    /// `$HOME` 是否被沙箱重定向到了应用自己的 container。
    ///
    /// 为 true 时**授权解决不了任何问题**：真实 `~/Library/Caches` 的
    /// `read_dir` 直接返回 EPERM。前端据此换掉引导，别把用户领去系统设置。
    home_redirected: bool,
    /// 构建形态，前端据此决定这张卡片讲什么。
    flavor: &'static str,
}

// ========== 文件夹访问授权（MAS 版） ==========

/// 一个可供授权的目录。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FolderTargetView {
    key: String,
    /// 相对真实 home 的路径，仅供展示「你要授权哪个目录」
    relative_path: String,
    reason_key: String,
    granted: bool,
    granted_path: Option<String>,
}

/// 当前授权状态。
#[tauri::command]
fn list_folder_access() -> Vec<FolderTargetView> {
    let granted = folder_access::load_grants();
    folder_access::offerable_targets()
        .into_iter()
        .map(|target| {
            let existing = granted
                .iter()
                .find(|g| g.target_key == target.key)
                .map(|g| g.path.display().to_string());
            FolderTargetView {
                key: target.key.to_string(),
                relative_path: target.relative.to_string(),
                reason_key: target.reason_key.to_string(),
                granted: existing.is_some(),
                granted_path: existing,
            }
        })
        .collect()
}

/// 让用户授权一个目录。
///
/// 走系统文件选择框 —— 沙箱里没有任何 entitlement 能让我们直接读用户目录，
/// 唯一合规的入口就是用户在标准对话框里亲手选定它。
///
/// `NSOpenPanel.runModal` 必须在主线程，所以这里只做参数准备，真正弹窗
/// 通过 `run_on_main_thread` 调度，结果与错误经 channel 回传。
#[tauri::command]
async fn grant_folder_access(
    app: tauri::AppHandle,
    target_key: String,
    prompt: String,
) -> Result<bool, UserError> {
    let (tx, rx) = std::sync::mpsc::channel::<Option<std::path::PathBuf>>();
    // 起始目录 = 用户点的**那一行**对应的真实路径。
    //
    // 之前只传了提示文案，面板于是开在「系统记住的上一个位置」（首次是
    // Documents）—— 点「废纸篓 ~/.Trash」的授权，面板却开在 Documents，
    // 用户得自己翻过去或 ⌘⇧G 手输。六个目标里五个在 ~/Library 或点号目录，
    // 等于每一次授权都要手动导航。
    //
    // 用 `scanner_home()`（passwd 里的真实 home）而不是 `$HOME`：沙箱里
    // `$HOME` 是应用自己的 container，面板会开在一个用户根本不认识的地方。
    let target_relative = folder_access::offerable_targets()
        .into_iter()
        .find(|candidate| candidate.key == target_key)
        .map(|candidate| candidate.relative.to_string());
    let start_directory = target_relative
        .map(|relative| folder_access::scanner_home().join(relative))
        .unwrap_or_else(folder_access::scanner_home);
    app.run_on_main_thread(move || {
        // SAFETY: pick_folder 会阻塞到用户做出选择，因此必须放到主线程 ——
        // NSOpenPanel 的 runModal 在别的线程调用会直接崩。
        let picked = unsafe { folder_access::ffi::pick_folder(&prompt, &start_directory) };
        let _ = tx.send(picked);
    })
    .map_err(|error| {
        UserError::new(
            ErrorCode::FOLDER_ACCESS_PANEL_FAILED,
            format!("无法调度文件选择框：{error}"),
        )
    })?;

    let picked = rx.recv().map_err(|error| {
        UserError::new(
            ErrorCode::FOLDER_ACCESS_PANEL_FAILED,
            format!("文件选择框异常：{error}"),
        )
    })?;
    let Some(path) = picked else {
        return Ok(false); // 用户取消，不是错误
    };

    let Some(bookmark) = folder_access::ffi::create_bookmark(&path) else {
        return Err(UserError::new(
            ErrorCode::FOLDER_ACCESS_BOOKMARK_FAILED,
            "无法为所选目录创建访问凭证",
        ));
    };
    let display_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| target_key.clone());
    folder_access::record_grant(folder_access::Grant {
        target_key,
        display_name,
        path,
        bookmark,
    })
    .map_err(|error| UserError::new(ErrorCode::FOLDER_ACCESS_STORE_FAILED, error))?;
    Ok(true)
}

/// 撤销一条授权。
#[tauri::command]
fn revoke_folder_access(target_key: String) -> Result<(), UserError> {
    folder_access::revoke_grant(&target_key)
        .map_err(|error| UserError::new(ErrorCode::FOLDER_ACCESS_STORE_FAILED, error))?;
    Ok(())
}

#[tauri::command]
fn get_fda_status() -> FdaStatus {
    let real_home = fda::real_home_for_probe();
    let home = dirs::home_dir();
    let home_redirected = match (&home, &real_home) {
        (Some(home), Some(real)) => fda::home_redirected(home, real),
        // 取不到任何一边时保守判 true：宁可显示「沙箱限制」也不要显示
        // 「已授权」，后者会让用户对着空列表永远看不到解释。
        (None, _) | (_, None) => true,
    };
    FdaStatus {
        user_cache: fda::user_cache_readable(),
        system_dirs: fda::system_dirs_readable(),
        home_redirected,
        flavor: flavor::CURRENT.as_str(),
    }
}

// ========== System & Process ==========

#[tauri::command]
async fn get_system_health(state: State<'_, AppState>) -> Result<SystemHealth, UserError> {
    let mut sys = state.sys.lock().map_err(|e| e.to_string())?;
    Ok(scanner::read_health(&mut sys))
}

#[tauri::command]
async fn scan_all(
    state: State<'_, AppState>,
) -> Result<SnapshotResult<scanner::ScanResult>, UserError> {
    let mut sys = state.sys.lock().map_err(|e| e.to_string())?;
    let result = scanner::scan(&mut sys);
    let policy = whitelist_policy(&state);
    let mut operations = state.operations.lock().map_err(|error| error.to_string())?;
    operation_commands::snapshot_process_scan(&mut operations, result, &policy)
}

/// 列出所有可见用户进程（不做分类过滤，用于进程管理页）。
/// 与 scan_all 不同：返回全部，前端自己做展示/搜索/排序。
///
/// MAS 形态改走 `process_monitor` 的只读列表。原因是 `sysinfo` 枚举进程靠
/// libproc 的 `proc_listallpids`，**那个调用被 App Sandbox 拦掉** ——
/// 实测 MAS 包里数出 0 个，于是这一页在 App Store 版是空的。
/// 换成 `sysctl` + `proc_pidinfo` 之后沙箱内能拿到 200+ 个进程，
/// 而**行形状完全一致**，所以前端与 `ProcessView` 一行都不用改。
///
/// 判定放在 flavor 模块而不是散落 `cfg`：那里是「两个构建形态分离」的唯一
/// 真相源（见 flavor.rs 的模块注释）。
#[tauri::command]
async fn list_all_processes(
    state: State<'_, AppState>,
) -> Result<SnapshotResult<Vec<scanner::ProcessRow>>, UserError> {
    let rows = match flavor::CURRENT {
        flavor::Flavor::DeveloperId => {
            let mut sys = state.sys.lock().map_err(|e| e.to_string())?;
            scanner::list_all(&mut sys)
        }
        flavor::Flavor::Mas => process_monitor::list_readonly_rows(),
    };
    let policy = whitelist_policy(&state);
    let mut operations = state.operations.lock().map_err(|error| error.to_string())?;
    operation_commands::snapshot_process_rows(&mut operations, rows, &policy)
}

// ========== 应用程序管理 ==========

#[tauri::command]
async fn list_applications(
    state: State<'_, AppState>,
) -> Result<SnapshotResult<Vec<applications::AppInfo>>, UserError> {
    let mut sys = state.sys.lock().map_err(|e| e.to_string())?;
    let apps = applications::list_running_apps(&mut sys);
    let policy = whitelist_policy(&state);
    let mut operations = state.operations.lock().map_err(|error| error.to_string())?;
    operation_commands::snapshot_applications(&mut operations, apps, &policy)
}

// ========== Docker 深度视图 ==========

#[tauri::command]
async fn docker_available() -> Result<bool, UserError> {
    Ok(docker::is_available().await)
}

#[tauri::command]
async fn docker_inventory(
    state: State<'_, AppState>,
) -> Result<SnapshotResult<docker::DockerInventory>, UserError> {
    let inventory = docker::inventory().await?;
    let mut operations = state.operations.lock().map_err(|error| error.to_string())?;
    operation_commands::snapshot_docker_inventory(&mut operations, inventory)
}

// ========== Cache ==========

#[tauri::command]
async fn scan_cache(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<SnapshotResult<CacheSnapshotView>, UserError> {
    use tauri::Emitter;
    let sink: crate::scan_progress::ProgressSink = std::sync::Arc::new(move |update| {
        let _ = app.emit("cache-scan-progress", &update);
    });
    let result = cache_scanner::scan(Some(sink)).await;
    let mut operations = state.operations.lock().map_err(|error| error.to_string())?;
    operation_commands::snapshot_cache(&mut operations, result)
}

// ========== 应用卸载 ==========

/// 扫描已安装应用列表
#[tauri::command]
async fn scan_installed_apps(
    state: State<'_, AppState>,
) -> Result<SnapshotResult<Vec<app_scanner::InstalledApp>>, UserError> {
    let mut sys = state.sys.lock().map_err(|e| e.to_string())?;
    let apps = app_scanner::scan_installed_apps(&mut sys);
    let mut operations = state.operations.lock().map_err(|error| error.to_string())?;
    operation_commands::snapshot_installed_apps(&mut operations, apps)
}

#[tauri::command]
async fn scan_app_residues_batch(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    app_snapshot_id: String,
    app_keys: Vec<String>,
) -> Result<SnapshotResult<Vec<ResidueAppGroup>>, UserError> {
    let selections = {
        let operations = state.operations.lock().map_err(|error| error.to_string())?;
        operation_commands::resolve_installed_apps(&operations, &app_snapshot_id, &app_keys)?
    };
    let groups = tauri::async_runtime::spawn_blocking(move || {
        use rayon::prelude::*;
        use tauri::Emitter;
        let index = residue_scanner::LibraryIndex::build();
        selections
            .par_iter()
            .map(|(app_key, identity)| {
                let residue = residue_scanner::scan_residues_with_index(
                    index.as_ref(),
                    &identity.bundle_id,
                    &identity.app_name,
                );
                let _ = app.emit(
                    "residue-scan-progress",
                    scan_progress::StageUpdate {
                        stage: residue.app_name.clone(),
                        state: "done",
                        item_count: residue.items.len(),
                        found_bytes: residue.total_bytes,
                    },
                );
                (app_key.clone(), residue)
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|error| {
        UserError::one(
            ErrorCode::RESIDUE_SCAN_FAILED,
            format!("扫描残留失败: {error}"),
            "reason",
            &error,
        )
    })?;
    let mut operations = state.operations.lock().map_err(|error| error.to_string())?;
    operation_commands::register_residue_groups(&mut operations, groups)
}

/// 检查应用是否正在运行
#[tauri::command]
async fn check_app_running(
    state: State<'_, AppState>,
    bundle_path: String,
) -> Result<bool, UserError> {
    let mut sys = state.sys.lock().map_err(|e| e.to_string())?;
    Ok(uninstaller::is_app_running(&bundle_path, &mut sys))
}

// ========== Operation Broker ==========

#[tauri::command]
async fn prepare_operation(
    window: WebviewWindow,
    state: State<'_, AppState>,
    request: PrepareOperationRequest,
) -> Result<operations::PreparedOperation, UserError> {
    let owner = window.label().to_owned();
    let mut operations = state.operations.lock().map_err(|error| error.to_string())?;
    operation_commands::prepare_operation_with(&mut operations, &owner, request)
}

#[tauri::command]
async fn execute_operation(
    window: WebviewWindow,
    state: State<'_, AppState>,
    operation_id: String,
) -> Result<OperationResult, UserError> {
    let owner = window.label().to_owned();
    let history = operation_commands::system_history(&state.storage);
    let policy = whitelist_policy(&state);
    let domains = operation_executor::system_domain_services(
        &operation_executor::SystemCacheCleaner,
        &uninstaller::SystemUninstaller,
        &applications::SystemAppQuitter,
    );
    let operations = Arc::clone(&state.operations);
    operation_commands::execute_operation_with(
        operations.as_ref(),
        &history,
        &owner,
        &operation_id,
        move |plan| async move { run_operation_plan(plan, policy, &domains).await },
    )
    .await
}

async fn run_operation_plan(
    plan: operations::ConsumedPlan,
    // MAS 形态下下面那道守卫会直接返回，`policy` 根本用不到；不加这个属性
    // 的话 `--features mas` 的构建会带一个 unused-variable 警告。
    #[cfg_attr(feature = "mas", allow(unused_variables))] policy: impl Fn(&str) -> bool
        + Send
        + Sync
        + 'static,
    domains: &operation_executor::DomainServices<
        '_,
        operation_executor::SystemCacheCleaner,
        uninstaller::SystemUninstaller,
        applications::SystemAppQuitter,
        docker::SystemDocker,
    >,
) -> Result<operation_executor::OperationOutcome, UserError> {
    if !plan.kind().is_termination() {
        return operation_executor::execute_domain_plan(plan, domains).await;
    }
    // 能不能终止进程只看 `flavor::CURRENT` —— 全项目只有这一处判断能力差异，
    // 不散落 `cfg(feature = "mas")`。注意这只是**构建形态的能力差异**，
    // 不是安全判定：判断依据是编译期常量，不掺任何文案、快照或用户输入。
    //
    // 守卫放在进入 signaller 之前，而不是让 kill(2) 去撞 EPERM —— 后者会报成
    // 「权限不足」，让用户以为是系统设置问题，真实原因是这构建压根没这能力。
    if !flavor::CURRENT.can_terminate_processes() {
        return Err(UserError::with(
            ErrorCode::PROCESS_TERMINATION_UNSUPPORTED,
            "App Store 版运行在系统沙箱内，不能终止其他进程",
            vec![],
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut observer = process_ops::SystemProcessObserver::with_policy(policy);
        let mut signaller = process_ops::SystemProcessSignaller;
        let mut processes = operation_executor::ProcessServices {
            observer: &mut observer,
            signaller: &mut signaller,
        };
        operation_executor::execute_termination_plan(plan, &mut processes)
    })
    .await
    .map_err(|error| {
        UserError::one(
            ErrorCode::PROCESS_EXECUTION_FAILED,
            format!("进程操作执行失败: {error}"),
            "reason",
            &error,
        )
    })?
}

// ========== History & Whitelist ==========

#[tauri::command]
async fn get_history(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<HistoryEntry>, UserError> {
    state.storage.recent_history(limit.unwrap_or(200))
}

#[tauri::command]
async fn get_whitelist(state: State<'_, AppState>) -> Result<Vec<WhitelistEntry>, UserError> {
    state.storage.list_whitelist()
}

#[tauri::command]
async fn add_whitelist(
    state: State<'_, AppState>,
    kind: String,
    value: String,
    note: String,
) -> Result<(), UserError> {
    state.storage.add_whitelist(&kind, &value, &note)
}

#[tauri::command]
async fn remove_whitelist(state: State<'_, AppState>, id: i64) -> Result<(), UserError> {
    state.storage.remove_whitelist(id)
}

// ========== Entry point ==========

fn setup_app(
    app: &mut tauri::App,
    storage: Arc<Storage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sys = System::new_all();
    sys.refresh_all();
    app.manage(AppState {
        sys: Mutex::new(sys),
        storage: storage.clone(),
        operations: Arc::new(Mutex::new(OperationStore::new())),
    });

    // 系统托盘
    tray::init_tray(app.handle())?;

    // 后台健康监控（2 秒一次）
    monitor::start_background_monitor(app.handle().clone());

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 无头沙箱探针：必须在任何 Tauri / 存储初始化之前返回。
    // 沙箱由内核按签名 entitlement 施加，与从 Finder 还是终端启动无关，
    // 所以这样跑出来的路径可达性与 GUI 里完全一致。
    if sandbox_probe::probe_requested(std::env::args().collect()) {
        sandbox_probe::print_report();
        return;
    }

    let storage = Arc::new(Storage::open().expect("无法初始化存储"));

    // MAS 形态下 updater 插件不注册，`builder` 之后不再被重新赋值，所以这里
    // 也不需要 `mut`。
    #[cfg_attr(feature = "mas", allow(unused_mut))]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));

    // MAS 走 App Store 更新（用户从 App Store 升级），自更新插件整块不注册；
    // capability/mas.json 里对应的两条 updater 权限也一并去掉，两边保持一致。
    //
    // 必须写成独立的 `cfg` 块再重新赋值，不能在方法链中间挂 `#[cfg]` ——
    // 属性只能加在 item 上，加在 `.plugin(...)` 这种表达式位置编译不过
    // （`error: expected ';', found '#'`）。
    #[cfg(not(feature = "mas"))]
    {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }

    builder
        .setup(move |app| setup_app(app, storage.clone()))
        .on_window_event(|window, event| {
            // 点 X 关闭 → 不退出应用，只把窗口藏起来，托盘保持驻留
            // 真正退出通过托盘菜单「退出 MacSlim」
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_build_flavor,
            get_fda_status,
            list_folder_access,
            grant_folder_access,
            revoke_folder_access,
            get_system_health,
            scan_all,
            list_all_processes,
            scan_cache,
            get_history,
            get_whitelist,
            add_whitelist,
            remove_whitelist,
            list_applications,
            docker_available,
            docker_inventory,
            scan_installed_apps,
            scan_app_residues_batch,
            check_app_running,
            prepare_operation,
            execute_operation,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
#[path = "lib_progress_tests.rs"]
mod lib_progress_tests;
