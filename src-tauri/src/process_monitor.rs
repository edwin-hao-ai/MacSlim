//! 只读进程列表：MAS 形态下的「进程管理 / 应用程序」两页。
//!
//! ## 为什么不走 `scanner::list_all`
//!
//! 它依赖 `sysinfo::Process`，而 `sysinfo` 枚举进程走 libproc 的
//! `proc_listallpids`，**这个调用被 App Sandbox 拦掉**（实测 MAS 包里
//! 数出 0 个）。所以 MAS 形态下这条路拿到的是空列表 —— 这就是交接文档里
//! 「进程管理 0 项」「应用程序 0」的真正成因，与 600 秒缓存无关。
//!
//! 要让 `scanner::list_all` 同时服务两种形态，就得把
//! `process_safety::safety_veto` 等一整套安全判定从 `&Process` 改成中性
//! 结构体。那是全项目最不该为一个只读视图去动的地方（AGENTS.md §4：
//! 判定与展示必须解耦）。所以另起这一套。
//!
//! ## 三条硬约束
//!
//! 1. **行形状与 `scanner::ProcessRow` 完全一致** —— 前端零改动。
//! 2. **`full_name` 永远是原始进程名**，绝不能塞展示名
//!    （AGENTS.md §8；`ProcessIdentity.name` 的唯一来源，终止前要逐字复核）。
//! 3. **`protected` 恒为 false** —— MAS 版没有任何终止入口，没有东西需要
//!    保护。设成 true 会让整张列表被 `opacity-70` 变灰，看起来像 App 坏了。
//!
//! 这里**不做**终止进程。看得到 ≠ 管得动。

use crate::process_snapshot::{self, ProcessSample};
use crate::scanner::ProcessRow;
use std::sync::Mutex;
use std::time::Instant;

/// PID 门槛：低于此值全是内核与系统守护进程。与 `scanner::list_all` 同口径，
/// 否则两个构建的列表长得不一样。
const PID_FLOOR: u32 = 50;

/// 上一次采样，用来算 CPU 百分比。
///
/// CPU 百分比是**两次采样的差值**，单次采样拿不到。用 `Mutex<Option<..>>`
/// 而不是让调用方传进来：调用方（`list_all_processes`）不该为了显示一个
/// 百分比而管采样节奏，那是实现细节。
static PREVIOUS_SAMPLE: Mutex<Option<(Vec<ProcessSample>, Instant)>> = Mutex::new(None);

/// 两次采样的最小间隔。比这更近就复用上一次的结果算不了百分比
/// （噪声大于信号），此时 CPU 显示 0。
const MIN_SAMPLE_GAP: std::time::Duration = std::time::Duration::from_millis(300);

/// 构造只读进程行。
#[must_use]
pub fn list_readonly_rows() -> Vec<ProcessRow> {
    let current_uid = nix::unistd::Uid::effective().as_raw();
    let (samples, cpu_percents) = sample_with_cpu();
    let now = epoch_secs();

    samples
        .into_iter()
        .filter(|sample| passes_filters(sample, current_uid))
        .map(|sample| row_from_sample(&sample, cpu_percents.get(&sample.pid).copied(), now))
        .collect()
}

/// 该不该把这个进程放进只读列表。
///
/// 提成纯函数是为了能直接测：一旦从行上回头验，uid 已经丢了，
/// 只能靠「这台机器上有没有 root 进程」这种环境相关的方式间接验。
///
/// 两条门槛与 `scanner::list_all` 逐条一致 —— 两边口径不同会让
/// Developer ID 版与 MAS 版的列表长得不一样。
pub fn passes_filters(sample: &ProcessSample, current_uid: u32) -> bool {
    sample.pid as u32 >= PID_FLOOR && sample.uid == current_uid
}

/// 采一次，并把 CPU 百分比算出来。
fn sample_with_cpu() -> (Vec<ProcessSample>, std::collections::HashMap<i32, f32>) {
    let samples = process_snapshot::process_snapshot();
    let now = Instant::now();
    let Ok(mut slot) = PREVIOUS_SAMPLE.lock() else {
        return (samples, std::collections::HashMap::new());
    };
    let percents = match slot.as_ref() {
        Some((previous, at)) if now.duration_since(*at) >= MIN_SAMPLE_GAP => {
            let seconds = now.duration_since(*at).as_secs_f64();
            process_snapshot::cpu_percent(previous, &samples, seconds)
        }
        // 首次调用，或两次调用挨得太近：本次没有可信的百分比。
        _ => std::collections::HashMap::new(),
    };
    *slot = Some((samples.clone(), now));
    (samples, percents)
}

/// 单个采样 → 一行。
fn row_from_sample(sample: &ProcessSample, cpu_percent: Option<f32>, now: u64) -> ProcessRow {
    let exe = sample.exe_path.clone().unwrap_or_default();
    // 展示名从 bundle 路径派生，原始名原样留在 full_name —— 两者分工不可混。
    let bundle = (!exe.is_empty())
        .then(|| crate::applications::find_app_bundle(std::path::Path::new(&exe)))
        .flatten();
    let bundle_name = bundle.map(|path| {
        path.file_stem()
            .map(|s| s.to_string_lossy().trim_end_matches(".app").to_string())
            .unwrap_or_default()
    });
    let (name, name_key) = display_name(&sample.name, bundle_name);

    ProcessRow {
        pid: sample.pid as u32,
        parent_pid: (sample.ppid > 0).then_some(sample.ppid as u32),
        name,
        name_key,
        full_name: sample.name.clone(),
        exe,
        start_time: sample.start_time_epoch_secs.unwrap_or(0),
        cpu_percent: cpu_percent.unwrap_or(0.0),
        memory_mb: sample.resident_bytes as f64 / 1024.0 / 1024.0,
        uptime_secs: sample
            .start_time_epoch_secs
            .map(|start| now.saturating_sub(start))
            .unwrap_or(0),
        // 拿不到进程状态；`proc_pidinfo` 不给状态位。运行中的进程本来
        // 就是绝大多数，标成 running 不会误导。
        status_key: "process.status.running".to_string(),
        // 端口检测靠 lsof，沙箱里 exec 不了；图标靠 sips，同理。
        ports: Vec::new(),
        icon_base64: None,
        protected: false,
        protected_reason_key: None,
        protected_reason_params: Vec::new(),
        whitelisted: false,
        // 由 `operation_commands::snapshot_process_rows` 登记时回填。
        selection_key: String::new(),
    }
}

/// 展示名优先级与 `scanner` 那条路一致：bundle 名 > bundle id 末段 > 原始名。
///
/// 三个 WebKit 固定映射也在此处理 —— 它们在前端也会再兜一层，但后端给对
/// 值能让首屏少一次闪烁。
fn display_name(raw_name: &str, bundle_name: Option<String>) -> (String, Option<String>) {
    if let Some(bundle) = bundle_name.filter(|b| !b.is_empty()) {
        return (bundle, None);
    }
    let key = webkit_display_key(raw_name);
    match key {
        Some(key) => (key.to_string(), Some(key.to_string())),
        None => (raw_name.to_string(), None),
    }
}

/// 三个 WebKit 进程名的固定展示名（与 scanner 的映射逐字一致）。
fn webkit_display_key(name: &str) -> Option<&'static str> {
    match name {
        "com.apple.WebKit.WebContent" => Some("process.webkit.webContent"),
        "com.apple.WebKit.Networking" => Some("process.webkit.networking"),
        "com.apple.WebKit.GPU" => Some("process.webkit.gpu"),
        _ => None,
    }
}

fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "process_monitor_tests.rs"]
mod tests;
