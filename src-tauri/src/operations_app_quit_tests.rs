use super::*;
use std::collections::HashSet;

fn child(pid: u32, name: &str, protected: bool, whitelisted: bool) -> ProcessTarget {
    ProcessTarget {
        identity: ProcessIdentity {
            pid,
            name: name.to_owned(),
            exe: format!("/Applications/Demo.app/Contents/MacOS/{name}"),
            start_time: 10 + u64::from(pid),
        },
        protected,
        whitelisted,
    }
}

fn app(label: &str, children: Vec<ProcessTarget>) -> AppIdentity {
    AppIdentity {
        bundle_path: format!("/Applications/{label}.app"),
        bundle_id: format!("com.example.{label}"),
        app_name: label.to_owned(),
        processes: children,
    }
}

fn demo_app() -> AppIdentity {
    app(
        "Demo",
        vec![
            child(700, "Demo", true, false),
            child(701, "Demo Helper", false, false),
        ],
    )
}

fn register(store: &mut OperationStore) -> ApplicationSnapshotRegistration {
    store
        .register_application_snapshot(vec![demo_app()])
        .expect("注册应用快照")
}

fn keys(registration: &ApplicationSnapshotRegistration) -> Vec<String> {
    registration.app_keys.clone()
}

#[test]
fn graceful_quit_plan_keeps_a_distinct_operation_kind() {
    assert_eq!(OperationKind::AppGracefulQuit.label(), "app_graceful_quit");
    assert_ne!(
        OperationKind::AppGracefulQuit.label(),
        OperationKind::AppTerminate.label()
    );
}

#[test]
fn graceful_quit_prepares_from_the_application_snapshot_keys() {
    let mut store = OperationStore::new();
    let registration = register(&mut store);
    let snapshot_id = registration.snapshot_id.clone();

    let prepared = store
        .prepare_app_graceful_quit(&snapshot_id, keys(&registration), "main")
        .expect("准备优雅退出");

    assert_eq!(prepared.kind, "app_graceful_quit");
    assert_eq!(prepared.item_count, 1);
    assert_eq!(prepared.estimated_bytes, 0);
    // 优雅退出与强制终止必须是**不同的两枚 key**：界面上一个是「优雅退出…」
    // 一个是「强制终止…」，共用一枚 key 就分不出来了。
    assert_eq!(
        super::describe_summary(&prepared),
        "opSummary.appGracefulQuit(apps=1, processes=2)"
    );
    assert_ne!(prepared.summary_key, "opSummary.processForce");
    assert!(!prepared.summary_key.contains(&prepared.operation_id));
}

#[test]
fn graceful_quit_summary_reports_process_totals_without_forcing() {
    let mut store = OperationStore::new();
    let registration = register(&mut store);
    let prepared = store
        .prepare_app_graceful_quit(&registration.snapshot_id, keys(&registration), "main")
        .expect("准备优雅退出");

    assert_eq!(
        super::describe_summary(&prepared),
        "opSummary.appGracefulQuit(apps=1, processes=2)"
    );
    assert_ne!(prepared.summary_key, "opSummary.processForce");
}

#[test]
fn graceful_quit_rejects_empty_and_unknown_selections() {
    let mut store = OperationStore::new();
    let registration = register(&mut store);

    assert_eq!(
        store
            .prepare_app_graceful_quit(&registration.snapshot_id, Vec::<String>::new(), "main")
            .expect_err("空选择必须拒绝"),
        "选择不能为空"
    );
    assert_eq!(
        store
            .prepare_app_graceful_quit(
                &registration.snapshot_id,
                vec!["forged-key".to_owned()],
                "main"
            )
            .expect_err("伪造 key 必须拒绝"),
        "选择项不存在"
    );
    assert_eq!(
        store
            .prepare_app_graceful_quit("forged-snapshot", keys(&registration), "main")
            .expect_err("伪造 snapshot 必须拒绝"),
        "快照不存在或已失效"
    );
}

#[test]
fn graceful_quit_never_selects_a_whitelisted_process() {
    let mut store = OperationStore::new();
    let registration = store
        .register_application_snapshot(vec![app(
            "Guarded",
            vec![child(800, "Guarded", true, true)],
        )])
        .expect("注册应用快照");

    assert_eq!(
        store
            .prepare_app_graceful_quit(&registration.snapshot_id, keys(&registration), "main")
            .expect_err("白名单应用不能被退出"),
        "白名单进程不会被退出"
    );
}

#[test]
fn graceful_quit_accepts_protected_processes() {
    let mut store = OperationStore::new();
    let registration = register(&mut store);

    let prepared = store
        .prepare_app_graceful_quit(&registration.snapshot_id, keys(&registration), "main")
        .expect("受保护进程不阻止优雅退出");

    assert_eq!(prepared.kind, "app_graceful_quit");
}

#[test]
fn graceful_quit_rejects_an_app_without_any_process() {
    let mut store = OperationStore::new();
    let registration = store
        .register_application_snapshot(vec![app("Ghost", Vec::new())])
        .expect("注册应用快照");

    assert_eq!(
        store
            .prepare_app_graceful_quit(&registration.snapshot_id, keys(&registration), "main")
            .expect_err("没有进程的应用不能被退出"),
        "目标应用没有可退出的进程，请重新扫描"
    );
}

#[test]
fn graceful_quit_plan_is_invalidated_by_a_newer_application_snapshot() {
    let mut store = OperationStore::new();
    let registration = register(&mut store);
    let prepared = store
        .prepare_app_graceful_quit(&registration.snapshot_id, keys(&registration), "main")
        .expect("准备优雅退出");
    register(&mut store);

    assert_eq!(
        store
            .consume(&prepared.operation_id, "main")
            .expect_err("新快照作废旧 operation"),
        "操作已使用或已失效"
    );
}

#[test]
fn graceful_quit_is_bound_to_the_requesting_owner() {
    let mut store = OperationStore::new();
    let registration = register(&mut store);
    let prepared = store
        .prepare_app_graceful_quit(&registration.snapshot_id, keys(&registration), "main")
        .expect("准备优雅退出");

    assert_eq!(
        store
            .consume(&prepared.operation_id, "other-window")
            .expect_err("owner 必须匹配"),
        "操作所有者不匹配"
    );
    assert!(store.consume(&prepared.operation_id, "main").is_ok());
}

#[test]
fn graceful_quit_and_termination_share_the_application_snapshot() {
    let mut store = OperationStore::new();
    let registration = register(&mut store);
    let quit = store
        .prepare_app_graceful_quit(&registration.snapshot_id, keys(&registration), "main")
        .expect("准备优雅退出");
    let terminate = store
        .prepare_app_termination(
            &registration.snapshot_id,
            keys(&registration),
            ProcessMode::Force,
            "main",
        )
        .expect("准备强制终止");

    let plans: HashSet<String> = [quit.kind, terminate.kind].into_iter().collect();
    assert_eq!(plans.len(), 2);
}

#[test]
fn app_termination_graceful_mode_is_refused_by_the_store() {
    let mut store = OperationStore::new();
    let registration = register(&mut store);

    let error = store
        .prepare_app_termination(
            &registration.snapshot_id,
            keys(&registration),
            ProcessMode::Graceful,
            "main",
        )
        .expect_err("优雅退出必须走专用操作");

    assert!(error.contains("app_graceful_quit"), "{error}");
    assert!(error.contains("优雅退出"), "{error}");
}
