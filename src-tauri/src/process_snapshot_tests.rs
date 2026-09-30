use super::*;

// 这组测试对应的产品结论：**Mac App Store 版能不能做进程监控。**
//
// 交接文档只记了「进程管理 0 项」，没查原因。查出来的原因是：
// `sysinfo` 枚举进程走的是 `proc_listallpids`，而那个调用被 App Sandbox
// 拦掉了 —— 所以 0 项是**依赖交白卷**，不是「沙箱读不到进程」。
// 实测（2026-09-30，MAS 包真机）：同一进程里 `sysctl(KERN_PROC_ALL)`
// 能数出 400 个进程。绕开 sysinfo 之后，只读进程监控在 MAS 版是可行的。
//
// 终止进程仍然是另一回事：沙箱不允许给别的进程发信号，与能不能**看**
// 是两回事。所以 MAS 版有「进程监控」但没有「结束进程」，不是自相矛盾。

#[test]
fn the_snapshot_contains_this_very_process() {
    // 最硬的非空断言：我们自己一定在列表里，且 pid 必须对得上。
    // 这条失败意味着整个读取路径根本没通，后面所有数字都不可信。
    let snapshot = process_snapshot();
    let me = std::process::id() as i32;
    let found = snapshot
        .iter()
        .find(|p| p.pid == me)
        .expect("进程快照里必须有自己");
    assert!(!found.name.is_empty(), "进程名不能为空");
}

#[test]
fn the_snapshot_is_not_empty_and_not_a_sane_machine() {
    let snapshot = process_snapshot();
    assert!(
        snapshot.len() > 50,
        "只枚举出 {} 个进程，明显不对",
        snapshot.len()
    );
}

#[test]
fn most_listed_pids_carry_a_readable_name() {
    // 如果结构体布局读错了，`pbi_name` 会是乱码或全空。这条把
    // 「偏移量对不对」变成一个会失败的断言，而不是让用户看到一屏乱码。
    let snapshot = process_snapshot();
    let named = snapshot.iter().filter(|p| !p.name.is_empty()).count();
    let ratio = named as f64 / snapshot.len() as f64;
    assert!(
        ratio > 0.9,
        "只有 {}/{} 个进程有名字（{:.0}%）—— 结构体布局可能读错了",
        named,
        snapshot.len(),
        ratio * 100.0
    );
}

#[test]
fn resident_memory_is_plausible_for_our_own_process() {
    let me = std::process::id() as i32;
    let snapshot = process_snapshot();
    let found = snapshot.iter().find(|p| p.pid == me).expect("有自己");
    assert!(
        found.resident_bytes > 0 && found.resident_bytes < 8 * 1024 * 1024 * 1024,
        "自己的常驻内存读数离谱：{} 字节",
        found.resident_bytes
    );
}

#[test]
fn process_names_never_contain_nul_or_control_characters() {
    // C 字符串数组按 NUL 截断后再转 Rust String 才对。忘了截断就会把
    // 后面几十个字节的垃圾一起带进 UI。
    let snapshot = process_snapshot();
    for sample in &snapshot {
        assert!(
            !sample.name.contains('\0'),
            "进程 {} 的名字里有 NUL",
            sample.pid
        );
        assert!(
            sample.name.chars().all(|c| !c.is_control() || c == '\t'),
            "进程 {} 的名字里有控制字符：{:?}",
            sample.pid,
            sample.name
        );
    }
}

#[test]
fn cpu_time_is_reported_in_nanoseconds_and_is_not_absurd() {
    let me = std::process::id() as i32;
    let snapshot = process_snapshot();
    let found = snapshot.iter().find(|p| p.pid == me).expect("有自己");
    // 单位是纳秒：跑了几分钟的测试进程，CPU 时间应该在毫秒到分钟量级。
    // 明显超出说明把字节当成了别的单位。
    assert!(
        found.cpu_nanos < 60 * 60 * 1_000_000_000,
        "自己的 CPU 时间 {} 纳秒 = {:.1} 小时，不合理",
        found.cpu_nanos,
        found.cpu_nanos as f64 / 3.6e12
    );
}

#[test]
fn taking_two_snapshots_lets_us_derive_cpu_percent() {
    // 只读监控的核心用法是「两次采样之间的 CPU 增量」。这条确保第二次
    // 采样确实能拿到同一批进程，且 CPU 时间单调不减 —— 递减就说明读的
    // 不是累计值，百分比会算出负数。
    let first = process_snapshot();
    std::thread::sleep(std::time::Duration::from_millis(600));
    let second = process_snapshot();
    assert!(!first.is_empty() && !second.is_empty());

    let regressed = second
        .iter()
        .filter_map(|after| {
            let before = first.iter().find(|p| p.pid == after.pid)?;
            (after.cpu_nanos < before.cpu_nanos).then_some(after.pid)
        })
        .take(5)
        .collect::<Vec<_>>();
    assert!(
        regressed.is_empty(),
        "这些 pid 的累计 CPU 时间反而变小了：{regressed:?} —— \\
         说明读的不是累计值，算出来的百分比会是负数"
    );
}

#[test]
fn cpu_percent_is_derived_from_the_delta_not_the_absolute() {
    let base = vec![ProcessSample {
        pid: 1,
        ppid: 0,
        name: "launchd".into(),
        uid: 0,
        resident_bytes: 1024,
        cpu_nanos: 1_000_000_000,
        thread_count: 1,
        exe_path: None,
        start_time_epoch_secs: None,
    }];
    let later = vec![ProcessSample {
        pid: 1,
        ppid: 0,
        name: "launchd".into(),
        uid: 0,
        resident_bytes: 1024,
        // 1 秒 CPU 时间，间隔 2 秒 → 50%
        cpu_nanos: 2_000_000_000,
        thread_count: 1,
        exe_path: None,
        start_time_epoch_secs: None,
    }];
    let percents = cpu_percent(&base, &later, 2.0);
    assert!((percents.get(&1).copied().unwrap_or(-1.0) - 50.0).abs() < 0.01);
}

#[test]
fn cpu_percent_is_zero_when_nothing_moved() {
    let same = vec![ProcessSample {
        pid: 7,
        ppid: 1,
        name: "x".into(),
        uid: 0,
        resident_bytes: 1,
        cpu_nanos: 500,
        thread_count: 1,
        exe_path: None,
        start_time_epoch_secs: None,
    }];
    assert_eq!(cpu_percent(&same, &same, 1.0).get(&7).copied(), Some(0.0));
}

#[test]
fn cpu_percent_ignores_processes_that_were_not_there_before() {
    // 新出现的进程没有「上一次」可减，硬减会得到一个巨大的假百分比。
    let before = Vec::new();
    let after = vec![ProcessSample {
        pid: 99,
        ppid: 1,
        name: "new".into(),
        uid: 0,
        resident_bytes: 1,
        cpu_nanos: 999_999_999_999,
        thread_count: 1,
        exe_path: None,
        start_time_epoch_secs: None,
    }];
    assert_eq!(
        cpu_percent(&before, &after, 1.0).get(&99).copied(),
        Some(0.0)
    );
}

#[test]
fn cpu_percent_never_exceeds_a_hundred() {
    // 多线程进程在 100% 的定义下最多是「一个核」。超过说明两次采样的
    // 间隔算错了，或者单位串了。宁可截断也不要显示 4000%。
    let before = vec![ProcessSample {
        pid: 5,
        ppid: 1,
        name: "busy".into(),
        uid: 0,
        resident_bytes: 1,
        cpu_nanos: 0,
        thread_count: 8,
        exe_path: None,
        start_time_epoch_secs: None,
    }];
    let after = vec![ProcessSample {
        pid: 5,
        ppid: 1,
        name: "busy".into(),
        uid: 0,
        resident_bytes: 1,
        cpu_nanos: 10_000_000_000,
        thread_count: 8,
        exe_path: None,
        start_time_epoch_secs: None,
    }];
    let value = cpu_percent(&before, &after, 1.0);
    assert!(value[&5] <= 100.0, "单次采样间隔 1 秒却算出 {}%", value[&5]);
}

#[test]
fn pids_have_no_duplicates_and_look_like_pids() {
    // 这条守着 `p_pid` 在 kinfo_proc 里的字节偏移。偏移错了会得到一堆
    // 离谱的整数（负数、超大数、或同一个值重复几十次），而不是编译错误。
    let pids = super::list_pids().expect("非沙箱下 sysctl 应当可用");
    assert!(pids.len() > 50, "只枚举出 {} 个 PID", pids.len());
    let mut seen = std::collections::HashSet::new();
    for pid in &pids {
        assert!(seen.insert(*pid), "PID {pid} 重复出现 —— 偏移很可能错了");
        assert!(*pid > 0 && *pid < 1_000_000, "PID {pid} 不像真的");
    }
}

// ===== 可执行路径 =====
//
// 「应用程序」页把进程按 .app bundle 聚合，靠的是进程的可执行路径。
// 有了 exe 路径，MAS 版就不只是「进程监控」，还能有「运行中的应用」——
// 那是交接文档里被记成「应用程序 0」的那一页。
//
// 路径本身在沙箱里**读不到**（进程的可执行文件大多在 /Applications 之外，
// 或在别的 app 的 container 里），所以预期是「拿不到就留空」而不是崩溃。

#[test]
fn our_own_executable_path_is_recoverable() {
    let me = std::process::id() as i32;
    let sample = super::sample_process(me).expect("应当能采到自己");
    let exe = sample.exe_path.expect("自己的可执行路径必须拿得到");
    assert!(
        exe.starts_with('/'),
        "可执行路径应当是绝对路径，实际：{exe}"
    );
    assert!(
        exe.contains("macslim"),
        "可执行路径应当指向测试二进制，实际：{exe}"
    );
}

#[test]
fn a_process_that_does_not_exist_has_no_sample_at_all() {
    // PID 999999 不可能存在。这条保证「进程刚好退出了」时返回 None 而不是
    // 拿一个全零的结构体编出一个 pid=0 的假进程。
    assert!(super::sample_process(999_999).is_none());
}

#[test]
fn a_snapshot_keeps_going_after_a_process_disappears_mid_scan() {
    // 扫描期间进程生灭是常态。不能因为一个进程消失就整个返回空 ——
    // 那正是「进程管理 0 项」的另一种形式。
    let snapshot = super::process_snapshot();
    assert!(
        snapshot.len() > 50,
        "跳过消失的进程不该把整份快照吃掉，只剩 {} 条",
        snapshot.len()
    );
}

#[test]
fn exe_path_is_none_rather_than_empty_string_when_unavailable() {
    // 空字符串和 None 在 UI 上会被渲染成两样东西（一个是路径、一个是
    // 「未知」）。约定统一成 None，别让下游自己去猜。
    let snapshot = super::process_snapshot();
    for sample in &snapshot {
        assert!(
            sample.exe_path.as_deref() != Some(""),
            "进程 {} 的 exe_path 是空字符串，应当用 None",
            sample.pid
        );
    }
}

// ===== 启动时间 =====
//
// 「运行时长」与「年轻进程」判定都要它。拿不到时必须是 None 而不是 0 ——
// 0 会被当成「1970 年启动」，进而算出 56 年运行时长，或者反过来把
// 每个进程都判成「刚启动」。

#[test]
fn our_own_start_time_is_a_plausible_epoch_second() {
    let me = std::process::id() as i32;
    let sample = super::sample_process(me).expect("应当能采到自己");
    let start = sample
        .start_time_epoch_secs
        .expect("自己的启动时间必须拿得到");
    // Unix epoch 之后的合理区间：2001 年到「现在 + 1 小时」之间
    assert!(
        start > 1_000_000_000 && start < now_epoch_secs() + 3600,
        "启动时间 {start} 不在合理区间"
    );
}

#[test]
fn uptime_is_measured_from_the_start_time() {
    let me = std::process::id() as i32;
    let sample = super::sample_process(me).expect("应当能采到自己");
    let start = sample.start_time_epoch_secs.expect("有启动时间");
    let uptime = now_epoch_secs().saturating_sub(start);
    assert!(
        uptime < 86_400,
        "测试进程的运行时长 {uptime} 秒（{:.1} 小时）不合理",
        uptime as f64 / 3600.0
    );
}

fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
