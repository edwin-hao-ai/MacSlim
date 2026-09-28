use super::*;
use crate::operations::OperationStore;
use crate::residue_policy;

fn residue_path(label: &str) -> String {
    dirs::home_dir()
        .expect("测试环境必须有用户主目录")
        .join("Library/Caches")
        .join(format!("com.example.{label}"))
        .to_string_lossy()
        .to_string()
}

fn residue_item(label: &str) -> ResidueItem {
    ResidueItem {
        path: residue_path(label),
        category: "Caches".to_owned(),
        size_bytes: 64,
        is_dev_tool: false,
        selected: true,
        selection_key: String::new(),
    }
}

fn app_residue(bundle_id: &str, labels: &[&str]) -> AppResidue {
    AppResidue {
        bundle_id: bundle_id.to_owned(),
        app_name: "alpha".to_owned(),
        items: labels.iter().map(|label| residue_item(label)).collect(),
        total_bytes: 64 * labels.len() as u64,
        scan_complete: true,
    }
}

#[test]
fn uninstaller_residue_registration_binds_keys_to_the_backend_app_key() {
    let mut store = OperationStore::new();
    let app_key = store
        .register_installed_apps(vec![crate::operations::InstalledAppIdentity {
            bundle_path: "/Applications/alpha.app".to_owned(),
            app_name: "alpha".to_owned(),
            bundle_id: "com.example.alpha".to_owned(),
            is_system: false,
            bundle_size_bytes: 0,
        }])
        .unwrap()
        .selection_keys[0]
        .clone();
    let mut scanned = app_residue("com.example.alpha", &["alpha-cache", "alpha-log"]);

    let registration = register_app_residues(&mut store, &app_key, &mut scanned).unwrap();

    assert_eq!(registration.selection_keys.len(), 2);
    assert!(scanned
        .items
        .iter()
        .all(|item| item.selection_key.len() == 64));
    assert_eq!(
        scanned.items[0].selection_key,
        registration.selection_keys[0]
    );
    assert_ne!(scanned.items[0].selection_key, residue_path("alpha-cache"));
}

#[test]
fn uninstaller_residue_registration_rejects_unknown_app_key() {
    let mut store = OperationStore::new();
    store
        .register_installed_apps(vec![crate::operations::InstalledAppIdentity {
            bundle_path: "/Applications/alpha.app".to_owned(),
            app_name: "alpha".to_owned(),
            bundle_id: "com.example.alpha".to_owned(),
            is_system: false,
            bundle_size_bytes: 0,
        }])
        .unwrap();
    let mut scanned = app_residue("com.example.alpha", &["alpha-cache"]);

    let error = register_app_residues(&mut store, "client-supplied-key", &mut scanned).unwrap_err();

    assert_eq!(error, "残留快照引用了未知应用选择项");
    assert!(scanned.items[0].selection_key.is_empty());
}

#[test]
fn uninstaller_residue_registration_requires_an_app_snapshot() {
    let mut store = OperationStore::new();
    let mut scanned = app_residue("com.example.alpha", &["alpha-cache"]);

    let error = register_app_residues(&mut store, "any-key", &mut scanned).unwrap_err();

    assert_eq!(error, "残留快照引用了未知应用选择项");
}

#[test]
fn uninstaller_residue_scan_is_empty_without_a_bundle_id() {
    let scanned = scan_residues("", "alpha");

    assert!(!scanned.scan_complete);
    assert!(scanned.items.is_empty());
    assert_eq!(scanned.total_bytes, 0);
}

#[test]
fn uninstaller_residue_matching_accepts_bundle_id_and_name() {
    assert!(matches_residue(
        "com.example.alpha",
        "com.example.alpha",
        "alpha"
    ));
    assert!(matches_residue("AlphaCache", "com.example.alpha", "alpha"));
    assert!(!matches_residue("OtherApp", "com.example.alpha", "alpha"));
}

fn two_app_residues() -> (AppResidue, AppResidue) {
    (
        app_residue("com.example.alpha", &["alpha-cache", "alpha-log"]),
        app_residue("com.example.beta", &["beta-cache"]),
    )
}

#[test]
fn uninstaller_residue_batch_registration_serves_every_app_in_one_snapshot() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![
            crate::operations::InstalledAppIdentity {
                bundle_path: "/Applications/alpha.app".to_owned(),
                app_name: "alpha".to_owned(),
                bundle_id: "com.example.alpha".to_owned(),
                is_system: false,
                bundle_size_bytes: 0,
            },
            crate::operations::InstalledAppIdentity {
                bundle_path: "/Applications/beta.app".to_owned(),
                app_name: "beta".to_owned(),
                bundle_id: "com.example.beta".to_owned(),
                is_system: false,
                bundle_size_bytes: 0,
            },
        ])
        .unwrap();
    let app_keys = apps.selection_keys.clone();
    let (mut alpha, mut beta) = two_app_residues();

    let registration = register_app_residue_batch(
        &mut store,
        vec![
            (app_keys[0].clone(), &mut alpha),
            (app_keys[1].clone(), &mut beta),
        ],
    );
    let registration = registration.unwrap();

    assert_eq!(registration.selection_keys.len(), 3);
    assert!(alpha
        .items
        .iter()
        .chain(beta.items.iter())
        .all(|item| item.selection_key.len() == 64));
    let keys: Vec<String> = alpha
        .items
        .iter()
        .chain(beta.items.iter())
        .map(|item| item.selection_key.clone())
        .collect();
    let prepared = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &registration.snapshot_id,
            [app_keys[0].as_str(), app_keys[1].as_str()],
            keys.iter().map(String::as_str),
            false,
            "main",
        )
        .unwrap();
    assert_eq!(prepared.item_count, 5);
}

#[test]
fn uninstaller_residue_batch_registration_rejects_empty_batches() {
    let mut store = OperationStore::new();
    let (mut alpha, _beta) = two_app_residues();

    assert!(register_app_residue_batch(&mut store, Vec::new()).is_err());
    assert!(register_app_residue_batch(&mut store, vec![(String::new(), &mut alpha)]).is_err());
    assert!(alpha.items.iter().all(|item| item.selection_key.is_empty()));
}

#[test]
fn uninstaller_residue_batch_registration_rejects_out_of_scope_path() {
    let mut store = OperationStore::new();
    let app_key = store
        .register_installed_apps(vec![crate::operations::InstalledAppIdentity {
            bundle_path: "/Applications/alpha.app".to_owned(),
            app_name: "alpha".to_owned(),
            bundle_id: "com.example.alpha".to_owned(),
            is_system: false,
            bundle_size_bytes: 0,
        }])
        .unwrap()
        .selection_keys[0]
        .clone();
    let mut alpha = app_residue("com.example.alpha", &["alpha-cache"]);
    alpha.items[0].path = "/etc/alpha".to_owned();

    let error = register_app_residue_batch(&mut store, vec![(app_key, &mut alpha)]).unwrap_err();

    assert!(error.contains("不在允许的清理范围"), "{}", error);
    assert!(alpha.items[0].selection_key.is_empty());
}

#[test]
fn uninstaller_residue_scan_drops_dev_tool_paths_outside_allowed_roots() {
    let mut items = Vec::new();
    let home = dirs::home_dir().expect("测试环境必须有用户主目录");

    let roots = residue_policy::allowed_roots("com.example.alpha");
    scan_dev_tool_paths("com.example.alpha", &home, &roots, &mut items);

    assert!(items.is_empty(), "没有 dev-tool 规则的应用不应产生额外残留");
}

#[test]
fn uninstaller_residue_batch_registration_skips_apps_without_residues() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![
            crate::operations::InstalledAppIdentity {
                bundle_path: "/Applications/alpha.app".to_owned(),
                app_name: "alpha".to_owned(),
                bundle_id: "com.example.alpha".to_owned(),
                is_system: false,
                bundle_size_bytes: 0,
            },
            crate::operations::InstalledAppIdentity {
                bundle_path: "/Applications/beta.app".to_owned(),
                app_name: "beta".to_owned(),
                bundle_id: "com.example.beta".to_owned(),
                is_system: false,
                bundle_size_bytes: 0,
            },
        ])
        .unwrap();
    let mut alpha = app_residue("com.example.alpha", &["alpha-cache"]);
    let mut beta = app_residue("com.example.beta", &[]);

    let registration = register_app_residue_batch(
        &mut store,
        vec![
            (apps.selection_keys[0].clone(), &mut alpha),
            (apps.selection_keys[1].clone(), &mut beta),
        ],
    )
    .unwrap();

    assert_eq!(registration.selection_keys.len(), 1);
    assert_eq!(alpha.items[0].selection_key.len(), 64);
    assert!(beta.items.is_empty());
    let prepared = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &registration.snapshot_id,
            [
                apps.selection_keys[0].as_str(),
                apps.selection_keys[1].as_str(),
            ],
            [registration.selection_keys[0].as_str()],
            false,
            "main",
        )
        .unwrap();
    assert_eq!(prepared.item_count, 3);
}

#[test]
fn uninstaller_residue_batch_registration_supports_all_apps_without_residues() {
    let mut store = OperationStore::new();
    let apps = store
        .register_installed_apps(vec![crate::operations::InstalledAppIdentity {
            bundle_path: "/Applications/alpha.app".to_owned(),
            app_name: "alpha".to_owned(),
            bundle_id: "com.example.alpha".to_owned(),
            is_system: false,
            bundle_size_bytes: 0,
        }])
        .unwrap();
    let mut alpha = app_residue("com.example.alpha", &[]);

    let registration = register_app_residue_batch(
        &mut store,
        vec![(apps.selection_keys[0].clone(), &mut alpha)],
    )
    .unwrap();

    assert!(registration.selection_keys.is_empty());
    let prepared = store
        .prepare_uninstall(
            &apps.snapshot_id,
            &registration.snapshot_id,
            vec![&apps.selection_keys[0]],
            Vec::<&str>::new(),
            false,
            "main",
        )
        .unwrap();
    assert_eq!(prepared.item_count, 1);
}

#[test]
fn uninstaller_scan_directory_drops_entries_outside_allowed_roots() {
    let matched = "com.example.scanfixture";
    let outside = outside_fixture_dir("macslim-scan-outside", matched);
    let mut items = Vec::new();

    let roots = residue_policy::allowed_roots(matched);
    scan_directory(
        &entries_of(&outside),
        "Caches",
        matched,
        matched,
        &roots,
        &mut items,
    );

    assert!(
        items.is_empty(),
        "允许根之外的匹配项不能产出残留：{:?}",
        items
    );
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn uninstaller_scan_directory_drops_symlinked_entries_that_escape_allowed_roots() {
    let matched = "com.example.scanescape";
    let outside = outside_fixture_dir("macslim-scan-escape-target", "payload");
    let link = dirs::home_dir()
        .expect("测试环境必须有用户主目录")
        .join("Library/Caches")
        .join(matched);
    let _ = std::fs::remove_file(&link);
    if std::os::unix::fs::symlink(&outside, &link).is_err() {
        let _ = std::fs::remove_dir_all(&outside);
        return;
    }
    let scan_root = outside_fixture_dir("macslim-scan-escape-root", "");
    let _ = std::fs::remove_dir(&scan_root);
    let holder =
        std::env::temp_dir().join(format!("macslim-scan-escape-root-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&holder);
    if std::fs::create_dir_all(holder.join(matched)).is_err() {
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_dir_all(&outside);
        return;
    }
    let _ = std::os::unix::fs::symlink(&link, holder.join(matched));
    let mut items = Vec::new();

    let roots = residue_policy::allowed_roots(matched);
    scan_directory(
        &entries_of(&holder),
        "Caches",
        matched,
        matched,
        &roots,
        &mut items,
    );

    let _ = std::fs::remove_file(&link);
    let _ = std::fs::remove_dir_all(&outside);
    let _ = std::fs::remove_dir_all(&holder);
    assert!(
        items.is_empty(),
        "指向允许根之外的软链接不能产出残留：{:?}",
        items
    );
}

#[test]
fn uninstaller_scan_directory_keeps_entries_inside_allowed_roots() {
    let matched = "com.example.scaninside";
    let dir = library_root_fixture("macslim-scan-inside", matched);
    let Some(dir) = dir else {
        return;
    };
    let mut items = Vec::new();

    let roots = residue_policy::allowed_roots(matched);
    scan_directory(
        &entries_of(&dir),
        "Caches",
        matched,
        matched,
        &roots,
        &mut items,
    );

    assert_eq!(items.len(), 1, "允许根之内的匹配项必须保留");
    assert!(items[0].path.starts_with(dir.to_string_lossy().as_ref()));
    assert!(items[0].selected);
    assert!(items[0].selection_key.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

fn entries_of(dir: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| {
                    (
                        entry.file_name().to_string_lossy().to_string(),
                        entry.path(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn outside_fixture_dir(name: &str, matched: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("临时目录必须可创建");
    if !matched.is_empty() {
        std::fs::create_dir_all(path.join(matched)).expect("临时子目录必须可创建");
    }
    path
}

fn library_root_fixture(name: &str, matched: &str) -> Option<std::path::PathBuf> {
    let dir = dirs::home_dir()
        .expect("测试环境必须有用户主目录")
        .join("Library/Caches")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(matched)).ok()?;
    Some(dir)
}
