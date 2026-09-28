use super::*;
use crate::operations::{InstalledAppIdentity, ResidueIdentity};
use std::path::{Path, PathBuf};

fn bundle(name: &str) -> String {
    format!("/Applications/{name}.app")
}

#[tokio::test]
async fn uninstaller_observe_apps_skips_missing_bundles() {
    let observed = SystemUninstaller
        .observe_apps(&[bundle("does-not-exist-macslim")])
        .await
        .unwrap();

    assert!(observed.is_empty());
}

#[tokio::test]
async fn uninstaller_observe_apps_rejects_non_bundle_paths() {
    let observed = SystemUninstaller
        .observe_apps(&["/".to_owned()])
        .await
        .unwrap();

    assert!(observed.is_empty(), "目录不是 .app，不能被当成应用");
}

#[tokio::test]
async fn uninstaller_observe_apps_reads_identity_from_bundle_metadata() {
    let terminal = PathBuf::from("/System/Applications/Utilities/Terminal.app");
    if !terminal.exists() {
        return;
    }

    let observed = SystemUninstaller
        .observe_apps(&[terminal.to_string_lossy().to_string()])
        .await
        .unwrap();

    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].bundle_path, terminal.to_string_lossy());
    assert_eq!(observed[0].bundle_id, "com.apple.Terminal");
    assert_eq!(observed[0].app_name, "Terminal");
}

#[test]
fn uninstaller_observe_apps_falls_back_to_bundle_file_name() {
    let missing = PathBuf::from("/Applications/MacSlimFallback.app");
    if missing.exists() {
        return;
    }

    assert!(read_installed_identity(&missing).is_none());
    assert_eq!(bundle_display_name(&missing, None), "MacSlimFallback");
    assert_eq!(
        bundle_display_name(&missing, Some("Beta".to_owned())),
        "Beta"
    );
    assert_eq!(bundle_display_name(Path::new("/"), None), "未知应用");
}

#[tokio::test]
async fn uninstaller_observe_residues_marks_absent_paths() {
    let observed = SystemUninstaller
        .observe_residues(&["/tmp/macslim-does-not-exist-9d2f".to_owned()])
        .await
        .unwrap();

    assert_eq!(observed.len(), 1);
    assert!(!observed[0].exists);
    assert_eq!(observed[0].path, "/tmp/macslim-does-not-exist-9d2f");
}

#[tokio::test]
async fn uninstaller_observe_residues_canonicalizes_existing_paths() {
    let existing = std::env::current_exe().expect("测试二进制必须存在");
    let requested = existing.to_string_lossy().to_string();

    let observed = SystemUninstaller
        .observe_residues(&[requested])
        .await
        .unwrap();

    assert_eq!(observed.len(), 1);
    assert!(observed[0].exists);
    assert_eq!(
        observed[0].path,
        existing
            .canonicalize()
            .expect("测试二进制必须可规范化")
            .to_string_lossy()
    );
}

#[tokio::test]
async fn uninstaller_observe_residues_keeps_requested_order_and_marks_existence() {
    let existing = std::env::current_exe().expect("测试二进制必须存在");
    let requested = existing.to_string_lossy().to_string();
    let missing = "/tmp/macslim-does-not-exist-9d2f".to_owned();

    let observed = SystemUninstaller
        .observe_residues(&[missing.clone(), requested])
        .await
        .unwrap();

    assert_eq!(observed.len(), 2);
    assert_eq!(observed[0].path, missing);
    assert!(!observed[0].exists);
    assert!(observed[1].exists);
}

#[test]
fn uninstaller_is_app_running_never_matches_unrelated_path() {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    assert!(!is_app_running("/nonexistent/macslim-app", &mut sys));
}

#[test]
fn uninstaller_quit_script_escapes_quotes_and_backslashes() {
    assert_eq!(quit_script("Alpha"), "tell application \"Alpha\" to quit");
    assert_eq!(
        quit_script("He said \"hi\""),
        "tell application \"He said \\\"hi\\\"\" to quit"
    );
    assert_eq!(
        quit_script("Back\\slash"),
        "tell application \"Back\\\\slash\" to quit"
    );
    assert_eq!(
        quit_script("Mixed \\\"q\\\" end"),
        "tell application \"Mixed \\\\\\\"q\\\\\\\" end\" to quit"
    );
}

#[test]
fn uninstaller_quit_script_keeps_app_name_inside_the_literal() {
    let script = quit_script("Evil\" & (do shell script \"rm -rf ~\") & \"");

    assert_eq!(script.matches('"').count(), 6, "6 个引号全部来自应用名本身");
    assert!(script.starts_with("tell application \"Evil\\\""));
    assert!(script.ends_with("\" to quit"));
}

#[test]
fn uninstaller_system_domain_stays_the_production_implementation() {
    fn assert_domain<T: crate::operation_executor::UninstallDomain>() {}
    assert_domain::<SystemUninstaller>();
}

#[test]
fn uninstaller_error_classification_covers_user_cancellation() {
    assert!(is_user_canceled("-128"));
    assert!(is_user_canceled("User canceled."));
    assert!(!is_user_canceled("connection refused"));
    assert!(is_permission_denied_msg("Operation not permitted"));
    assert!(!is_permission_denied_msg("disk full"));
    assert!(is_permission_denied_io(&std::io::Error::from(
        std::io::ErrorKind::PermissionDenied
    )));
}

#[test]
fn uninstaller_admin_move_script_quotes_every_backend_path() {
    let script = admin_move_script(
        &[
            "/Applications/alpha's.app",
            "/Users/test/Library/Caches/alpha",
        ],
        Path::new("/Users/test/.Trash"),
    );

    assert_eq!(script.matches("/bin/mv -f").count(), 2);
    assert!(script.contains(r"'\''"));
    assert!(script.contains("'/Users/test/.Trash'"));
    assert!(!script.contains("rm -rf"));
    assert!(script.contains(" ; "));
}

#[test]
fn uninstaller_target_is_built_only_from_backend_identity() {
    let app = InstalledAppIdentity {
        bundle_path: "/Applications/alpha.app".to_owned(),
        app_name: "alpha".to_owned(),
        bundle_id: "com.example.alpha".to_owned(),
        is_system: false,
        bundle_size_bytes: 0,
    };
    let residues = vec![ResidueIdentity {
        app_key: "key".to_owned(),
        path: "/Users/test/Library/Caches/alpha".to_owned(),
        category: "Caches".to_owned(),
        size_bytes: 8,
    }];

    let target = UninstallTarget::from_plan(&app, &residues);

    assert_eq!(target.bundle_path, "/Applications/alpha.app");
    assert_eq!(target.app_name, "alpha");
    assert_eq!(target.bundle_id, "com.example.alpha");
    assert_eq!(
        target.residue_paths,
        vec!["/Users/test/Library/Caches/alpha"]
    );
}
