use super::*;

// ============================================================================
// 这一组测试对应一个产品决策：**MAS 版靠什么读用户目录。**
// ============================================================================
//
// 背景是实测（2026-09-30，MAS 包真机，详见 docs/mas-capability-matrix.md）：
// App Sandbox 把 `$HOME` 重定向到应用自己的 container，真实 home 下的
// `~/Library/Caches` 的 `read_dir` 直接返回 EPERM —— 是**被拦**，不是空的。
//
// 竞品调研的结论也指向同一条路：
// - Apple 文档明说沙箱开了 FDA 也仍强制执行自己的文件限制
// - CleanMyMac 的 App Store 版至今仍教用户开 FDA，却**能**清 User Cache
//   —— 它多半用的是 security-scoped bookmark 逐目录授权
// - PureSpace（App Store 版，$1.99/月）明确写着「首次用文件选择框授权
//   ~/Library，设置里可撤销」
//
// 所以这里实现的是「用户授权哪些目录，我们就能清哪些目录」，而不是
// 「想办法绕过沙箱」。前者 Apple 有官方文档、有活下来的先例；后者过不了审。

#[test]
fn the_offerable_targets_are_the_places_where_junk_actually_lands() {
    let keys: Vec<&str> = offerable_targets().iter().map(|t| t.key).collect();
    for expected in [
        "user_caches", // ~/Library/Caches —— 最大的一块
        "user_logs",   // ~/Library/Logs
        "xcode",       // ~/Library/Developer —— 完整版实测 8.80 GB
        "npm",         // ~/.npm
        "cargo",       // ~/.cargo
        "trash",       // ~/.Trash
    ] {
        assert!(keys.contains(&expected), "授权清单里少了 {expected}");
    }
}

#[test]
fn every_target_declares_its_relative_path_and_a_reason() {
    // 每个授权项都必须说清「授权之后能清什么」。笼统的「授权访问」会让用户
    // 在弹窗前无法判断该不该点 —— 而这个弹窗是要用户交出目录访问权的。
    for target in offerable_targets() {
        assert!(!target.relative.is_empty(), "{} 没有相对路径", target.key);
        assert!(
            !target.reason_key.is_empty(),
            "{} 没有说明文案 key",
            target.key
        );
    }
}

#[test]
fn target_keys_are_unique_and_paths_do_not_collide() {
    let mut keys = std::collections::HashSet::new();
    let mut paths = std::collections::HashSet::new();
    for target in offerable_targets() {
        assert!(keys.insert(target.key), "key {} 重复", target.key);
        assert!(
            paths.insert(target.relative),
            "两个授权项指向同一个路径：{}",
            target.relative
        );
    }
}

#[test]
fn targets_render_against_the_real_home_never_the_container() {
    // 沙箱里 `$HOME` 是 container。用它拼路径 = 授权一个空目录，
    // 用户会困惑「我明明授权了却还是 0」。
    let real = PathBuf::from("/Users/edwinhao");
    for target in offerable_targets() {
        let path = target_path(&target, &real);
        assert!(
            !path.starts_with("/Users/edwinhao/Library/Containers/"),
            "{} 拼出了 container 路径：{}",
            target.key,
            path.display()
        );
    }
}

#[test]
fn paths_are_resolved_under_the_real_home_and_stay_inside_it() {
    // 防目录穿越：授权项的相对路径是代码里写死的常量，但拼装逻辑必须
    // 仍然保证结果落在真实 home 之下 —— 这是删除操作的边界所在。
    let real = PathBuf::from("/Users/edwinhao");
    for target in offerable_targets() {
        let path = target_path(&target, &real);
        assert!(
            path.starts_with(&real),
            "{} 逃出了真实 home：{}",
            target.key,
            path.display()
        );
    }
}

#[test]
fn a_granted_folder_is_matched_by_exact_path_or_by_ancestry() {
    // 授权了 `~/Library` 就该覆盖 `~/Library/Caches`（用户不想逐个子目录
    // 点一遍）。反过来，授权 `~/.npm` **不能**覆盖 `~/.npm-cache` ——
    // 前缀相同但不是祖先。
    let grants = vec![Grant {
        target_key: "user_caches".into(),
        display_name: "用户缓存".into(),
        path: PathBuf::from("/Users/x/Library"),
        bookmark: "AAAA".into(),
    }];
    assert!(grant_covers(&grants, Path::new("/Users/x/Library/Caches")));
    assert!(grant_covers(&grants, Path::new("/Users/x/Library")));
    assert!(!grant_covers(&grants, Path::new("/Users/x/.npm")));
    assert!(!grant_covers(&grants, Path::new("/Users/x/Library-backup")));
    assert!(!grant_covers(&grants, Path::new("/Users/x")));
}

#[test]
fn a_sibling_with_a_shared_prefix_is_not_covered() {
    // `Path::starts_with` 是按路径分量比的，这条专门钉住那个语义 ——
    // 用字符串前缀判会在这里放行一个用户没授权过的目录，而我们会去删它。
    let grants = vec![Grant {
        target_key: "user_caches".into(),
        display_name: "用户缓存".into(),
        path: PathBuf::from("/Users/x/Library/Caches"),
        bookmark: "AAAA".into(),
    }];
    assert!(grant_covers(
        &grants,
        Path::new("/Users/x/Library/Caches/com.foo")
    ));
    assert!(!grant_covers(
        &grants,
        Path::new("/Users/x/Library/CachesBackup")
    ));
}

#[test]
fn no_grants_means_nothing_is_covered() {
    assert!(!grant_covers(&[], Path::new("/Users/x/Library/Caches")));
}

#[test]
fn scan_roots_are_deduplicated_so_one_folder_is_not_counted_twice() {
    // 用户既授权了 `~/Library` 又单独授权了 `~/.npm`：扫描根里不能出现
    // 两次同一个路径，否则那个目录的体积会被算两遍，「可释放空间」虚高。
    let real = PathBuf::from("/Users/x");
    let grants = vec![
        Grant {
            target_key: "library".into(),
            display_name: "Library".into(),
            path: real.join("Library"),
            bookmark: "A".into(),
        },
        Grant {
            target_key: "npm".into(),
            display_name: "npm".into(),
            path: real.join(".npm"),
            bookmark: "B".into(),
        },
    ];
    let roots = scan_roots(&grants, &real);
    let mut seen = std::collections::HashSet::new();
    for root in &roots {
        assert!(seen.insert(root.clone()), "扫描根重复：{}", root.display());
    }
    assert_eq!(roots.len(), 2);
}

#[test]
fn a_grant_outside_the_real_home_is_never_used_as_a_scan_root() {
    // 书签解析出来的路径理论上不会跑到 home 之外，但一旦数据库被改坏
    // （用户手改、备份恢复、越狱环境），这里必须挡住 —— 我们要在那个
    // 路径下执行删除。
    let real = PathBuf::from("/Users/x");
    let grants = vec![
        Grant {
            target_key: "evil".into(),
            display_name: "越界".into(),
            path: PathBuf::from("/etc"),
            bookmark: "A".into(),
        },
        Grant {
            target_key: "ok".into(),
            display_name: "正常".into(),
            path: real.join(".npm"),
            bookmark: "B".into(),
        },
    ];
    let roots = scan_roots(&grants, &real);
    assert_eq!(roots, vec![real.join(".npm")]);
}

#[test]
fn no_grants_produce_no_scan_roots_and_an_explicit_reason() {
    // 「扫不到东西」必须能说清是「没授权」而不是「这台机器很干净」。
    let (roots, reason) = scan_roots_with_reason(&[], Path::new("/Users/x"));
    assert!(roots.is_empty());
    assert_eq!(reason, RootsReason::NoFoldersGranted);
}

#[test]
fn granted_roots_are_reported_as_scannable() {
    let real = PathBuf::from("/Users/x");
    let grants = vec![Grant {
        target_key: "npm".into(),
        display_name: "npm".into(),
        path: real.join(".npm"),
        bookmark: "A".into(),
    }];
    let (roots, reason) = scan_roots_with_reason(&grants, &real);
    assert_eq!(reason, RootsReason::Ready);
    assert_eq!(roots, vec![real.join(".npm")]);
}

#[test]
fn grant_serialization_round_trips() {
    // 授权要跨启动存活，所以必须能存成 JSON 再读回来。丢字段等于用户
    // 每次启动都要重新授权一遍。
    let grants = vec![Grant {
        target_key: "user_caches".into(),
        display_name: "用户缓存".into(),
        path: PathBuf::from("/Users/x/Library/Caches"),
        bookmark: "Ym9va21hcms=".into(),
    }];
    let json = serde_json::to_string(&grants).expect("能序列化");
    let back: Vec<Grant> = serde_json::from_str(&json).expect("能反序列化");
    assert_eq!(back, grants);
    assert!(json.contains("bookmark"), "书签数据必须落盘");
}

#[test]
fn a_grant_with_a_corrupt_bookmark_is_rejected_rather_than_trusted() {
    // 解析不了的书签必须在**读取时**就被丢掉，而不是等到扫描时才发现。
    let json =
        r#"[{"target_key":"x","display_name":"x","path":"/tmp/x","bookmark":"@@@not-base64@@@"}]"#;
    let parsed: Vec<Grant> = serde_json::from_str(json).expect("结构合法");
    assert!(
        !bookmark_is_plausible(&parsed[0].bookmark),
        "明显不是 base64 的书签必须判为不可信"
    );
    assert!(bookmark_is_plausible("Ym9va21hcms="));
}

// ===== 书签往返（真机实测靠它） =====
//
// 这一组在开发机上跑：开发机不在沙箱里，但 `startAccessingSecurityScopedResource`
// 对「本来就能访问的路径」同样返回 true。所以它能验证 **FFI 声明本身** 对不对
// （四个方法名、参数顺序、BOOL 出参类型、+1 返回值有没有泄漏），
// 验证不了「沙箱里能不能给一个原本无权访问的目录授权」—— 那一步必须靠用户点
// 文件选择框。
//
// MAS 包里的完整验证走 `--probe-bookmark`：在一个**沙箱可读**的路径上跑完整
// 往返，从而验证 entitlement（bookmarks.app-scope）确实生效。
// 两者都过，才说明整条链路是对的。

#[test]
fn a_bookmark_round_trips_on_this_machine() {
    let Some(home) = crate::sandbox_probe::real_home() else {
        panic!("开发机必须有真实 home");
    };
    // 挑一个必然存在、必然可读的目录
    let target = home.join("Library");
    assert!(target.is_dir(), "探针目录不存在：{target:?}");

    let bookmark = super::ffi::create_bookmark(&target).expect("应当能创建书签");
    assert!(
        super::bookmark_is_plausible(&bookmark),
        "生成的书签不像 base64：{bookmark}"
    );

    let scope = super::ffi::resolve_and_access(&bookmark).expect("应当能解析并取得访问权");
    assert_eq!(
        std::path::Path::new(scope.path()),
        target.as_path(),
        "解析出来的路径和原路径不一致"
    );
}

#[test]
fn the_scope_can_actually_read_the_directory_it_granted() {
    // 这是「startAccessingSecurityScopedResource 真的被调用了」的唯一硬证据。
    // 少了那一步，解析照样成功、路径也对，但 read_dir 会 EPERM。
    let Some(home) = crate::sandbox_probe::real_home() else {
        panic!("开发机必须有真实 home");
    };
    let target = home.join("Library");
    let bookmark = super::ffi::create_bookmark(&target).expect("应当能创建书签");
    let scope = super::ffi::resolve_and_access(&bookmark).expect("应当能解析");
    assert!(
        std::fs::read_dir(scope.path()).is_ok(),
        "拿到访问作用域却读不了目录 —— startAccessing 没生效"
    );
}

#[test]
fn opening_and_closing_the_scope_repeatedly_does_not_leak_or_break() {
    // start/stop 必须严格配对。不配对的后果不是立刻崩，而是授权计数漂移，
    // 到某次刷新时突然「读不到了」—— 那种问题现场几乎无法定位。
    let Some(home) = crate::sandbox_probe::real_home() else {
        panic!("开发机必须有真实 home");
    };
    let target = home.join("Library");
    let bookmark = super::ffi::create_bookmark(&target).expect("应当能创建书签");
    for _ in 0..5 {
        let scope = super::ffi::resolve_and_access(&bookmark).expect("应当能解析");
        assert!(std::fs::read_dir(scope.path()).is_ok());
        drop(scope);
    }
}

#[test]
fn a_garbage_bookmark_yields_no_scope_rather_than_a_broken_one() {
    // 解析失败必须返回 None。不能返回一个「有路径但读不了」的对象 ——
    // 那会让扫描阶段拿着一堆 EPERM，还以为是自己实现有问题。
    assert!(super::ffi::resolve_and_access("bm90LWEtYm9va21hcms=").is_none());
    assert!(super::ffi::resolve_and_access("").is_none());
    assert!(super::ffi::resolve_and_access("!!!!").is_none());
}

#[test]
fn creating_a_bookmark_for_a_missing_path_is_reported_as_a_failure() {
    let missing = std::path::PathBuf::from("/definitely/not/here/xyz");
    assert!(
        super::ffi::create_bookmark(&missing).is_none(),
        "给不存在的路径建书签应当失败，而不是给一个永远解析不出东西的假书签"
    );
}
