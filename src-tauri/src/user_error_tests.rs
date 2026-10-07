//! `ErrorCode` 的源码层门禁。
//!
//! 这些断言的强度参照第 1～3 片踩过的坑：**「数量对得上」不等于「集合对得上」**。
//! 只要 code 是从源码里按 `ErrorCode::` 前缀抽出来的，某处把命名空间/常量名
//! 写错，它就会从抽取结果里悄悄消失，而「总数 ≥ N」这种断言照样绿。
//! 所以这里全部用**逐条集合比对**，并且逐文件都比。

use super::*;

/// 本文件自身 + `*_tests.rs` + `operation_commands_fakes.rs`（被
/// `operation_commands_tests.rs` 以 `#[path]` 挂进来的测试支撑模块）都不是
/// 生产代码，跳过。
const NON_PRODUCTION: &[&str] = &["user_error.rs", "user_error_tests.rs"];

fn is_test_support(name: &str) -> bool {
    name.ends_with("_tests.rs") || name == "operation_commands_fakes.rs"
}

/// 切掉内联测试模块。
///
/// 不能简单地在**第一个** `#[cfg(test)]` 处截断：`process_ops.rs:35` 那种
/// `#[cfg(test)] pub(crate) fn with_default_policy()` 标在生产函数上，
/// 截在那里会连带丢掉后面全部 `ErrorCode::` 引用。只在 `#[cfg(test)]` 后面
/// 跟着 `mod` / `#[path]` 时才认为测试模块开始。
fn production_source(source: &str) -> &str {
    let mut cut = source.len();
    let mut cursor = 0usize;
    while let Some(at) = source[cursor..].find("#[cfg(test)]") {
        let at = cursor + at;
        let rest = source[at + "#[cfg(test)]".len()..].trim_start_matches('\n');
        if rest.starts_with("mod ") || rest.starts_with("#[path") {
            cut = cut.min(at);
            break;
        }
        cursor = at + 1;
    }
    &source[..cut]
}

fn const_to_wire(source: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = source;
    while let Some(at) = rest.find("pub const ") {
        let tail = &rest[at + "pub const ".len()..];
        let Some(end) = tail.find(";\n") else { break };
        let decl = &tail[..end];
        let name = decl.split(':').next().unwrap_or("").trim().to_owned();
        let Some(open) = decl.find("Self(\"") else {
            rest = &tail[end + 2..];
            continue;
        };
        let close = decl[open + "Self(\"".len()..]
            .find('"')
            .expect("Self(\" 必须闭合");
        out.push((
            name,
            decl[open + "Self(\"".len()..open + "Self(\"".len() + close].to_owned(),
        ));
        rest = &tail[end + 2..];
    }
    out
}

/// 读 `src/<file>` 的源码（相对 crate 根）。
fn read_source(file: &str) -> String {
    let path = format!("{}/src/{file}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("读不到 {path}: {error}"))
}

/// 抽一个文件里「生产代码真正构造的 code 集合」（wire 名，已去重排序）。
fn codes_used_in(file: &str) -> Vec<String> {
    let source_owned = read_source(file);
    let source = source_owned.as_str();
    let mut out = Vec::new();
    let mut rest = production_source(source);
    while let Some(at) = rest.find("ErrorCode::") {
        let tail = &rest[at + "ErrorCode::".len()..];
        let name: String = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        out.push(name.clone());
        rest = &tail[name.len()..];
    }
    // 常量名 → wire 名
    let declared = read_source("user_error.rs");
    let owned = const_to_wire(&declared);
    let map: std::collections::HashMap<&str, &str> = owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut wires: Vec<String> = out
        .into_iter()
        .map(|name| {
            map.get(name.as_str())
                .unwrap_or_else(|| panic!("{file} 用了不存在的 ErrorCode 常量 {name}"))
                .to_string()
        })
        .collect();
    wires.sort();
    wires.dedup();
    wires
}

fn all_wire_names() -> Vec<String> {
    let mut names: Vec<String> = ErrorCode::ALL
        .iter()
        .map(|c| c.as_str().to_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn every_wire_name_is_pure_ascii_snake_case() {
    for code in ErrorCode::ALL {
        let wire = code.as_str();
        assert!(
            wire.is_ascii(),
            "错误码必须是纯 ASCII（中文界面之外的机种也读得到）：{wire:?}"
        );
        assert_eq!(
            wire,
            wire.to_ascii_lowercase(),
            "错误码必须是小写 snake_case（前端直接拿它当词典 key）：{wire:?}"
        );
        assert!(
            wire.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "错误码只允许小写字母、数字与下划线：{wire:?}"
        );
        assert!(!wire.starts_with('_') && !wire.ends_with('_'), "{wire:?}");
    }
}

/// 命名纪律：`pub const SNAPSHOT_STALE` 的 wire 名必须正好是 `snapshot_stale`。
///
/// 这条是给「手写两遍」兜底的：`ErrorCode` 是「常量 + 字符串」两份东西，
/// 少写一个就是线上多一个前端查不到的 code。派生出 wire 名就不必手写第二遍。
#[test]
fn const_name_lowercase_equals_wire_name() {
    let declared = read_source("user_error.rs");
    for (name, wire) in const_to_wire(&declared) {
        assert_eq!(
            name.to_ascii_lowercase(),
            wire,
            "常量名 {name} 与 wire 名 {wire} 不一致（前端按 wire 名查 error.<code>）"
        );
    }
}

/// `ALL` 必须去重且按 wire 名排序 —— 前端门禁按 `ALL` 求差集，重复项会掩盖漏译。
#[test]
fn all_is_sorted_and_deduped() {
    let names = all_wire_names();
    let mut deduped = names.clone();
    deduped.dedup();
    assert_eq!(names.len(), deduped.len(), "ALL 里有重复项");
    assert!(
        names.windows(2).all(|w| w[0] < w[1]),
        "ALL 必须按 wire 名排序"
    );
}

/// **`ALL` 恰好等于生产代码构造出来的全部 code**（逐条集合比对，不比数量）。
///
/// `INTERNAL` 刻意不在 `ALL` 里：它是 `From<String>` 的兜底，不是「前端需要
/// 按 code 分类或查译文」的那一批。
#[test]
fn all_equals_the_codes_constructed_in_production_code() {
    let mut used: Vec<String> = Vec::new();
    for entry in
        std::fs::read_dir(format!("{}/src", env!("CARGO_MANIFEST_DIR"))).expect("能读 src 目录")
    {
        let path = entry.expect("能读目录项").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("文件名是 UTF-8")
            .to_owned();
        if NON_PRODUCTION.contains(&name.as_str()) || is_test_support(&name) {
            continue;
        }
        used.extend(codes_used_in(&name));
    }
    used.retain(|wire| wire != ErrorCode::INTERNAL.as_str());
    used.sort();
    used.dedup();

    assert_eq!(
        used,
        all_wire_names(),
        "生产代码构造的 code 集合与 ErrorCode::ALL 不一致。\n\
         新增 code 必须三件事同时做：(1) 追加到 ALL；(2) 在生产代码里真的构造它；\n\
         (3) 在 src/i18n/en.ts 与 zh-CN.ts 各加一条 error.<code>。\n\
         多出来的 code 前端永远查不到译文；少掉的 code 前端只能显示裸 code。"
    );
}

/// 逐文件集合比对：某个文件里多/少了一枚 code 立刻红。
///
/// 上一轮第 2 片的教训（ledger §5 变异 7）：`process_safety.rs` 与
/// `operation_commands.rs` 有一枚同名 key，「总数 ≥ 4」这种断言两边都满足，
/// 门禁形同虚设。这里对**每个**构造 code 的文件都写死精确集合。
#[test]
fn each_file_constructs_exactly_its_declared_codes() {
    let expected: &[(&str, &[&str])] = &[
        ("app_scanner.rs", &["app_selection_count_mismatch"]),
        ("applications.rs", &["app_child_selection_mismatch"]),
        (
            "cache_cleaner.rs",
            &[
                "admin_privileges_required",
                "authorization_cancelled",
                "cache_ancestor_check_failed",
                "cache_busy_app_skipped",
                "cache_command_exit_nonzero",
                "cache_command_failed",
                "cache_docker_action_mismatch",
                "cache_item_missing_path",
                "cache_ownership_changed",
                "cache_pip_broken",
                "delete_failed",
                "path_not_whitelisted",
                "pre_delete_recheck_failed",
                "refuse_symlink",
                "refuse_symlink_ancestor",
                "stale_missing_canonical",
                "stale_missing_ownership",
                "stale_path_changed",
                "stale_path_kind_invalid",
                "stale_path_missing_parent",
                "stale_path_not_dir",
                "stale_path_not_node_modules",
                "stale_path_outside_projects",
            ],
        ),
        (
            "cli_operations.rs",
            &[
                "history_storage_unavailable",
                "result_not_cache_summary",
                "result_not_kill_report",
            ],
        ),
        (
            "docker.rs",
            &[
                "docker_cli_failed",
                "docker_prune_rejects_target",
                "docker_selection_count_mismatch",
            ],
        ),
        (
            "lib.rs",
            // `process_termination_unsupported` 只在 `--features mas` 下真正被
            // 构造（默认形态那段被 cfg 掉了），但清单是形态无关的静态断言，
            // 所以照样要列在这里。
            &[
                // 文件夹访问授权（App Store 版读用户目录的唯一合规入口）。
                // 三条都对应 grant_folder_access / revoke_folder_access 的失败分支。
                "folder_access_bookmark_failed",
                "folder_access_panel_failed",
                "folder_access_store_failed",
                "process_execution_failed",
                "process_termination_unsupported",
                "residue_scan_failed",
            ],
        ),
        (
            "operation_commands.rs",
            &[
                "cache_selection_count_mismatch",
                "history_write_failed",
                "operation_id_mismatch",
                "operation_lock_broken",
                // 新增：`rejection_entry` 现在把「执行前复核未通过」的错误码
                // 一并写进历史（reason_code），供界面本地化渲染失败原因。
                "pre_delete_recheck_failed",
                "residue_batch_empty",
            ],
        ),
        (
            "operation_executor.rs",
            &[
                "app_bundle_id_changed",
                "app_child_not_in_app",
                "app_gone",
                "app_name_changed",
                "app_no_terminable_process",
                "blocking_channel_plan_mismatch",
                "docker_id_reused",
                "docker_inventory_changed",
                "docker_not_running",
                "docker_referenced_changed",
                "docker_resource_gone",
                "no_docker_targets",
                "no_quittable_app_targets",
                "no_terminable_process_targets",
                "no_uninstallable_app_targets",
                "plan_kind_mismatch",
                "process_gone",
                "process_identity_changed",
                "process_pid_reused",
                "process_plan_requires_blocking",
                "process_protection_changed",
                "residue_gone",
                "residue_not_in_app",
                "residue_path_changed",
                "residue_path_duplicated",
                "residue_recheck_count_mismatch",
            ],
        ),
        (
            "operation_registry.rs",
            &[
                "app_no_quittable_process",
                "protected_force_only",
                "random_id_failed",
                "selection_duplicated",
                "selection_empty",
                "selection_key_generation_failed",
                "selection_missing",
                "whitelisted_app_not_quit",
                "whitelisted_process_cannot_terminate",
            ],
        ),
        (
            "operations.rs",
            &[
                "operation_missing_snapshot",
                "operation_owner_empty",
                "operation_owner_mismatch",
                "operation_snapshot_stale",
                "operation_time_invalid",
                "operation_used_or_expired",
                "residue_batch_missing_app_key",
                "residue_snapshot_empty",
                "residue_unknown_app_key",
                "selection_missing",
                "snapshot_dedicated_entry_required",
                "snapshot_payload_mismatch",
                "snapshot_stale",
            ],
        ),
        (
            "operations_prepare.rs",
            &[
                "app_ambiguous_name",
                "app_selected_twice",
                "docker_prune_rejects_selection",
                "docker_selection_type_mismatch",
                // 兜底 code：GRACEFUL_QUIT_ROUTE 这类「内部不变量」提示，
                // 正常流程点不到 UI，所以不进 ALL，但确实在这里构造。
                "internal",
                "residue_not_in_selected_app",
                "residue_path_duplicated",
                // 「应用选择项不存在」与 operations.rs 的「选择项不存在」共用 code：
                // 两条旧文案都含「选择项不存在」，旧分类同为 stale。
                "selection_missing",
                "snapshot_payload_mismatch",
                "snapshot_stale",
                "system_app_cannot_uninstall",
            ],
        ),
        (
            "process_ops.rs",
            &[
                "kill_already_gone",
                "kill_failed",
                "kill_permission_denied",
                "kill_respawned",
                "kill_still_alive",
                "kill_terminated",
            ],
        ),
        ("residue_policy.rs", &["residue_path_out_of_scope"]),
        (
            "residue_scanner.rs",
            &["residue_batch_empty", "residue_selection_count_mismatch"],
        ),
        (
            "uninstaller.rs",
            &["authorization_cancelled", "move_to_trash_failed"],
        ),
    ];

    for (file, wanted) in expected {
        let mut wanted: Vec<String> = wanted.iter().map(|s| (*s).to_owned()).collect();
        wanted.sort();
        assert_eq!(
            codes_used_in(file),
            wanted,
            "{file} 构造的 code 集合与门禁写死的清单不一致。\
             改了这里必须同时更新本清单（这是刻意的摩擦：\
             「多了一枚 code」和「少了一枚 code」都要有人明确决定）。"
        );
    }
}

/// i18n 词条路径就是 `error.<wire 名>`，前端据此查表。
///
/// wire 名保持 snake_case（不转成 camelCase）是有意的：i18n 的 `get()` 走
/// `path.split(".")` 取对象属性，下划线是合法属性名，于是后端与前端之间
/// 不需要任何「snake → camel」转换函数 —— 少一个转换就少一处能出错的地方。
#[test]
fn i18n_key_is_the_error_namespace_plus_the_wire_name() {
    assert_eq!(ErrorCode::SNAPSHOT_STALE.i18n_key(), "error.snapshot_stale");
    assert_eq!(
        ErrorCode::HISTORY_WRITE_FAILED.i18n_key(),
        "error.history_write_failed"
    );
    for code in ErrorCode::ALL {
        assert_eq!(code.i18n_key(), format!("error.{}", code.as_str()));
    }
}

/// 兜底 code 刻意不在 `ALL` 里：前端遇到它必须回落到中文消息，而不是显示
/// 一个查不到译文的裸 code。
#[test]
fn internal_code_is_absent_from_the_translated_set() {
    assert!(!ErrorCode::ALL.contains(&ErrorCode::INTERNAL));
    assert!(!all_wire_names().contains(&ErrorCode::INTERNAL.as_str().to_owned()));
}

/// 序列化形状必须与前端 `src/lib/tauri.ts` 的 `TauriError` 逐字对应。
#[test]
fn serialized_shape_matches_the_frontend_contract() {
    let plain = UserError::new(ErrorCode::SNAPSHOT_STALE, "快照不存在或已失效");
    assert_eq!(
        serde_json::to_value(&plain).expect("可序列化"),
        serde_json::json!({ "code": "snapshot_stale", "message": "快照不存在或已失效" })
    );

    let with_params = UserError::with(
        ErrorCode::PROCESS_GONE,
        "进程已不存在（PID 42），请重新扫描",
        vec![("pid".to_owned(), "42".to_owned())],
    );
    assert_eq!(
        serde_json::to_value(&with_params).expect("可序列化"),
        serde_json::json!({
            "code": "process_gone",
            "message": "进程已不存在（PID 42），请重新扫描",
            "params": [["pid", "42"]],
        })
    );
}

/// `From<String>` / `From<&str>` 的兜底语义：消息一字不改，code 落到 `INTERNAL`。
#[test]
fn string_conversions_fall_back_to_internal_without_touching_the_message() {
    let from_string: UserError = "某个第三方错误".to_owned().into();
    assert_eq!(from_string.code, ErrorCode::INTERNAL);
    assert_eq!(from_string.message, "某个第三方错误");
    assert!(from_string.params.is_empty());

    let from_str: UserError = "另一个错误".into();
    assert_eq!(from_str.code, ErrorCode::INTERNAL);
    assert_eq!(from_str.message, "另一个错误");

    let back: String = from_str.into();
    assert_eq!(back, "另一个错误");
}

/// 插值参数名必须是纯 ASCII —— 参数名是 `error.<code>` 词条里的占位符名。
///
/// 只看**参数位**：`UserError::one(code, message, "name", value)` 的第 3 个实参，
/// `UserError::with(code, message, vec![("name", …), …])` 里每个元组的第 1 个。
/// 消息位（`"进程已不存在（PID {pid}），请重新扫描"`）里当然可以有中文 ——
/// 它是给 CLI 看的中文兜底，不是占位符名。
#[test]
fn param_names_are_pure_ascii() {
    let mut checked = 0usize;
    for entry in
        std::fs::read_dir(format!("{}/src", env!("CARGO_MANIFEST_DIR"))).expect("能读 src 目录")
    {
        let path = entry.expect("能读目录项").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("文件名是 UTF-8")
            .to_owned();
        if NON_PRODUCTION.contains(&name.as_str()) || is_test_support(&name) {
            continue;
        }
        let owned = read_source(&name);
        let source = production_source(&owned);
        let mut cursor = 0usize;
        while let Some(at) = source[cursor..].find("UserError::") {
            let tail = &source[cursor + at + "UserError::".len()..];
            cursor += at + "UserError::".len();
            let (ctor, rest) = if let Some(rest) = tail.strip_prefix("one(") {
                ("one", rest)
            } else if let Some(rest) = tail.strip_prefix("with(") {
                ("with", rest)
            } else {
                continue;
            };
            let args = split_top_level_args(rest);
            if args.len() < 3 {
                continue;
            }
            let param_slot = if ctor == "one" {
                vec![args[2].clone()]
            } else {
                tuple_first_literals(&args[2])
            };
            for slot in param_slot {
                let Some(literal) = first_string_literal(&slot) else {
                    continue;
                };
                checked += 1;
                assert!(
                    literal.is_ascii(),
                    "{name} 的插值参数名 {literal:?} 必须是纯 ASCII —— \
                     它是 error.<code> 词条里的占位符名"
                );
                assert!(
                    !literal.is_empty()
                        && literal
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                    "{name} 的插值参数名 {literal:?} 必须是 snake_case（前端按名字插值）"
                );
            }
        }
    }
    assert!(checked > 0, "参数名门禁一条都没跑到，说明扫描逻辑坏了");
}

/// 按顶层逗号切分实参（正确跳过字符串、括号、花括号与尖括号里的逗号）。
fn split_top_level_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut current = String::new();
    for ch in input.chars() {
        if in_string {
            current.push(ch);
            if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                current.push(ch);
            }
            '(' | '[' | '{' | '<' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' | '}' | '>' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                args.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        args.push(current);
    }
    args
}

/// 从 `vec![("a", …), ("b", …)]` 里抽出每个元组的第 1 个字面量。
fn tuple_first_literals(vec_expr: &str) -> Vec<String> {
    vec_expr
        .match_indices("(\"")
        .map(|(at, _)| {
            let rest = &vec_expr[at + 1..];
            let close = rest.find('"').expect("字面量必须闭合");
            rest[..close].to_owned()
        })
        .collect()
}

fn first_string_literal(text: &str) -> Option<String> {
    let at = text.find('"')?;
    let rest = &text[at + 1..];
    let close = rest.find('"')?;
    Some(rest[..close].to_owned())
}

/// 前端必须能处理「词典里没有的 code」。
///
/// 契约：`classifyOperationError` 拿到未知 code 时归到 `failed`，而
/// `errorText` 会回落到 `message`（中文兜底），**不会**把裸 code 显示给用户。
/// 这条测试在 Rust 侧把「未知 code」这个形状固定下来。
#[test]
fn an_unmapped_code_still_produces_a_usable_fallback() {
    let unknown = ErrorCode::from_wire("totally_made_up");
    assert!(!ErrorCode::ALL.contains(&unknown));
    assert_eq!(unknown.i18n_key(), "error.totally_made_up");

    let error = UserError::new(unknown, "某个还没登记的错误");
    assert_eq!(error.code.as_str(), "totally_made_up");
    // 前端 `errorText()` 查不到 `error.totally_made_up` 时会拿到 key 本身，
    // 于是回落到 `message`（中文兜底）—— 用户看到的是中文，不是裸 code。
    assert_eq!(error.code.i18n_key(), "error.totally_made_up");
    assert_eq!(error.message, "某个还没登记的错误");
}
