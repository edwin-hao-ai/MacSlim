use super::*;

fn installed_identity(label: &str) -> InstalledAppIdentity {
    InstalledAppIdentity {
        bundle_path: format!("/Applications/{label}.app"),
        app_name: label.to_owned(),
        bundle_id: format!("com.example.{label}"),
        is_system: false,
        bundle_size_bytes: 0,
    }
}

fn library_path(label: &str) -> String {
    dirs::home_dir()
        .expect("测试环境必须有用户主目录")
        .join("Library/Caches")
        .join(format!("com.example.{label}"))
        .to_string_lossy()
        .to_string()
}

fn residue_identity(app_key: &str, label: &str) -> ResidueIdentity {
    ResidueIdentity {
        app_key: app_key.to_owned(),
        path: library_path(label),
        category: "Caches".to_owned(),
        size_bytes: 128,
    }
}

fn register_uninstall_fixture(
    store: &mut OperationStore,
) -> (SnapshotRegistration, SnapshotRegistration) {
    let apps = store
        .register_installed_apps(vec![
            installed_identity("alpha"),
            installed_identity("beta"),
        ])
        .unwrap();
    let residues = store
        .register_residues(vec![
            residue_identity(&apps.selection_keys[0], "alpha-cache"),
            residue_identity(&apps.selection_keys[1], "beta-cache"),
        ])
        .unwrap();
    (apps, residues)
}
fn batch_fixture(store: &mut OperationStore) -> (SnapshotRegistration, SnapshotRegistration) {
    let apps = store
        .register_installed_apps(vec![
            installed_identity("alpha"),
            installed_identity("beta"),
        ])
        .unwrap();
    let residues = store
        .register_residues_for_apps(vec![
            (
                apps.selection_keys[0].clone(),
                vec![
                    residue_identity("", "alpha-cache"),
                    residue_identity("", "alpha-log"),
                ],
            ),
            (
                apps.selection_keys[1].clone(),
                vec![residue_identity("", "beta-cache")],
            ),
        ])
        .unwrap();
    (apps, residues)
}

#[test]
fn residue_snapshot_rejects_app_key_outside_active_app_snapshot() {
    let mut store = OperationStore::new();
    store
        .register_installed_apps(vec![installed_identity("alpha")])
        .unwrap();

    let error = store
        .register_residues(vec![residue_identity("not-a-backend-key", "alpha-cache")])
        .unwrap_err();

    assert_eq!(error, "残留快照引用了未知应用选择项");
}
#[test]
fn residue_snapshot_requires_the_app_snapshot_to_be_registered() {
    let mut store = OperationStore::new();

    let error = store
        .register_residues(vec![residue_identity("any-key", "alpha-cache")])
        .unwrap_err();

    assert_eq!(error, "残留快照引用了未知应用选择项");
}
#[test]
fn residue_snapshot_must_use_the_dedicated_registration_entry() {
    let mut store = OperationStore::new();
    let app_key = store
        .register_installed_apps(vec![installed_identity("alpha")])
        .unwrap()
        .selection_keys[0]
        .clone();

    let error = store
        .register_snapshot(
            SnapshotKind::Residue,
            SnapshotPayload::Residue(vec![residue_identity(&app_key, "alpha-cache")]),
        )
        .unwrap_err();

    assert_eq!(error, "残留快照必须使用专用注册接口");
}
#[test]
fn uninstall_batch_residues_serve_every_app_in_one_prepare() {
    let mut store = OperationStore::new();
    let (apps, residues) = batch_fixture(&mut store);

    let prepared = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0], &apps.selection_keys[1]],
            vec![
                &residues.selection_keys[0],
                &residues.selection_keys[1],
                &residues.selection_keys[2],
            ],
            false,
            "main",
        )
        .unwrap();

    assert_eq!(prepared.item_count, 5);
    let OperationPlan::Uninstall { targets, .. } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected uninstall plan")
    };
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].app_key, apps.selection_keys[0]);
    assert_eq!(targets[0].residues.len(), 2);
    assert_eq!(targets[1].app_key, apps.selection_keys[1]);
    assert_eq!(targets[1].residues.len(), 1);
    assert!(targets
        .iter()
        .flat_map(|target| target.residues.iter())
        .all(|residue| residue.path.starts_with(
            dirs::home_dir()
                .expect("测试环境必须有用户主目录")
                .to_string_lossy()
                .to_string()
                .as_str()
        )));
}
#[test]
fn uninstall_batch_residues_bind_each_key_to_its_own_app() {
    let mut store = OperationStore::new();
    let (apps, residues) = batch_fixture(&mut store);
    let prepared = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[1]],
            vec![&residues.selection_keys[2]],
            false,
            "main",
        )
        .unwrap();

    let OperationPlan::Uninstall { targets, .. } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected uninstall plan")
    };

    assert_eq!(targets[0].residues[0].app_key, apps.selection_keys[1]);
    assert_eq!(targets[0].residues[0].path, library_path("beta-cache"));
}
#[test]
fn uninstall_batch_residues_keep_input_key_order() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![
            installed_identity("alpha"),
            installed_identity("beta"),
        ])
        .unwrap();

    let residues = store
        .register_residues_for_apps(vec![
            (
                apps.selection_keys[1].clone(),
                vec![residue_identity("", "beta-cache")],
            ),
            (
                apps.selection_keys[0].clone(),
                vec![residue_identity("", "alpha-cache")],
            ),
        ])
        .unwrap();

    assert_eq!(residues.selection_keys.len(), 2);
    let prepared = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0], &apps.selection_keys[1]],
            vec![&residues.selection_keys[0], &residues.selection_keys[1]],
            false,
            "main",
        )
        .unwrap();
    let OperationPlan::Uninstall { targets, .. } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected uninstall plan")
    };
    assert_eq!(targets[0].residues.len(), 1);
    assert_eq!(targets[0].residues[0].path, library_path("alpha-cache"));
    assert_eq!(targets[1].residues.len(), 1);
    assert_eq!(targets[1].residues[0].path, library_path("beta-cache"));
}
#[test]
fn uninstall_batch_residue_key_index_follows_the_input_order() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![
            installed_identity("alpha"),
            installed_identity("beta"),
        ])
        .unwrap();
    let residues = store
        .register_residues_for_apps(vec![
            (
                apps.selection_keys[1].clone(),
                vec![
                    residue_identity("", "beta-cache"),
                    residue_identity("", "beta-log"),
                ],
            ),
            (
                apps.selection_keys[0].clone(),
                vec![residue_identity("", "alpha-cache")],
            ),
        ])
        .unwrap();

    let first_key = residues.selection_keys[0].clone();
    let prepared = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[1]],
            vec![&first_key],
            false,
            "main",
        )
        .unwrap();
    let OperationPlan::Uninstall { targets, .. } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected uninstall plan")
    };

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].app.app_name, "beta");
    assert_eq!(targets[0].residues[0].path, library_path("beta-cache"));
}
#[test]
fn uninstall_batch_residues_reject_unknown_app_key() {
    let mut store = OperationStore::new();
    store
        .register_installed_apps(vec![installed_identity("alpha")])
        .unwrap();

    let error = store
        .register_residues_for_apps(vec![(
            "client-supplied-key".to_owned(),
            vec![residue_identity("", "alpha-cache")],
        )])
        .unwrap_err();

    assert_eq!(error, "残留快照引用了未知应用选择项");
}
#[test]
fn uninstall_batch_residues_reject_empty_input() {
    let mut store = OperationStore::new();
    store
        .register_installed_apps(vec![installed_identity("alpha")])
        .unwrap();

    assert!(store.register_residues_for_apps(Vec::new()).is_err());
    assert!(store
        .register_residues_for_apps(vec![(String::new(), Vec::new())])
        .is_err());
}
#[test]
fn uninstall_residues_outside_allowed_roots_are_rejected() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![installed_identity("alpha")])
        .unwrap();
    let app_key = apps.selection_keys[0].clone();
    let home = dirs::home_dir().expect("测试环境必须有用户主目录");

    for outside in [
        "/tmp/alpha-cache".to_owned(),
        "/etc/alpha".to_owned(),
        home.join("Documents/alpha").to_string_lossy().to_string(),
        home.join("Library").to_string_lossy().to_string(),
        home.join("Library/Caches/../../../etc")
            .to_string_lossy()
            .to_string(),
    ] {
        let error = store
            .register_residues(vec![ResidueIdentity {
                app_key: app_key.clone(),
                path: outside.clone(),
                category: "Caches".to_owned(),
                size_bytes: 8,
            }])
            .unwrap_err();
        assert!(error.contains("不在允许的清理范围"), "{outside} -> {error}");
    }
    assert!(store.operations.is_empty());
}
#[test]
fn uninstall_residues_inside_allowed_roots_are_accepted() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![installed_identity("alpha")])
        .unwrap();
    let home = dirs::home_dir().expect("测试环境必须有用户主目录");

    let residues = store
        .register_residues(vec![
            residue_identity(&apps.selection_keys[0], "alpha-cache"),
            ResidueIdentity {
                app_key: apps.selection_keys[0].clone(),
                path: home
                    .join("Library/Logs/com.example.alpha")
                    .to_string_lossy()
                    .to_string(),
                category: "Logs".to_owned(),
                size_bytes: 8,
            },
        ])
        .unwrap();

    assert_eq!(residues.selection_keys.len(), 2);
}
#[test]
fn uninstall_residues_allowed_roots_follow_the_backend_bundle_id() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![InstalledAppIdentity {
            bundle_path: "/Applications/Code.app".to_owned(),
            app_name: "Code".to_owned(),
            bundle_id: "com.microsoft.VSCode".to_owned(),
            is_system: false,
            bundle_size_bytes: 0,
        }])
        .unwrap();
    let home = dirs::home_dir().expect("测试环境必须有用户主目录");

    let allowed = store
        .register_residues(vec![ResidueIdentity {
            app_key: apps.selection_keys[0].clone(),
            path: home
                .join(".vscode/extensions")
                .to_string_lossy()
                .to_string(),
            category: "VS Code 专属数据".to_owned(),
            size_bytes: 8,
        }])
        .unwrap();
    assert_eq!(allowed.selection_keys.len(), 1);

    let other = store
        .register_installed_apps(vec![installed_identity("alpha")])
        .unwrap();
    let error = store
        .register_residues(vec![ResidueIdentity {
            app_key: other.selection_keys[0].clone(),
            path: home
                .join(".vscode/extensions")
                .to_string_lossy()
                .to_string(),
            category: "VS Code 专属数据".to_owned(),
            size_bytes: 8,
        }])
        .unwrap_err();

    assert!(error.contains("不在允许的清理范围"), "{}", error);
}
#[test]
fn new_installed_app_snapshot_invalidates_residue_snapshot_and_operation() {
    let mut store = OperationStore::new();
    let (apps, residues) = register_uninstall_fixture(&mut store);
    let prepared = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0]],
            vec![&residues.selection_keys[0]],
            false,
            "main",
        )
        .unwrap();

    store
        .register_installed_apps(vec![installed_identity("gamma")])
        .unwrap();

    assert!(store.consume(&prepared.operation_id, "main").is_err());
    let error = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0]],
            vec![&residues.selection_keys[0]],
            false,
            "main",
        )
        .unwrap_err();
    assert_eq!(error, "快照不存在或已失效");
}

#[test]
fn uninstall_batch_ignores_apps_without_residues() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![
            installed_identity("alpha"),
            installed_identity("beta"),
        ])
        .unwrap();

    let residues = store
        .register_residues_for_apps(vec![
            (
                apps.selection_keys[0].clone(),
                vec![residue_identity("", "alpha-cache")],
            ),
            (apps.selection_keys[1].clone(), Vec::new()),
        ])
        .unwrap();

    assert_eq!(residues.selection_keys.len(), 1);
    let prepared = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0], &apps.selection_keys[1]],
            vec![&residues.selection_keys[0]],
            false,
            "main",
        )
        .unwrap();
    assert_eq!(prepared.item_count, 3, "2 个应用 + 1 项残留");
    let OperationPlan::Uninstall { targets, .. } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected uninstall plan")
    };
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].residues.len(), 1);
    assert!(targets[1].residues.is_empty());
}

#[test]
fn uninstall_batch_with_no_residues_at_all_still_uninstalls_apps() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![
            installed_identity("alpha"),
            installed_identity("beta"),
        ])
        .unwrap();

    let residues = store
        .register_residues_for_apps(vec![
            (apps.selection_keys[0].clone(), Vec::new()),
            (apps.selection_keys[1].clone(), Vec::new()),
        ])
        .unwrap();

    assert!(residues.selection_keys.is_empty());
    let prepared = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0], &apps.selection_keys[1]],
            Vec::<&str>::new(),
            false,
            "main",
        )
        .unwrap();

    assert_eq!(prepared.item_count, 2);
    assert_eq!(prepared.estimated_bytes, 0);
    let OperationPlan::Uninstall { targets, .. } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected uninstall plan")
    };
    assert_eq!(targets.len(), 2);
    assert!(targets.iter().all(|target| target.residues.is_empty()));
}

#[test]
fn uninstall_batch_still_rejects_empty_app_key() {
    let mut store = OperationStore::new();
    store
        .register_installed_apps(vec![installed_identity("alpha")])
        .unwrap();

    assert!(store
        .register_residues_for_apps(vec![(
            String::new(),
            vec![residue_identity("", "alpha-cache")]
        )])
        .is_err());
    assert!(store
        .register_residues_for_apps(vec![(String::new(), Vec::new())])
        .is_err());
}
