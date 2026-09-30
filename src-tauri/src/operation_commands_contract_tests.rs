use super::*;

// ===== 静态契约 =====

fn registered_commands() -> Vec<String> {
    let source = include_str!("lib.rs");
    let start = source
        .find("tauri::generate_handler![")
        .expect("lib.rs 必须注册 invoke handler");
    let tail = &source[start..];
    let end = tail.find(']').expect("invoke handler 必须闭合");
    tail[..end]
        .lines()
        .map(|line| line.trim().trim_end_matches(',').to_owned())
        .filter(|line| !line.is_empty() && !line.starts_with("tauri::"))
        .collect()
}

#[test]
fn invoke_handler_only_exposes_trusted_commands() {
    assert_eq!(
        registered_commands(),
        vec![
            // 纯元数据：下发构建形态（developer_id / mas），让前端在 App Store
            // 版里藏掉沙箱里做不到的「终止进程」入口。零副作用。
            "get_build_flavor",
            // 纯元数据：探测完全磁盘访问权限。分「用户缓存」与「系统目录」
            // 两类，因为它们挡住的**能力**不同（缓存清理 vs 进程枚举）。
            "get_fda_status",
            "get_system_health",
            "scan_all",
            "list_all_processes",
            "scan_cache",
            "get_history",
            "get_whitelist",
            "add_whitelist",
            "remove_whitelist",
            "list_applications",
            "docker_available",
            "docker_inventory",
            "scan_installed_apps",
            "scan_app_residues_batch",
            "check_app_running",
            "prepare_operation",
            "execute_operation",
        ]
    );
}

#[test]
fn raw_destructive_commands_are_gone_from_the_tauri_surface() {
    let source = include_str!("lib.rs");
    for removed in [
        "kill_processes",
        "clean_cache",
        "quit_application",
        "force_quit_application",
        "uninstall_apps",
        "quit_and_uninstall",
        "docker_remove_image",
        "docker_remove_container",
        "docker_remove_volume",
        "docker_prune_all",
        "scan_app_residues(",
    ] {
        assert!(
            !source.contains(removed),
            "lib.rs 仍残留 raw 入口: {removed}"
        );
    }
}

#[test]
fn operation_kind_labels_are_stable() {
    assert_eq!(OperationKind::Cache.label(), "cache");
    assert_eq!(OperationKind::Process.label(), "process");
    assert_eq!(OperationKind::AppTerminate.label(), "app_terminate");
    assert_eq!(OperationKind::Uninstall.label(), "uninstall");
    assert_eq!(OperationKind::Docker.label(), "docker");
}

#[test]
fn residue_batch_scanner_receives_backend_identity_only() {
    let mut harness = Harness::new();
    let apps = snapshot_installed_apps(
        &mut harness.store.lock().unwrap(),
        vec![installed_app("Alpha"), installed_app("Beta")],
    )
    .expect("已安装应用快照");
    let app_keys: Vec<String> = apps
        .value
        .iter()
        .map(|app| app.selection_key.clone())
        .collect();
    let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);

    scan_residue_batch(&mut harness, &apps.snapshot_id, &app_keys, |identity| {
        recorder
            .lock()
            .unwrap()
            .push((identity.bundle_id.clone(), identity.app_name.clone()));
        app_residue(&identity.bundle_id, &identity.app_name, &[])
    })
    .expect("批量残留扫描");

    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            ("com.example.Alpha".to_owned(), "Alpha".to_owned()),
            ("com.example.Beta".to_owned(), "Beta".to_owned()),
        ]
    );
}

#[test]
fn residue_paths_outside_allowed_roots_are_rejected() {
    let mut harness = Harness::new();
    let apps = snapshot_installed_apps(
        &mut harness.store.lock().unwrap(),
        vec![installed_app("Alpha")],
    )
    .expect("已安装应用快照");
    let app_key = apps.value[0].selection_key.clone();
    let outside = PathBuf::from("/etc/hosts").to_string_lossy().to_string();

    let error = scan_residue_batch(
        &mut harness,
        &apps.snapshot_id,
        std::slice::from_ref(&app_key),
        |_identity| AppResidue {
            bundle_id: "com.example.Alpha".to_owned(),
            app_name: "Alpha".to_owned(),
            items: vec![ResidueItem {
                path: outside.clone(),
                category: "Caches".to_owned(),
                size_bytes: 1,
                is_dev_tool: false,
                selected: true,
                selection_key: String::new(),
            }],
            total_bytes: 1,
            scan_complete: true,
        },
    )
    .expect_err("越界残留路径必须被拒绝");

    assert!(error.contains("允许的清理范围"), "{error}");
}

#[test]
fn every_snapshot_kind_has_exactly_one_producer() {
    let source = include_str!("operation_commands.rs");
    let kinds: Vec<String> = source
        .match_indices("SnapshotKind::")
        .map(|(index, _)| {
            source[index + "SnapshotKind::".len()..]
                .chars()
                .take_while(|character| character.is_alphanumeric())
                .collect()
        })
        .collect();
    let mut unique = kinds.clone();
    unique.sort();
    unique.dedup();

    assert_eq!(
        kinds.len(),
        unique.len(),
        "同一个 SnapshotKind 被多个生产者写入，会互相顶掉: {kinds:?}"
    );
    assert_eq!(
        unique,
        vec![
            "Application",
            "Cache",
            "Docker",
            "InstalledApps",
            "OverviewProcess",
            "Process",
            "Residue",
        ],
        "snapshot 槽位集合发生变化，必须同步 plan/spec 与前端"
    );
}
