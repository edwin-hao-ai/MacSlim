use super::*;
use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    dirs::home_dir().expect("测试环境必须有用户主目录")
}

fn library_root(subdir: &str) -> PathBuf {
    home().join("Library").join(subdir)
}

fn resolved(path: &Path) -> PathBuf {
    resolve_canonical(path).expect("路径必须可解析")
}

#[test]
fn residue_policy_library_subdirs_cover_the_fixed_scanner_list() {
    assert!(LIBRARY_SUBDIRS.contains(&"Application Support"));
    assert!(LIBRARY_SUBDIRS.contains(&"Caches"));
    assert!(LIBRARY_SUBDIRS.contains(&"Group Containers"));
    assert_eq!(LIBRARY_SUBDIRS.len(), 9);
}

#[test]
fn residue_policy_allowed_roots_are_library_subdirs_plus_dev_tool_paths() {
    let roots = allowed_roots("com.microsoft.VSCode");

    assert!(roots
        .containers
        .contains(&library_root("Application Support")));
    assert!(roots.containers.contains(&library_root("Caches")));
    assert!(roots.exact.contains(&home().join(".vscode/extensions")));
}

#[test]
fn residue_policy_roots_exclude_home_itself_and_absolute_rule_paths() {
    let roots = allowed_roots("com.docker.docker");

    assert!(!roots.containers.contains(&home()));
    assert!(!roots.containers.contains(&home().join("Library")));
    assert!(!roots.exact.contains(&home()));
    assert!(roots.exact.iter().all(|root| root.is_absolute()));
    assert!(roots
        .containers
        .iter()
        .chain(roots.exact.iter())
        .all(|root| !root.to_string_lossy().starts_with("//")));
}

#[test]
fn residue_policy_dev_tool_root_itself_is_allowed() {
    let roots = allowed_roots("com.microsoft.VSCode");
    let declared = home().join(".vscode/extensions");

    assert!(ensure_within_roots(&declared, &roots).is_ok());
    assert!(ensure_within_roots(&declared.join("ms-python"), &roots).is_ok());
    assert!(ensure_within_roots(&home().join(".vscode"), &roots).is_err());
}

#[test]
fn residue_policy_dev_tool_root_is_bound_to_its_bundle_id() {
    let vscode = allowed_roots("com.microsoft.VSCode");
    let other = allowed_roots("com.example.alpha");

    assert!(ensure_within_roots(&home().join(".vscode/extensions"), &vscode).is_ok());
    assert!(ensure_within_roots(&home().join(".vscode/extensions"), &other).is_err());
}

#[test]
fn residue_policy_accepts_path_inside_allowed_root() {
    let roots = allowed_roots("com.example.alpha");

    assert!(ensure_within_roots(&library_root("Caches").join("com.example.alpha"), &roots).is_ok());
}

#[test]
fn residue_policy_rejects_the_root_itself() {
    let roots = allowed_roots("com.example.alpha");

    let error = ensure_within_roots(&library_root("Caches"), &roots).unwrap_err();

    assert!(error.contains("不在允许的清理范围"), "{}", error);
}

#[test]
fn residue_policy_rejects_substring_match_outside_allowed_roots() {
    let roots = allowed_roots("com.example.alpha");

    assert!(ensure_within_roots(Path::new("/etc"), &roots).is_err());
    assert!(ensure_within_roots(Path::new("/Users/other/Library/Caches/alpha"), &roots).is_err());
    assert!(ensure_within_roots(&home().join("Documents/alpha"), &roots).is_err());
    assert!(ensure_within_roots(&home(), &roots).is_err());
}

#[test]
fn residue_policy_rejects_dotdot_and_relative_paths() {
    let roots = allowed_roots("com.example.alpha");

    let dotdot = library_root("Caches").join("com.example.alpha/../../../../etc");
    assert!(ensure_within_roots(&dotdot, &roots).is_err());
    assert!(ensure_within_roots(Path::new("Caches/alpha"), &roots).is_err());
    assert!(ensure_within_roots(Path::new(""), &roots).is_err());
}

#[test]
fn residue_policy_rejects_absolute_dev_tool_rule_path() {
    let roots = allowed_roots("com.apple.dt.Xcode");

    assert!(
        ensure_within_roots(Path::new("/Library/LaunchDaemons/com.apple.xcode"), &roots).is_err()
    );
    assert!(ensure_within_roots(Path::new("/usr/local/bin/xcodebuild"), &roots).is_err());
}

#[test]
fn residue_policy_rejects_symlink_that_escapes_allowed_root() {
    let outside = fixture_dir("macslim-policy-outside");
    let Some(escape) = fixture_link("macslim-policy-escape", &outside) else {
        cleanup_fixture(Path::new(""), Some(&outside));
        return;
    };
    let roots = allowed_roots("com.example.alpha");

    let error = ensure_within_roots(&escape.join("payload"), &roots).unwrap_err();

    assert!(error.contains("不在允许的清理范围"), "{}", error);
    cleanup_fixture(&escape, Some(&outside));
}

#[test]
fn residue_policy_rejects_symlink_target_itself() {
    let outside = fixture_dir("macslim-policy-outside-leaf");
    let Some(escape) = fixture_link("macslim-policy-escape-leaf", &outside) else {
        cleanup_fixture(Path::new(""), Some(&outside));
        return;
    };
    let roots = allowed_roots("com.example.alpha");

    let error = ensure_within_roots(&escape, &roots).unwrap_err();

    assert!(error.contains("不在允许的清理范围"), "{}", error);
    cleanup_fixture(&escape, Some(&outside));
}

#[test]
fn residue_policy_symlink_between_allowed_roots_is_accepted() {
    let real_dir =
        library_root("Logs").join(format!("macslim-policy-inside-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&real_dir);
    if std::fs::create_dir_all(&real_dir).is_err() {
        return;
    }
    let Some(link_dir) = fixture_link("macslim-policy-inside", &real_dir) else {
        let _ = std::fs::remove_dir_all(&real_dir);
        return;
    };
    let roots = allowed_roots("com.example.alpha");

    let accepted = ensure_within_roots(&link_dir.join("com.example.alpha"), &roots);

    cleanup_fixture(&link_dir, None);
    let _ = std::fs::remove_dir_all(&real_dir);
    assert!(accepted.is_ok(), "允许根之间的软链接必须被接受");
}

fn fixture_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("临时目录必须可创建");
    path
}

fn fixture_link(name: &str, target: &Path) -> Option<PathBuf> {
    let path = library_root("Caches").join(name);
    let _ = std::fs::remove_file(&path);
    std::os::unix::fs::symlink(target, &path).ok().map(|_| path)
}

fn cleanup_fixture(link: &Path, target: Option<&Path>) {
    if !link.as_os_str().is_empty() {
        let _ = std::fs::remove_file(link);
    }
    if let Some(target) = target {
        let _ = std::fs::remove_dir_all(target);
    }
}

#[test]
fn residue_policy_resolves_canonical_through_existing_ancestor() {
    let missing = home().join("Library/Caches/macslim-policy-missing/child");

    let resolved_path = resolved(&missing);

    assert_eq!(resolved_path, missing);
    assert!(resolved_path.is_absolute());
}

#[test]
fn residue_policy_convenience_check_reports_unknown_bundle_without_roots() {
    let outside = Path::new("/tmp/macslim-policy-outside");

    let alpha = allowed_roots("com.example.alpha");
    let empty = allowed_roots("");
    assert!(ensure_within_roots(outside, &alpha).is_err());
    assert!(ensure_within_roots(outside, &empty).is_err());
    assert!(ensure_within_roots(&library_root("Caches").join("com.example.alpha"), &alpha).is_ok());
}

#[test]
fn residue_policy_dotdot_detection_is_component_aware() {
    assert!(has_dotdot(Path::new("Caches/../etc")));
    assert!(!has_dotdot(Path::new("Caches/alpha..backup/child")));
    assert!(!has_dotdot(Path::new("Caches/alpha")));
}

#[test]
fn residue_policy_roots_are_canonicalized_like_candidates() {
    let roots = allowed_roots("com.example.alpha");

    assert!(roots
        .containers
        .iter()
        .chain(roots.exact.iter())
        .all(|root| !root.to_string_lossy().contains("~")));
    assert!(roots
        .containers
        .iter()
        .all(|root| root.is_absolute() && !root.as_os_str().is_empty()));
    assert_eq!(
        roots.containers.len(),
        LIBRARY_SUBDIRS.len(),
        "9 个 Library 子目录都必须成为根"
    );
}

#[test]
fn residue_policy_skips_roots_that_cannot_be_resolved() {
    let missing = home().join("Library/macslim-policy-missing-root/Caches");
    let mut roots = allowed_roots("com.example.alpha");
    roots.containers.push(missing);
    roots
        .exact
        .push(home().join(".macslim-policy-missing-exact"));

    let accepted = ensure_within_roots(&library_root("Caches").join("com.example.alpha"), &roots);
    let rejected = ensure_within_roots(Path::new("/etc/alpha"), &roots);

    assert!(accepted.is_ok(), "无法解析的根不应影响其它根");
    assert!(rejected.is_err());
}

#[test]
fn residue_policy_accepts_library_root_reached_through_symlink() {
    let real_root =
        library_root("Logs").join(format!("macslim-policy-liblink-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&real_root);
    if std::fs::create_dir_all(&real_root).is_err() {
        return;
    }
    let link = home().join("Library/macslim-policy-liblink-parent");
    let _ = std::fs::remove_file(&link);
    if std::os::unix::fs::symlink(&real_root, &link).is_err() {
        let _ = std::fs::remove_dir_all(&real_root);
        return;
    }
    let mut roots = allowed_roots("com.example.alpha");
    roots.containers.push(link.clone());
    roots.exact.push(link.clone());
    let candidate = link.join("com.example.alpha");

    let accepted = ensure_within_roots(&candidate, &roots);
    let roots_now_canonical = canonical_roots(&roots);
    let still_accepted = ensure_within_roots(&candidate, &roots_now_canonical);

    let _ = std::fs::remove_file(&link);
    let _ = std::fs::remove_dir_all(&real_root);
    assert!(accepted.is_ok(), "symlink 根与其真实目标必须等价");
    assert!(still_accepted.is_ok());
}

#[test]
fn residue_policy_canonical_roots_keep_symlink_root_escape_rejected() {
    let outside = fixture_dir("macslim-policy-canon-outside");
    let escape = fixture_link("macslim-policy-canon-escape", &outside);
    if escape.is_none() {
        cleanup_fixture(Path::new(""), Some(&outside));
        return;
    }
    let roots = canonical_roots(&allowed_roots("com.example.alpha"));
    let Some(escape) = escape else {
        cleanup_fixture(Path::new(""), Some(&outside));
        return;
    };

    let error = ensure_within_roots(&escape.join("payload"), &roots).unwrap_err();

    assert!(error.contains("不在允许的清理范围"), "{}", error);
    cleanup_fixture(&escape, Some(&outside));
}

#[test]
fn residue_policy_prefix_helper_names_match_their_behaviour() {
    let root = Path::new("/Users/test/Library/Caches");

    assert!(is_strictly_under_root(
        Path::new("/Users/test/Library/Caches/alpha"),
        root
    ));
    assert!(!is_strictly_under_root(root, root));
    assert!(under_root(root, root), "dev-tool 根本身允许");
    assert!(!under_root(Path::new("/Users/test/Library"), root));
}

fn canonical_roots(roots: &ResidueRoots) -> ResidueRoots {
    ResidueRoots {
        containers: roots
            .containers
            .iter()
            .filter_map(|root| resolve_canonical(root))
            .collect(),
        exact: roots
            .exact
            .iter()
            .filter_map(|root| resolve_canonical(root))
            .collect(),
    }
}
