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


class HistoryLabelTests(unittest.TestCase):
    def labels(self) -> dict[str, str]:
        return {
            "operation_types.rs": REPOSITORY_FILES["operation_types.rs"],
            "HistoryView.tsx": REPOSITORY_FILES["HistoryView.tsx"],
            "cli.rs": REPOSITORY_FILES["cli_rs"],
            "en.ts": REPOSITORY_FILES["en.ts"],
            "zh-CN.ts": REPOSITORY_FILES["zh-CN.ts"],
        }

    def test_backend_operation_labels_are_exactly_the_broker_kinds(self):
        self.assertEqual(
            MODULE.rust_operation_kind_labels(REPOSITORY_FILES["operation_types.rs"]),
            {"cache", "process", "app_terminate", "app_graceful_quit", "uninstall", "docker"},
        )

    def test_rejects_a_history_view_without_a_graceful_quit_label(self):
        sources = self.labels()
        sources["HistoryView.tsx"] = sources["HistoryView.tsx"].replace(
            '      case "app_graceful_quit":\n        return t("history.opAppGracefulQuit");',
            '      case "app_graceful_quit":\n        return op;',
        )
        errors = MODULE.validate_operation_history_labels(sources)
        self.assertTrue(mark_anything(errors, "app_graceful_quit", "t("), errors)

    def test_rejects_a_raw_key_fallback_for_a_known_kind(self):
        sources = self.labels()
        sources["HistoryView.tsx"] = sources["HistoryView.tsx"].replace(
            '        return t("history.opDocker");', "        return op;"
        )
        errors = MODULE.validate_operation_history_labels(sources)
        self.assertTrue(mark_anything(errors, "docker", "t("), errors)

    def test_rejects_a_history_view_without_the_graceful_quit_branch(self):
        sources = self.labels()
        sources["HistoryView.tsx"] = sources["HistoryView.tsx"].replace(
            '      case "app_graceful_quit":\n        return t("history.opAppGracefulQuit");', ""
        )
        errors = MODULE.validate_operation_history_labels(sources)
        self.assertTrue(mark_anything(errors, "app_graceful_quit"), errors)

    def test_rejects_a_cli_label_table_without_a_graceful_quit_label(self):
        sources = self.labels()
        sources["cli.rs"] = sources["cli.rs"].replace(
            '        "app_graceful_quit" => "应用优雅退出",\n', ""
        )
        errors = MODULE.validate_operation_history_labels(sources)
        self.assertTrue(mark_anything(errors, "app_graceful_quit"), errors)

    def test_rejects_a_cli_label_table_that_drops_a_broker_kind(self):
        sources = self.labels()
        sources["cli.rs"] = sources["cli.rs"].replace(
            '        "docker" => "Docker 清理",\n', ""
        )
        errors = MODULE.validate_operation_history_labels(sources)
        self.assertTrue(mark_anything(errors, "docker"), errors)

    def test_allows_legacy_history_labels_to_be_dropped(self):
        sources = self.labels()
        sources["cli.rs"] = sources["cli.rs"].replace(
            '        "process" | "process_kill" => "进程清理",',
            '        "process" => "进程清理",',
        ).replace(
            '        "cache" | "cache_clean" => "缓存清理",', '        "cache" => "缓存清理",'
        ).replace(
            '        "uninstall" | "app_uninstall" => "应用卸载",', '        "uninstall" => "应用卸载",'
        ).replace(
            '        "app_terminate" | "app_quit" | "app_force_quit" => "强制退出应用",',
            '        "app_terminate" => "强制退出应用",',
        )
        self.assertEqual(MODULE.validate_operation_history_labels(sources), [])

    def test_rejects_a_cli_fallback_that_echoes_the_raw_key(self):
        sources = self.labels()
        sources["cli.rs"] = sources["cli.rs"].replace('        _ => "未知操作",', "        _ => op,")
        errors = MODULE.validate_operation_history_labels(sources)
        self.assertTrue(mark_anything(errors, "裸 key"), errors)

    def test_rejects_a_locale_missing_an_operation_label(self):
        sources = self.labels()
        sources["en.ts"] = sources["en.ts"].replace('    opDocker: "Docker cleanup",', "")
        errors = MODULE.validate_operation_history_labels(sources)
        self.assertTrue(mark_anything(errors, "opDocker"), errors)

    def test_rejects_a_locale_missing_the_graceful_quit_label(self):
        sources = self.labels()
        sources["zh-CN.ts"] = sources["zh-CN.ts"].replace(
            '    opAppGracefulQuit: "应用优雅退出",', ""
        )
        errors = MODULE.validate_operation_history_labels(sources)
        self.assertTrue(mark_anything(errors, "opAppGracefulQuit"), errors)

    def test_rejects_broken_locale_parity(self):
        sources = self.labels()
        sources["zh-CN.ts"] = sources["zh-CN.ts"].replace(
            '    opCache: "缓存清理",', '    opCache: "缓存清理",\n    opOnlyInZh: "仅中文",'
        )
        errors = MODULE.validate_operation_history_labels(sources)
        self.assertTrue(mark_anything(errors, "一致"), errors)


if __name__ == "__main__":
    unittest.main()
