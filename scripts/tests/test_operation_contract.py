from __future__ import annotations

import re
import unittest

try: from scripts.tests.operation_contract_support import (
    APPENDED_RUST_QUOTE,
    BALANCED_APOSTROPHE,
    CACHE_VARIANT,
    CLI_SESSION_BLOCK,
    MODULE,
    RAW_WRAPPER_CALL,
    REPOSITORY_FILES,
    UNCLOSED_APOSTROPHE,
    add_handler_command,
    add_handler_entry,
    broken_quote_files,
    broken_quote_frontend,
    frontend_sources,
    mark_anything,
    package_json,
    parse_failure_needle,
    ROOT,
)
except ModuleNotFoundError: from operation_contract_support import (
    APPENDED_RUST_QUOTE,
    BALANCED_APOSTROPHE,
    CACHE_VARIANT,
    CLI_SESSION_BLOCK,
    MODULE,
    RAW_WRAPPER_CALL,
    REPOSITORY_FILES,
    UNCLOSED_APOSTROPHE,
    add_handler_command,
    add_handler_entry,
    broken_quote_files,
    broken_quote_frontend,
    frontend_sources,
    mark_anything,
    package_json,
    parse_failure_needle,
)


class RepositoryContractTests(unittest.TestCase):
    def test_repository_sources_satisfy_the_operation_contract(self):
        self.assertEqual(MODULE.validate_operation_contract(REPOSITORY_FILES), [])

    def test_repository_runs_its_own_validator(self):
        self.assertEqual(MODULE.validate_operation_contract_files(ROOT), [])

    def test_repository_pins_the_registered_ipc_commands(self):
        # 17 条（原 16 + `get_build_flavor`）。新增那条是纯元数据读取：下发
        # 当前构建形态，让前端能在 MAS 版里藏掉「终止进程」这个沙箱里做不到的
        # 入口。**它是刻意的偏离**，不是顺手加的 ——
        # 替代方案（Vite define 复制一份 flavor）会让 Rust 和前端出现两个真相源，
        # 那正是 src-tauri/src/flavor.rs 的注释里明确警告的漂移。
        # 要加/删命令时，先想清楚它属于 DESTRUCTIVE_IPC_COMMANDS 吗。
        commands = MODULE.rust_invoke_handler_commands(REPOSITORY_FILES["lib.rs"])
        self.assertEqual(commands, list(MODULE.EXPECTED_IPC_COMMANDS))
        self.assertEqual(len(MODULE.EXPECTED_IPC_COMMANDS), 17)
        self.assertEqual(
            [c for c in MODULE.EXPECTED_IPC_COMMANDS
             if c in MODULE.DESTRUCTIVE_IPC_COMMANDS],
            ["prepare_operation", "execute_operation"],
        )

    def test_destructive_surface_is_exactly_prepare_and_execute(self):
        commands = MODULE.rust_invoke_handler_commands(REPOSITORY_FILES["lib.rs"])
        destructive = [name for name in commands if name in MODULE.DESTRUCTIVE_IPC_COMMANDS]
        self.assertEqual(destructive, ["prepare_operation", "execute_operation"])

    def test_repository_frontend_invokes_only_registered_commands(self):
        invoked = MODULE.typescript_invoke_commands(REPOSITORY_FILES["tauri.ts"])
        self.assertEqual(len(invoked), len(MODULE.EXPECTED_IPC_COMMANDS))
        self.assertEqual(
            {value.strip('"') for value in invoked.values()},
            set(MODULE.EXPECTED_IPC_COMMANDS),
        )

    def test_repository_request_variants_match_the_pinned_field_sets(self):
        self.assertEqual(
            MODULE.typescript_request_variants(REPOSITORY_FILES["tauri.ts"]),
            MODULE.EXPECTED_REQUEST_VARIANTS,
        )

    def test_repository_locales_stay_in_parity(self):
        self.assertEqual(
            MODULE.typescript_nested_keys(REPOSITORY_FILES["en.ts"]),
            MODULE.typescript_nested_keys(REPOSITORY_FILES["zh-CN.ts"]),
        )

    def test_repository_locale_keys_reach_nested_paths(self):
        keys = MODULE.typescript_nested_keys(REPOSITORY_FILES["zh-CN.ts"])
        self.assertIn("history.opDocker", keys)
        self.assertIn("common.appName", keys)

    def test_repository_history_label_lookup_covers_every_operation_kind(self):
        errors = MODULE.validate_operation_history_labels(
            {
                "operation_types.rs": REPOSITORY_FILES["operation_types.rs"],
                "HistoryView.tsx": REPOSITORY_FILES["HistoryView.tsx"],
                "cli.rs": REPOSITORY_FILES["cli_rs"],
                "en.ts": REPOSITORY_FILES["en.ts"],
                "zh-CN.ts": REPOSITORY_FILES["zh-CN.ts"],
            }
        )
        self.assertEqual(errors, [])

    def test_repository_random_ids_come_from_a_direct_getrandom_dependency(self):
        self.assertEqual(
            MODULE.validate_random_id_dependency(REPOSITORY_FILES["Cargo.toml"]), []
        )

    def test_repository_backend_contract_passes_on_its_own(self):
        errors = MODULE.validate_backend_operation_contract(REPOSITORY_FILES)
        self.assertEqual(errors, [])

    def test_repository_cli_surface_passes_on_its_own(self):
        self.assertEqual(MODULE.validate_cli_operation_surface(REPOSITORY_FILES["cli_rs"]), [])
        self.assertEqual(MODULE.validate_cli_fail_closed(REPOSITORY_FILES["cli_operations.rs"]), [])


#: 优雅退出路由守卫的变异夹具。
#:
#: 改造前这一段是 `Err(GRACEFUL_QUIT_ROUTE.to_owned())`；错误结构化之后它变成
#: `Err(UserError::new(ErrorCode::INTERNAL, GRACEFUL_QUIT_ROUTE))`。这里**从真实
#: 源码里挖**而不是写死字面量 —— 写死过一次，改造后 `str.replace` 变成 no-op，
#: 变异根本没发生，门禁看着绿其实什么都没验（ledger 第 0 片同一个坑）。
_GRACEFUL_ROUTE_RE = re.compile(
    r"[ \t]*if mode == ProcessMode::Graceful \{.*?\n[ \t]*\}\n", re.S
)


def _cut_graceful_route(source: str) -> str:
    mutated, hits = _GRACEFUL_ROUTE_RE.subn("", source, count=1)
    assert hits == 1, "优雅退出路由块没匹配上，变异测试空转了"
    # 只断言守卫块没了 —— 常量声明 `pub(crate) const GRACEFUL_QUIT_ROUTE` 还在文件里
    assert "if mode == ProcessMode::Graceful" not in mutated
    return mutated


def _graceful_route_block() -> str:
    return (
        "        if mode == ProcessMode::Graceful {\n"
        "            return Err(UserError::new(ErrorCode::INTERNAL, GRACEFUL_QUIT_ROUTE));\n"
        "        }\n"
    )

class IpcCommandSurfaceTests(unittest.TestCase):
    def base(self) -> str:
        return REPOSITORY_FILES["lib.rs"]

    def test_rejects_a_raw_destructive_command_in_the_invoke_handler(self):
        for command in sorted(MODULE.RAW_IPC_COMMANDS):
            with self.subTest(command=command):
                errors = MODULE.validate_ipc_command_surface(add_handler_command(self.base(), command))
                self.assertTrue(mark_anything(errors, command), errors)

    def test_rejects_a_declared_command_missing_from_the_handler(self):
        source, hits = re.subn(
            r"(#\[tauri::command\]\nasync fn docker_available\(\n?)",
            "#[tauri::command]\n"
            "async fn docker_remove_image(\n    state: State<'_, AppState>,\n"
            ") -> Result<bool, String> {\n    let _ = state;\n    Ok(true)\n}\n\n"
            "#[tauri::command]\n"
            "async fn docker_available() -> Result<bool, String> {\n",
            self.base(),
            count=1,
        )
        self.assertEqual(hits, 1, "docker_available 签名没匹配上，变异测试空转了")
        errors = MODULE.validate_ipc_command_surface(source)
        self.assertTrue(mark_anything(errors, "docker_remove_image", "docker_available"), errors)

    def test_rejects_a_registered_command_without_a_declaration(self):
        errors = MODULE.validate_ipc_command_surface(add_handler_entry(self.base(), "ghost_command"))
        self.assertTrue(mark_anything(errors, "ghost_command"), errors)

    def test_ignores_raw_command_names_inside_comments_and_strings(self):
        decoy = (
            "// kill_processes clean_cache quit_application force_quit_application uninstall_apps\n"
            '// docker_remove_image docker_remove_container docker_remove_volume docker_prune_all\n'
            'const NOTE: &str = "kill_processes clean_cache quit_and_uninstall scan_app_residues";\n'
        )
        self.assertEqual(MODULE.validate_ipc_command_surface(decoy + self.base()), [])

    def test_rejects_a_stray_generate_handler_block(self):
        source = (
            "fn stray() {\n    let _ = tauri::generate_handler![kill_processes];\n}\n\n" + self.base()
        )
        errors = MODULE.validate_ipc_command_surface(source)
        self.assertTrue(mark_anything(errors, "generate_handler", "kill_processes"), errors)

    def test_rejects_a_second_invoke_handler(self):
        source = self.base().replace(
            "pub fn run_tauri() {",
            "fn extra_builder() {\n"
            "    let builder = tauri::Builder::default();\n"
            "    let _ = builder.invoke_handler;\n"
            "}\n\npub fn run_tauri() {",
        )
        errors = MODULE.validate_ipc_command_surface(source)
        self.assertTrue(mark_anything(errors, "invoke_handler"), errors)

    def test_rejects_a_replaced_destructive_entry_point(self):
        source = add_handler_command(self.base(), "kill_processes").replace(
            "            execute_operation,\n", ""
        )
        errors = MODULE.validate_ipc_command_surface(source)
        self.assertTrue(mark_anything(errors, "execute_operation"), errors)

    def test_rejects_a_handler_without_any_command(self):
        source = self.base().replace("            check_app_running,", "            /* removed */")
        errors = MODULE.validate_ipc_command_surface(source)
        self.assertTrue(mark_anything(errors, "generate_handler", "17 个可信 command"), errors)
        self.assertTrue(mark_anything(errors, "check_app_running"), errors)


class CliSurfaceTests(unittest.TestCase):
    def cli(self) -> str:
        return REPOSITORY_FILES["cli_rs"]

    def test_rejects_a_raw_target_flag(self):
        for flag in sorted(MODULE.RAW_CLI_FLAGS):
            with self.subTest(flag=flag):
                source = self.cli().replace(
                    '        "--history" => Ok(Command::History),',
                    f'        "{flag}" => Ok(Command::History),\n        "--history" => Ok(Command::History),',
                )
                errors = MODULE.validate_cli_operation_surface(source)
                self.assertTrue(mark_anything(errors, flag), errors)

    def test_rejects_a_direct_executor_call_in_the_cli(self):
        for function in sorted(MODULE.RAW_CLI_FUNCTIONS):
            with self.subTest(function=function):
                source = self.cli().replace(
                    "fn print_help() {",
                    f"fn bypass() {{\n    let _ = {function}();\n}}\n\nfn print_help() {{",
                )
                errors = MODULE.validate_cli_operation_surface(source)
                self.assertTrue(mark_anything(errors, function), errors)

    def test_rejects_a_migration_bridge_import(self):
        source = self.cli().replace(
            "use macslim_lib::scanner::Risk;",
            "use macslim_lib::process_ops::graceful_kill;\nuse macslim_lib::scanner::Risk;",
        )
        errors = MODULE.validate_cli_operation_surface(source)
        self.assertTrue(mark_anything(errors, "graceful_kill"), errors)

    def test_ignores_removed_entry_names_inside_cli_comments(self):
        source = self.cli().replace(
            "fn print_help() {",
            "// kill_processes --pid /Applications/Alpha.app cache_cleaner_clean\n\nfn print_help() {",
        )
        self.assertEqual(MODULE.validate_cli_operation_surface(source), [])

    def test_rejects_a_cli_without_argument_parsing(self):
        source = self.cli().replace("fn parse_args(args: &[String])", "fn parse_args_disabled(args: &[String])")
        errors = MODULE.validate_cli_operation_surface(source)
        self.assertTrue(mark_anything(errors, "parse_args"), errors)


class CliFailClosedTests(unittest.TestCase):
    def cli_operations(self) -> str:
        return REPOSITORY_FILES["cli_operations.rs"]

    def test_rejects_a_silently_degraded_storage_open(self):
        for suffix in (".ok()", ".unwrap_or_default()", ".ok()?  ", ".expect(\"存储\")"):
            with self.subTest(suffix=suffix):
                source = self.cli_operations().replace("Storage::open(),", f"Storage::open(){suffix},")
                errors = MODULE.validate_cli_fail_closed(source)
                self.assertTrue(mark_anything(errors, "Storage::open", "fail-closed"), errors)

    def test_rejects_a_process_clean_without_a_storage_session(self):
        source = self.cli_operations().replace(CLI_SESSION_BLOCK, "    let _ = &storage;")
        errors = MODULE.validate_cli_fail_closed(source)
        self.assertTrue(mark_anything(errors, "CliSession::open"), errors)

    def test_rejects_a_process_clean_that_builds_the_observer_first(self):
        source = self.cli_operations().replace(
            CLI_SESSION_BLOCK,
            "    let mut observer = build_observer(Arc::clone(&session.storage));\n"
            "    let session = CliSession::open(storage)?;\n"
            "    let whitelist = session.whitelist();",
        )
        errors = MODULE.validate_cli_fail_closed(source)
        self.assertTrue(mark_anything(errors, "CliSession::open", "build_observer"), errors)

    def test_rejects_a_cache_clean_without_a_storage_session(self):
        source = self.cli_operations().replace(
            "    let session = CliSession::open(storage)?;\n    clean_snapshot_with(scan, scope, cleaner, &session.history()).await",
            "    clean_snapshot_with(scan, scope, cleaner, &CliHistory::default()).await",
        )
        errors = MODULE.validate_cli_fail_closed(source)
        self.assertTrue(mark_anything(errors, "CliSession::open"), errors)

    def test_rejects_a_cli_operations_module_without_session_helpers(self):
        source = self.cli_operations().replace("fn clean_cache_in_session", "fn cache_clean_entry")
        errors = MODULE.validate_cli_fail_closed(source)
        self.assertTrue(mark_anything(errors, "clean_cache_in_session"), errors)

    def test_rejects_a_clean_that_runs_before_the_session_and_hides_behind_a_comment(self):
        source = self.cli_operations().replace(
            "    let session = CliSession::open(storage)?;\n    clean_snapshot_with(scan, scope, cleaner, &session.history()).await",
            "    clean_snapshot_with(scan, scope, cleaner, &CliHistory::default()).await\n"
            "    let session = CliSession::open(storage)?;\n"
            "    let _ = &session;",
        )
        errors = MODULE.validate_cli_fail_closed(source)
        self.assertTrue(mark_anything(errors, "clean_cache_in_session", "clean_snapshot_with"), errors)

    def test_rejects_a_cache_session_order_forged_by_a_trailing_comment(self):
        source = self.cli_operations().replace(
            "    let session = CliSession::open(storage)?;\n    clean_snapshot_with(scan, scope, cleaner, &session.history()).await",
            "    let session = CliSession::open(storage)?;\n"
            "    // clean_snapshot_with 已在上方完成\n"
            "    let _ = cleaner;",
        )
        errors = MODULE.validate_cli_fail_closed(source)
        self.assertTrue(mark_anything(errors, "clean_cache_in_session", "clean_snapshot_with"), errors)

    def test_rejects_a_cache_session_order_forged_by_a_block_comment(self):
        source = self.cli_operations().replace(
            "    let session = CliSession::open(storage)?;\n    clean_snapshot_with(scan, scope, cleaner, &session.history()).await",
            "    let session = CliSession::open(storage)?;\n"
            "    /* clean_snapshot_with 已在上方完成 */\n"
            "    let _ = cleaner;",
        )
        errors = MODULE.validate_cli_fail_closed(source)
        self.assertTrue(mark_anything(errors, "clean_cache_in_session", "clean_snapshot_with"), errors)

    def test_rejects_a_process_session_order_forged_by_a_string_literal(self):
        source = self.cli_operations().replace(
            CLI_SESSION_BLOCK,
            "    let session = CliSession::open(storage)?;\n"
            '    let note = "build_observer 已在上方完成";\n'
            "    let _ = (signaller, note, session);",
        )
        errors = MODULE.validate_cli_fail_closed(source)
        self.assertTrue(mark_anything(errors, "clean_processes_in_session", "build_observer"), errors)


class BackendContractTests(unittest.TestCase):
    def backend(self) -> dict[str, str]:
        return dict(REPOSITORY_FILES)

    def test_rejects_a_prepare_request_without_deny_unknown_fields(self):
        sources = self.backend()
        sources["operation_commands.rs"] = sources["operation_commands.rs"].replace(
            '#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]',
            '#[serde(tag = "type", rename_all = "snake_case")]',
        )
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "deny_unknown_fields"), errors)

    def test_rejects_a_prepare_request_without_a_type_tag(self):
        sources = self.backend()
        sources["operation_commands.rs"] = sources["operation_commands.rs"].replace(
            '#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]',
            '#[serde(rename_all = "snake_case", deny_unknown_fields)]',
        )
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, 'tag = "type"'), errors)

    def test_rejects_a_serde_attribute_attached_to_another_type(self):
        sources = self.backend()
        sources["operation_commands.rs"] = sources["operation_commands.rs"].replace(
            '#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]\n'
            "pub enum PrepareOperationRequest {",
            "pub enum PrepareOperationRequest {",
        )
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "deny_unknown_fields"), errors)

    def test_rejects_a_prepare_request_without_the_graceful_quit_variant(self):
        sources = self.backend()
        sources["operation_commands.rs"] = sources["operation_commands.rs"].replace(
            "    AppGracefulQuit {\n        snapshot_id: String,\n        app_keys: Vec<String>,\n    },\n",
            "",
        )
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "AppGracefulQuit"), errors)

    def test_rejects_a_result_without_the_graceful_quit_variant(self):
        sources = self.backend()
        sources["operation_commands.rs"] = sources["operation_commands.rs"].replace(
            "    AppGracefulQuit(Vec<crate::applications::AppGracefulQuitReport>),\n", ""
        )
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "AppGracefulQuit"), errors)

    def test_rejects_a_consume_without_the_owner_check(self):
        sources = self.backend()
        # 用正则而不是逐字 `replace`：错误文案结构化改造后这一段的
        # 换行/缩进/构造方式都变过，**逐字夹具会静默变成 no-op** ——
        # 变异没发生，门禁看着绿，其实什么都没验。
        sources["operations.rs"], hits = re.subn(
            r"if entry\.owner != owner \{[^}]*\n        \}\n",
            "",
            sources["operations.rs"],
            count=1,
        )
        self.assertEqual(hits, 1, "owner 校验块没被挖掉，变异测试空转了")
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "owner"), errors)

    def test_rejects_a_store_without_a_consume_entry_point(self):
        sources = self.backend()
        sources["operations.rs"] = sources["operations.rs"].replace("    pub(crate) fn consume(", "    pub(crate) fn take(")
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "consume"), errors)

    def test_rejects_a_history_entry_carrying_the_operation_id(self):
        sources = self.backend()
        sources["operation_commands.rs"] = sources["operation_commands.rs"].replace(
            "    OperationHistoryEntry {\n        operation,\n        target: format!(\"{} 个应用\", reports.len()),",
            "    OperationHistoryEntry {\n        operation,\n        operation_id: String::new(),\n"
            "        target: format!(\"{} 个应用\", reports.len()),",
        )
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "operation_id"), errors)

    def test_rejects_a_history_operation_taken_from_the_raw_id(self):
        sources = self.backend()
        sources["operation_commands.rs"] = sources["operation_commands.rs"].replace(
            "    let operation = outcome.kind().label().to_owned();",
            "    let operation = outcome.kind().label().to_owned();\n"
            "    let operation = operation_id.to_owned();",
        )
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "operation_id", "label()"), errors)

    def test_rejects_a_history_builder_that_disappears(self):
        sources = self.backend()
        sources["operation_commands.rs"] = sources["operation_commands.rs"].replace(
            "fn quit_entry(", "fn removed_quit_entry("
        )
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "quit_entry"), errors)

    def test_rejects_a_graceful_termination_mode_that_reaches_identity_selection(self):
        sources = self.backend()
        sources["operations_prepare.rs"] = _cut_graceful_route(
            sources["operations_prepare.rs"]
        )
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "GRACEFUL_QUIT_ROUTE", "prepare_app_termination"), errors)

    def test_rejects_a_graceful_route_after_identity_selection(self):
        sources = self.backend()
        # 变异：把路由守卫从 `self.snapshot(...)` **之前**挪到目标选择之后。
        # 必须先摘掉原来那份 —— 校验器取的是**第一处** `ProcessMode::Graceful`
        # 与 `GRACEFUL_QUIT_ROUTE`，只插不摘的话原位那份仍然满足「早于」，
        # 变异等于没发生。
        prepare = _cut_graceful_route(sources["operations_prepare.rs"])
        prepare, hits = re.subn(
            r"(\n[ \t]*ensure_targets_terminable\(&child_targets, mode\)\?;)",
            r"\1\n" + _graceful_route_block().rstrip("\n"),
            prepare,
            count=1,
        )
        self.assertEqual(hits, 1, "ensure_targets_terminable 调用没匹配上，变异测试空转了")
        late = prepare.find("if mode == ProcessMode::Graceful")
        self.assertGreater(late, 0, "守卫块没插回去")
        self.assertGreater(
            late,
            prepare.find("self.snapshot("),
            "变异没把守卫挪到目标选择之后",
        )
        sources["operations_prepare.rs"] = prepare
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "GRACEFUL_QUIT_ROUTE"), errors)

    def test_rejects_graceful_kill_on_the_app_level_path(self):
        for name in ("applications.rs", "operation_executor.rs", "operation_commands.rs", "cli_rs", "cli_operations.rs", "lib.rs"):
            with self.subTest(source=name):
                sources = self.backend()
                sources[name] += "\nfn leak() {\n    let _ = process_ops::graceful_kill(1);\n}\n"
                errors = MODULE.validate_backend_operation_contract(sources)
                self.assertTrue(mark_anything(errors, "graceful_kill"), errors)

    def test_ignores_graceful_kill_mentions_in_comments(self):
        sources = self.backend()
        sources["applications.rs"] += "\n// 应用级退出不得走 process_ops::graceful_kill\n"
        self.assertEqual(MODULE.validate_backend_operation_contract(sources), [])

    def test_requires_graceful_kill_to_stay_crate_private(self):
        sources = self.backend()
        sources["process_ops.rs"] = sources["process_ops.rs"].replace(
            "pub(crate) fn graceful_kill", "pub fn graceful_kill"
        )
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "graceful_kill", "pub"), errors)

    def test_requires_the_graceful_quit_plan_to_reach_the_quit_domain(self):
        sources = self.backend()
        sources["operation_executor.rs"] = sources["operation_executor.rs"].replace(
            "run_app_graceful_quit(targets, domains.app_quit)", "run_app_graceful_quit(targets, domains.uninstall)"
        )
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "app_quit"), errors)

    def test_requires_the_app_quitter_to_reach_the_applescript_helper(self):
        sources = self.backend()
        sources["applications.rs"] = sources["applications.rs"].replace(
            "crate::uninstaller::quit_app(&requested).await", "Ok(())"
        )
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "quit_app"), errors)

    def test_rejects_an_execute_command_taking_a_client_collection(self):
        sources = self.backend()
        sources["lib.rs"], hits = re.subn(
            r"(async fn execute_operation\(.*?operation_id: String,\n)(\) -> Result<OperationResult, )",
            r"\1    extra: Vec<String>,\n\2",
            sources["lib.rs"],
            count=1,
            flags=re.S,
        )
        self.assertEqual(hits, 1, "execute_operation 签名没匹配上，变异测试空转了")
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "Vec<String>"), errors)

    def test_rejects_a_prepare_command_without_window_owner(self):
        sources = self.backend()
        sources["lib.rs"] = sources["lib.rs"].replace(
            "    let owner = window.label().to_owned();\n    let mut operations",
            "    let owner = \"main\".to_owned();\n    let mut operations",
        )
        errors = MODULE.validate_backend_operation_contract(sources)
        self.assertTrue(mark_anything(errors, "WebviewWindow", "owner"), errors)


class RandomIdDependencyTests(unittest.TestCase):
    def cargo(self) -> str:
        return REPOSITORY_FILES["Cargo.toml"]

    def test_rejects_a_missing_getrandom_dependency(self):
        errors = MODULE.validate_random_id_dependency(self.cargo().replace('getrandom = "0.2"\n', ""))
        self.assertTrue(mark_anything(errors, "getrandom"), errors)

    def test_rejects_a_relaxed_getrandom_requirement(self):
        errors = MODULE.validate_random_id_dependency(
            self.cargo().replace('getrandom = "0.2"', 'getrandom = "*"')
        )
        self.assertTrue(mark_anything(errors, "0.2"), errors)

    def test_rejects_getrandom_also_declared_as_a_build_dependency(self):
        cargo = self.cargo().replace("[build-dependencies]", '[build-dependencies]\ngetrandom = "0.2"')
        errors = MODULE.validate_random_id_dependency(cargo)
        self.assertTrue(mark_anything(errors, "build-dependencies"), errors)

    def test_rejects_an_unparsable_manifest(self):
        errors = MODULE.validate_random_id_dependency("[dependencies")
        self.assertTrue(mark_anything(errors, "无法解析"), errors)


if __name__ == "__main__":
    unittest.main()
