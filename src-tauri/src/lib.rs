pub mod app_scanner;
pub mod applications;
pub mod cache_cleaner;
pub mod cache_scanner;
pub mod cli_operations;
pub mod dev_tool_rules;
pub mod docker;
pub mod i18n_text;
pub mod monitor;
pub(crate) mod operation_commands;
#[allow(dead_code)]
pub(crate) mod operation_executor;
pub mod operations;
pub mod ports;
pub(crate) mod process_ops;
pub mod process_safety;
pub(crate) mod residue_policy;
pub mod residue_scanner;
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

fn whitelist_policy(state: &AppState) -> impl Fn(&str) -> bool + Send + Sync + 'static {
    process_whitelist_policy(state.storage.clone())
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
#[tauri::command]
async fn list_all_processes(
    state: State<'_, AppState>,
) -> Result<SnapshotResult<Vec<scanner::ProcessRow>>, UserError> {
    let mut sys = state.sys.lock().map_err(|e| e.to_string())?;
    let rows = scanner::list_all(&mut sys);
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
    policy: impl Fn(&str) -> bool + Send + Sync + 'static,
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
    let storage = Arc::new(Storage::open().expect("无法初始化存储"));

    tauri::Builder::default()
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
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
