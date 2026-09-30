use super::*;
use crate::process_snapshot::ProcessSample;

// 这一组测试守的是 MAS 版「进程管理 / 应用程序」两页的行为。
//
// ## 为什么单独一个模块，而不是给 scanner 加一条分支
//
// `scanner::list_all` 走 `sysinfo::Process`，而那条路在沙箱里枚举不到进程
// （`proc_listallpids` 被拦）。要让它同时服务两种形态，就得把
// `process_safety::safety_veto` 等一整套安全判定从 `&Process` 改成
// 中性结构体 —— 而那是全项目最不该为了一个只读视图去动的地方
// （AGENTS.md §4：判定与展示必须解耦）。
//
// 所以这里另起一套**只读**行构造，并遵守三条硬约束：
//
// 1. 行形状与 `scanner::ProcessRow` **完全一致** —— 前端零改动，
//    `ProcessView` 那 761 行一行都不用动。
// 2. `full_name` 永远是**原始进程名**，绝不能塞展示名
//    （AGENTS.md §8 硬约束；`ProcessIdentity.name` 的唯一来源）。
// 3. `protected` 恒为 false —— MAS 版没有任何终止入口，**没有东西需要保护**。
//    设成 true 会让整张列表被 `opacity-70` 变灰，看起来像坏了。

#[test]
fn rows_are_built_from_the_real_snapshot() {
    let rows = super::list_readonly_rows();
    assert!(rows.len() > 50, "只读列表只有 {} 行，明显不对", rows.len());
    let me = std::process::id() as u32;
    assert!(
        rows.iter().any(|r| r.pid == me),
        "只读列表里必须有自己（pid {me}）"
    );
}

#[test]
fn full_name_is_always_the_raw_process_name_never_the_display_name() {
    // 这是 AGENTS.md §8 的硬约束。`revalidate_targets` 会在终止前把
    // `ProcessIdentity.name` 与现场读到的原始名逐字比较 —— 一旦这里塞了
    // 「Google Chrome」这种展示名，一键清理会全部报「进程身份已变化」。
    let rows = super::list_readonly_rows();
    let with_bundle = rows
        .iter()
        .find(|r| r.exe.contains(".app/Contents/"))
        .expect("本机总有带 bundle 路径的进程");
    assert!(
        !with_bundle.full_name.contains(".app"),
        "full_name 里出现了 bundle 路径，说明塞的是展示名：{}",
        with_bundle.full_name
    );
    assert!(!with_bundle.full_name.is_empty(), "full_name 不能为空");
}

#[test]
fn a_bundle_backed_process_gets_a_human_readable_name() {
    // 反过来也要成立：用户看到的不该是一堆 `/Applications/....app` 路径。
    // 展示名从 bundle 路径派生，`full_name` 保持原始名，两者分工明确。
    let rows = super::list_readonly_rows();
    let with_bundle = rows
        .iter()
        .find(|r| r.exe.contains(".app/Contents/"))
        .expect("本机总有带 bundle 路径的进程");
    assert_ne!(
        with_bundle.name, with_bundle.full_name,
        "带 bundle 的进程应当有可读名，而不只是原始名"
    );
}

#[test]
fn nothing_is_marked_protected_because_nothing_can_be_terminated() {
    // 反向断言：如果哪天这里开始返回 true，整张列表会被 ProcessList 的
    // opacity-70 变灰 —— 用户看到的是「App 坏了」，不是「这里在保护你」。
    let rows = super::list_readonly_rows();
    let protected: Vec<&ProcessRow> = rows.iter().filter(|r| r.protected).collect();
    assert!(
        protected.is_empty(),
        "只读视图不该有 protected 行，实际 {} 行被标了",
        protected.len()
    );
}

#[test]
fn no_row_claims_protected_reasons_or_whitelist() {
    let rows = super::list_readonly_rows();
    for row in &rows {
        assert!(
            row.protected_reason_key.is_none(),
            "pid {} 不该有保护原因",
            row.pid
        );
        assert!(!row.whitelisted, "pid {} 不该被标白名单", row.pid);
    }
}

#[test]
fn ports_are_always_empty_in_read_only_mode() {
    // 端口检测走 lsof，沙箱里 exec 不了。宁可空着也不要给一个假端口。
    let rows = super::list_readonly_rows();
    for row in &rows {
        assert!(row.ports.is_empty(), "pid {} 不该有端口数据", row.pid);
    }
}

#[test]
fn icons_are_always_absent_in_read_only_mode() {
    // 图标靠 exec sips，沙箱里不可用。MAS 构建下 `icns_to_base64_png`
    // 直接返回 None，这里再兜一层，防止将来有人把它接回来。
    let rows = super::list_readonly_rows();
    for row in &rows {
        assert!(row.icon_base64.is_none(), "pid {} 不该有图标", row.pid);
    }
}

#[test]
fn system_processes_below_the_pid_floor_are_left_out() {
    // 与 sysinfo 那条路同一个门槛（pid < 50 全是内核/系统守护进程）。
    // 两边口径不一致会让两个构建的列表长得不一样。
    let rows = super::list_readonly_rows();
    assert!(rows.iter().all(|r| r.pid >= 50), "出现了 pid < 50 的行");
}

fn sample_with(pid: i32, uid: u32) -> ProcessSample {
    ProcessSample {
        pid,
        ppid: 1,
        name: "x".into(),
        uid,
        resident_bytes: 1024,
        cpu_nanos: 0,
        thread_count: 1,
        exe_path: None,
        start_time_epoch_secs: None,
    }
}

#[test]
fn processes_below_the_pid_floor_are_filtered_out() {
    // 内核与系统守护进程。sysinfo 那条路用的是同一条门槛。
    assert!(!super::passes_filters(&sample_with(49, 501), 501));
    assert!(super::passes_filters(&sample_with(50, 501), 501));
}

#[test]
fn processes_belonging_to_other_users_are_filtered_out() {
    // 展示其他用户的进程对普通用户没有意义（root 守护进程一大片），
    // 而 sysinfo 那条路同样按当前用户过滤。两边必须一致。
    assert!(
        !super::passes_filters(&sample_with(500, 0), 501),
        "root 进程不该出现在普通用户的列表里"
    );
    assert!(super::passes_filters(&sample_with(500, 501), 501));
}

#[test]
fn every_row_exposes_a_monotonic_uptime() {
    // uptime 来自启动时间；拿不到启动时间时必须是 0 而不是 56 年。
    let rows = super::list_readonly_rows();
    let with_start: Vec<&ProcessRow> = rows.iter().filter(|r| r.start_time > 0).collect();
    assert!(!with_start.is_empty(), "应当有能读到启动时间的进程");
    for row in &rows {
        assert!(
            row.uptime_secs < 10 * 365 * 86_400,
            "pid {} 的运行时长离谱",
            row.pid
        );
    }
}

#[test]
fn cpu_percent_is_available_and_bounded() {
    let rows = super::list_readonly_rows();
    for row in &rows {
        assert!(
            row.cpu_percent >= 0.0 && row.cpu_percent <= 100.0,
            "pid {} 的 CPU 百分比越界：{}",
            row.pid,
            row.cpu_percent
        );
    }
}

#[test]
fn memory_is_reported_in_megabytes_from_resident_bytes() {
    let me = std::process::id() as u32;
    let rows = super::list_readonly_rows();
    let row = rows.iter().find(|r| r.pid == me).expect("有自己");
    assert!(
        row.memory_mb > 0.0 && row.memory_mb < 8192.0,
        "自己的内存读数离谱：{} MB",
        row.memory_mb
    );
}

#[test]
fn selection_keys_are_left_for_the_registration_layer_to_fill() {
    // `selection_key` 由 `operation_commands::snapshot_process_rows` 在
    // 登记快照时回填。构造阶段必须留空，否则会覆盖掉登记层算出的值 ——
    // 而那个值是终止前身份复核的依据。
    let rows = super::list_readonly_rows();
    for row in &rows {
        assert!(
            row.selection_key.is_empty(),
            "构造阶段不该填 selection_key（pid {}）",
            row.pid
        );
    }
}

#[test]
fn the_whole_list_can_be_built_repeatedly_without_panicking() {
    // 前端会周期性刷新。这里防的是「刷新到一半崩掉」—— 那种问题在现场
    // 只会表现为进程页变空白，没有任何线索。
    for _ in 0..3 {
        let rows = super::list_readonly_rows();
        assert!(!rows.is_empty());
    }
}
