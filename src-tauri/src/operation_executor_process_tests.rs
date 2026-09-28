use super::*;
use crate::operations::{
    AppIdentity, ConsumedPlan, DockerAction, DockerResourceKind, DockerTarget, OperationStore,
    ProcessIdentity, ProcessMode, ProcessTarget,
};
use crate::process_ops::{
    KillOutcome, LiveProcess, ProcessObserver, ProcessSignaller, SystemProcessObserver,
};
use std::sync::{Arc, Mutex};

fn consumed_docker_plan(store: &mut OperationStore) -> ConsumedPlan {
    let registration = store
        .register_docker_resources(vec![DockerTarget {
            resource_type: DockerResourceKind::Image,
            id: "image-id".into(),
            name: "image".into(),
            size_bytes: 0,
            referenced: false,
            reclaimable: false,
        }])
        .unwrap();
    let prepared = store
        .prepare_docker(
            &registration.snapshot_id,
            DockerAction::RemoveImage,
            vec![&registration.selection_keys[0]],
            "main",
        )
        .unwrap();
    store.consume(&prepared.operation_id, "main").unwrap()
}

// ===== 进程与应用终止 =====

#[derive(Clone, Default)]
struct FakeObserver {
    live: Arc<Mutex<Vec<LiveProcess>>>,
    observed: Arc<Mutex<Vec<Vec<u32>>>>,
}

impl FakeObserver {
    fn with_processes(processes: Vec<LiveProcess>) -> Self {
        Self {
            live: Arc::new(Mutex::new(processes)),
            observed: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn set_live(&self, processes: Vec<LiveProcess>) {
        *self.live.lock().unwrap() = processes;
    }
}

impl ProcessObserver for FakeObserver {
    fn observe(&mut self, pids: &[u32]) -> Result<Vec<LiveProcess>, UserError> {
        self.observed.lock().unwrap().push(pids.to_vec());
        Ok(self.live.lock().unwrap().clone())
    }
}

#[derive(Clone, Default)]
struct FakeSignaller {
    calls: Arc<Mutex<Vec<(u32, ProcessMode)>>>,
    outcome: Arc<Mutex<Option<KillOutcome>>>,
}

impl ProcessSignaller for FakeSignaller {
    fn terminate(&mut self, target: &ProcessTarget, mode: ProcessMode) -> KillOutcome {
        self.calls.lock().unwrap().push((target.identity.pid, mode));
        self.outcome
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(KillOutcome::Success)
    }
}

fn live_process(pid: u32, name: &str, exe: &str, start_time: u64) -> LiveProcess {
    LiveProcess {
        identity: ProcessIdentity {
            pid,
            name: name.to_owned(),
            exe: exe.to_owned(),
            start_time,
        },
        protected: false,
        whitelisted: false,
    }
}

fn process_target(pid: u32, name: &str, exe: &str, start_time: u64) -> ProcessTarget {
    ProcessTarget {
        identity: live_process(pid, name, exe, start_time).identity,
        protected: false,
        whitelisted: false,
    }
}

fn consumed_process_plan(
    store: &mut OperationStore,
    targets: Vec<ProcessTarget>,
    mode: ProcessMode,
) -> ConsumedPlan {
    let registration = store.register_process_snapshot(targets).unwrap();
    let keys: Vec<&str> = registration
        .selection_keys
        .iter()
        .map(String::as_str)
        .collect();
    let prepared = store
        .prepare_process(&registration.snapshot_id, keys, mode, "main")
        .unwrap();
    store.consume(&prepared.operation_id, "main").unwrap()
}

fn app_identity(label: &str, children: &[(u32, &str, u64)]) -> AppIdentity {
    AppIdentity {
        bundle_path: format!("/Applications/{label}.app"),
        bundle_id: format!("com.example.{label}"),
        app_name: label.to_owned(),
        processes: children
            .iter()
            .map(|(pid, name, start_time)| ProcessTarget {
                identity: ProcessIdentity {
                    pid: *pid,
                    name: (*name).to_owned(),
                    exe: format!("/Applications/{label}.app/Contents/MacOS/{name}"),
                    start_time: *start_time,
                },
                protected: false,
                whitelisted: false,
            })
            .collect(),
    }
}

fn consumed_app_plan(
    store: &mut OperationStore,
    apps: Vec<AppIdentity>,
    mode: ProcessMode,
) -> ConsumedPlan {
    let registration = store.register_application_snapshot(apps).unwrap();
    let keys: Vec<&str> = registration.app_keys.iter().map(String::as_str).collect();
    let prepared = store
        .prepare_app_termination(&registration.snapshot_id, keys, mode, "main")
        .unwrap();
    store.consume(&prepared.operation_id, "main").unwrap()
}

#[tokio::test]
async fn process_plan_graceful_mode_dispatches_tree_termination() {
    let target = process_target(4242, "sleepy", "/usr/bin/sleepy", 100);
    let mut store = OperationStore::new();
    let consumed = consumed_process_plan(&mut store, vec![target], ProcessMode::Graceful);
    let observer =
        FakeObserver::with_processes(vec![live_process(4242, "sleepy", "/usr/bin/sleepy", 100)]);
    let signaller = FakeSignaller::default();

    let report = execute_process_plan(consumed, &mut observer.clone(), &mut signaller.clone())
        .await
        .unwrap();

    assert_eq!(
        *signaller.calls.lock().unwrap(),
        vec![(4242, ProcessMode::Graceful)]
    );
    assert_eq!(report.killed, vec![4242]);
    assert!(report.failed.is_empty());
    assert_eq!(report.details[0].name, "sleepy");
}

#[tokio::test]
async fn process_plan_force_mode_dispatches_force_termination() {
    let target = process_target(4243, "stubborn", "/usr/bin/stubborn", 100);
    let mut store = OperationStore::new();
    let consumed = consumed_process_plan(&mut store, vec![target], ProcessMode::Force);
    let observer = FakeObserver::with_processes(vec![live_process(
        4243,
        "stubborn",
        "/usr/bin/stubborn",
        100,
    )]);
    let signaller = FakeSignaller::default();

    execute_process_plan(consumed, &mut observer.clone(), &mut signaller.clone())
        .await
        .unwrap();

    assert_eq!(
        *signaller.calls.lock().unwrap(),
        vec![(4243, ProcessMode::Force)]
    );
}

#[tokio::test]
async fn process_plan_rejects_pid_reuse_start_time_change() {
    let target = process_target(4244, "node", "/usr/local/bin/node", 100);
    let mut store = OperationStore::new();
    let consumed = consumed_process_plan(&mut store, vec![target], ProcessMode::Graceful);
    let observer =
        FakeObserver::with_processes(vec![live_process(4244, "node", "/usr/local/bin/node", 900)]);
    let signaller = FakeSignaller::default();

    let error = execute_process_plan(consumed, &mut observer.clone(), &mut signaller.clone())
        .await
        .unwrap_err();

    assert!(error.contains("复用"), "{}", error);
    assert!(signaller.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn process_plan_rejects_name_or_exe_change() {
    let mut store = OperationStore::new();
    let signaller = FakeSignaller::default();

    let name_plan = consumed_process_plan(
        &mut store,
        vec![process_target(4245, "node", "/usr/local/bin/node", 100)],
        ProcessMode::Graceful,
    );
    let renamed = FakeObserver::with_processes(vec![live_process(
        4245,
        "node2",
        "/usr/local/bin/node",
        100,
    )]);
    let name_error = execute_process_plan(name_plan, &mut renamed.clone(), &mut signaller.clone())
        .await
        .unwrap_err();
    assert!(name_error.contains("身份"), "{}", name_error);

    let exe_plan = consumed_process_plan(
        &mut store,
        vec![process_target(4246, "node", "/usr/local/bin/node", 100)],
        ProcessMode::Graceful,
    );
    let replaced =
        FakeObserver::with_processes(vec![live_process(4246, "node", "/tmp/evil/node", 100)]);
    let exe_error = execute_process_plan(exe_plan, &mut replaced.clone(), &mut signaller.clone())
        .await
        .unwrap_err();

    assert!(exe_error.contains("身份"), "{}", exe_error);
    assert!(signaller.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn process_plan_rejects_vanished_process() {
    let target = process_target(4247, "gone", "/usr/bin/gone", 100);
    let mut store = OperationStore::new();
    let consumed = consumed_process_plan(&mut store, vec![target], ProcessMode::Graceful);
    let observer = FakeObserver::default();
    let signaller = FakeSignaller::default();

    let error = execute_process_plan(consumed, &mut observer.clone(), &mut signaller.clone())
        .await
        .unwrap_err();

    assert!(error.contains("不存在"), "{}", error);
    assert!(signaller.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn process_plan_rejects_protection_or_whitelist_state_change() {
    let target = ProcessTarget {
        identity: live_process(4248, "guard", "/usr/bin/guard", 100).identity,
        protected: false,
        whitelisted: false,
    };
    let mut store = OperationStore::new();
    let signaller = FakeSignaller::default();

    let protected_plan =
        consumed_process_plan(&mut store, vec![target.clone()], ProcessMode::Graceful);
    let mut now_protected = live_process(4248, "guard", "/usr/bin/guard", 100);
    now_protected.protected = true;
    let protected_observer = FakeObserver::with_processes(vec![now_protected]);
    let protected_error = execute_process_plan(
        protected_plan,
        &mut protected_observer.clone(),
        &mut signaller.clone(),
    )
    .await
    .unwrap_err();

    assert!(protected_error.contains("保护状态"), "{}", protected_error);

    let whitelist_plan = consumed_process_plan(&mut store, vec![target], ProcessMode::Force);
    let mut now_whitelisted = live_process(4248, "guard", "/usr/bin/guard", 100);
    now_whitelisted.whitelisted = true;
    let mut whitelist_observer = FakeObserver::with_processes(vec![now_whitelisted]);
    let whitelist_error = execute_process_plan(
        whitelist_plan,
        &mut whitelist_observer,
        &mut signaller.clone(),
    )
    .await
    .unwrap_err();

    assert!(whitelist_error.contains("保护状态"), "{}", whitelist_error);
    assert!(signaller.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn process_plan_signals_nothing_when_one_target_mismatches() {
    let first = process_target(4249, "first", "/usr/bin/first", 100);
    let second = process_target(4250, "second", "/usr/bin/second", 200);
    let mut store = OperationStore::new();
    let consumed = consumed_process_plan(&mut store, vec![first, second], ProcessMode::Graceful);
    let observer =
        FakeObserver::with_processes(vec![live_process(4249, "first", "/usr/bin/first", 100)]);
    let signaller = FakeSignaller::default();

    let error = execute_process_plan(consumed, &mut observer.clone(), &mut signaller.clone())
        .await
        .unwrap_err();

    assert!(error.contains("不存在"), "{}", error);
    assert!(signaller.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn process_plan_reports_failed_targets_without_panicking() {
    let target = process_target(4251, "denied", "/usr/bin/denied", 100);
    let mut store = OperationStore::new();
    let consumed = consumed_process_plan(&mut store, vec![target], ProcessMode::Graceful);
    let observer =
        FakeObserver::with_processes(vec![live_process(4251, "denied", "/usr/bin/denied", 100)]);
    let signaller = FakeSignaller {
        calls: Arc::new(Mutex::new(Vec::new())),
        outcome: Arc::new(Mutex::new(Some(KillOutcome::PermissionDenied))),
    };

    let report = execute_process_plan(consumed, &mut observer.clone(), &mut signaller.clone())
        .await
        .unwrap();

    assert!(report.killed.is_empty());
    assert_eq!(report.failed, vec![4251]);
    assert!(!report.details[0].success);
}

#[tokio::test]
async fn process_executor_rejects_non_process_plan_after_consume() {
    let mut store = OperationStore::new();
    let consumed = consumed_docker_plan(&mut store);
    let mut observer = FakeObserver::default();
    let mut signaller = FakeSignaller::default();

    let error = execute_process_plan(consumed, &mut observer, &mut signaller)
        .await
        .unwrap_err();

    assert_eq!(error, "操作计划不是进程计划");
    assert!(signaller.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn process_executor_observer_only_receives_backend_pids() {
    let target = process_target(4252, "scoped", "/usr/bin/scoped", 100);
    let mut store = OperationStore::new();
    let consumed = consumed_process_plan(&mut store, vec![target], ProcessMode::Graceful);
    let mut observer =
        FakeObserver::with_processes(vec![live_process(4252, "scoped", "/usr/bin/scoped", 100)]);
    let mut signaller = FakeSignaller::default();

    execute_process_plan(consumed, &mut observer, &mut signaller)
        .await
        .unwrap();

    assert_eq!(*observer.observed.lock().unwrap(), vec![vec![4252]]);
}

#[tokio::test]
async fn app_termination_expands_app_key_to_backend_process_set() {
    let apps = vec![app_identity(
        "Editor",
        &[(5001, "Editor", 100), (5002, "Editor Helper", 110)],
    )];
    let mut store = OperationStore::new();
    let consumed = consumed_app_plan(&mut store, apps, ProcessMode::Force);
    let observer = FakeObserver::with_processes(vec![
        live_process(
            5001,
            "Editor",
            "/Applications/Editor.app/Contents/MacOS/Editor",
            100,
        ),
        live_process(
            5002,
            "Editor Helper",
            "/Applications/Editor.app/Contents/MacOS/Editor Helper",
            110,
        ),
    ]);
    let signaller = FakeSignaller::default();

    let report =
        execute_app_termination_plan(consumed, &mut observer.clone(), &mut signaller.clone())
            .await
            .unwrap();

    assert_eq!(
        *signaller.calls.lock().unwrap(),
        vec![(5001, ProcessMode::Force), (5002, ProcessMode::Force)]
    );
    assert_eq!(report.killed, vec![5001, 5002]);
}

#[tokio::test]
async fn app_termination_rejects_child_process_from_another_app() {
    let mut apps = vec![app_identity("Editor", &[(5003, "Editor", 100)])];
    apps[0].processes[0].identity.exe = "/Applications/Other.app/Contents/MacOS/Other".to_owned();
    let mut store = OperationStore::new();
    let consumed = consumed_app_plan(&mut store, apps, ProcessMode::Force);
    let observer = FakeObserver::with_processes(vec![live_process(
        5003,
        "Editor",
        "/Applications/Other.app/Contents/MacOS/Other",
        100,
    )]);
    let signaller = FakeSignaller::default();

    let error =
        execute_app_termination_plan(consumed, &mut observer.clone(), &mut signaller.clone())
            .await
            .unwrap_err();

    assert!(error.contains("不属于"), "{}", error);
    assert!(signaller.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn app_termination_rejects_app_without_backend_processes() {
    let mut apps = vec![app_identity("Empty", &[])];
    apps[0].processes.clear();
    let mut store = OperationStore::new();
    let consumed = consumed_app_plan(&mut store, apps, ProcessMode::Force);
    let mut observer = FakeObserver::default();
    let mut signaller = FakeSignaller::default();

    let error = execute_app_termination_plan(consumed, &mut observer, &mut signaller)
        .await
        .unwrap_err();

    assert!(error.contains("没有可终止的进程"), "{}", error);
    assert!(signaller.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn app_termination_rejects_child_identity_change() {
    let apps = vec![app_identity("Notes", &[(5004, "Notes", 100)])];
    let mut store = OperationStore::new();
    let consumed = consumed_app_plan(&mut store, apps, ProcessMode::Force);
    let observer = FakeObserver::with_processes(vec![live_process(
        5004,
        "Notes",
        "/Applications/Notes.app/Contents/MacOS/Notes",
        777,
    )]);
    let signaller = FakeSignaller::default();

    let error =
        execute_app_termination_plan(consumed, &mut observer.clone(), &mut signaller.clone())
            .await
            .unwrap_err();

    assert!(error.contains("复用"), "{}", error);
    assert!(signaller.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn app_termination_rejects_non_app_plan_after_consume() {
    let target = process_target(4253, "solo", "/usr/bin/solo", 100);
    let mut store = OperationStore::new();
    let consumed = consumed_process_plan(&mut store, vec![target], ProcessMode::Graceful);
    let mut observer = FakeObserver::default();
    let mut signaller = FakeSignaller::default();

    let error = execute_app_termination_plan(consumed, &mut observer, &mut signaller)
        .await
        .unwrap_err();

    assert_eq!(error, "操作计划不是应用终止计划");
    assert!(signaller.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn app_termination_child_keys_cannot_be_used_as_app_keys() {
    let mut store = OperationStore::new();
    let registration = store
        .register_application_snapshot(vec![app_identity("Editor", &[(5005, "Editor", 100)])])
        .unwrap();
    let child_key = registration.child_bindings[0].child_key.clone();

    let error = store
        .prepare_app_termination(
            &registration.snapshot_id,
            vec![child_key.as_str()],
            ProcessMode::Force,
            "main",
        )
        .unwrap_err();

    assert_eq!(error, "选择项不存在");
}

#[test]
fn process_plan_and_app_plan_shares_one_consumed_boundary() {
    let mut store = OperationStore::new();
    let target = process_target(4254, "once", "/usr/bin/once", 100);
    let registration = store.register_process_snapshot(vec![target]).unwrap();
    let prepared = store
        .prepare_process(
            &registration.snapshot_id,
            vec![registration.selection_keys[0].as_str()],
            ProcessMode::Graceful,
            "main",
        )
        .unwrap();

    assert!(store.consume(&prepared.operation_id, "main").is_ok());
    assert!(store.consume(&prepared.operation_id, "main").is_err());
}

#[test]
fn system_process_observer_reports_backend_identity_for_current_process() {
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let pid = std::process::id();
    let proc = sys
        .process(sysinfo::Pid::from_u32(pid))
        .expect("当前进程必须存在");
    let expected = ProcessIdentity::from_process(proc);
    let mut observer = crate::process_ops::SystemProcessObserver::with_default_policy();

    let observed = observer.observe(&[pid]).unwrap();

    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].identity.pid, pid);
    assert_eq!(observed[0].identity, expected);
    assert!(observed[0].identity.start_time > 0);
    assert!(!observed[0].identity.name.is_empty());
    assert_eq!(
        observed[0].whitelisted,
        crate::whitelist::is_whitelisted(&observed[0].identity.name)
    );
    assert!(
        observed[0].protected
            || !crate::process_safety::is_multiprocess_family(&observed[0].identity.name)
    );
}

#[test]
fn system_process_observer_omits_vanished_pid() {
    let mut observer = crate::process_ops::SystemProcessObserver::with_default_policy();

    let observed = observer.observe(&[u32::MAX]).unwrap();

    assert!(observed.is_empty());
}

#[test]
fn process_executor_public_boundary_rejects_empty_process_snapshot() {
    let mut store = OperationStore::new();
    let registration = store.register_process_snapshot(Vec::new()).unwrap();
    let prepared = store
        .prepare_process(
            &registration.snapshot_id,
            Vec::<&str>::new(),
            ProcessMode::Graceful,
            "main",
        )
        .unwrap_err();

    assert_eq!(prepared, "选择不能为空");
}

#[test]
fn process_executor_report_types_round_trip_display_fields() {
    let report =
        crate::process_ops::kill_report(vec![(7, "seven".to_owned(), KillOutcome::Success)]);
    assert_eq!(report.killed, vec![7]);
    assert_eq!(report.details.len(), 1);
    assert_eq!(report.details[0].name, "seven");
    assert!(report.details[0].success);
}

#[tokio::test]
async fn process_plan_with_injected_policy_false_passes_revalidation() {
    let pid = std::process::id();
    let mut scan_observer = SystemProcessObserver::with_policy(|_| false);
    let scanned = scan_observer.observe(&[pid]).unwrap();
    let target = ProcessTarget {
        identity: scanned[0].identity.clone(),
        protected: scanned[0].protected,
        whitelisted: scanned[0].whitelisted,
    };
    let mut store = OperationStore::new();
    let consumed = consumed_process_plan(&mut store, vec![target], ProcessMode::Force);
    let mut exec_observer = SystemProcessObserver::with_policy(|_| false);
    let signaller = FakeSignaller::default();

    let report = execute_process_plan(consumed, &mut exec_observer, &mut signaller.clone())
        .await
        .unwrap();

    assert_eq!(report.killed, vec![pid]);
    assert_eq!(signaller.calls.lock().unwrap().len(), 1);
    assert_eq!(
        signaller.calls.lock().unwrap()[0],
        (pid, ProcessMode::Force)
    );
}

#[tokio::test]
async fn process_plan_with_injected_policy_true_is_rejected_by_revalidation() {
    let pid = std::process::id();
    let mut scan_observer = SystemProcessObserver::with_policy(|_| false);
    let scanned = scan_observer.observe(&[pid]).unwrap();
    let target = ProcessTarget {
        identity: scanned[0].identity.clone(),
        protected: scanned[0].protected,
        whitelisted: scanned[0].whitelisted,
    };
    let mut store = OperationStore::new();
    let consumed = consumed_process_plan(&mut store, vec![target], ProcessMode::Force);
    let mut exec_observer = SystemProcessObserver::with_policy(|_| true);
    let signaller = FakeSignaller::default();

    let error = execute_process_plan(consumed, &mut exec_observer, &mut signaller.clone())
        .await
        .unwrap_err();

    assert!(error.contains("保护状态"), "{}", error);
    assert!(signaller.calls.lock().unwrap().is_empty());
}

#[test]
fn process_gate_rule_rejects_whitelist_and_requires_force_for_protected() {
    let plain = process_target(4260, "plain", "/usr/bin/plain", 100);
    let protected = ProcessTarget {
        protected: true,
        ..process_target(4261, "guarded", "/usr/bin/guarded", 100)
    };
    let whitelisted = ProcessTarget {
        protected: true,
        whitelisted: true,
        ..process_target(4262, "whitelisted", "/usr/bin/whitelisted", 100)
    };

    assert_eq!(
        crate::operations::ensure_targets_terminable(
            std::slice::from_ref(&whitelisted),
            ProcessMode::Force,
        )
        .unwrap_err(),
        "白名单进程不能终止"
    );
    assert_eq!(
        crate::operations::ensure_targets_terminable(
            std::slice::from_ref(&whitelisted),
            ProcessMode::Graceful
        )
        .unwrap_err(),
        "白名单进程不能终止"
    );
    assert_eq!(
        crate::operations::ensure_targets_terminable(
            std::slice::from_ref(&protected),
            ProcessMode::Graceful,
        )
        .unwrap_err(),
        "受保护进程只能强制终止"
    );
    assert!(crate::operations::ensure_targets_terminable(
        std::slice::from_ref(&protected),
        ProcessMode::Force
    )
    .is_ok());
    assert!(crate::operations::ensure_targets_terminable(
        std::slice::from_ref(&plain),
        ProcessMode::Graceful
    )
    .is_ok());
}

#[test]
fn system_process_observer_with_policy_true_marks_current_process_whitelisted() {
    let pid = std::process::id();
    let mut observer = SystemProcessObserver::with_policy(|_| true);

    let observed = observer.observe(&[pid]).unwrap();

    assert_eq!(observed.len(), 1);
    assert!(observed[0].whitelisted);
    assert!(observed[0].protected);
}

#[test]
fn system_process_observer_with_policy_false_keeps_process_unlisted() {
    let pid = std::process::id();
    let mut observer = SystemProcessObserver::with_policy(|_| false);

    let observed = observer.observe(&[pid]).unwrap();

    assert_eq!(observed.len(), 1);
    assert!(!observed[0].whitelisted);
    assert!(observed[0].protected);
    assert!(crate::process_safety::is_young_process(
        sysinfo::System::new_all()
            .process(sysinfo::Pid::from_u32(pid))
            .expect("当前测试进程必须存在")
    ));
}
