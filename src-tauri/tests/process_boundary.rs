//! 进程域 crate 边界的**编译级**回归测试。
//!
//! 本文件是一个独立 crate，和 `src/bin/cli.rs` 站在同一边：它只能看见
//! `macslim_lib` 里 `pub` 的东西。所以「公共面恰好等于 CLI 需要的 DTO 集合」
//! 这件事由本文件能否编译来强制：
//!
//! * 把 `ProcessKillReport` / `ProcessKillDetail` 挪回 `process_ops`（变成
//!   `pub(crate)`）→ 本文件立刻编译失败；
//! * 把 `ProcessTarget` / `AppIdentity` 重新暴露成 `pub` → 静态门禁
//!   `validate_process_boundary` 报错（`pub` 化会让外部 crate 有能力伪造
//!   可执行目标，必须同时被类型边界和门禁两侧夹住）。
//!
//! 本文件不含任何真实动作：只构造 DTO、读字段、断言序列化和默认值。
use macslim_lib::operations::{
    OperationKind, PreparedOperation, ProcessKillDetail, ProcessKillReport, ProcessMode,
};
use macslim_lib::user_error::UserError;

#[test]
fn public_process_report_dto_is_constructible_and_readable() {
    let mut report = ProcessKillReport::default();
    report.record(ProcessKillDetail {
        pid: 4242,
        name: "sleepy".to_owned(),
        success: true,
        message: UserError::from("已终止"),
    });
    report.record(ProcessKillDetail {
        pid: 4243,
        name: "stubborn".to_owned(),
        success: false,
        message: UserError::from("权限不足"),
    });

    assert_eq!(report.killed, vec![4242]);
    assert_eq!(report.failed, vec![4243]);
    assert_eq!(report.details.len(), 2);
    assert!(report.details[0].success);
    assert!(!report.details[1].success);
}

#[test]
fn public_prepared_operation_dto_keeps_the_execution_guard_fields() {
    let prepared = PreparedOperation {
        operation_id: "op-public-dto".to_owned(),
        kind: OperationKind::Process.label().to_owned(),
        expires_at_ms: 600_000,
        item_count: 2,
        estimated_bytes: 4_096,
        // 摘要是 i18n key + 插值参数，不是文案：公共面上也不给外部 crate
        // 塞中文的位置。
        summary_key: "opSummary.processGraceful".to_owned(),
        summary_params: vec![("count".to_owned(), "2".to_owned())],
    };

    assert_eq!(prepared.operation_id, "op-public-dto");
    assert_eq!(prepared.kind, "process");
    assert_eq!(prepared.expires_at_ms, 600_000);
    assert_eq!(prepared.item_count, 2);
    assert_eq!(prepared.estimated_bytes, 4_096);
    assert_eq!(prepared.summary_key, "opSummary.processGraceful");
    assert_eq!(
        prepared.summary_params,
        vec![("count".to_owned(), "2".to_owned())]
    );
}

#[test]
fn public_process_mode_stays_a_two_valued_typed_enum() {
    assert_ne!(ProcessMode::Graceful, ProcessMode::Force);
}

#[test]
fn public_operation_kind_labels_cover_every_broker_domain() {
    let labels: Vec<&str> = [
        OperationKind::Cache,
        OperationKind::Process,
        OperationKind::AppTerminate,
        OperationKind::AppGracefulQuit,
        OperationKind::Uninstall,
        OperationKind::Docker,
    ]
    .iter()
    .map(|kind| kind.label())
    .collect();

    assert_eq!(
        labels,
        vec![
            "cache",
            "process",
            "app_terminate",
            "app_graceful_quit",
            "uninstall",
            "docker"
        ]
    );
    assert!(OperationKind::Process.is_termination());
    assert!(OperationKind::AppTerminate.is_termination());
    assert!(!OperationKind::AppGracefulQuit.is_termination());
    assert!(!OperationKind::Uninstall.is_termination());
}
