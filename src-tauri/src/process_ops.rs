use crate::operations::{
    ProcessIdentity, ProcessKillDetail, ProcessKillReport, ProcessMode, ProcessTarget,
};
use crate::user_error::{ErrorCode, UserError};
use nix::errno::Errno;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use std::collections::HashSet;
use std::thread::sleep;
use std::time::Duration;
use sysinfo::{Process, ProcessesToUpdate, System};

impl ProcessIdentity {
    pub fn from_process(proc: &Process) -> Self {
        Self {
            pid: proc.pid().as_u32(),
            name: proc.name().to_string_lossy().to_string(),
            exe: proc
                .exe()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default(),
            start_time: proc.start_time(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LiveProcess {
    pub(crate) identity: ProcessIdentity,
    pub(crate) protected: bool,
    pub(crate) whitelisted: bool,
}

impl LiveProcess {
    #[cfg(test)]
    pub(crate) fn from_process(proc: &Process, parent_pids: &HashSet<u32>) -> Self {
        let identity = ProcessIdentity::from_process(proc);
        let name = identity.name.clone();
        let protection = crate::process_safety::evaluate_protection(proc, &name, parent_pids);
        Self {
            identity,
            protected: protection.protected,
            whitelisted: protection.whitelisted,
        }
    }
}

pub(crate) trait ProcessObserver {
    fn observe(&mut self, pids: &[u32]) -> Result<Vec<LiveProcess>, UserError>;
}

pub(crate) trait ProcessSignaller {
    fn terminate(&mut self, target: &ProcessTarget, mode: ProcessMode) -> KillOutcome;

    fn terminate_all(&mut self, targets: &[ProcessTarget], mode: ProcessMode) -> ProcessKillReport {
        let outcomes = targets
            .iter()
            .map(|target| {
                (
                    target.identity.pid,
                    target.identity.name.clone(),
                    self.terminate(target, mode),
                )
            })
            .collect();
        kill_report(outcomes)
    }
}

pub(crate) type WhitelistPolicy = Box<dyn Fn(&str) -> bool + Send + Sync>;

pub(crate) struct SystemProcessObserver {
    policy: WhitelistPolicy,
}

impl SystemProcessObserver {
    #[cfg(test)]
    pub(crate) fn with_default_policy() -> Self {
        Self::with_policy(crate::whitelist::is_whitelisted)
    }

    pub(crate) fn with_policy<F>(policy: F) -> Self
    where
        F: Fn(&str) -> bool + Send + Sync + 'static,
    {
        Self {
            policy: Box::new(policy),
        }
    }

    fn observe_system(&self, pids: &[u32]) -> Vec<LiveProcess> {
        let mut sys = System::new_all();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        let parent_pids = crate::process_safety::collect_parent_pids(&sys);
        pids.iter()
            .filter_map(|pid| sys.process(sysinfo::Pid::from_u32(*pid)))
            .map(|proc| {
                let identity = ProcessIdentity::from_process(proc);
                let whitelisted = (self.policy)(&identity.name);
                let protection = crate::process_safety::evaluate_protection_with(
                    proc,
                    &identity.name,
                    &parent_pids,
                    whitelisted,
                );
                LiveProcess {
                    identity,
                    protected: protection.protected,
                    whitelisted: protection.whitelisted,
                }
            })
            .collect()
    }
}

impl ProcessObserver for SystemProcessObserver {
    fn observe(&mut self, pids: &[u32]) -> Result<Vec<LiveProcess>, UserError> {
        if pids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.observe_system(pids))
    }
}

pub(crate) struct SystemProcessSignaller;

impl ProcessSignaller for SystemProcessSignaller {
    fn terminate(&mut self, target: &ProcessTarget, mode: ProcessMode) -> KillOutcome {
        match mode {
            ProcessMode::Graceful => graceful_kill(target.identity.pid),
            ProcessMode::Force => force_kill_tree(target.identity.pid),
        }
    }
}

pub(crate) fn kill_report(outcomes: Vec<(u32, String, KillOutcome)>) -> ProcessKillReport {
    let mut report = ProcessKillReport::default();
    for (pid, name, outcome) in outcomes {
        report.record(ProcessKillDetail {
            pid,
            name,
            success: outcome.is_ok(),
            message: outcome.message(),
        });
    }
    report
}

/// 终止一个进程的结果
#[derive(Debug, Clone)]
pub(crate) enum KillOutcome {
    Success,
    AlreadyGone,
    PermissionDenied,
    /// 目标 PID 已死，但同名同路径的进程以新 PID 重新出现 —— 说明有 supervisor 守护
    RespawnedAs {
        new_pid: u32,
        name: String,
    },
    /// SIGKILL 发了进程还在（非常罕见，一般只有僵死或受保护才会这样）
    StillAlive,
    Failed(String),
}

impl KillOutcome {
    pub(crate) fn is_ok(&self) -> bool {
        matches!(self, KillOutcome::Success | KillOutcome::AlreadyGone)
    }
    /// 结果行的结构化文案。`code` 与人类可读消息分离：
    /// 前端拿 `code` 查 `error.<code>` 译文，`message` 只作 CLI / 兜底。
    pub(crate) fn message(&self) -> UserError {
        match self {
            KillOutcome::Success => UserError::new(ErrorCode::KILL_TERMINATED, "已终止"),
            KillOutcome::AlreadyGone => {
                UserError::new(ErrorCode::KILL_ALREADY_GONE, "进程已不存在")
            }
            KillOutcome::PermissionDenied => UserError::new(
                ErrorCode::KILL_PERMISSION_DENIED,
                "权限不足（通常是 root 或系统进程，MacSlim 不应该看到这类进程）",
            ),
            KillOutcome::RespawnedAs { new_pid, name } => UserError::with(
                ErrorCode::KILL_RESPAWNED,
                format!(
                    "原进程已终止，但一个 supervisor 立刻以新 PID {new_pid} 重启了 `{name}`。\
                     请从上游启动器（launchd agent / pm2 / nvm / Cursor / VS Code 等）停止，\
                     或把此进程名加入白名单屏蔽显示。"
                ),
                vec![
                    ("new_pid".to_owned(), new_pid.to_string()),
                    ("name".to_owned(), name.clone()),
                ],
            ),
            KillOutcome::StillAlive => UserError::new(
                ErrorCode::KILL_STILL_ALIVE,
                "SIGKILL 已发送，但系统报告进程仍存活。可能是僵死进程或受内核保护。",
            ),
            KillOutcome::Failed(e) => {
                UserError::one(ErrorCode::KILL_FAILED, format!("失败: {e}"), "reason", e)
            }
        }
    }
}

/// 优雅终止：
/// 1. 收集整个进程子树（pid 自己 + 所有后代）
/// 2. 从叶子往上 SIGTERM，避免父进程重启子进程
/// 3. 等 3 秒
/// 4. 仍存活的再 SIGKILL
///
/// 这是 macOS / Linux 通用的「杀进程树」做法。npm / pnpm / vite / pm2 等
/// supervisor 启动的 node，单独杀 node 会被立刻 respawn，必须连 supervisor
/// 一起杀（或者用户要求）。这里我们选择**杀整棵子树**而保留父进程 —— 这样
/// 父进程得到 SIGCHLD 之后就会自然退出或进入等待态，不会反弹。
pub(crate) fn graceful_kill(pid: u32) -> KillOutcome {
    let mut sys = System::new();
    sys.refresh_all();

    let target = Pid::from_raw(pid as i32);
    if !process_exists(target) {
        return KillOutcome::AlreadyGone;
    }

    // 记下目标进程的 name / exe，之后用来判断是否被 supervisor 重启
    // 注意：sysinfo::Pid 不同于 nix::unistd::Pid
    let target_name: Option<String>;
    let target_exe: Option<std::path::PathBuf>;
    {
        let p = sys.process(sysinfo::Pid::from_u32(pid));
        target_name = p.map(|proc| proc.name().to_string_lossy().to_string());
        target_exe = p.and_then(|proc| proc.exe().map(|e| e.to_path_buf()));
    }

    // 收集整棵子树（含自身）
    let tree = collect_descendants(&sys, pid);
    let mut ordered: Vec<u32> = tree.iter().copied().collect();
    ordered.sort_by(|a, b| {
        let da = depth(&sys, *a);
        let db = depth(&sys, *b);
        db.cmp(&da)
    });

    // 第一轮：SIGTERM（从叶子往上发）
    for p in &ordered {
        let np = Pid::from_raw(*p as i32);
        match kill(np, Signal::SIGTERM) {
            Ok(_) | Err(Errno::ESRCH) => {}
            Err(Errno::EPERM) => return KillOutcome::PermissionDenied,
            Err(error) => {
                return KillOutcome::Failed(format!("SIGTERM 发送失败: {error}"));
            }
        }
    }

    // 等 3 秒判断目标是否消失
    let mut target_gone = false;
    for _ in 0..6 {
        sleep(Duration::from_millis(500));
        if !process_exists(target) {
            target_gone = true;
            break;
        }
    }

    // 目标还没死 → SIGKILL 子树
    if !target_gone {
        for p in &ordered {
            let np = Pid::from_raw(*p as i32);
            let _ = kill(np, Signal::SIGKILL);
        }
        sleep(Duration::from_secs(1));
        target_gone = !process_exists(target);
    }

    if !target_gone {
        // SIGKILL 都失败了（极罕见）
        return KillOutcome::StillAlive;
    }

    // 目标进程确实死了 —— 但看 supervisor 有没有立刻以新 PID 拉起同名进程
    // 给 supervisor 一点时间（2 秒）复活
    sleep(Duration::from_millis(1200));
    let respawn = detect_respawn(pid, target_name.as_deref(), target_exe.as_deref());
    if let Some((new_pid, name)) = respawn {
        return KillOutcome::RespawnedAs { new_pid, name };
    }

    KillOutcome::Success
}

/// 判断是否有同名同路径的新进程冒出来（supervisor 重启）
fn detect_respawn(
    old_pid: u32,
    name: Option<&str>,
    exe: Option<&std::path::Path>,
) -> Option<(u32, String)> {
    let mut sys = System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    for (pid, proc) in sys.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == old_pid {
            continue; // 同 PID 不算重启
        }
        let pname = proc.name().to_string_lossy().to_string();
        let pexe = proc.exe();

        // name + exe 都一致才算重启（避免把普通同名进程误判为重启）
        let name_match = name.map(|n| n == pname).unwrap_or(false);
        let exe_match = match (exe, pexe) {
            (Some(a), Some(b)) => a == b,
            // 有一个没路径信息 → 只匹配 name 也算
            _ => name_match,
        };

        if name_match && exe_match {
            // 为避免匹配到已经运行很久的同名进程（巧合），
            // 要求新进程启动时间距「kill 时刻」很近（最近 3 秒内）
            let started = proc.start_time();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if now.saturating_sub(started) < 3 {
                return Some((pid_u32, pname));
            }
        }
    }
    None
}

/// 非破坏性探测：进程是否存在
fn process_exists(pid: Pid) -> bool {
    // kill(pid, 0) 是 POSIX 标准用法：不发信号但做权限检查
    // Ok(()) = 存在且有权限； ESRCH = 不存在； EPERM = 存在但无权限
    matches!(kill(pid, None), Ok(()) | Err(Errno::EPERM))
}

/// 对给定 pid，收集它和它所有后代进程的 pid 集合
fn collect_descendants(sys: &System, root: u32) -> HashSet<u32> {
    let mut out = HashSet::new();
    out.insert(root);

    // BFS 扫父子关系
    let mut frontier = vec![root];
    while let Some(parent) = frontier.pop() {
        for proc in sys.processes().values() {
            if let Some(ppid) = proc.parent() {
                let ppid = ppid.as_u32();
                let child = proc.pid().as_u32();
                if ppid == parent && !out.contains(&child) {
                    out.insert(child);
                    frontier.push(child);
                }
            }
        }
    }
    out
}

/// 返回进程到「某祖先」的深度（粗略用）
fn depth(sys: &System, pid: u32) -> usize {
    let mut d = 0usize;
    let mut cur = pid;
    for _ in 0..32 {
        // 最多往上追 32 层
        let mut next: Option<u32> = None;
        for proc in sys.processes().values() {
            if proc.pid().as_u32() == cur {
                if let Some(p) = proc.parent() {
                    next = Some(p.as_u32());
                }
                break;
            }
        }
        match next {
            Some(p) if p != 0 && p != 1 => {
                cur = p;
                d += 1;
            }
            _ => break,
        }
    }
    d
}

pub(crate) fn force_kill_tree(pid: u32) -> KillOutcome {
    let mut sys = System::new();
    sys.refresh_all();
    let target = Pid::from_raw(pid as i32);
    if !process_exists(target) {
        return KillOutcome::AlreadyGone;
    }
    for tree_pid in collect_descendants(&sys, pid) {
        let _ = kill(Pid::from_raw(tree_pid as i32), Signal::SIGKILL);
    }
    for _ in 0..4 {
        sleep(Duration::from_millis(250));
        if !process_exists(target) {
            return KillOutcome::Success;
        }
    }
    KillOutcome::StillAlive
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::{ProcessIdentity, ProcessMode, ProcessTarget};

    #[test]
    fn process_identity_is_built_from_one_sysinfo_sample() {
        let mut sys = System::new_all();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let proc = sys.processes().values().next().expect("需要真实进程");

        let identity = ProcessIdentity::from_process(proc);

        assert_eq!(identity.pid, proc.pid().as_u32());
        assert_eq!(identity.name, proc.name().to_string_lossy().to_string());
        assert_eq!(
            identity.exe,
            proc.exe()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default()
        );
        assert_eq!(identity.start_time, proc.start_time());
    }

    #[test]
    fn process_live_sample_matches_identity_builder() {
        let mut sys = System::new_all();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let pid = std::process::id();
        let proc = sys
            .process(sysinfo::Pid::from_u32(pid))
            .expect("当前测试进程必须存在");
        let parent_pids = crate::process_safety::collect_parent_pids(&sys);
        let name = proc.name().to_string_lossy().to_string();

        let live = LiveProcess::from_process(proc, &parent_pids);

        assert_eq!(live.identity, ProcessIdentity::from_process(proc));
        assert_eq!(live.identity.pid, pid);
        assert_eq!(live.whitelisted, crate::whitelist::is_whitelisted(&name));
        assert!(!live.whitelisted);
        assert!(crate::process_safety::is_young_process(proc));
        assert!(live.protected);
    }

    #[test]
    fn process_kill_report_marks_denied_and_already_gone() {
        let report = kill_report(vec![
            (11, "ok".to_owned(), KillOutcome::Success),
            (12, "denied".to_owned(), KillOutcome::PermissionDenied),
            (13, "gone".to_owned(), KillOutcome::AlreadyGone),
        ]);

        assert_eq!(report.killed, vec![11, 13]);
        assert_eq!(report.failed, vec![12]);
        assert_eq!(report.details.len(), 3);
        assert!(report.details[0].success);
        assert!(!report.details[1].success);
        assert!(report.details[1].message.contains("权限"));
    }

    #[test]
    fn process_kill_report_keeps_respawn_reason_visible() {
        let report = kill_report(vec![(
            21,
            "supervised".to_owned(),
            KillOutcome::RespawnedAs {
                new_pid: 22,
                name: "supervised".to_owned(),
            },
        )]);

        assert!(report.failed.contains(&21));
        assert!(report.details[0].message.contains("22"));
    }

    #[test]
    fn process_target_terminates_with_the_requested_mode() {
        let target = ProcessTarget {
            identity: ProcessIdentity {
                pid: 31,
                name: "sleepy".to_owned(),
                exe: "/usr/bin/sleepy".to_owned(),
                start_time: 1,
            },
            protected: false,
            whitelisted: false,
        };
        let mut recorder = RecordingSignaller::default();

        let outcome = recorder.terminate(&target, ProcessMode::Force);

        assert!(outcome.is_ok());
        assert_eq!(recorder.calls, vec![(31, ProcessMode::Force)]);
    }

    #[test]
    fn process_system_signaller_stays_the_production_implementation() {
        fn assert_signaller<T: ProcessSignaller + Send>() {}
        fn assert_observer<T: ProcessObserver + Send>() {}

        assert_signaller::<SystemProcessSignaller>();
        assert_observer::<SystemProcessObserver>();
    }

    #[derive(Default)]
    struct RecordingSignaller {
        calls: Vec<(u32, ProcessMode)>,
    }

    impl ProcessSignaller for RecordingSignaller {
        fn terminate(&mut self, target: &ProcessTarget, mode: ProcessMode) -> KillOutcome {
            self.calls.push((target.identity.pid, mode));
            KillOutcome::Success
        }
    }
}
