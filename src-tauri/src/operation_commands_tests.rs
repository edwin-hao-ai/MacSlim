use super::*;
use crate::app_scanner::InstalledApp;
use crate::applications::{AppChildProcess, AppInfo};
use crate::cache_cleaner::{CleanReport, CleanSummary};
use crate::cache_scanner::{CacheCategory, CacheScanResult, Safety};
use crate::docker::{
    DockerBuilderCache, DockerContainer, DockerImage, DockerInventory, DockerVolume,
};
use crate::operation_executor::{
    execute_domain_plan, execute_termination_plan, CacheCleaner, DockerDomain, DomainFuture,
    DomainServices, ProcessServices, UninstallDomain,
};
use crate::operations::{
    CacheAction, DockerAction, LiveResidue, OperationKind, OperationStore, ProcessMode,
    SnapshotKind, SNAPSHOT_TTL_MS,
};
use crate::process_ops::{KillOutcome, LiveProcess, ProcessObserver, ProcessSignaller};
use crate::residue_scanner::{AppResidue, ResidueItem};
use crate::scanner::{ProcessInfo, ProcessKind, ProcessRow, Risk, ScanResult, SystemHealth};
use crate::uninstaller::{MoveResult, UninstallReport};
use crate::user_error::UserError;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

// ===== fixtures =====

pub(crate) fn cache_item(label: &str) -> CacheItem {
    CacheItem {
        id: label.to_owned(),
        category: CacheCategory::Npm,
        label_key: "cache.item.npmCache".into(),
        label_params: Vec::new(),
        description_key: "cache.desc.npmCache".into(),
        description_params: Vec::new(),
        path: Some(format!("/tmp/{label}")),
        size_bytes: 128,
        safety: Safety::Safe,
        default_select: true,
        action: CacheAction::Npm,
        stale_owner_uid: None,
        stale_canonical_path: None,
        recover_hint: String::new(),
    }
}

pub(crate) fn process_info(pid: u32, name: &str) -> ProcessInfo {
    ProcessInfo {
        pid,
        name: name.to_owned(),
        exe: format!("/usr/local/bin/{name}"),
        start_time: 1_000,
        cpu_percent: 0.5,
        memory_mb: 12.0,
        kind: ProcessKind::Idle,
        risk: Risk::Low,
        default_select: false,
        reason_key: "process.reason.highCpu".to_owned(),
        reason_params: Vec::new(),
        ports: Vec::new(),
        icon_base64: None,
        selection_key: String::new(),
        protected: false,
        protected_reason_key: None,
        protected_reason_params: Vec::new(),
        whitelisted: false,
    }
}

pub(crate) fn process_row(pid: u32, name: &str) -> ProcessRow {
    ProcessRow {
        pid,
        parent_pid: Some(1),
        name: name.to_owned(),
        full_name: name.to_owned(),
        exe: format!("/usr/local/bin/{name}"),
        start_time: 1_000,
        cpu_percent: 0.5,
        memory_mb: 12.0,
        uptime_secs: 3_600,
        name_key: None,
        status_key: "process.status.sleep".to_owned(),
        ports: Vec::new(),
        icon_base64: None,
        protected: false,
        protected_reason_key: None,
        protected_reason_params: Vec::new(),
        whitelisted: false,
        selection_key: String::new(),
    }
}

/// 展示名与原始名**不同**的进程行。
///
/// 必须存在这种形态的 fixture：`process_row` 让 `name == full_name`，会让
/// 「白名单判定误用展示名」这个 bug 在整个测试套件里结构性隐形 —— 因为
/// 两个名字喂给 policy 得到同一个结论，改对改错都测不出来。
pub(crate) fn process_row_with_display_name(pid: u32, name: &str, full_name: &str) -> ProcessRow {
    ProcessRow {
        name: name.to_owned(),
        full_name: full_name.to_owned(),
        exe: format!("/Applications/Alpha.app/Contents/Library/XPCServices/{full_name}"),
        ..process_row(pid, full_name)
    }
}

pub(crate) fn app_child(pid: u32, name: &str) -> AppChildProcess {
    AppChildProcess {
        pid,
        parent_pid: Some(1),
        name: name.to_owned(),
        exe: format!("/Applications/Alpha.app/Contents/MacOS/{name}"),
        start_time: 1_000,
        memory_mb: 8.0,
        cpu_percent: 0.1,
        ports: Vec::new(),
        is_main: true,
        depth: 0,
        protected: false,
        protected_reason_key: None,
        protected_reason_params: Vec::new(),
        whitelisted: false,
        selection_key: String::new(),
    }
}

pub(crate) fn app_info(name: &str) -> AppInfo {
    AppInfo {
        bundle_path: format!("/Applications/{name}.app"),
        name: name.to_owned(),
        bundle_id: format!("com.example.{name}"),
        icon_base64: None,
        main_pid: 42,
        all_pids: vec![42],
        children: vec![app_child(42, name)],
        memory_mb: 8.0,
        cpu_percent: 0.1,
        uptime_secs: 3_600,
        ports: Vec::new(),
        is_system: false,
        protected_process_count: 0,
        whitelisted_process_count: 0,
        selection_key: String::new(),
    }
}

pub(crate) fn installed_app(name: &str) -> InstalledApp {
    InstalledApp {
        bundle_path: format!("/Applications/{name}.app"),
        name: name.to_owned(),
        bundle_id: format!("com.example.{name}"),
        icon_base64: None,
        bundle_size_bytes: 2_048,
        is_system: false,
        is_running: true,
        estimated_residue_bytes: 0,
        selection_key: String::new(),
    }
}

pub(crate) fn library_path(label: &str) -> String {
    dirs::home_dir()
        .expect("测试环境必须有用户主目录")
        .join("Library/Caches")
        .join(label)
        .to_string_lossy()
        .to_string()
}

pub(crate) fn residue_item(label: &str) -> ResidueItem {
    ResidueItem {
        path: library_path(label),
        category: "Caches".to_owned(),
        size_bytes: 64,
        is_dev_tool: false,
        selected: true,
        selection_key: String::new(),
    }
}

pub(crate) fn app_residue(bundle_id: &str, app_name: &str, labels: &[&str]) -> AppResidue {
    let items: Vec<ResidueItem> = labels.iter().map(|label| residue_item(label)).collect();
    let total_bytes = items.iter().map(|item| item.size_bytes).sum();
    AppResidue {
        bundle_id: bundle_id.to_owned(),
        app_name: app_name.to_owned(),
        items,
        total_bytes,
        scan_complete: true,
    }
}

pub(crate) fn docker_inventory_fixture() -> DockerInventory {
    DockerInventory {
        daemon_running: true,
        images: vec![DockerImage {
            id: "sha256-image".to_owned(),
            repository: "nginx".to_owned(),
            tag: "latest".to_owned(),
            size_bytes: 900,
            created: "2026-01-15".to_owned(),
            dangling: false,
            in_use: false,
            selection_key: String::new(),
        }],
        containers: vec![DockerContainer {
            id: "container-id".to_owned(),
            name: "web".to_owned(),
            image: "nginx".to_owned(),
            status: "Up 2 hours".to_owned(),
            running: true,
            size_bytes: 10,
            created: "2026-01-15".to_owned(),
            selection_key: String::new(),
        }],
        volumes: vec![DockerVolume {
            name: "data".to_owned(),
            driver: "local".to_owned(),
            size_bytes: 0,
            in_use: false,
            selection_key: String::new(),
        }],
        builder: DockerBuilderCache {
            total_bytes: 10,
            reclaimable_bytes: 10,
        },
        reclaimable_bytes: 920,
    }
}

pub(crate) fn health() -> SystemHealth {
    SystemHealth {
        cpu_percent: 1.0,
        memory_used_mb: 100.0,
        memory_total_mb: 1_024.0,
        memory_percent: 10.0,
        disk_used_gb: 100.0,
        disk_total_gb: 500.0,
        disk_percent: 20.0,
    }
}

pub(crate) fn scan_result_fixture() -> ScanResult {
    ScanResult {
        health: health(),
        processes: vec![process_info(11, "alpha"), process_info(12, "beta")],
        scanned_at_ms: 1_700_000_000_000,
    }
}

pub(crate) fn clean_summary() -> CleanSummary {
    CleanSummary {
        reports: vec![CleanReport {
            id: "npm".to_owned(),
            label_key: "cache.item.npmCache".into(),
            label_params: Vec::new(),
            success: true,
            deleted_bytes: 128,
            duration_ms: 3,
            error: None,
        }],
        deleted_bytes: 128,
        reclaimed_bytes: None,
        success_count: 1,
        fail_count: 0,
    }
}

pub(crate) fn uninstall_report(app_name: &str) -> UninstallReport {
    UninstallReport {
        app_name: app_name.to_owned(),
        bundle_id: format!("com.example.{app_name}"),
        total_freed_bytes: 2_048,
        moved_count: 1,
        failed_count: 0,
        details: vec![MoveResult {
            path: format!("/Applications/{app_name}.app"),
            success: true,
            error: None,
            size_bytes: 2_048,
        }],
        quit_error: None,
    }
}

pub(crate) fn key_of(value: &str) -> String {
    value.to_owned()
}

pub(crate) fn cache_fixture(
    harness: &mut Harness,
    label: &str,
) -> SnapshotResult<CacheSnapshotView> {
    snapshot_cache(
        &mut harness.store.lock().unwrap(),
        CacheScanResult {
            items: vec![cache_item(label)],
            total_bytes: 128,
            scanned_at_ms: 1,
        },
    )
    .expect("缓存快照必须注册成功")
}

pub(crate) fn alpha_fixture(harness: &mut Harness) -> SnapshotResult<Vec<InstalledApp>> {
    snapshot_installed_apps(
        &mut harness.store.lock().unwrap(),
        vec![installed_app("Alpha")],
    )
    .expect("已安装应用快照必须注册成功")
}

pub(crate) fn two_app_fixture(harness: &mut Harness) -> SnapshotResult<Vec<InstalledApp>> {
    snapshot_installed_apps(
        &mut harness.store.lock().unwrap(),
        vec![installed_app("Alpha"), installed_app("Beta")],
    )
    .expect("已安装应用快照必须注册成功")
}

pub(crate) fn app_keys(apps: &SnapshotResult<Vec<InstalledApp>>) -> Vec<String> {
    apps.value
        .iter()
        .map(|app| app.selection_key.clone())
        .collect()
}

pub(crate) fn residue_keys(residues: &SnapshotResult<Vec<ResidueAppGroup>>) -> Vec<String> {
    residues
        .value
        .iter()
        .flat_map(|group| {
            group
                .residue
                .items
                .iter()
                .map(|item| item.selection_key.clone())
        })
        .collect()
}

pub(crate) fn residue_for(identity: &crate::operations::InstalledAppIdentity) -> AppResidue {
    let bundle_id = identity.bundle_id.clone();
    app_residue(
        &bundle_id,
        &identity.app_name,
        &[&format!("{bundle_id}-cache")],
    )
}

pub(crate) fn cache_plan_fixture(harness: &mut Harness) -> crate::operations::PreparedOperation {
    let snapshot = cache_fixture(harness, "npm");
    prepare(
        harness,
        cache_request(
            &snapshot.snapshot_id,
            vec![key_of(&snapshot.value.items[0].selection_key)],
        ),
    )
    .expect("缓存计划")
}

pub(crate) fn process_plan_fixture(
    harness: &mut Harness,
) -> (
    crate::operations::PreparedOperation,
    SnapshotResult<scanner::ScanResult>,
) {
    let policy = process_whitelist_policy(Arc::new(FakeWhitelist::new(&[])));
    let snapshot = snapshot_process_scan(
        &mut harness.store.lock().unwrap(),
        scan_result_fixture(),
        &policy,
    )
    .expect("进程快照");
    let prepared = prepare(
        harness,
        process_request(
            &snapshot.snapshot_id,
            vec![key_of(&snapshot.value.processes[0].selection_key)],
            ProcessMode::Graceful,
        ),
    )
    .expect("进程计划");
    (prepared, snapshot)
}

pub(crate) fn uninstall_plan_fixture(
    harness: &mut Harness,
) -> crate::operations::PreparedOperation {
    let installed = alpha_fixture(harness);
    let app_key = installed.value[0].selection_key.clone();
    let residues = scan_residue_batch(
        harness,
        &installed.snapshot_id,
        std::slice::from_ref(&app_key),
        residue_for,
    )
    .expect("残留快照");
    prepare(
        harness,
        uninstall_request(
            &installed.snapshot_id,
            &residues.snapshot_id,
            vec![app_key],
            vec![key_of(&residues.value[0].residue.items[0].selection_key)],
        ),
    )
    .expect("卸载计划")
}

pub(crate) fn live_process_with(pid: u32, name: &str, exe: &str) -> LiveProcess {
    LiveProcess {
        identity: crate::operations::ProcessIdentity {
            pid,
            name: name.to_owned(),
            exe: exe.to_owned(),
            start_time: 1_000,
        },
        protected: false,
        whitelisted: false,
    }
}

pub(crate) fn scan_residue_batch<F>(
    harness: &mut Harness,
    app_snapshot_id: &str,
    app_keys: &[String],
    mut scan: F,
) -> Result<SnapshotResult<Vec<ResidueAppGroup>>, UserError>
where
    F: FnMut(&crate::operations::InstalledAppIdentity) -> AppResidue,
{
    let selections =
        resolve_installed_apps(&harness.store.lock().unwrap(), app_snapshot_id, app_keys)?;
    let groups = selections
        .into_iter()
        .map(|(app_key, identity)| {
            let residue = scan(&identity);
            (app_key, residue)
        })
        .collect();
    register_residue_groups(&mut harness.store.lock().unwrap(), groups)
}

pub(crate) fn prepare(
    harness: &mut Harness,
    request: PrepareOperationRequest,
) -> Result<crate::operations::PreparedOperation, UserError> {
    prepare_operation_with(&mut harness.store.lock().unwrap(), "main", request)
}

pub(crate) fn cache_request(snapshot_id: &str, keys: Vec<String>) -> PrepareOperationRequest {
    PrepareOperationRequest::Cache {
        snapshot_id: snapshot_id.to_owned(),
        item_keys: keys,
    }
}

pub(crate) fn process_request(
    snapshot_id: &str,
    keys: Vec<String>,
    mode: ProcessMode,
) -> PrepareOperationRequest {
    PrepareOperationRequest::Process {
        snapshot_id: snapshot_id.to_owned(),
        process_keys: keys,
        mode,
    }
}

pub(crate) fn docker_request(
    snapshot_id: &str,
    action: DockerAction,
    keys: Vec<String>,
) -> PrepareOperationRequest {
    PrepareOperationRequest::Docker {
        snapshot_id: snapshot_id.to_owned(),
        action,
        target_keys: keys,
    }
}

pub(crate) fn graceful_quit_request(
    snapshot_id: &str,
    app_keys: Vec<String>,
) -> PrepareOperationRequest {
    PrepareOperationRequest::AppGracefulQuit {
        snapshot_id: snapshot_id.to_owned(),
        app_keys,
    }
}

pub(crate) fn app_request(
    snapshot_id: &str,
    keys: Vec<String>,
    mode: ProcessMode,
) -> PrepareOperationRequest {
    PrepareOperationRequest::AppTerminate {
        snapshot_id: snapshot_id.to_owned(),
        app_keys: keys,
        mode,
    }
}

pub(crate) fn uninstall_request(
    app_snapshot_id: &str,
    residue_snapshot_id: &str,
    app_keys: Vec<String>,
    residue_keys: Vec<String>,
) -> PrepareOperationRequest {
    PrepareOperationRequest::Uninstall {
        app_snapshot_id: app_snapshot_id.to_owned(),
        residue_snapshot_id: residue_snapshot_id.to_owned(),
        app_keys,
        residue_keys,
        quit_running: false,
    }
}

pub(crate) struct InterleavedProcessSnapshots {
    first_overview: SnapshotResult<scanner::ScanResult>,
    managed: SnapshotResult<Vec<ProcessRow>>,
    second_overview: SnapshotResult<scanner::ScanResult>,
}

pub(crate) fn interleaved_process_snapshots(
    harness: &mut Harness,
    policy: &impl Fn(&str) -> bool,
) -> InterleavedProcessSnapshots {
    let first_overview = snapshot_process_scan(
        &mut harness.store.lock().unwrap(),
        scan_result_fixture(),
        policy,
    )
    .expect("主扫描快照必须注册成功");
    let managed = snapshot_process_rows(
        &mut harness.store.lock().unwrap(),
        vec![process_row(21, "gamma")],
        policy,
    )
    .expect("进程管理快照必须注册成功");
    let second_overview = snapshot_process_scan(
        &mut harness.store.lock().unwrap(),
        scan_result_fixture(),
        policy,
    )
    .expect("再次扫描必须注册成功");
    InterleavedProcessSnapshots {
        first_overview,
        managed,
        second_overview,
    }
}

pub(crate) fn live_process(pid: u32, name: &str) -> LiveProcess {
    live_process_with(pid, name, &format!("/usr/local/bin/{name}"))
}

#[path = "operation_commands_fakes.rs"]
mod fakes;

use fakes::*;

#[path = "operation_commands_scan_tests.rs"]
mod scan_tests;

#[path = "operation_commands_prepare_tests.rs"]
mod prepare_tests;

#[path = "operation_commands_execute_tests.rs"]
mod execute_tests;

#[path = "operation_commands_contract_tests.rs"]
mod contract_tests;

#[path = "operation_commands_integration_tests.rs"]
mod integration_tests;
