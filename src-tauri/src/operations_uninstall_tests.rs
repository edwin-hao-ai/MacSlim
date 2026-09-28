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

fn system_identity(label: &str) -> InstalledAppIdentity {
    InstalledAppIdentity {
        is_system: true,
        ..installed_identity(label)
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
#[test]
fn uninstall_snapshot_keys_are_opaque_and_per_resource() {
    let mut store = OperationStore::new();
    let (apps, residues) = register_uninstall_fixture(&mut store);

    assert_eq!(apps.selection_keys.len(), 2);
    assert_eq!(residues.selection_keys.len(), 2);
    assert!(apps
        .selection_keys
        .iter()
        .chain(residues.selection_keys.iter())
        .all(|key| key.len() == 64 && key != "alpha" && key != "beta"));
    assert!(apps
        .selection_keys
        .iter()
        .chain(residues.selection_keys.iter())
        .all(|key| key != "/Applications/alpha.app"));
    assert!(apps
        .selection_keys
        .iter()
        .all(|key| !residues.selection_keys.contains(key)));
}
#[test]
fn uninstall_plan_carries_backend_identity_and_selection_keys() {
    let mut store = OperationStore::new();
    let (apps, residues) = register_uninstall_fixture(&mut store);
    let prepared = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[1]],
            vec![&residues.selection_keys[1]],
            true,
            "main",
        )
        .unwrap();
    assert_eq!(prepared.kind, "uninstall");
    assert_eq!(prepared.item_count, 2);
    assert_eq!(prepared.estimated_bytes, 128);
    assert_eq!(
        super::describe_summary(&prepared),
        "opSummary.uninstallQuitFirst(apps=1, residues=1, size=128 B)"
    );

    let OperationPlan::Uninstall {
        targets,
        quit_running,
    } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected uninstall plan")
    };
    assert!(quit_running);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].app, installed_identity("beta"));
    assert_eq!(targets[0].app_key, apps.selection_keys[1]);
    assert_eq!(targets[0].residues.len(), 1);
    assert_eq!(targets[0].residues[0].app_key, apps.selection_keys[1]);
    assert_eq!(
        targets[0].residues[0].path,
        residue_identity("", "beta-cache").path
    );
}
#[test]
fn uninstall_rejects_client_supplied_paths_and_names() {
    let mut store = OperationStore::new();
    let (apps, residues) = register_uninstall_fixture(&mut store);

    assert!(store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec!["/Applications/alpha.app"],
            Vec::<&str>::new(),
            false,
            "main",
        )
        .is_err());
    assert!(store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0]],
            vec![library_path("alpha-cache")],
            false,
            "main",
        )
        .is_err());
    assert!(store.operations.is_empty());
}
#[test]
fn uninstall_rejects_keys_from_another_snapshot_of_same_kind() {
    let mut store = OperationStore::new();
    let (apps, residues) = register_uninstall_fixture(&mut store);
    let stale_residues = residues.clone();
    store
        .register_residues(vec![residue_identity(&apps.selection_keys[0], "fresh")])
        .unwrap();

    let error = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0]],
            vec![&stale_residues.selection_keys[0]],
            false,
            "main",
        )
        .unwrap_err();

    assert_eq!(error, "快照不存在或已失效");
}
#[test]
fn uninstall_rejects_app_key_used_as_residue_key() {
    let mut store = OperationStore::new();
    let (apps, residues) = register_uninstall_fixture(&mut store);

    let error = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0]],
            vec![&apps.selection_keys[1]],
            false,
            "main",
        )
        .unwrap_err();

    assert_eq!(error, "选择项不存在");
}
#[test]
fn uninstall_rejects_residue_key_bound_to_another_app() {
    let mut store = OperationStore::new();
    let (apps, residues) = register_uninstall_fixture(&mut store);

    let error = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0]],
            vec![&residues.selection_keys[1]],
            false,
            "main",
        )
        .unwrap_err();

    assert_eq!(error, "残留选择项不属于所选应用");
}
#[test]
fn uninstall_rejects_duplicate_residue_paths_inside_one_app() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![installed_identity("alpha")])
        .unwrap();
    let app_key = apps.selection_keys[0].clone();
    let mut shared = residue_identity(&app_key, "shared");
    let duplicated = ResidueIdentity {
        app_key: app_key.clone(),
        ..shared.clone()
    };
    shared.size_bytes = 0;
    let residues = store.register_residues(vec![shared, duplicated]).unwrap();

    let error = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&app_key],
            vec![&residues.selection_keys[0], &residues.selection_keys[1]],
            false,
            "main",
        )
        .unwrap_err();

    assert_eq!(error, "残留路径重复");
}
#[test]
fn uninstall_rejects_duplicate_residue_paths_across_apps() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![
            installed_identity("alpha"),
            installed_identity("beta"),
        ])
        .unwrap();
    let residues = store
        .register_residues(vec![
            residue_identity(&apps.selection_keys[0], "shared"),
            residue_identity(&apps.selection_keys[1], "shared"),
        ])
        .unwrap();

    let error = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0], &apps.selection_keys[1]],
            vec![&residues.selection_keys[0], &residues.selection_keys[1]],
            false,
            "main",
        )
        .unwrap_err();

    assert_eq!(error, "残留路径重复");
}
#[test]
fn uninstall_rejects_system_core_app() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![installed_identity("alpha"), system_identity("Finder")])
        .unwrap();
    let residues = store
        .register_residues_for_apps(vec![(
            apps.selection_keys[1].clone(),
            vec![residue_identity("", "finder-cache")],
        )])
        .unwrap();

    let error = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[1]],
            Vec::<&str>::new(),
            false,
            "main",
        )
        .unwrap_err();

    assert_eq!(error, "系统核心应用不能卸载");
    assert!(store.operations.is_empty());
}
#[test]
fn uninstall_rejects_same_bundle_registered_twice() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![
            installed_identity("alpha"),
            installed_identity("alpha"),
        ])
        .unwrap();
    let residues = store.register_residues(Vec::new()).unwrap();

    let error = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0], &apps.selection_keys[1]],
            Vec::<&str>::new(),
            false,
            "main",
        )
        .unwrap_err();

    assert_eq!(error, "同一应用被重复选择");
}
#[test]
fn uninstall_rejects_duplicate_quit_name() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![
            InstalledAppIdentity {
                bundle_path: "/Applications/alpha/Notes.app".to_owned(),
                app_name: "Notes".to_owned(),
                bundle_id: "com.example.alpha".to_owned(),
                is_system: false,
                bundle_size_bytes: 0,
            },
            InstalledAppIdentity {
                bundle_path: "/Applications/beta/Notes.app".to_owned(),
                app_name: "Notes".to_owned(),
                bundle_id: "com.example.beta".to_owned(),
                is_system: false,
                bundle_size_bytes: 0,
            },
        ])
        .unwrap();
    let residues = store.register_residues(Vec::new()).unwrap();

    let error = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0], &apps.selection_keys[1]],
            Vec::<&str>::new(),
            true,
            "main",
        )
        .unwrap_err();

    assert_eq!(error, "同名应用无法精确定位，请分开卸载");
    assert!(store.operations.is_empty());
}
#[test]
fn uninstall_estimated_bytes_include_bundle_size() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![InstalledAppIdentity {
            bundle_size_bytes: 4096,
            ..installed_identity("alpha")
        }])
        .unwrap();
    let residues = store.register_residues(Vec::new()).unwrap();

    let prepared = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0]],
            Vec::<&str>::new(),
            false,
            "main",
        )
        .unwrap();

    assert_eq!(prepared.estimated_bytes, 4096);
}
#[test]
fn prepared_summaries_are_domain_chinese_for_every_kind() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![installed_identity("alpha")])
        .unwrap();
    let residues = store
        .register_residues(vec![residue_identity(
            &apps.selection_keys[0],
            "alpha-cache",
        )])
        .unwrap();

    let uninstall = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &residues.snapshot_id,
            vec![&apps.selection_keys[0]],
            vec![&residues.selection_keys[0]],
            false,
            "main",
        )
        .unwrap();
    assert_eq!(
        super::describe_summary(&uninstall),
        "opSummary.uninstall(apps=1, residues=1, size=128 B)"
    );

    let process = store
        .register_process_snapshot(vec![crate::operations::ProcessTarget {
            identity: crate::operations::ProcessIdentity {
                pid: 42,
                name: "alpha".to_owned(),
                exe: "/usr/local/bin/alpha".to_owned(),
                start_time: 1,
            },
            protected: false,
            whitelisted: false,
        }])
        .unwrap();
    let forced = store
        .prepare_process(
            &process.snapshot_id,
            vec![&process.selection_keys[0]],
            ProcessMode::Force,
            "main",
        )
        .unwrap();
    assert_eq!(
        super::describe_summary(&forced),
        "opSummary.processForce(count=1)"
    );

    let apps_running = store
        .register_application_snapshot(vec![crate::operations::AppIdentity {
            bundle_path: "/Applications/alpha.app".to_owned(),
            bundle_id: "com.example.alpha".to_owned(),
            app_name: "alpha".to_owned(),
            processes: vec![crate::operations::ProcessTarget {
                identity: crate::operations::ProcessIdentity {
                    pid: 43,
                    name: "alpha".to_owned(),
                    exe: "/Applications/alpha.app/Contents/MacOS/alpha".to_owned(),
                    start_time: 1,
                },
                protected: false,
                whitelisted: false,
            }],
        }])
        .unwrap();
    let terminated = store
        .prepare_app_termination(
            &apps_running.snapshot_id,
            vec![&apps_running.app_keys[0]],
            ProcessMode::Force,
            "main",
        )
        .unwrap();
    assert_eq!(
        super::describe_summary(&terminated),
        "opSummary.appTerminate(apps=1, processes=1)"
    );
}
