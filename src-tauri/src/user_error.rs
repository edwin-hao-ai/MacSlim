//! 用户可见错误 / 结果消息的**结构化载体** —— `code` 与人类可读消息分离。
//!
//! ## 为什么不是 `Err(String)`
//!
//! 改造前全链路是 `Err("中文")`，前端 `src/lib/tauri.ts` 的
//! `classifyOperationError` **拿中文子串做分类**（`message.includes("请重新扫描")`）。
//! 这意味着**文案参与了控制流**：文案一改（改英文、改 key），分类就静默失效，
//! 所有错误退化成 `opError.failed`。
//!
//! 现在错误带一个稳定的纯 ASCII `code`（`snapshot_stale` / `permission_denied` …），
//! 前端按 `code` 比较、按 `error.<code>` 查译文，**不再匹配任何文案字符串**。
//!
//! ## 三条铁律
//!
//! 1. **`code` 与文案彻底分离**：`code` 是纯 ASCII 枚举常量，`message` 只是
//!    给人看的中文兜底（CLI 保留中文是既定取舍，见 `docs`）。
//! 2. **`code` 绝不参与安全判定**。白名单、风险等级、默认选中、selection key、
//!    TTL、单次消费、owner 绑定全部只看枚举 / 布尔 / 随机 hex。
//!    唯一特别小心的地方：不要把 `code` 塞进 `ProcessIdentity.name` ——
//!    `operation_executor::revalidate_targets` 会拿那个字段和现场 `proc.name()`
//!    逐字比较。
//! 3. **后端不另抄英文译文**。译文只存在于 `src/i18n/*.ts`。
//!
//! ## 为什么不直接用枚举
//!
//! `ErrorCode` 是「私有字段 + 关联常量」的新类型，不是 `enum`：
//!
//! - 错误码是**跨语言契约**，前端 `error-i18n.test.ts` 逐条比对字典集合；
//!   枚举的 `as_str()` 需要一份手写 `match`，多一份必然漂移的地方。
//! - 字段私有 ⇒ 除本模块外无法构造任意 `ErrorCode`；拼错只能拼错常量名，
//!   编译器立刻报「找不到关联项」，而不是静默发一个前端查不到的 code。
//! - `ALL` 是**唯一权威清单**，前端门禁与 Rust 源码层门禁都以它为准。

use crate::i18n_text::I18nParams;
use serde::Serialize;
use std::ops::Deref;

/// 稳定的错误 / 结果消息标识。线上格式就是它自己（snake_case 纯 ASCII）。
///
/// 字段私有：外部只能通过下面的关联常量取值。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ErrorCode(&'static str);

impl ErrorCode {
    // ===== broker：快照 / 操作生命周期 =====
    pub const SNAPSHOT_STALE: Self = Self("snapshot_stale");
    pub const SNAPSHOT_PAYLOAD_MISMATCH: Self = Self("snapshot_payload_mismatch");
    pub const SNAPSHOT_DEDICATED_ENTRY_REQUIRED: Self = Self("snapshot_dedicated_entry_required");
    pub const OPERATION_USED_OR_EXPIRED: Self = Self("operation_used_or_expired");
    pub const OPERATION_SNAPSHOT_STALE: Self = Self("operation_snapshot_stale");
    pub const OPERATION_TIME_INVALID: Self = Self("operation_time_invalid");
    pub const OPERATION_OWNER_MISMATCH: Self = Self("operation_owner_mismatch");
    pub const OPERATION_OWNER_EMPTY: Self = Self("operation_owner_empty");
    pub const OPERATION_MISSING_SNAPSHOT: Self = Self("operation_missing_snapshot");
    pub const OPERATION_ID_MISMATCH: Self = Self("operation_id_mismatch");
    pub const OPERATION_LOCK_BROKEN: Self = Self("operation_lock_broken");
    pub const SELECTION_EMPTY: Self = Self("selection_empty");
    pub const SELECTION_DUPLICATED: Self = Self("selection_duplicated");
    pub const SELECTION_MISSING: Self = Self("selection_missing");
    pub const RANDOM_ID_FAILED: Self = Self("random_id_failed");
    pub const SELECTION_KEY_GENERATION_FAILED: Self = Self("selection_key_generation_failed");
    pub const PLAN_KIND_MISMATCH: Self = Self("plan_kind_mismatch");

    // ===== 进程终止 =====
    pub const WHITELISTED_PROCESS_CANNOT_TERMINATE: Self =
        Self("whitelisted_process_cannot_terminate");
    pub const WHITELISTED_APP_NOT_QUIT: Self = Self("whitelisted_app_not_quit");
    pub const PROTECTED_FORCE_ONLY: Self = Self("protected_force_only");
    pub const NO_TERMINABLE_PROCESS_TARGETS: Self = Self("no_terminable_process_targets");
    pub const NO_QUITTABLE_APP_TARGETS: Self = Self("no_quittable_app_targets");
    pub const PROCESS_GONE: Self = Self("process_gone");
    pub const PROCESS_PID_REUSED: Self = Self("process_pid_reused");
    pub const PROCESS_IDENTITY_CHANGED: Self = Self("process_identity_changed");
    pub const PROCESS_PROTECTION_CHANGED: Self = Self("process_protection_changed");
    pub const PROCESS_PLAN_REQUIRES_BLOCKING: Self = Self("process_plan_requires_blocking");
    pub const BLOCKING_CHANNEL_PLAN_MISMATCH: Self = Self("blocking_channel_plan_mismatch");
    pub const PROCESS_EXECUTION_FAILED: Self = Self("process_execution_failed");
    pub const KILL_TERMINATED: Self = Self("kill_terminated");
    pub const KILL_ALREADY_GONE: Self = Self("kill_already_gone");
    pub const KILL_PERMISSION_DENIED: Self = Self("kill_permission_denied");
    pub const KILL_RESPAWNED: Self = Self("kill_respawned");
    pub const KILL_STILL_ALIVE: Self = Self("kill_still_alive");
    pub const KILL_FAILED: Self = Self("kill_failed");

    // ===== 应用 / 残留 / 卸载 =====
    pub const APP_GONE: Self = Self("app_gone");
    pub const APP_NAME_CHANGED: Self = Self("app_name_changed");
    pub const APP_BUNDLE_ID_CHANGED: Self = Self("app_bundle_id_changed");
    pub const APP_CHILD_NOT_IN_APP: Self = Self("app_child_not_in_app");
    pub const APP_NO_TERMINABLE_PROCESS: Self = Self("app_no_terminable_process");
    pub const APP_NO_QUITTABLE_PROCESS: Self = Self("app_no_quittable_process");
    pub const NO_UNINSTALLABLE_APP_TARGETS: Self = Self("no_uninstallable_app_targets");
    pub const APP_CHILD_SELECTION_MISMATCH: Self = Self("app_child_selection_mismatch");
    pub const APP_SELECTION_COUNT_MISMATCH: Self = Self("app_selection_count_mismatch");
    pub const APP_SELECTED_TWICE: Self = Self("app_selected_twice");
    pub const APP_AMBIGUOUS_NAME: Self = Self("app_ambiguous_name");
    pub const SYSTEM_APP_CANNOT_UNINSTALL: Self = Self("system_app_cannot_uninstall");
    pub const RESIDUE_BATCH_EMPTY: Self = Self("residue_batch_empty");
    pub const RESIDUE_BATCH_MISSING_APP_KEY: Self = Self("residue_batch_missing_app_key");
    pub const RESIDUE_SNAPSHOT_EMPTY: Self = Self("residue_snapshot_empty");
    pub const RESIDUE_UNKNOWN_APP_KEY: Self = Self("residue_unknown_app_key");
    pub const RESIDUE_SELECTION_COUNT_MISMATCH: Self = Self("residue_selection_count_mismatch");
    pub const RESIDUE_NOT_IN_APP: Self = Self("residue_not_in_app");
    pub const RESIDUE_NOT_IN_SELECTED_APP: Self = Self("residue_not_in_selected_app");
    pub const RESIDUE_PATH_DUPLICATED: Self = Self("residue_path_duplicated");
    pub const RESIDUE_RECHECK_COUNT_MISMATCH: Self = Self("residue_recheck_count_mismatch");
    pub const RESIDUE_GONE: Self = Self("residue_gone");
    pub const RESIDUE_PATH_CHANGED: Self = Self("residue_path_changed");
    pub const RESIDUE_PATH_OUT_OF_SCOPE: Self = Self("residue_path_out_of_scope");
    pub const RESIDUE_SCAN_FAILED: Self = Self("residue_scan_failed");
    pub const MOVE_TO_TRASH_FAILED: Self = Self("move_to_trash_failed");

    // ===== 构建形态差异 =====
    /// Mac App Store 版不支持终止进程。
    ///
    /// App Sandbox 下沙箱进程不能给其他进程发信号，且**没有任何 entitlement
    /// 能放行**这一项。所以 MAS flavor 在进入 signaller 之前就直接拒绝，
    /// 而不是让 `kill(2)` 撞 EPERM 抛一个「权限不足」—— 后者会让用户以为
    /// 是自己的系统设置有问题，而真实原因是这个构建形态压根没有该能力。
    ///
    /// 文案只在前端词典里（`error.process_termination_unsupported`），
    /// Rust 侧不抄译文。
    pub const PROCESS_TERMINATION_UNSUPPORTED: Self = Self("process_termination_unsupported");

    // ===== 缓存清理 =====
    pub const CACHE_SELECTION_COUNT_MISMATCH: Self = Self("cache_selection_count_mismatch");
    pub const CACHE_BUSY_APP_SKIPPED: Self = Self("cache_busy_app_skipped");
    pub const CACHE_DOCKER_ACTION_MISMATCH: Self = Self("cache_docker_action_mismatch");
    pub const CACHE_ITEM_MISSING_PATH: Self = Self("cache_item_missing_path");
    pub const CACHE_OWNERSHIP_CHANGED: Self = Self("cache_ownership_changed");
    pub const CACHE_ANCESTOR_CHECK_FAILED: Self = Self("cache_ancestor_check_failed");
    pub const PATH_NOT_WHITELISTED: Self = Self("path_not_whitelisted");
    pub const PRE_DELETE_RECHECK_FAILED: Self = Self("pre_delete_recheck_failed");
    pub const DELETE_FAILED: Self = Self("delete_failed");
    pub const AUTHORIZATION_CANCELLED: Self = Self("authorization_cancelled");
    pub const ADMIN_PRIVILEGES_REQUIRED: Self = Self("admin_privileges_required");
    pub const REFUSE_SYMLINK_ANCESTOR: Self = Self("refuse_symlink_ancestor");
    pub const REFUSE_SYMLINK: Self = Self("refuse_symlink");
    pub const STALE_PATH_NOT_DIR: Self = Self("stale_path_not_dir");
    pub const STALE_MISSING_CANONICAL: Self = Self("stale_missing_canonical");
    pub const STALE_PATH_CHANGED: Self = Self("stale_path_changed");
    pub const STALE_PATH_KIND_INVALID: Self = Self("stale_path_kind_invalid");
    pub const STALE_PATH_NOT_NODE_MODULES: Self = Self("stale_path_not_node_modules");
    pub const STALE_PATH_MISSING_PARENT: Self = Self("stale_path_missing_parent");
    pub const STALE_PATH_OUTSIDE_PROJECTS: Self = Self("stale_path_outside_projects");
    pub const STALE_MISSING_OWNERSHIP: Self = Self("stale_missing_ownership");
    pub const CACHE_COMMAND_FAILED: Self = Self("cache_command_failed");
    pub const CACHE_COMMAND_EXIT_NONZERO: Self = Self("cache_command_exit_nonzero");
    pub const CACHE_PIP_BROKEN: Self = Self("cache_pip_broken");

    // ===== Docker =====
    pub const DOCKER_NOT_RUNNING: Self = Self("docker_not_running");
    pub const NO_DOCKER_TARGETS: Self = Self("no_docker_targets");
    pub const DOCKER_INVENTORY_CHANGED: Self = Self("docker_inventory_changed");
    pub const DOCKER_RESOURCE_GONE: Self = Self("docker_resource_gone");
    pub const DOCKER_ID_REUSED: Self = Self("docker_id_reused");
    pub const DOCKER_REFERENCED_CHANGED: Self = Self("docker_referenced_changed");
    pub const DOCKER_SELECTION_COUNT_MISMATCH: Self = Self("docker_selection_count_mismatch");
    pub const DOCKER_SELECTION_TYPE_MISMATCH: Self = Self("docker_selection_type_mismatch");
    pub const DOCKER_PRUNE_REJECTS_SELECTION: Self = Self("docker_prune_rejects_selection");
    pub const DOCKER_PRUNE_REJECTS_TARGET: Self = Self("docker_prune_rejects_target");
    pub const DOCKER_CLI_FAILED: Self = Self("docker_cli_failed");

    // ===== 历史 / 结果形状 =====
    pub const HISTORY_WRITE_FAILED: Self = Self("history_write_failed");
    pub const HISTORY_STORAGE_UNAVAILABLE: Self = Self("history_storage_unavailable");
    pub const RESULT_NOT_CACHE_SUMMARY: Self = Self("result_not_cache_summary");
    pub const RESULT_NOT_KILL_REPORT: Self = Self("result_not_kill_report");

    // ===== 兜底 =====
    /// 任何 `From<String>` / `From<&str>` 落进来的错误。
    /// 刻意**不在** i18n 词典里：正常流程不该出现，出现即「有没分类的内部错误」。
    pub const INTERNAL: Self = Self("internal");

    /// 权威清单：前端门禁与 Rust 源码层门禁都以它为准。
    ///
    /// ⚠️ 新增 code 必须同时（1）在这里追加、（2）在两份词典里加 `error.<code>`、
    /// （3）在生产代码里真的构造它 —— 三者由 `user_error_tests.rs` 逐条比对卡死。
    pub const ALL: &'static [ErrorCode] = &[
        Self::ADMIN_PRIVILEGES_REQUIRED,
        Self::APP_AMBIGUOUS_NAME,
        Self::APP_BUNDLE_ID_CHANGED,
        Self::APP_CHILD_NOT_IN_APP,
        Self::APP_CHILD_SELECTION_MISMATCH,
        Self::APP_GONE,
        Self::APP_NAME_CHANGED,
        Self::APP_NO_QUITTABLE_PROCESS,
        Self::APP_NO_TERMINABLE_PROCESS,
        Self::APP_SELECTED_TWICE,
        Self::APP_SELECTION_COUNT_MISMATCH,
        Self::AUTHORIZATION_CANCELLED,
        Self::BLOCKING_CHANNEL_PLAN_MISMATCH,
        Self::CACHE_ANCESTOR_CHECK_FAILED,
        Self::CACHE_BUSY_APP_SKIPPED,
        Self::CACHE_COMMAND_EXIT_NONZERO,
        Self::CACHE_COMMAND_FAILED,
        Self::CACHE_DOCKER_ACTION_MISMATCH,
        Self::CACHE_ITEM_MISSING_PATH,
        Self::CACHE_OWNERSHIP_CHANGED,
        Self::CACHE_PIP_BROKEN,
        Self::CACHE_SELECTION_COUNT_MISMATCH,
        Self::DELETE_FAILED,
        Self::DOCKER_CLI_FAILED,
        Self::DOCKER_ID_REUSED,
        Self::DOCKER_INVENTORY_CHANGED,
        Self::DOCKER_NOT_RUNNING,
        Self::DOCKER_PRUNE_REJECTS_SELECTION,
        Self::DOCKER_PRUNE_REJECTS_TARGET,
        Self::DOCKER_REFERENCED_CHANGED,
        Self::DOCKER_RESOURCE_GONE,
        Self::DOCKER_SELECTION_COUNT_MISMATCH,
        Self::DOCKER_SELECTION_TYPE_MISMATCH,
        Self::HISTORY_STORAGE_UNAVAILABLE,
        Self::HISTORY_WRITE_FAILED,
        Self::KILL_ALREADY_GONE,
        Self::KILL_FAILED,
        Self::KILL_PERMISSION_DENIED,
        Self::KILL_RESPAWNED,
        Self::KILL_STILL_ALIVE,
        Self::KILL_TERMINATED,
        Self::MOVE_TO_TRASH_FAILED,
        Self::PROCESS_TERMINATION_UNSUPPORTED,
        Self::NO_DOCKER_TARGETS,
        Self::NO_QUITTABLE_APP_TARGETS,
        Self::NO_TERMINABLE_PROCESS_TARGETS,
        Self::NO_UNINSTALLABLE_APP_TARGETS,
        Self::OPERATION_ID_MISMATCH,
        Self::OPERATION_LOCK_BROKEN,
        Self::OPERATION_MISSING_SNAPSHOT,
        Self::OPERATION_OWNER_EMPTY,
        Self::OPERATION_OWNER_MISMATCH,
        Self::OPERATION_SNAPSHOT_STALE,
        Self::OPERATION_TIME_INVALID,
        Self::OPERATION_USED_OR_EXPIRED,
        Self::PATH_NOT_WHITELISTED,
        Self::PLAN_KIND_MISMATCH,
        Self::PRE_DELETE_RECHECK_FAILED,
        Self::PROCESS_EXECUTION_FAILED,
        Self::PROCESS_GONE,
        Self::PROCESS_IDENTITY_CHANGED,
        Self::PROCESS_PID_REUSED,
        Self::PROCESS_PLAN_REQUIRES_BLOCKING,
        Self::PROCESS_PROTECTION_CHANGED,
        Self::PROTECTED_FORCE_ONLY,
        Self::RANDOM_ID_FAILED,
        Self::REFUSE_SYMLINK,
        Self::REFUSE_SYMLINK_ANCESTOR,
        Self::RESIDUE_BATCH_EMPTY,
        Self::RESIDUE_BATCH_MISSING_APP_KEY,
        Self::RESIDUE_GONE,
        Self::RESIDUE_NOT_IN_APP,
        Self::RESIDUE_NOT_IN_SELECTED_APP,
        Self::RESIDUE_PATH_CHANGED,
        Self::RESIDUE_PATH_DUPLICATED,
        Self::RESIDUE_PATH_OUT_OF_SCOPE,
        Self::RESIDUE_RECHECK_COUNT_MISMATCH,
        Self::RESIDUE_SCAN_FAILED,
        Self::RESIDUE_SELECTION_COUNT_MISMATCH,
        Self::RESIDUE_SNAPSHOT_EMPTY,
        Self::RESIDUE_UNKNOWN_APP_KEY,
        Self::RESULT_NOT_CACHE_SUMMARY,
        Self::RESULT_NOT_KILL_REPORT,
        Self::SELECTION_DUPLICATED,
        Self::SELECTION_EMPTY,
        Self::SELECTION_KEY_GENERATION_FAILED,
        Self::SELECTION_MISSING,
        Self::SNAPSHOT_DEDICATED_ENTRY_REQUIRED,
        Self::SNAPSHOT_PAYLOAD_MISMATCH,
        Self::SNAPSHOT_STALE,
        Self::STALE_MISSING_CANONICAL,
        Self::STALE_MISSING_OWNERSHIP,
        Self::STALE_PATH_CHANGED,
        Self::STALE_PATH_KIND_INVALID,
        Self::STALE_PATH_MISSING_PARENT,
        Self::STALE_PATH_NOT_DIR,
        Self::STALE_PATH_NOT_NODE_MODULES,
        Self::STALE_PATH_OUTSIDE_PROJECTS,
        Self::SYSTEM_APP_CANNOT_UNINSTALL,
        Self::WHITELISTED_APP_NOT_QUIT,
        Self::WHITELISTED_PROCESS_CANNOT_TERMINATE,
    ];

    /// 线上格式 / 字典 key 的后半段。
    pub fn as_str(&self) -> &'static str {
        self.0
    }

    /// i18n 词条路径：前端 `t("error.<code>")`。
    pub fn i18n_key(&self) -> String {
        format!("error.{}", self.as_str())
    }

    #[cfg(test)]
    /// 仅测试用：模拟「前端传回一个 `ALL` 之外的 code」。
    ///
    /// 用 `Box::leak` 造一个 `&'static str`，是为了让「字典里没有这枚 code 时
    /// 前端必须回落到中文兜底」这条前端行为在 Rust 侧也能被构造出来测。
    pub(crate) fn from_wire(raw: &str) -> Self {
        Self(Box::leak(raw.to_owned().into_boxed_str()))
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 用户可见错误 / 结果消息：`code` 供前端分类与查表，`message` 是中文兜底。
///
/// 序列化形状（前端 `src/lib/tauri.ts` 的 `TauriError` 逐字对应）：
///
/// ```json
/// { "code": "process_gone", "message": "进程已不存在（PID 123），请重新扫描",
///   "params": [["pid", "123"]] }
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UserError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "I18nParams::is_empty")]
    pub params: I18nParams,
}

impl UserError {
    /// 无插值参数。
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            params: Vec::new(),
        }
    }

    /// 带插值参数。参数值是**要显示的内容本身**（PID、路径、命令 stderr），
    /// 不是文案 —— 译文里对应位置是占位符。
    pub fn with(code: ErrorCode, message: impl Into<String>, params: I18nParams) -> Self {
        Self {
            code,
            message: message.into(),
            params,
        }
    }

    /// 单参数快捷构造。
    pub fn one(
        code: ErrorCode,
        message: impl Into<String>,
        name: &str,
        value: impl ToString,
    ) -> Self {
        Self::with(code, message, vec![(name.to_owned(), value.to_string())])
    }
}

impl Deref for UserError {
    type Target = str;

    /// 让 `error.contains("中文")` 这类既有断言继续成立。
    ///
    /// 这是**有意保留的兼容层**：改造前的测试断言里有约 30 处拿中文子串比对错误
    /// 消息，它们是「文案没变」的证据（行为等价性），不该跟着改造一起删掉。
    /// 但 `Deref` 只暴露展示层语义，**不能**用于分支判断 —— 想判断必须比 `code`。
    fn deref(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for UserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// 与裸字符串比较的**测试兼容层**。
///
/// 改造前约 30 处 `assert_eq!(err, "中文…")` / `assert!(err.contains("中文"))`
/// 断言的是错误消息文本。它们同时是「改造后文案一字未改」的行为等价性证据，
/// 所以保留原样（见本文件顶部与 `Deref` 的说明）。
///
/// ⚠️ 这组 impl 只用于断言。**生产代码不得用它做分支判断** —— 要判断必须比
/// `code`，否则又会回到「文案参与控制流」的老坑。
impl PartialEq<str> for UserError {
    fn eq(&self, other: &str) -> bool {
        self.message == other
    }
}

impl PartialEq<&str> for UserError {
    fn eq(&self, other: &&str) -> bool {
        self.message == *other
    }
}

impl PartialEq<String> for UserError {
    fn eq(&self, other: &String) -> bool {
        self.message == *other
    }
}

impl PartialEq<UserError> for str {
    fn eq(&self, other: &UserError) -> bool {
        self == other.message
    }
}

impl PartialEq<UserError> for &str {
    fn eq(&self, other: &UserError) -> bool {
        *self == other.message
    }
}

impl PartialEq<UserError> for String {
    fn eq(&self, other: &UserError) -> bool {
        *self == other.message
    }
}

impl std::error::Error for UserError {}

impl From<String> for UserError {
    fn from(message: String) -> Self {
        Self::new(ErrorCode::INTERNAL, message)
    }
}

impl From<&str> for UserError {
    fn from(message: &str) -> Self {
        Self::new(ErrorCode::INTERNAL, message)
    }
}

impl From<UserError> for String {
    fn from(error: UserError) -> Self {
        error.message
    }
}

#[cfg(test)]
#[path = "user_error_tests.rs"]
mod tests;
