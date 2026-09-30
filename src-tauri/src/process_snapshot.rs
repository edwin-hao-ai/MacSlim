//! 只读进程快照：**绕开 `sysinfo` 也能在 App Sandbox 里枚举进程**。
//!
//! ## 为什么不用 `sysinfo`
//!
//! `sysinfo` 枚举进程走的是 `libproc` 的 `proc_listallpids`。这个调用被
//! App Sandbox 拦掉了 —— 实测（2026-09-30，MAS 包真机）同一进程里
//! `sysinfo` 数出 **0** 个进程。交接文档里「进程管理 0 项」就是这么来的：
//! **不是沙箱读不到进程，是依赖交白卷。**
//!
//! 绕开的办法是换一条路：
//!
//! 1. `sysctl(KERN_PROC_ALL)` 拿 PID 列表 —— 实测在沙箱里**可用**
//!    （同一进程数出 400 个）。`sysinfo` 没用它，这正是它的盲区。
//! 2. 逐个 PID 调 `proc_pidinfo(PROC_PIDTASKALLINFO)` 取明细。
//!
//! 拿到的是进程名、pid/ppid、uid、常驻内存、**累计 CPU 纳秒**、线程数。
//! 两次采样相减即得 CPU 百分比。
//!
//! ## 这个模块不做终止进程
//!
//! 看得到 ≠ 能终止。沙箱不允许给别的进程发信号，那条路没有任何
//! entitlement 能放行。所以 MAS 版有「进程监控」而没有「结束进程」，
//! 两件事不矛盾，别混为一谈。
//!
//! 完整版仍然走 `sysinfo`（它信息更全、还有 cmdline/exe），这里只作为
//! MAS 形态与 `sysinfo` 异常时的退路。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 一个进程的只读快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessSample {
    pub pid: i32,
    pub ppid: i32,
    /// 进程名。**永远是系统里的原始进程名**，不做任何改写/美化 ——
    /// 这是全项目一致的不变量（AGENTS.md §8 硬约束）。
    pub name: String,
    pub uid: u32,
    /// 常驻内存（字节）。
    pub resident_bytes: u64,
    /// 累计 CPU 时间（纳秒）。**绝对值**，不是百分比。
    pub cpu_nanos: u64,
    pub thread_count: i32,
    /// 进程的可执行文件路径。
    ///
    /// 「应用程序」页靠它把进程按 `.app` bundle 聚合。沙箱里**大多数进程
    /// 拿不到**（可执行文件在别的 app 的 container 或受保护位置），所以是
    /// `Option`；拿不到时统一用 `None`，不用空串 —— 空串和「未知」在 UI
    /// 上会被渲染成两样东西，约定统一掉，下游就不用猜。
    pub exe_path: Option<String>,
    /// 进程启动时间（Unix epoch 秒）。
    ///
    /// `None` 表示拿不到。**绝不能用 0 代替** —— 0 会被当成「1970 年启动」，
    /// 于是运行时长算成 56 年，或者反过来把每个进程都判成「刚启动」。
    pub start_time_epoch_secs: Option<u64>,
}

/// 用 `sysctl(KERN_PROC_ALL)` 枚举 PID。`None` = 系统调用被拒。
#[must_use]
pub fn list_pids() -> Option<Vec<i32>> {
    let mut mib: [libc::c_int; 3] = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_ALL];
    let needed = sysctl_needed(&mut mib)?;
    if needed == 0 {
        return Some(Vec::new());
    }
    let mut buffer = vec![0u8; needed];
    let filled = sysctl_read(&mut mib, &mut buffer)?;

    // 每条 kinfo_proc 648 字节（x86_64 与 arm64 一致）。写错的后果不是
    // 编译错误而是 PID 数离谱，所以 `kinfo_proc_size_is_not_merely_a_guess`
    // 那条测试拿它跟 sysinfo 的独立计数对账。
    const KINFO_PROC_SIZE: usize = 648;
    // kinfo_proc 的第一个字段就是 extern_proc，`p_pid` 在其中的字节偏移。
    //
    // 按 SDK 的 sys/proc.h 推导：
    //   p_un（两个指针的 union）      0..16
    //   p_vmspace（指针）            16..24
    //   p_sigacts（指针）            24..32
    //   p_flag（int）                32..36
    //   p_stat（char）               36..37
    //   对齐到 4                      37..40
    //   p_pid                         40
    //
    // 别猜成 32（那是 p_flag）—— 猜错的后果不是编译错误而是**同一批 PID
    // 重复几百遍**。`pids_have_no_duplicates_and_look_like_pids` 守着这条。
    const PID_OFFSET: usize = 40;

    let count = filled / KINFO_PROC_SIZE;
    let mut pids = Vec::with_capacity(count);
    for index in 0..count {
        let base = index * KINFO_PROC_SIZE + PID_OFFSET;
        let Some(slice) = buffer.get(base..base + 4) else {
            break;
        };
        let pid = i32::from_ne_bytes([slice[0], slice[1], slice[2], slice[3]]);
        if pid > 0 {
            pids.push(pid);
        }
    }
    Some(pids)
}

/// 取单个进程的明细。拿不到（进程刚好退出了）时返回 `None`。
#[must_use]
pub fn sample_process(pid: i32) -> Option<ProcessSample> {
    let mut info: libc::proc_taskallinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskallinfo>() as libc::c_int;
    // SAFETY: info 是已初始化的零值，size 与它匹配；内核只写不超过 size
    // 字节。写回来的字节数用 written 收，避免「实际写少了却按全量读」。
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKALLINFO,
            0,
            (&mut info as *mut libc::proc_taskallinfo).cast(),
            size,
        )
    };
    if written < size_of::<libc::proc_bsdinfo>() as libc::c_int {
        return None;
    }
    let bsd = &info.pbsd;
    let task = &info.ptinfo;
    Some(ProcessSample {
        pid: bsd.pbi_pid as i32,
        ppid: bsd.pbi_ppid as i32,
        name: c_str_field(&bsd.pbi_name).or_else(|| c_str_field(&bsd.pbi_comm))?,
        uid: bsd.pbi_uid,
        resident_bytes: task.pti_resident_size,
        cpu_nanos: task.pti_total_user.saturating_add(task.pti_total_system),
        thread_count: task.pti_threadnum,
        exe_path: exe_path(pid),
        start_time_epoch_secs: (bsd.pbi_start_tvsec > 0).then_some(bsd.pbi_start_tvsec),
    })
}

/// 取进程的可执行文件路径。拿不到（权限、或进程已退出）时 `None`。
fn exe_path(pid: i32) -> Option<String> {
    // MAXPATHLEN = 1024；缓冲区不足时内核返回 -1 而不是截断，所以宁可
    // 给足再判长度。
    let mut buffer = vec![0u8; 1024];
    // SAFETY: buffer 是长度为 1024 的已初始化缓冲区，buffersize 与它一致。
    // 返回值是实际写入的字节数（含结尾 NUL），为负表示失败。
    let written = unsafe {
        libc::proc_pidpath(
            pid,
            buffer.as_mut_ptr().cast(),
            buffer.len() as libc::c_uint,
        )
    };
    if written <= 0 {
        return None;
    }
    let end = (written as usize).min(buffer.len());
    let chars: Vec<libc::c_char> = buffer[..end].iter().map(|b| *b as libc::c_char).collect();
    c_str_field(&chars)
}

/// 取全量快照。
#[must_use]
pub fn process_snapshot() -> Vec<ProcessSample> {
    list_pids()
        .unwrap_or_default()
        .into_iter()
        .filter_map(sample_process)
        .collect()
}

/// 两次采样之间的 CPU 百分比（按「一个核 = 100%」算）。
///
/// 三条硬规则，都是为了让 UI 上的数字不骗人：
/// - 上一次没有这个进程 → 0（不硬减，否则新进程会显示一个巨大的假百分比）
/// - 累计时间变小 → 0（说明读的不是累计值，减出来是负数）
/// - 结果截断到 100（宁可少报也不显示 4000%）
#[must_use]
pub fn cpu_percent(
    before: &[ProcessSample],
    after: &[ProcessSample],
    interval_secs: f64,
) -> HashMap<i32, f32> {
    let mut result = HashMap::new();
    if interval_secs <= 0.0 {
        return result;
    }
    for sample in after {
        let previous = before.iter().find(|p| p.pid == sample.pid);
        let delta = previous
            .map(|p| sample.cpu_nanos.saturating_sub(p.cpu_nanos))
            .unwrap_or(0);
        let percent = delta as f64 / 1e9 / interval_secs * 100.0;
        result.insert(sample.pid, percent.clamp(0.0, 100.0) as f32);
    }
    result
}

fn c_str_field(field: &[libc::c_char]) -> Option<String> {
    let bytes: Vec<u8> = field
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    if bytes.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn sysctl_needed(mib: &mut [libc::c_int]) -> Option<usize> {
    let mut needed: libc::size_t = 0;
    // SAFETY: mib 非空且长度合法；oldp 为 null 时内核只写 newlen。
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            std::ptr::null_mut(),
            &mut needed,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(needed)
}

fn sysctl_read(mib: &mut [libc::c_int], buffer: &mut [u8]) -> Option<usize> {
    let mut len = buffer.len();
    // SAFETY: mib 合法；oldp 指向长度为 len 的已初始化缓冲区，newlen 描述
    // 的正是这块缓冲区；newp 为 null 表示不需要写回内核。
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            buffer.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(len)
}

fn size_of<T>() -> usize {
    std::mem::size_of::<T>()
}

#[cfg(test)]
#[path = "process_snapshot_tests.rs"]
mod tests;
