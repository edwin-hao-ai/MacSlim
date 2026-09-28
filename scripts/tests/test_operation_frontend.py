from __future__ import annotations

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


class FrontendSurfaceTests(unittest.TestCase):
    def frontend(self) -> dict[str, str]:
        return MODULE.frontend_sources(ROOT)

    def test_rejects_a_raw_destructive_wrapper_in_the_tauri_module(self):
        sources = self.frontend()
        sources["src/lib/tauri.ts"] += (
            "\nexport async function killProcesses(pids: number[]): Promise<unknown> {\n"
            '  return invoke("kill_processes", { pids });\n}\n'
        )
        errors = MODULE.validate_frontend_operation_surface(sources)
        self.assertTrue(mark_anything(errors, "killProcesses", "kill_processes"), errors)

    def test_rejects_a_raw_wrapper_call_inside_a_view(self):
        sources = self.frontend()
        sources["src/views/CacheView.tsx"] += (
            '\nexport const legacy = () => killProcesses([1], ["x"]);\n'
        )
        errors = MODULE.validate_frontend_operation_surface(sources)
        self.assertTrue(mark_anything(errors, "killProcesses"), errors)

    def test_ignores_raw_wrapper_names_inside_view_comments(self):
        sources = self.frontend()
        sources["src/views/CacheView.tsx"] += (
            '\n// killProcesses([1], ["x"]); uninstallApps(targets); cleanCache(items);\n'
        )
        self.assertEqual(MODULE.validate_frontend_operation_surface(sources), [])

    def test_rejects_a_deleted_raw_type_reference(self):
        sources = self.frontend()
        sources["src/lib/tauri.ts"] += "\nexport type Legacy = UninstallTarget;\n"
        errors = MODULE.validate_frontend_operation_surface(sources)
        self.assertTrue(mark_anything(errors, "UninstallTarget"), errors)

    def test_rejects_an_invoke_call_outside_the_tauri_module(self):
        sources = self.frontend()
        sources["src/views/ScanView.tsx"] += (
            '\nimport { invoke } from "@tauri-apps/api/core";\nvoid invoke("scan_all");\n'
        )
        errors = MODULE.validate_frontend_operation_surface(sources)
        self.assertTrue(mark_anything(errors, "tauri.ts"), errors)

    def test_rejects_a_computed_invoke_command(self):
        sources = self.frontend()
        sources["src/lib/tauri.ts"] += (
            '\nexport const command = "scan_all";\nexport const extra = () => invoke(command);\n'
        )
        errors = MODULE.validate_frontend_operation_surface(sources)
        self.assertTrue(mark_anything(errors, "字面量"), errors)

    def test_rejects_a_raw_target_field_in_a_request_variant(self):
        sources = self.frontend()
        sources["src/lib/tauri.ts"] = sources["src/lib/tauri.ts"].replace(
            CACHE_VARIANT,
            f'{CACHE_VARIANT[:-2]}; bundle_path: string }}',
        )
        errors = MODULE.validate_frontend_operation_surface(sources)
        self.assertTrue(mark_anything(errors, "bundle_path"), errors)

    def test_rejects_a_new_request_variant(self):
        sources = self.frontend()
        sources["src/lib/tauri.ts"] = sources["src/lib/tauri.ts"].replace(
            CACHE_VARIANT,
            '  | { type: "raw"; target: string }\n' + CACHE_VARIANT,
            1,
        )
        errors = MODULE.validate_frontend_operation_surface(sources)
        self.assertTrue(mark_anything(errors, "raw"), errors)

    def test_rejects_a_shell_command_field_on_a_cache_item(self):
        sources = self.frontend()
        sources["src/lib/tauri.ts"] = sources["src/lib/tauri.ts"].replace(
            "  recover_hint: string;\n};", "  recover_hint: string;\n  command: string | null;\n};"
        )
        errors = MODULE.validate_frontend_operation_surface(sources)
        self.assertTrue(mark_anything(errors, "command"), errors)

    def test_repository_cache_item_carries_no_shell_command(self):
        fields = MODULE.typescript_type_fields(REPOSITORY_FILES["tauri.ts"], "CacheItem")
        self.assertNotIn("command", fields)
        self.assertIn("path", fields)


class UnparseableSourceTests(unittest.TestCase):
    def test_unterminated_apostrophe_fails_closed_instead_of_masking_the_file(self):
        sources = broken_quote_frontend()
        errors = MODULE.validate_frontend_operation_surface(sources)
        self.assertTrue(
            mark_anything(errors, *parse_failure_needle("src/views/CacheView.tsx")), errors
        )
        self.assertTrue(mark_anything(errors, "未闭合", "CacheView.tsx"), errors)

    def test_unterminated_quote_stops_the_whole_contract(self):
        errors = MODULE.validate_operation_contract(broken_quote_files())
        self.assertTrue(
            mark_anything(errors, *parse_failure_needle("src/views/CacheView.tsx")), errors
        )

    def test_unterminated_quote_is_reported_for_every_affected_file(self):
        sources = frontend_sources()
        for name in ("src/views/ProcessView.tsx", "src/lib/format.ts", "src/App.tsx"):
            with self.subTest(source=name):
                sources[name] = UNCLOSED_APOSTROPHE + sources[name]
                errors = MODULE.validate_frontend_operation_surface(sources)
                self.assertTrue(mark_anything(errors, *parse_failure_needle(name)), errors)

    def test_unterminated_quote_is_reported_for_rust_sources_too(self):
        files = dict(REPOSITORY_FILES)
        files["lib.rs"] = files["lib.rs"] + APPENDED_RUST_QUOTE
        errors = MODULE.validate_ipc_command_surface(files["lib.rs"])
        self.assertTrue(mark_anything(errors, *parse_failure_needle("lib.rs")), errors)
        errors = MODULE.validate_operation_contract(files)
        self.assertTrue(mark_anything(errors, "lib.rs", "未闭合"), errors)

    def test_backend_and_history_validators_also_fail_closed(self):
        files = dict(REPOSITORY_FILES)
        files["operation_commands.rs"] = files["operation_commands.rs"] + APPENDED_RUST_QUOTE
        self.assertTrue(
            mark_anything(
                MODULE.validate_backend_operation_contract(files),
                *parse_failure_needle("operation_commands.rs"),
            )
        )
        labels = {
            "operation_types.rs": files["operation_types.rs"],
            "HistoryView.tsx": files["HistoryView.tsx"],
            "cli.rs": files["cli_rs"],
            "en.ts": files["en.ts"],
            "zh-CN.ts": files["zh-CN.ts"],
        }
        labels["cli.rs"] = labels["cli.rs"] + APPENDED_RUST_QUOTE
        self.assertTrue(
            mark_anything(
                MODULE.validate_operation_history_labels(labels), *parse_failure_needle("cli.rs")
            )
        )

    def test_parser_raises_a_value_error_on_an_unterminated_string(self):
        with self.assertRaises(ValueError) as context:
            MODULE.mask_code("const note = 'oops;\nconst other = 1;\n", MODULE.TS_QUOTES)
        self.assertIn("未闭合", str(context.exception))
        self.assertIsInstance(context.exception, MODULE.SourceParseError)

    def test_broken_package_json_is_reported_in_chinese(self):
        files = dict(REPOSITORY_FILES)
        files["package.json"] = '{"scripts": {'
        self.assertTrue(MODULE.validate_operation_contract(files))

    def test_balanced_apostrophe_still_follows_the_original_rules(self):
        sources = frontend_sources()
        original = sources["src/views/CacheView.tsx"]
        for prefix, label in (
            (BALANCED_APOSTROPHE, "转义撇号字符串"),
            ("// don't show raw commands\n", "行注释"),
            ('const OTHER = "don\'t";\n', "双引号内撇号"),
        ):
            with self.subTest(form=label):
                sources["src/views/CacheView.tsx"] = prefix + original
                self.assertEqual(MODULE.validate_frontend_operation_surface(sources), [])

    def test_balanced_apostrophe_does_not_disable_the_raw_wrapper_gate(self):
        sources = frontend_sources()
        sources["src/views/CacheView.tsx"] = (
            BALANCED_APOSTROPHE + sources["src/views/CacheView.tsx"] + RAW_WRAPPER_CALL
        )
        errors = MODULE.validate_frontend_operation_surface(sources)
        self.assertTrue(mark_anything(errors, "killProcesses"), errors)

    def test_repository_frontend_parses_without_a_single_parse_failure(self):
        self.assertEqual(MODULE.frontend_parse_errors(frontend_sources()), [])
        self.assertEqual(MODULE.parse_errors(REPOSITORY_FILES), [])


if __name__ == "__main__":
    unittest.main()
