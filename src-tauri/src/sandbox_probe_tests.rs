use super::*;

// ===== home 重定向判定 =====
//
// 这是整个探针的立论点：App Sandbox 会把进程的 `$HOME` 指向自己的
// container（`~/Library/Containers/<bundleid>/Data`），而全代码库用
// `dirs::home_dir()` 拼用户路径。于是 `~/Library/Caches` 在 MAS 版里
// 解析到**应用自己那个空 container**下的 Caches —— 扫出 0 B 不是权限
// 被拦，是压根扫错了地方。给多少 FDA 都不会变。

#[test]
fn detects_home_redirected_into_its_own_container() {
    let real = PathBuf::from("/Users/edwinhao");
    let container = PathBuf::from("/Users/edwinhao/Library/Containers/com.vgoapp.macslim/Data");
    assert!(home_points_at_container(&container, &real));
}

#[test]
fn a_normal_home_is_not_a_container() {
    let real = PathBuf::from("/Users/edwinhao");
    assert!(!home_points_at_container(
        Path::new("/Users/edwinhao"),
        &real
    ));
}

#[test]
fn the_containers_directory_itself_is_not_inside_a_container() {
    // 真实 home 是 Containers 的**祖先**，方向不能弄反。
    let real = PathBuf::from("/Users/edwinhao/Library/Containers");
    let inside = PathBuf::from("/Users/edwinhao/Library/Containers/com.foo.app/Data");
    assert!(!home_points_at_container(&real, &inside));
    assert!(home_points_at_container(&inside, &real));
}

#[test]
fn a_sibling_with_a_shared_string_prefix_is_not_a_container() {
    // 朴素 `starts_with` 会把 `Containers-backup` 误判成 container。
    // 判定必须按**路径分量**比，不是按字符串前缀。
    let real = PathBuf::from("/Users/edwinhao");
    let sibling = PathBuf::from("/Users/edwinhao/Library/Containers-backup/Data");
    assert!(!home_points_at_container(&sibling, &real));
}

#[test]
fn a_grandchild_of_the_container_still_counts() {
    // `Data/Library/Caches` 这种更深的路径也要认出来。
    let real = PathBuf::from("/Users/edwinhao");
    let deep = PathBuf::from("/Users/edwinhao/Library/Containers/com.foo.app/Data/Library/Caches");
    assert!(home_points_at_container(&deep, &real));
}

// ===== 探针清单 =====

#[test]
fn every_probe_key_is_unique() {
    let mut seen = std::collections::HashSet::new();
    for probe in probe_catalog() {
        assert!(seen.insert(probe.key), "探针 key 重复：{}", probe.key);
    }
}

#[test]
fn every_probe_declares_the_capability_it_gates() {
    // 探针存在的意义是回答「哪个产品能力被挡住了」。没有对应能力的探针
    // 是噪音，会让人误以为那个路径重要。
    for probe in probe_catalog() {
        assert!(
            !probe.gates.is_empty(),
            "探针 {} 没有声明它守护的能力，无法解释为什么要测它",
            probe.key
        );
    }
}

#[test]
fn catalog_covers_the_paths_the_product_actually_depends_on() {
    let keys: std::collections::HashSet<&str> = probe_catalog().iter().map(|p| p.key).collect();
    for required in [
        "applications",  // 应用列表 / 卸载（实测唯一能用的页面）
        "user_caches",   // 用户缓存 —— 交接文档里 0 B 的那一个
        "xcode_derived", // 开发者缓存，完整版实测 8.80 GB
        "npm_cache",     // npm 缓存
        "system_caches", // 系统缓存，CleanMyMac 的 AS 版也砍了这项
        "trash",         // 废纸篓
        "downloads",     // 过期下载清理只需这个
        "own_container", // 沙箱内必然可写，用来当「探针本身没坏」的对照
    ] {
        assert!(keys.contains(required), "探针清单缺 {required}");
    }
}

#[test]
fn probes_never_point_at_the_container_home() {
    // 拼路径时必须用**真实 home**，否则测的还是那个空 container，
    // 得到的结论全是「可读」—— 恰好与事实相反。这是本模块最容易犯的错。
    // 唯一例外是 own_container：它**故意**指向 container，作用是当对照组
    // （证明探针机制本身没坏、区别只在于 home 选对了没有）。
    let real = PathBuf::from("/Users/edwinhao");
    for probe in probe_catalog() {
        if probe.key == "own_container" {
            continue;
        }
        let rendered = render_probe_path(&probe, &real);
        // 用生产代码里那个判定本身来断言，而不是重写一遍前缀比较 ——
        // `Path::starts_with` 是按路径分量比的（尾部斜杠会被吞掉），
        // 所以 `~/Library/Containers` 这个**父目录**也会被字符串式断言误伤。
        assert!(
            !home_points_at_container(&rendered, &real),
            "探针 {} 拼出了 container 路径：{}",
            probe.key,
            rendered.display()
        );
    }
}

#[test]
fn the_control_probe_really_does_point_at_the_container() {
    // 反向断言：对照组要是哪天也指向了真实 home，这套探针就失去意义了 ——
    // 「沙箱下能读」将不再有任何对照。
    let real = PathBuf::from("/Users/edwinhao");
    let probe = probe_catalog()
        .into_iter()
        .find(|p| p.key == "own_container")
        .expect("清单里有 own_container");
    let rendered = render_probe_path(&probe, &real);
    assert!(
        home_points_at_container(&rendered, &real),
        "对照组探针必须落在 container 内，实际：{}",
        rendered.display()
    );
}

#[test]
fn user_scaches_probe_points_at_the_real_user_library_caches() {
    let real = PathBuf::from("/Users/edwinhao");
    let probe = probe_catalog()
        .into_iter()
        .find(|p| p.key == "user_caches")
        .expect("清单里有 user_caches");
    assert_eq!(
        render_probe_path(&probe, &real),
        PathBuf::from("/Users/edwinhao/Library/Caches")
    );
}

#[test]
fn absolute_probes_ignore_the_home_argument_entirely() {
    let probe = probe_catalog()
        .into_iter()
        .find(|p| p.key == "applications")
        .expect("清单里有 applications");
    assert_eq!(
        render_probe_path(&probe, Path::new("/Users/whatever")),
        PathBuf::from("/Applications")
    );
}

// ===== 结论推导 =====

#[test]
fn a_missing_directory_is_reported_as_absent_not_as_blocked() {
    // 没装 Xcode 的机器上 ~/Library/Developer/Xcode 就不存在。
    // 把它算成「被沙箱拦住」会让 FDA 引导指向一个根本无关的权限。
    let finding = interpret(PathProbe {
        key: "xcode_derived".into(),
        label: "Xcode 编译缓存".into(),
        path: "/Users/x/Library/Developer/Xcode".into(),
        exists: false,
        listable: false,
        entries: None,
    });
    assert_eq!(finding.status, ProbeStatus::Absent);
}

#[test]
fn an_existing_but_unlistable_directory_is_reported_as_blocked() {
    let finding = interpret(PathProbe {
        key: "user_caches".into(),
        label: "用户缓存".into(),
        path: "/Users/x/Library/Caches".into(),
        exists: true,
        listable: false,
        entries: None,
    });
    assert_eq!(finding.status, ProbeStatus::Blocked);
}

#[test]
fn a_listable_directory_reports_its_entry_count() {
    let finding = interpret(PathProbe {
        key: "user_caches".into(),
        label: "用户缓存".into(),
        path: "/Users/x/Library/Caches".into(),
        exists: true,
        listable: true,
        entries: Some(412),
    });
    assert_eq!(finding.status, ProbeStatus::Readable);
    assert_eq!(finding.entries, Some(412));
}

#[test]
fn a_readable_but_empty_directory_is_not_reported_as_blocked() {
    // 沙箱重定向时的典型形态：路径可读、条目 0。若把「空」也当成被拦，
    // 就会得出「必须授权 FDA」的错误结论。
    let finding = interpret(PathProbe {
        key: "user_caches".into(),
        label: "用户缓存".into(),
        path: "/Users/x/Library/Containers/com.vgoapp.macslim/Data/Library/Caches".into(),
        exists: true,
        listable: true,
        entries: Some(0),
    });
    assert_eq!(finding.status, ProbeStatus::Readable);
    assert_eq!(finding.entries, Some(0));
}

// ===== 报告 =====

#[test]
fn report_serializes_to_json_with_the_fields_diagnostics_depend_on() {
    let report = ProbeReport {
        flavor: "mas".into(),
        home_env: Some("/Users/x/Library/Containers/com.vgoapp.macslim/Data".into()),
        real_home: Some("/Users/x".into()),
        home_is_container: true,
        findings: vec![interpret(PathProbe {
            key: "user_caches".into(),
            label: "用户缓存".into(),
            path: "/Users/x/Library/Caches".into(),
            exists: true,
            listable: false,
            entries: None,
        })],
        processes: ProcessVisibility {
            sysinfo_count: 0,
            sysctl_count: None,
            sysctl_error: Some("模拟：被拒".into()),
            can_signal_self: true,
            snapshot_count: 0,
            top_by_memory: vec![],
            with_exe_path: 0,
            exe_samples: vec![],
            bookmark_supported: false,
            bookmark_path: None,
            bookmark_error: None,
        },
        health_readable: true,
    };
    let json = serde_json::to_string(&report).expect("报告必须能序列化");
    for key in [
        "flavor",
        "home_env",
        "real_home",
        "home_is_container",
        "findings",
        "processes",
        "health_readable",
    ] {
        assert!(json.contains(key), "报告 JSON 缺字段 {key}：{json}");
    }
    let back: ProbeReport = serde_json::from_str(&json).expect("报告必须能反序列化");
    assert!(back.home_is_container);
    assert_eq!(back.findings[0].status, ProbeStatus::Blocked);
}

#[test]
fn the_one_line_verdict_names_the_actual_root_cause() {
    // 汇报给人看的结论必须区分「沙箱把 ~ 换成了 container」与
    // 「真的缺权限」—— 这两者的解法完全相反：前者要改代码拼真实路径，
    // 后者要去申请授权。混为一谈会让整个 FDA 引导方向跑偏。
    let redirected = verdict(VerdictInput {
        home_is_container: true,
        blocked: vec!["user_caches".into()],
        readable_non_empty: vec![],
    });
    assert!(
        redirected.contains("container"),
        "home 被重定向时结论必须点明 container，实际：{redirected}"
    );

    let denied = verdict(VerdictInput {
        home_is_container: false,
        blocked: vec!["system_caches".into()],
        readable_non_empty: vec![],
    });
    assert!(
        !denied.contains("container"),
        "非重定向场景不该提 container，实际：{denied}"
    );
    assert!(denied.contains("system_caches"), "实际：{denied}");
}

#[test]
fn verdict_does_not_claim_a_problem_when_nothing_is_blocked() {
    let text = verdict(VerdictInput {
        home_is_container: false,
        blocked: vec![],
        readable_non_empty: vec!["user_caches".into()],
    });
    assert!(text.contains("可用"), "实际：{text}");
}

// ===== 无头探针入口 =====

#[test]
fn the_probe_flag_is_recognised_and_nothing_else_is() {
    assert!(probe_requested(
        ["macslim", "--probe-sandbox"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    ));
    // 普通启动、以及拼错的名字，都必须走正常 GUI 流程
    assert!(!probe_requested(vec!["macslim".to_string()]));
    assert!(!probe_requested(vec![
        "macslim".to_string(),
        "--probe".to_string()
    ]));
    assert!(!probe_requested(vec![
        "macslim".to_string(),
        "probe-sandbox".to_string()
    ]));
}

#[test]
fn running_the_probe_on_this_machine_finds_the_real_home() {
    // 开发机不在沙箱里，$HOME 就是真实 home —— 这是探针自身的自检。
    // 如果这条挂了，说明 real_home() 在这台机器上就取不到值，
    // 那么 MAS 包跑出来的报告也全是「跳过」，不能拿来下结论。
    let report = super::run();
    let real = report.real_home.expect("passwd 库里必须有当前用户的 home");
    assert!(
        PathBuf::from(&real).is_absolute(),
        "real_home 必须是绝对路径：{real}"
    );
    assert!(
        PathBuf::from(&real).exists(),
        "real_home 必须真实存在：{real}"
    );
    assert!(
        !super::home_points_at_container(Path::new(&real), Path::new(&real)),
        "real_home 不该被判成 container"
    );
}

#[test]
fn the_probe_measures_something_rather_than_reporting_nothing() {
    let report = super::run();
    assert!(
        report.findings.len() >= 10,
        "探针只跑了 {} 条，清单或执行有问题",
        report.findings.len()
    );
    // 每条结论都必须带上门的能力标签，否则报告没法用来定位问题
    for finding in &report.findings {
        assert!(
            !finding.gates.is_empty(),
            "结论 {} 没有能力标签",
            finding.key
        );
    }
}

// ===== 进程可见性 =====
//
// 交接文档只记了「进程管理 0 项」这个现象，没查是沙箱拦的还是我们代码的问题。
// sysinfo 的 `apple-sandbox` feature 是**返回全 0 的桩**（见 sysinfo 0.33.1
// `src/unix/apple/app_store/process.rs`，每个方法都 `None`/`0`），它只保证
// MAS 构建能编译，不代表沙箱真的读不到。所以必须另找一条路实测。

#[test]
fn sysctl_can_enumerate_processes_on_an_unrestricted_process() {
    // 开发机不在沙箱里，这条必须过 —— 否则说明探针本身写错了，
    // 拿它去判断 MAS 里的 0 就毫无意义。
    let count = super::sysctl_process_count();
    assert!(
        count.is_some_and(|n| n > 50),
        "非沙箱环境下 sysctl KERN_PROC_ALL 应当枚举到进程，实际：{count:?}"
    );
}

#[test]
fn we_can_always_signal_our_own_process() {
    // 对照项：沙箱允许进程给自己发信号（至少 SIGCONT 这种无害的）。
    // 它恒为真，所以它恒为真时不能说明任何权限问题 —— 存在的意义是
    // 提醒别把「能给自己发信号」误当成「能枚举/能杀别的进程」。
    assert!(super::can_signal_self());
}

#[test]
fn the_report_says_how_processes_were_counted() {
    // 报告必须同时给出两条路径的结果，否则看到 0 不知道该怪谁。
    let visibility = super::process_visibility();
    assert!(visibility.sysinfo_count > 0 || visibility.sysctl_count.is_some());
    assert_eq!(
        visibility.sysinfo_count > 0,
        visibility.sysctl_count.unwrap_or(0) > 0,
        "两条枚举路径的结论必须一致：0 与非 0 并存说明其中一条坏了"
    );
}

#[test]
fn kinfo_proc_size_is_not_merely_a_guess() {
    // sysctl 返回的是一整块 kinfo_proc 数组，除以单条大小才是进程数。
    // 大小写错不会编译失败，只会让数字离谱 —— 所以拿它跟 sysinfo 的
    // 独立计数对账，偏差超过三成就说明常量错了。
    let sysinfo_count = super::process_visibility().sysinfo_count;
    let sysctl_count = super::sysctl_process_count().expect("非沙箱下应当读得到");
    assert!(
        sysinfo_count > 50,
        "sysinfo 基准本身不对劲：{sysinfo_count}"
    );
    let ratio = sysctl_count as f64 / sysinfo_count as f64;
    assert!(
        (0.7..=1.4).contains(&ratio),
        "sysctl 数出 {sysctl_count} 个、sysinfo 数出 {sysinfo_count} 个，\
         比值 {ratio:.2} 偏离过大 —— kinfo_proc 尺寸大概写错了（当前 {}）",
        super::kinfo_proc_size()
    );
}
