use super::*;
use std::collections::HashSet;

fn is_lower_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn process_target(pid: u32, label: &str, start_time: u64) -> ProcessTarget {
    ProcessTarget {
        identity: ProcessIdentity {
            pid,
            name: label.to_owned(),
            exe: format!("/usr/local/bin/{label}"),
            start_time,
        },
        protected: false,
        whitelisted: false,
    }
}

fn guarded_target(pid: u32, label: &str, protected: bool, whitelisted: bool) -> ProcessTarget {
    ProcessTarget {
        identity: ProcessIdentity {
            pid,
            name: label.to_owned(),
            exe: format!("/usr/local/bin/{label}"),
            start_time: 10 + u64::from(pid),
        },
        protected,
        whitelisted,
    }
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

#[test]
fn process_snapshot_keys_and_targets_are_one_to_one() {
    let mut store = OperationStore::new();
    let registration = store
        .register_process_snapshot(vec![
            process_target(101, "first", 10),
            process_target(102, "second", 20),
        ])
        .unwrap();
    assert_eq!(registration.selection_keys.len(), 2);
    assert!(registration.selection_keys.iter().all(|key| {
        is_lower_hex(key) && key != "101" && key != "102" && key != "first" && key != "second"
    }));

    let prepared = store
        .prepare_process(
            &registration.snapshot_id,
            vec![&registration.selection_keys[1]],
            ProcessMode::Graceful,
            "main",
        )
        .unwrap();
    let OperationPlan::Process { mode, targets } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected process plan")
    };

    assert_eq!(mode, ProcessMode::Graceful);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].identity.pid, 102);
    assert_eq!(targets[0].identity.start_time, 20);
    assert_eq!(targets[0].identity.exe, "/usr/local/bin/second");
}

#[test]
fn process_snapshot_rejects_empty_duplicate_and_foreign_keys() {
    let mut store = OperationStore::new();
    let application = store
        .register_application_snapshot(vec![app_identity("notes", &[(203, "notes", 32)])])
        .unwrap();
    let process = store
        .register_process_snapshot(vec![process_target(201, "p", 30)])
        .unwrap();
    let key = process.selection_keys[0].clone();
    let snapshot_id = process.snapshot_id.clone();

    assert_eq!(
        store
            .prepare_process(&snapshot_id, Vec::<&str>::new(), ProcessMode::Force, "main")
            .unwrap_err(),
        "选择不能为空"
    );
    assert!(store
        .prepare_process(
            &snapshot_id,
            vec![key.as_str(), key.as_str()],
            ProcessMode::Force,
            "main"
        )
        .is_err());
    assert!(store
        .prepare_process(
            &snapshot_id,
            vec![application.app_keys[0].as_str()],
            ProcessMode::Force,
            "main"
        )
        .is_err());
    assert!(store
        .prepare_process(&snapshot_id, vec!["201"], ProcessMode::Force, "main")
        .is_err());

    let replaced = store
        .register_process_snapshot(vec![process_target(301, "next", 40)])
        .unwrap();
    assert_ne!(replaced.selection_keys[0], key);
    assert!(store
        .prepare_process(&snapshot_id, vec![key.as_str()], ProcessMode::Force, "main")
        .is_err());
}

#[test]
fn process_snapshot_keeps_protection_state_from_backend_scan() {
    let mut store = OperationStore::new();
    let mut guarded = process_target(401, "guarded", 50);
    guarded.protected = true;
    let registration = store
        .register_process_snapshot(vec![guarded, process_target(402, "plain", 60)])
        .unwrap();

    let prepared = store
        .prepare_process(
            &registration.snapshot_id,
            vec![
                registration.selection_keys[0].as_str(),
                registration.selection_keys[1].as_str(),
            ],
            ProcessMode::Force,
            "main",
        )
        .unwrap();
    let OperationPlan::Process { targets, .. } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected process plan")
    };

    assert!(targets[0].protected);
    assert!(!targets[0].whitelisted);
    assert!(!targets[1].protected && !targets[1].whitelisted);
}

#[test]
fn application_snapshot_binds_distinct_app_and_child_keys() {
    let mut store = OperationStore::new();
    let registration = store
        .register_application_snapshot(vec![
            app_identity("editor", &[(501, "editor", 70), (502, "editor helper", 71)]),
            app_identity("notes", &[(503, "notes", 72)]),
        ])
        .unwrap();

    assert_eq!(registration.app_keys.len(), 2);
    assert_eq!(registration.child_bindings.len(), 3);
    let mut all: Vec<&str> = registration.app_keys.iter().map(String::as_str).collect();
    all.extend(
        registration
            .child_bindings
            .iter()
            .map(|binding| binding.child_key.as_str()),
    );
    let unique: HashSet<&str> = all.iter().copied().collect();
    assert_eq!(unique.len(), 5);
    assert!(all.iter().all(|key| is_lower_hex(key)));
}

#[test]
fn application_child_keys_stay_inside_their_own_app() {
    let mut store = OperationStore::new();
    let registration = store
        .register_application_snapshot(vec![
            app_identity("editor", &[(601, "editor", 80)]),
            app_identity("notes", &[(602, "notes", 81)]),
        ])
        .unwrap();
    let editor_key = registration.app_keys[0].clone();
    let notes_key = registration.app_keys[1].clone();
    let editor_child = registration.child_bindings[0].child_key.clone();
    let notes_child = registration.child_bindings[1].child_key.clone();

    assert_eq!(registration.child_bindings[0].app_key, editor_key);
    assert_eq!(registration.child_bindings[1].app_key, notes_key);
    assert_eq!(registration.child_bindings[0].pid, 601);
    assert_eq!(registration.child_bindings[1].pid, 602);
    assert_ne!(editor_child, notes_child);
    for child in [&editor_child, &notes_child] {
        assert!(store
            .prepare_app_termination(
                &registration.snapshot_id,
                vec![child.as_str()],
                ProcessMode::Force,
                "main"
            )
            .is_err());
    }
}

#[test]
fn application_snapshot_rejects_empty_duplicate_and_foreign_app_keys() {
    let mut store = OperationStore::new();
    let process = store
        .register_process_snapshot(vec![process_target(701, "p", 90)])
        .unwrap();
    let registration = store
        .register_application_snapshot(vec![
            app_identity("editor", &[(702, "editor", 91)]),
            app_identity("notes", &[(703, "notes", 92)]),
        ])
        .unwrap();
    let first = registration.app_keys[0].clone();

    assert!(store
        .prepare_app_termination(
            &registration.snapshot_id,
            Vec::<&str>::new(),
            ProcessMode::Force,
            "main"
        )
        .is_err());
    assert!(store
        .prepare_app_termination(
            &registration.snapshot_id,
            vec![first.as_str(), first.as_str()],
            ProcessMode::Force,
            "main"
        )
        .is_err());
    assert!(store
        .prepare_app_termination(
            &registration.snapshot_id,
            vec![process.selection_keys[0].as_str()],
            ProcessMode::Force,
            "main"
        )
        .is_err());
    assert!(store
        .prepare_app_termination(
            &registration.snapshot_id,
            vec!["/Applications/editor.app"],
            ProcessMode::Force,
            "main"
        )
        .is_err());
}

#[test]
fn application_plan_expands_only_backend_process_identities() {
    let mut store = OperationStore::new();
    let registration = store
        .register_application_snapshot(vec![app_identity("editor", &[(801, "editor", 100)])])
        .unwrap();
    let prepared = store
        .prepare_app_termination(
            &registration.snapshot_id,
            vec![registration.app_keys[0].as_str()],
            ProcessMode::Force,
            "main",
        )
        .unwrap();
    let OperationPlan::AppTerminate { mode, targets } = store
        .consume(&prepared.operation_id, "main")
        .unwrap()
        .into_plan()
    else {
        panic!("expected app termination plan")
    };

    assert_eq!(mode, ProcessMode::Force);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].bundle_path, "/Applications/editor.app");
    assert_eq!(targets[0].processes.len(), 1);
    assert_eq!(targets[0].processes[0].identity.pid, 801);
    assert_eq!(targets[0].processes[0].identity.start_time, 100);
}

#[test]
fn application_snapshot_replacement_invalidates_app_operation() {
    let mut store = OperationStore::new();
    let registration = store
        .register_application_snapshot(vec![app_identity("editor", &[(901, "editor", 110)])])
        .unwrap();
    let prepared = store
        .prepare_app_termination(
            &registration.snapshot_id,
            vec![registration.app_keys[0].as_str()],
            ProcessMode::Force,
            "main",
        )
        .unwrap();

    store
        .register_application_snapshot(vec![app_identity("notes", &[(902, "notes", 111)])])
        .unwrap();

    assert!(store.consume(&prepared.operation_id, "main").is_err());
}

#[test]
fn prepare_process_gate_rejects_whitelisted_targets_in_both_modes() {
    for mode in [ProcessMode::Graceful, ProcessMode::Force] {
        let mut store = OperationStore::new();
        let registration = store
            .register_process_snapshot(vec![
                process_target(1101, "plain", 10),
                guarded_target(1102, "safari", true, true),
            ])
            .unwrap();

        let error = store
            .prepare_process(
                &registration.snapshot_id,
                vec![
                    registration.selection_keys[0].as_str(),
                    registration.selection_keys[1].as_str(),
                ],
                mode,
                "main",
            )
            .unwrap_err();

        assert_eq!(error, "白名单进程不能终止", "mode {:?}", mode);
        assert!(store.operations.is_empty());
    }
}

#[test]
fn prepare_process_gate_requires_force_mode_for_protected_targets() {
    let mut store = OperationStore::new();
    let registration = store
        .register_process_snapshot(vec![guarded_target(1103, "chrome-helper", true, false)])
        .unwrap();
    let key = registration.selection_keys[0].clone();

    let error = store
        .prepare_process(
            &registration.snapshot_id,
            vec![key.as_str()],
            ProcessMode::Graceful,
            "main",
        )
        .unwrap_err();
    assert_eq!(error, "受保护进程只能强制终止");
    assert!(store.operations.is_empty());

    let prepared = store
        .prepare_process(
            &registration.snapshot_id,
            vec![key.as_str()],
            ProcessMode::Force,
            "main",
        )
        .unwrap();
    assert_eq!(prepared.item_count, 1);
}

#[test]
fn prepare_process_gate_allows_plain_targets_in_graceful_mode() {
    let mut store = OperationStore::new();
    let registration = store
        .register_process_snapshot(vec![process_target(1104, "plain", 20)])
        .unwrap();

    let prepared = store
        .prepare_process(
            &registration.snapshot_id,
            vec![registration.selection_keys[0].as_str()],
            ProcessMode::Graceful,
            "main",
        )
        .unwrap();

    assert_eq!(prepared.kind, "process");
}

#[test]
fn prepare_app_termination_gate_rejects_whitelisted_child() {
    let mut store = OperationStore::new();
    let mut app = app_identity("editor", &[(1105, "editor", 30)]);
    app.processes[0].whitelisted = true;
    app.processes[0].protected = true;
    let registration = store.register_application_snapshot(vec![app]).unwrap();

    let error = store
        .prepare_app_termination(
            &registration.snapshot_id,
            vec![registration.app_keys[0].as_str()],
            ProcessMode::Force,
            "main",
        )
        .unwrap_err();

    assert_eq!(error, "白名单进程不能终止");
    assert!(store.operations.is_empty());
}

#[test]
fn prepare_app_termination_refuses_graceful_and_keeps_protected_apps_forceable() {
    let mut store = OperationStore::new();
    let mut app = app_identity("editor", &[(1106, "editor", 40), (1107, "helper", 41)]);
    app.processes[0].protected = true;
    let registration = store.register_application_snapshot(vec![app]).unwrap();
    let key = registration.app_keys[0].clone();

    let error = store
        .prepare_app_termination(
            &registration.snapshot_id,
            vec![key.as_str()],
            ProcessMode::Graceful,
            "main",
        )
        .unwrap_err();

    assert!(error.contains("app_graceful_quit"), "{error}");
    assert!(store.operations.is_empty());

    let forced = store
        .prepare_app_termination(
            &registration.snapshot_id,
            vec![key.as_str()],
            ProcessMode::Force,
            "main",
        )
        .expect("受保护应用仍可强制终止");

    assert_eq!(forced.kind, "app_terminate");
}

#[test]
fn overview_and_managed_process_snapshots_use_independent_slots() {
    let mut store = OperationStore::new();
    let overview = store
        .register_overview_process_snapshot(vec![process_target(501, "overview", 1)])
        .unwrap();
    let managed = store
        .register_process_snapshot(vec![process_target(502, "managed", 2)])
        .unwrap();
    let overview_operation = store
        .prepare_process(
            &overview.snapshot_id,
            vec![&overview.selection_keys[0]],
            ProcessMode::Graceful,
            "main",
        )
        .unwrap();
    let managed_operation = store
        .prepare_process(
            &managed.snapshot_id,
            vec![&managed.selection_keys[0]],
            ProcessMode::Graceful,
            "main",
        )
        .unwrap();

    store
        .register_overview_process_snapshot(vec![process_target(503, "fresh", 3)])
        .unwrap();

    let managed_again = store
        .prepare_process(
            &managed.snapshot_id,
            vec![&managed.selection_keys[0]],
            ProcessMode::Graceful,
            "main",
        )
        .expect("进程管理快照不能被主扫描顶掉");
    assert!(store
        .consume(&overview_operation.operation_id, "main")
        .is_err());
    assert!(store
        .consume(&managed_operation.operation_id, "main")
        .is_ok());
    assert!(store.consume(&managed_again.operation_id, "main").is_ok());
}

#[test]
fn process_prepare_rejects_snapshot_from_the_other_slot() {
    let mut store = OperationStore::new();
    let overview = store
        .register_overview_process_snapshot(vec![process_target(601, "overview", 1)])
        .unwrap();
    let managed = store
        .register_process_snapshot(vec![process_target(602, "managed", 2)])
        .unwrap();

    let error = store
        .prepare_process(
            &overview.snapshot_id,
            vec![&managed.selection_keys[0]],
            ProcessMode::Graceful,
            "main",
        )
        .unwrap_err();
    assert_eq!(error, "选择项不存在");

    let error = store
        .prepare_process(
            &managed.snapshot_id,
            vec![&overview.selection_keys[0]],
            ProcessMode::Graceful,
            "main",
        )
        .unwrap_err();
    assert_eq!(error, "选择项不存在");
}

#[test]
fn overview_snapshot_has_its_own_expiry_and_active_entry() {
    let mut store = OperationStore::new();
    let overview = store
        .register_overview_process_snapshot(vec![process_target(701, "overview", 1)])
        .unwrap();
    let managed = store
        .register_process_snapshot(vec![process_target(702, "managed", 2)])
        .unwrap();

    let overview_expiry = store
        .snapshot_expiry_ms(SnapshotKind::OverviewProcess)
        .unwrap();
    let managed_expiry = store.snapshot_expiry_ms(SnapshotKind::Process).unwrap();
    assert!(overview_expiry > 0 && managed_expiry > 0);
    assert!(store
        .active_snapshots
        .contains_key(&SnapshotKind::OverviewProcess));
    assert!(store.active_snapshots.contains_key(&SnapshotKind::Process));
    assert_eq!(
        store.active_snapshots[&SnapshotKind::OverviewProcess].id,
        overview.snapshot_id
    );
    assert_eq!(
        store.active_snapshots[&SnapshotKind::Process].id,
        managed.snapshot_id
    );
}
