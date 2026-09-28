"""进程域 crate 边界门禁的契约测试。

`bin/cli.rs` 是独立 crate，`lib.rs` 里 `pub mod` 的任何东西它都能直接调用。
进程终止是唯一「可以对任意 PID 发信号」的能力，所以 `process_ops` 模块与
`SystemProcessSignaller` / `ProcessSignaller` / `ProcessObserver` / `ProcessTarget`
必须收为 crate 私有；CLI 只允许依赖 `operations` 里的公共 DTO。
"""
from __future__ import annotations

import re
import unittest

try: from scripts.tests.operation_contract_support import MODULE, REPOSITORY_FILES
except ModuleNotFoundError: from operation_contract_support import MODULE, REPOSITORY_FILES

RAW_CLI_PROBE = """
fn probe_boundary() {
    let target = macslim_lib::operations::ProcessTarget {
        identity: macslim_lib::operations::ProcessIdentity {
            pid: 1,
            name: String::new(),
            exe: String::new(),
            start_time: 0,
        },
        protected: false,
        whitelisted: false,
    };
    let mut signaller = macslim_lib::process_ops::SystemProcessSignaller;
    signaller.terminate_all(&[target], macslim_lib::operations::ProcessMode::Force);
}
"""


def repository() -> dict[str, str]:
    return dict(REPOSITORY_FILES)


class ProcessBoundaryTests(unittest.TestCase):
    def sources(self) -> dict[str, str]:
        return repository()

    def boundary_errors(self, sources: dict[str, str]) -> list[str]:
        return MODULE.validate_process_boundary(sources)

    def assertBoundaryFails(self, sources: dict[str, str], *needles: str) -> list[str]:
        errors = self.boundary_errors(sources)
        joined = " ".join(errors)
        self.assertTrue(
            any(needle in joined for needle in needles), f"未命中 {needles}: {errors}"
        )
        return errors

    def test_repository_boundary_is_closed(self):
        self.assertEqual(self.boundary_errors(self.sources()), [])

    def test_rejects_a_public_process_ops_module(self):
        sources = self.sources()
        sources["lib.rs"] = sources["lib.rs"].replace(
            "pub(crate) mod process_ops;", "pub mod process_ops;"
        )
        self.assertBoundaryFails(sources, "process_ops", "外部 crate")

    def test_rejects_a_public_system_process_signaller(self):
        sources = self.sources()
        sources["process_ops.rs"] = sources["process_ops.rs"].replace(
            "pub(crate) struct SystemProcessSignaller;",
            "pub struct SystemProcessSignaller;",
        )
        self.assertBoundaryFails(sources, "SystemProcessSignaller")

    def test_rejects_a_public_process_signaller_trait(self):
        sources = self.sources()
        sources["process_ops.rs"] = sources["process_ops.rs"].replace(
            "pub(crate) trait ProcessSignaller {", "pub trait ProcessSignaller {"
        )
        self.assertBoundaryFails(sources, "ProcessSignaller")

    def test_rejects_a_public_process_observer_trait(self):
        sources = self.sources()
        sources["process_ops.rs"] = sources["process_ops.rs"].replace(
            "pub(crate) trait ProcessObserver {", "pub trait ProcessObserver {"
        )
        self.assertBoundaryFails(sources, "ProcessObserver")

    def test_rejects_a_public_system_process_observer(self):
        sources = self.sources()
        sources["process_ops.rs"] = sources["process_ops.rs"].replace(
            "pub(crate) struct SystemProcessObserver {", "pub struct SystemProcessObserver {"
        )
        self.assertBoundaryFails(sources, "SystemProcessObserver")

    def test_rejects_a_public_live_process_sample(self):
        sources = self.sources()
        sources["process_ops.rs"] = sources["process_ops.rs"].replace(
            "pub(crate) struct LiveProcess {", "pub struct LiveProcess {"
        )
        self.assertBoundaryFails(sources, "LiveProcess")

    def test_rejects_a_public_kill_outcome(self):
        sources = self.sources()
        sources["process_ops.rs"] = sources["process_ops.rs"].replace(
            "pub(crate) enum KillOutcome {", "pub enum KillOutcome {"
        )
        self.assertBoundaryFails(sources, "KillOutcome")

    def test_rejects_a_public_process_target(self):
        sources = self.sources()
        sources["operation_types.rs"] = sources["operation_types.rs"].replace(
            "pub(crate) struct ProcessTarget {", "pub struct ProcessTarget {"
        )
        self.assertBoundaryFails(sources, "ProcessTarget")

    def test_rejects_a_cli_that_imports_the_process_ops_module(self):
        sources = self.sources()
        sources["cli_rs"] = sources["cli_rs"].replace(
            "use macslim_lib::operations::ProcessKillReport;",
            "use macslim_lib::process_ops::ProcessKillReport;",
        )
        self.assertBoundaryFails(sources, "cli.rs", "process_ops")

    def test_rejects_a_cli_that_reaches_process_ops_fully_qualified(self):
        sources = self.sources()
        sources["cli_rs"] = sources["cli_rs"] + RAW_CLI_PROBE
        self.assertBoundaryFails(sources, "cli.rs", "process_ops")

    def test_rejects_a_cli_that_renames_the_signaller_alias(self):
        sources = self.sources()
        sources["cli_rs"] = sources["cli_rs"].replace(
            "use macslim_lib::operations::ProcessKillReport;",
            "use macslim_lib::process_ops::{\n"
            "    ProcessKillReport, SystemProcessSignaller as Signaller,\n"
            "};",
        )
        self.assertBoundaryFails(sources, "cli.rs", "process_ops")

    def test_rejects_a_cli_that_routes_signalling_through_a_local_helper(self):
        sources = self.sources()
        sources["cli_rs"] = sources["cli_rs"].replace(
            "fn risk_label(risk: &Risk) -> &'static str {",
            "fn stop_pid(pid: u32) {\n"
            "    let _ = macslim_lib::cli_operations::scan_processes();\n"
            "    let _ = pid;\n"
            "}\n\n"
            "fn risk_label(risk: &Risk) -> &'static str {",
        )
        sources["cli_rs"] = sources["cli_rs"].replace(
            "use macslim_lib::operations::ProcessKillReport;",
            "use macslim_lib::process_ops::ProcessKillReport;",
        )
        self.assertBoundaryFails(sources, "cli.rs", "process_ops")

    def test_rejects_a_lib_that_re_exports_process_ops_publicly(self):
        sources = self.sources()
        sources["lib.rs"] = sources["lib.rs"].replace(
            "pub use scanner::read_health as scanner_read_health;",
            "pub use process_ops::SystemProcessSignaller;\n"
            "pub use scanner::read_health as scanner_read_health;",
        )
        self.assertBoundaryFails(sources, "process_ops")

    def test_ignores_a_comment_that_only_mentions_process_ops(self):
        sources = self.sources()
        sources["cli_rs"] = sources["cli_rs"].replace(
            "use macslim_lib::operations::ProcessKillReport;",
            "// 不得再从 process_ops 引入可执行能力\n"
            "use macslim_lib::operations::ProcessKillReport;",
        )
        self.assertEqual(self.boundary_errors(sources), [])

    def test_rejects_a_cli_that_imports_a_crate_private_graceful_kill(self):
        sources = self.sources()
        sources["cli_rs"] = sources["cli_rs"].replace(
            "use macslim_lib::operations::ProcessKillReport;",
            "use macslim_lib::process_ops::graceful_kill;",
        )
        self.assertBoundaryFails(sources, "cli.rs", "process_ops")

    def test_rejects_a_cli_that_reaches_an_unapproved_library_module(self):
        sources = self.sources()
        sources["cli_rs"] = sources["cli_rs"].replace(
            "use macslim_lib::scanner::Risk;",
            "use macslim_lib::uninstaller::SystemUninstaller;",
        )
        self.assertBoundaryFails(sources, "uninstaller", "未授权")

    def test_rejects_a_removed_process_ops_module_declaration(self):
        sources = self.sources()
        sources["lib.rs"] = sources["lib.rs"].replace(
            "pub(crate) mod process_ops;\n", ""
        )
        self.assertBoundaryFails(sources, "process_ops", "缺少")

    def test_rejects_a_cli_that_calls_a_real_raw_docker_function(self):
        sources = self.sources()
        sources["cli_rs"] = sources["cli_rs"].replace(
            "fn risk_label(risk: &Risk) -> &'static str {",
            "fn sweep() {\n"
            "    let mut inventory = macslim_lib::docker::DockerInventory::default();\n"
            "    let _ = macslim_lib::docker::prune_all(&mut inventory);\n"
            "}\n\n"
            "fn risk_label(risk: &Risk) -> &'static str {",
        )
        errors = MODULE.validate_cli_operation_surface(sources["cli_rs"])
        joined = " ".join(errors)
        self.assertIn("prune_all", joined)
        self.assertIn("docker", joined)

    def test_rejects_a_cli_that_reimports_a_real_raw_function_by_another_name(self):
        sources = self.sources()
        sources["cli_rs"] = sources["cli_rs"].replace(
            "use macslim_lib::operations::ProcessKillReport;",
            "use macslim_lib::uninstaller::uninstall_app as remove_bundle;",
        )
        errors = MODULE.validate_cli_operation_surface(sources["cli_rs"])
        joined = " ".join(errors)
        self.assertIn("uninstall_app", joined)
        self.assertIn("uninstaller", joined)

    def test_every_raw_cli_function_name_is_a_real_symbol_in_the_crate(self):
        sources = self.sources()
        crate = "\n".join(
            mask
            for name, source in sources.items()
            if name.endswith(".rs") or name == "cli_rs"
            for mask in (MODULE.mask_code(source, MODULE.RUST_QUOTES),)
        )
        phantom = sorted(
            name
            for name in MODULE.RAW_CLI_FUNCTIONS
            if re.search(r"\bfn\s+" + re.escape(name) + r"\b", crate) is None
        )
        self.assertEqual(phantom, [], f"RAW_CLI_FUNCTIONS 里的幻影符号: {phantom}")

    def test_the_cli_touches_none_of_the_raw_destructive_symbols(self):
        cli = MODULE.mask_code(self.sources()["cli_rs"], MODULE.RUST_QUOTES)
        touched = sorted(
            name
            for name in MODULE.RAW_CLI_FUNCTIONS
            if re.search(r"\b" + re.escape(name) + r"\b", cli) is not None
        )
        self.assertEqual(touched, [], f"CLI 仍直接触碰: {touched}")


if __name__ == "__main__":
    unittest.main()
