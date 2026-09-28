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


def with_verify(replacement: str, target: str = " && bun run test:rust &&") -> dict:
    package = package_json()
    package["scripts"]["verify"] = package["scripts"]["verify"].replace(
        target, replacement
    )
    return package


class VerifyChainTests(unittest.TestCase):
    def package(self) -> dict:
        return package_json()

    def test_verify_runs_every_frontend_rust_and_python_suite(self):
        self.assertEqual(MODULE.validate_verify_chain(self.package()), [])

    def test_rejects_a_verify_chain_without_the_rust_suite(self):
        package = self.package()
        package["scripts"]["verify"] = package["scripts"]["verify"].replace(
            " && bun run test:rust", ""
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "test:rust"), errors)

    def test_rejects_a_verify_chain_without_the_frontend_tests(self):
        package = self.package()
        package["scripts"]["verify"] = package["scripts"]["verify"].replace(
            " && bun run test &&", " &&"
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "缺少 test", "bun run test"), errors)

    def test_rejects_a_python_suite_that_discovers_nothing(self):
        package = self.package()
        package["scripts"]["test:python"] = "python3 -c pass"
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "scripts/tests", "test_*.py"), errors)

    def test_rejects_a_rust_suite_without_clippy_or_fmt(self):
        package = self.package()
        package["scripts"]["test:rust"] = "cargo test --manifest-path src-tauri/Cargo.toml --all-targets"
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "clippy", "fmt"), errors)

    def test_rejects_a_frontend_gate_that_runs_the_rust_suite(self):
        package = self.package()
        package["scripts"]["verify:frontend"] += " && bun run test:rust"
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "不得包含 test:rust"), errors)

    def test_rejects_an_echo_that_merely_mentions_the_rust_suite(self):
        errors = MODULE.validate_verify_chain(
            with_verify(" && echo bun run test:rust &&")
        )
        self.assertTrue(mark_anything(errors, "echo", "test:rust"), errors)
        self.assertTrue(mark_anything(errors, "不能代替真实执行"), errors)

    def test_rejects_an_echo_that_merely_mentions_the_security_gate(self):
        errors = MODULE.validate_verify_chain(
            with_verify(" && echo bun run security:check &&", " && bun run security:check &&")
        )
        self.assertTrue(mark_anything(errors, "echo", "security:check"), errors)

    def test_rejects_a_verify_chain_separated_by_semicolons(self):
        package = self.package()
        package["scripts"]["verify"] = package["scripts"]["verify"].replace(" && ", " ; ")
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "顶层分隔符", ";"), errors)

    def test_rejects_every_other_top_level_separator(self):
        for separator in (";", "||", "&", "|"):
            with self.subTest(separator=separator):
                package = self.package()
                package["scripts"]["verify"] = "bun run lint" + (
                    f" {separator} " + "bun run test"
                )
                errors = MODULE.validate_verify_chain(package)
                self.assertTrue(mark_anything(errors, "顶层分隔符", separator), errors)

    def test_rejects_decoy_commands_in_any_segment(self):
        for decoy in ("echo", "printf", "true", "false"):
            with self.subTest(decoy=decoy):
                package = self.package()
                package["scripts"]["verify"] = (
                    "bun run lint && bun run typecheck && bun run test && bun run test:python"
                    f" && bun run security:check && {decoy} bun run test:rust && bun run build"
                )
                errors = MODULE.validate_verify_chain(package)
                self.assertTrue(mark_anything(errors, decoy, "不能代替真实执行"), errors)

    def test_rejects_a_bun_segment_with_extra_arguments(self):
        package = self.package()
        package["scripts"]["verify"] = package["scripts"]["verify"].replace(
            "bun run test:rust", "bun run test:rust --silent"
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "精确为", "bun run <script>"), errors)

    def test_rejects_a_bun_segment_pointing_at_an_unknown_script(self):
        package = self.package()
        package["scripts"]["verify"] = package["scripts"]["verify"].replace(
            "bun run security:check", "bun run security:scan"
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "不存在的 script", "security:scan"), errors)

    def test_rejects_a_non_bun_command_inside_verify(self):
        package = self.package()
        package["scripts"]["verify"] = (
            "bun run lint && bun run typecheck && bun run test && bun run test:python"
            " && npm run security:check && bun run test:rust && bun run build"
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "bun run <script>", "npm"), errors)

    def test_rejects_a_python_suite_with_an_extra_stage(self):
        package = self.package()
        package["scripts"]["test:python"] = (
            "PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover"
            " -s scripts/tests -p \"test_*.py\" && echo done"
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "精确命令链", "test:python"), errors)

    def test_rejects_a_python_suite_without_the_bytecode_guard(self):
        package = self.package()
        package["scripts"]["test:python"] = package["scripts"]["test:python"].replace(
            "PYTHONDONTWRITEBYTECODE=1 ", ""
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "精确命令链", "PYTHONDONTWRITEBYTECODE"), errors)

    def test_rejects_a_rust_suite_with_an_echo_stage(self):
        package = self.package()
        package["scripts"]["test:rust"] = (
            "cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check"
            " && echo cargo clippy --all-targets"
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "echo", "clippy"), errors)

    def test_rejects_a_rust_suite_separated_by_semicolons(self):
        package = self.package()
        package["scripts"]["test:rust"] = package["scripts"]["test:rust"].replace(
            " && ", " ; "
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "顶层分隔符", ";"), errors)

    def test_rejects_a_rust_suite_with_an_unregistered_cargo_subcommand(self):
        package = self.package()
        package["scripts"]["test:rust"] = (
            "cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check"
            " && cargo build && cargo test --all-targets"
            " && cargo clippy --all-targets -- -D warnings"
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "未登记的 cargo 子命令", "build"), errors)

    def test_rejects_a_rust_stage_that_drops_its_own_flags(self):
        package = self.package()
        package["scripts"]["test:rust"] = package["scripts"]["test:rust"].replace(
            "cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings",
            "cargo clippy --manifest-path src-tauri/Cargo.toml",
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "cargo clippy", "--all-targets"), errors)

    def test_rejects_a_frontend_gate_with_a_non_bun_segment(self):
        package = self.package()
        package["scripts"]["verify:frontend"] += " && npm run build"
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "bun run <script>", "npm"), errors)

    def test_rejects_a_rust_suite_that_compiles_tests_without_running_them(self):
        package = self.package()
        package["scripts"]["test:rust"] = package["scripts"]["test:rust"].replace(
            "cargo test --manifest-path src-tauri/Cargo.toml --all-targets",
            "cargo test --manifest-path src-tauri/Cargo.toml --all-targets --no-run",
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(
            mark_anything(errors, "cargo test", "--no-run", "抑制执行"), errors
        )

    def test_rejects_a_rust_suite_that_only_lists_tests(self):
        package = self.package()
        package["scripts"]["test:rust"] = package["scripts"]["test:rust"].replace(
            "cargo test --manifest-path src-tauri/Cargo.toml --all-targets",
            "cargo test --manifest-path src-tauri/Cargo.toml --all-targets -- --list",
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "cargo test", "--list", "抑制执行"), errors)

    def test_rejects_a_rust_suite_that_silences_cargo_output(self):
        for flag in ("-q", "--quiet"):
            with self.subTest(flag=flag):
                package = self.package()
                package["scripts"]["test:rust"] = package["scripts"]["test:rust"].replace(
                    "cargo test --manifest-path src-tauri/Cargo.toml --all-targets",
                    f"cargo test --manifest-path src-tauri/Cargo.toml --all-targets {flag}",
                )
                errors = MODULE.validate_verify_chain(package)
                self.assertTrue(
                    mark_anything(errors, "cargo test", flag, "抑制执行"), errors
                )

    def test_rejects_a_suppressing_flag_on_any_cargo_stage(self):
        for subcommand, anchor in (
            ("clippy", "cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets"),
            ("fmt", "cargo fmt --manifest-path src-tauri/Cargo.toml --all"),
        ):
            with self.subTest(subcommand=subcommand):
                package = self.package()
                package["scripts"]["test:rust"] = package["scripts"]["test:rust"].replace(
                    anchor, f"{anchor} --quiet"
                )
                errors = MODULE.validate_verify_chain(package)
                self.assertTrue(
                    mark_anything(errors, f"cargo {subcommand}", "--quiet", "抑制执行"),
                    errors,
                )

    def test_rejects_a_rust_suite_whose_test_stage_filters_tests_by_name(self):
        package = self.package()
        package["scripts"]["test:rust"] = package["scripts"]["test:rust"].replace(
            "cargo test --manifest-path src-tauri/Cargo.toml --all-targets",
            "cargo test --manifest-path src-tauri/Cargo.toml --all-targets -- --exact quiet",
        )
        errors = MODULE.validate_verify_chain(package)
        self.assertTrue(mark_anything(errors, "cargo test", "抑制执行"), errors)


if __name__ == "__main__":
    unittest.main()
