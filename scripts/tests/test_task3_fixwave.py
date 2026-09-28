from __future__ import annotations

import importlib.util
import json
import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts"
VALIDATOR_PATH = SCRIPTS / "verify-security-config.py"
README_PATH = SCRIPTS / "README.md"
PUBLISH_PATH = SCRIPTS / "publish-update.sh"
ASSETS_PATH = SCRIPTS / "publish_assets.py"
ARTIFACT_PATH = SCRIPTS / "updater_artifact.py"
PACKAGE_PATH = ROOT / "package.json"
SPEC = importlib.util.spec_from_file_location("task3_fixwave_validator", VALIDATOR_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)
TAURI = {
    "bundle": {"createUpdaterArtifacts": True},
    "plugins": {"updater": {"endpoints": ["https://example.test/latest.json"]}},
}


def read(path: pathlib.Path) -> str:
    return path.read_text(encoding="utf-8")


def section_bounds(text: str) -> tuple[int, int]:
    start = text.index("## Updater 发布门禁")
    end = text.find("\n## ", start + 1)
    return start, len(text) if end < 0 else end


def replace_in_documentation_section(text: str, old: str, new: str) -> str:
    start, end = section_bounds(text)
    section = text[start:end]
    return text[:start] + section.replace(old, new, 1) + text[end:]


def move_target_table_outside_section(text: str) -> str:
    start, end = section_bounds(text)
    section = text[start:end]
    rows = [
        line
        for line in section.splitlines()
        if line.startswith("| `arm`")
        or line.startswith("| `intel`")
        or line.startswith("| `universal`")
    ]
    without_rows = "\n".join(line for line in section.splitlines() if line not in rows)
    moved = text[:start] + without_rows + text[end:]
    insertion = moved.index("## 发布前检查") + len("## 发布前检查")
    return moved[:insertion] + "\n\n" + "\n".join(rows) + moved[insertion:]


def hide_call(source: str, marker: str, condition: str = "False") -> str:
    for line in source.splitlines(keepends=True):
        if marker in line:
            indentation = line[: len(line) - len(line.lstrip())]
            body = line.strip()
            replacement = f"{indentation}if {condition}:\n{indentation}    {body}"
            return source.replace(line, replacement, 1)
    raise AssertionError(marker)


class Task3FixWaveTests(unittest.TestCase):
    def documentation_errors(self, readme: str, package: dict) -> list[str]:
        return MODULE.validate_release_documentation_contract(readme, package)

    def test_documentation_rejects_commented_or_non_bash_commands(self) -> None:
        readme = read(README_PATH)
        package = json.loads(read(PACKAGE_PATH))
        rebuild = 'python3 scripts/updater_artifact.py rebuild --app "$APP_PATH" --archive "$ARCHIVE_PATH" --target "$RUST_TARGET"'
        commented = replace_in_documentation_section(readme, rebuild, f"# {rebuild}")
        removed = replace_in_documentation_section(readme, rebuild, "")
        non_bash = replace_in_documentation_section(
            readme,
            "```bash\nTARGET=arm",
            "```text\nTARGET=arm",
        )
        for mutated in (commented, removed, non_bash):
            with self.subTest(mutated=mutated[:40]):
                self.assertTrue(self.documentation_errors(mutated, package))

    def test_documentation_requires_section_scoped_markers_and_table(self) -> None:
        readme = read(README_PATH)
        package = json.loads(read(PACKAGE_PATH))
        moved_table = move_target_table_outside_section(readme)
        start, end = section_bounds(readme)
        section = readme[start:end].replace("staging", "temporary-area")
        moved_marker = readme[:start] + section + readme[end:]
        moved_marker = moved_marker.replace(
            "## 发布前检查", "staging\n\n## 发布前检查", 1
        )
        for mutated in (moved_table, moved_marker):
            with self.subTest(mutated=mutated[:40]):
                self.assertTrue(self.documentation_errors(mutated, package))

    def test_documentation_rejects_stamp_aliases_in_executable_code(self) -> None:
        readme = read(README_PATH)
        package = json.loads(read(PACKAGE_PATH))
        publish = './scripts/publish-update.sh 0.2.2 "本次更新说明" "$TARGET"'
        for command in (
            "python -m scripts.updater_artifact stamp --archive x --target y",
            "python3 -m scripts.updater_artifact stamp --archive x --target y",
            "python3 scripts/updater_artifact.py stamp --archive x --target y",
        ):
            mutated = replace_in_documentation_section(readme, publish, f"{command}\n{publish}")
            with self.subTest(command=command):
                self.assertTrue(self.documentation_errors(mutated, package))

    def test_updater_artifact_ast_rejects_alias_and_parser_variants(self) -> None:
        source = (
            "def _write_stamp(archive, target):\n    pass\n"
            "def rebuild(app, archive, target):\n    pass\n"
            "def verify_stamp(archive, target):\n    pass\n"
            "def _restore(records):\n    pass\n"
            "def select_commit_files(staging, version):\n    pass\n"
            "_restore(records)\n"
            "select_commit_files(staging, version)\n"
            "commands.add_parser('verify')\n"
            "commands.add_parser('rebuild')\n"
        )
        mutations = (
            source + "write_stamp = _write_stamp\n",
            source + "commands.add_parser('stamp')\n",
            source + "PARSER = 'st' + 'amp'\ncommands.add_parser(PARSER)\n",
            source + "commands.add_parser(\"st\" + \"amp\")\n",
        )
        for mutated in mutations:
            with self.subTest(mutated=mutated[-50:]):
                self.assertTrue(MODULE.validate_updater_artifact_contract(mutated))
        self.assertEqual(MODULE.validate_updater_artifact_contract(source), [])

    def test_package_contract_rejects_printf_only_commands(self) -> None:
        package = json.loads(read(PACKAGE_PATH))
        scripts = dict(package["scripts"])
        scripts["test:python"] = (
            "printf 'PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover "
            "-s scripts/tests -p \"test_*.py\"'"
        )
        scripts["verify"] = "printf 'bun run test:python'"
        mutated = dict(package)
        mutated["scripts"] = scripts
        errors = MODULE.validate_release_documentation_contract(read(README_PATH), mutated)
        self.assertTrue(any("test:python" in error for error in errors))
        self.assertTrue(any("verify" in error for error in errors))

    def test_publish_contract_rejects_literal_config_and_post_case_override(self) -> None:
        source = read(PUBLISH_PATH)
        literal = source.replace(
            'CONFIGURED_VERSION="$(python3 -c \'import json; print(json.load(open("src-tauri/tauri.conf.json", encoding="utf-8"))["version"])\')"',
            'CONFIGURED_VERSION="9.9.9" # src-tauri/tauri.conf.json',
        )
        override = source.replace(
            "\nesac\n",
            "\nesac\nRUST_TARGET=\"wrong-target\"\nARCH=\"wrong-arch\"\n",
            1,
        )
        for mutated in (literal, override):
            with self.subTest(mutated=mutated[:40]):
                self.assertTrue(MODULE.validate_updater_contract(TAURI, mutated))

    def test_publish_contract_rejects_commented_preflight_and_commit_before_manifest(self) -> None:
        source = read(PUBLISH_PATH)
        preflight = (
            'if [ "${MACSlim_PUBLISH_PREFLIGHT_ONLY:-}" = "1" ]; then\n'
            '  echo "==> Updater 发布预检通过"\n'
            "  exit 0\n"
            "fi\n"
        )
        commented_preflight = "\n".join(f"# {line}" for line in preflight.splitlines()) + "\n"
        without_preflight = source.replace(preflight, commented_preflight, 1)
        commit = (
            "python3 scripts/publish_assets.py commit \\\n"
            '  --staging-root "$STAGING_ROOT" \\\n'
            '  --live-root "." \\\n'
            '  --version "$VERSION"\n'
        )
        without_commit = source.replace(commit, "", 1)
        early_commit = without_commit.replace("python3 - <<'PY'\n", commit + "python3 - <<'PY'\n", 1)
        for mutated in (without_preflight, early_commit):
            with self.subTest(mutated=mutated[:40]):
                self.assertTrue(MODULE.validate_updater_contract(TAURI, mutated))

    def test_publish_assets_ast_rejects_commented_transaction_calls(self) -> None:
        source = read(ASSETS_PATH)
        mutations = (
            source.replace("os.replace(backup, destination)", "# os.replace(backup, destination)"),
            source.replace("os.fsync(temporary.fileno())", "# os.fsync(temporary.fileno())").replace(
                "os.fsync(descriptor)", "# os.fsync(descriptor)"
            ),
            source.replace("_restore(records)", "# _restore(records)"),
            source.replace("select_commit_files(staging, version)", "# select_commit_files(staging, version)"),
            source.replace("def _restore(", "def disabled_restore("),
            source.replace("def select_commit_files(", "def disabled_select_commit_files("),
        )
        for mutated in mutations:
            with self.subTest(mutated=mutated[-60:]):
                self.assertTrue(MODULE.validate_publish_assets_contract(mutated))
        self.assertEqual(MODULE.validate_publish_assets_contract(source), [])

    def test_workspace_python_cache_paths_are_absent(self) -> None:
        self.assertFalse((SCRIPTS / "__pycache__").exists())
        self.assertFalse((SCRIPTS / "tests" / "__pycache__").exists())

    def test_current_documentation_and_sources_pass(self) -> None:
        self.assertEqual(
            MODULE.validate_release_documentation_contract(
                read(README_PATH), json.loads(read(PACKAGE_PATH))
            ),
            [],
        )
        self.assertEqual(MODULE.validate_updater_artifact_contract(read(ARTIFACT_PATH)), [])
        self.assertEqual(MODULE.validate_publish_assets_contract(read(ASSETS_PATH)), [])
        self.assertEqual(MODULE.validate_updater_contract(TAURI, read(PUBLISH_PATH)), [])
    def test_documentation_requires_complete_rebuild_and_first_command_order(self) -> None:
        readme = read(README_PATH)
        package = json.loads(read(PACKAGE_PATH))
        rebuild = 'python3 scripts/updater_artifact.py rebuild --app "$APP_PATH" --archive "$ARCHIVE_PATH" --target "$RUST_TARGET"'
        incomplete = replace_in_documentation_section(
            readme, rebuild, 'python3 scripts/updater_artifact.py rebuild --app "$APP_PATH"'
        )
        early_publish = replace_in_documentation_section(
            readme,
            rebuild,
            './scripts/publish-update.sh 0.2.2 "early" "$TARGET"\n' + rebuild,
        )
        for mutated in (incomplete, early_publish):
            with self.subTest(mutated=mutated[:40]):
                self.assertTrue(self.documentation_errors(mutated, package))

    def test_updater_artifact_ast_rejects_keyword_and_dynamic_aliases(self) -> None:
        source = (
            "def _write_stamp(archive, target):\n    pass\n"
            "def rebuild(app, archive, target):\n    pass\n"
            "def verify_stamp(archive, target):\n    pass\n"
            "commands.add_parser('verify')\n"
            "commands.add_parser('rebuild')\n"
        )
        mutations = (
            source + "commands.add_parser(name='stamp')\n",
            source + 'globals()["write_stamp"] = _write_stamp\n',
            source + 'setattr(module, "write_stamp", _write_stamp)\n',
            source + "module.write_stamp = _write_stamp\n",
            source + 'mapping["write_stamp"] = _write_stamp\n',
        )
        for mutated in mutations:
            with self.subTest(mutated=mutated[-60:]):
                self.assertTrue(MODULE.validate_updater_artifact_contract(mutated))

    def test_documentation_rejects_absolute_interpreter_stamp_aliases(self) -> None:
        readme = read(README_PATH)
        package = json.loads(read(PACKAGE_PATH))
        publish = './scripts/publish-update.sh 0.2.2 "本次更新说明" "$TARGET"'
        for command in (
            "/usr/bin/python3 scripts/updater_artifact.py stamp --archive x --target y",
            "/opt/homebrew/bin/python3 scripts/updater_artifact.py stamp --archive x --target y",
            "python -m scripts.updater_artifact stamp --archive x --target y",
        ):
            mutated = replace_in_documentation_section(readme, publish, f"{command}\n{publish}")
            with self.subTest(command=command):
                self.assertTrue(self.documentation_errors(mutated, package))

    def test_package_contract_rejects_failure_swallowing_separators(self) -> None:
        package = json.loads(read(PACKAGE_PATH))
        for separator in (" || true", "; true", " & true"):
            scripts = dict(package["scripts"])
            scripts["verify"] = scripts["verify"] + separator
            mutated = dict(package)
            mutated["scripts"] = scripts
            errors = MODULE.validate_release_documentation_contract(read(README_PATH), mutated)
            self.assertTrue(any("verify" in error for error in errors), separator)

    def test_publish_contract_rejects_config_semantics_and_dynamic_overrides(self) -> None:
        source = read(PUBLISH_PATH)
        config_line = next(line for line in source.splitlines() if line.startswith("CONFIGURED_VERSION="))
        second_assignment = source.replace(config_line, config_line + '\nCONFIGURED_VERSION="9.9.9"', 1)
        semantic_config = source.replace(
            config_line,
            'CONFIGURED_VERSION="$(python3 -c \'print("src-tauri/tauri.conf.json")\')"',
        )
        mutations = [second_assignment, semantic_config]
        for override in (
            'declare RUST_TARGET="wrong"',
            'RUST_TARGET+="wrong"',
            'eval "RUST_TARGET=wrong"',
            'export ARCH="wrong"',
        ):
            mutations.append(source.replace("\nesac\n", "\nesac\n" + override + "\n", 1))
        mutations.append(
            source.replace('    ARCH="aarch64"\n', '    ARCH="aarch64"\n    RUST_TARGET+="wrong"\n', 1)
        )
        for mutated in mutations:
            with self.subTest(mutated=mutated[:50]):
                self.assertTrue(MODULE.validate_updater_contract(TAURI, mutated))

    def test_publish_contract_rejects_preflight_side_effects_and_non_toplevel_branch(self) -> None:
        source = read(PUBLISH_PATH)
        preflight = (
            'if [ "${MACSlim_PUBLISH_PREFLIGHT_ONLY:-}" = "1" ]; then\n'
            '  echo "==> Updater 发布预检通过"\n'
            "  exit 0\n"
            "fi\n"
        )
        with_mktemp = source.replace(
            preflight,
            preflight.replace('  echo "==> Updater 发布预检通过"', '  mktemp -d\n  echo "==> Updater 发布预检通过"'),
            1,
        )
        without_preflight = source.replace(preflight, "", 1)
        function_lines = "".join(f"  {line}\n" for line in preflight.splitlines())
        insertion = without_preflight.index('STAGING_ROOT="$(mktemp -d')
        nested = (
            without_preflight[:insertion]
            + "preflight_only() {\n"
            + function_lines
            + "}\n\n"
            + without_preflight[insertion:]
        )
        for mutated in (with_mktemp, nested):
            with self.subTest(mutated=mutated[:50]):
                self.assertTrue(MODULE.validate_updater_contract(TAURI, mutated))

    def test_publish_contract_rejects_direct_manifest_writes_before_commit(self) -> None:
        source = read(PUBLISH_PATH)
        commit = (
            "python3 scripts/publish_assets.py commit \\\n"
            '  --staging-root "$STAGING_ROOT" \\\n'
            '  --live-root "." \\\n'
            '  --version "$VERSION"\n'
        )
        mutated = source.replace(commit, "", 1)
        mutated = mutated.replace(
            '    write_manifest(update_dir / "latest.json")\n',
            '    path.write_text("{}", encoding="utf-8")\n',
        )
        mutated = mutated.replace(
            '        write_manifest(update_dir / platform / f"{old_version}.json")\n',
            '        path.write_text("{}", encoding="utf-8")\n',
        )
        mutated = mutated.replace('PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")\n', commit + 'PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")\n', 1)
        errors = MODULE.validate_updater_contract(TAURI, mutated)
        self.assertTrue(any("manifest" in error or "commit" in error for error in errors))

    def test_transaction_ast_rejects_renamed_root_and_dead_required_calls(self) -> None:
        source = read(ASSETS_PATH)
        mutations = [source.replace("def commit_staged_assets(", "def renamed_commit_staged_assets(", 1)]
        for marker in (
            "select_commit_files(staging, version)",
            "_copy_to_sibling(source, destination)",
            "_backup_existing(destination, backup)",
            "_restore(records)",
            "replace_func(temporary, destination)",
        ):
            mutations.append(hide_call(source, marker))
        for mutated in mutations:
            with self.subTest(mutated=mutated[-70:]):
                self.assertTrue(MODULE.validate_publish_assets_contract(mutated))
        self.assertEqual(MODULE.validate_publish_assets_contract(source), [])
    def test_package_contract_requires_exact_python_command_and_live_verify_segment(self) -> None:
        package = json.loads(read(PACKAGE_PATH))
        for suffix in (" || true", " ; true"):
            scripts = dict(package["scripts"])
            scripts["test:python"] = scripts["test:python"] + suffix
            mutated = dict(package)
            mutated["scripts"] = scripts
            errors = MODULE.validate_release_documentation_contract(read(README_PATH), mutated)
            self.assertTrue(any("test:python" in error for error in errors), suffix)
        for verify in ("false && bun run test:python", "if false; then bun run test:python; fi"):
            scripts = dict(package["scripts"])
            scripts["verify"] = verify
            mutated = dict(package)
            mutated["scripts"] = scripts
            errors = MODULE.validate_release_documentation_contract(read(README_PATH), mutated)
            self.assertTrue(any("verify" in error for error in errors), verify)

    def test_publish_contract_rejects_semantic_config_mismatch_and_shell_overrides(self) -> None:
        source = read(PUBLISH_PATH)
        config_line = next(line for line in source.splitlines() if line.startswith("CONFIGURED_VERSION="))
        wrong_reader = source.replace(
            config_line,
            'CONFIGURED_VERSION="$(python3 -c \'import json; print(json.loads(open("other.json"))["version"]); print("src-tauri/tauri.conf.json")\')"',
        )
        mutations = [wrong_reader]
        for override in (
            'printf -v RUST_TARGET wrong',
            'read RUST_TARGET value',
            'printf -v ARCH wrong',
            'read ARCH value',
        ):
            mutations.append(source.replace("\nesac\n", "\nesac\n" + override + "\n", 1))
        for mutated in mutations:
            with self.subTest(mutated=mutated[:60]):
                self.assertTrue(MODULE.validate_updater_contract(TAURI, mutated))

    def test_publish_contract_rejects_command_substitution_and_nested_false_preflight(self) -> None:
        source = read(PUBLISH_PATH)
        preflight = (
            'if [ "${MACSlim_PUBLISH_PREFLIGHT_ONLY:-}" = "1" ]; then\n'
            '  echo "==> Updater 发布预检通过"\n'
            "  exit 0\n"
            "fi\n"
        )
        command_mktemp = source.replace(
            preflight,
            preflight.replace('  echo "==> Updater 发布预检通过"', '  command mktemp -d\n  echo "==> Updater 发布预检通过"'),
            1,
        )
        command_substitution = source.replace(
            preflight,
            preflight.replace('  echo "==> Updater 发布预检通过"', '  echo "$(touch /tmp/preflight-side-effect)"\n  echo "==> Updater 发布预检通过"'),
            1,
        )
        nested_false = source.replace(
            preflight,
            preflight.replace("  exit 0", "  if false; then\n    exit 0\n  fi\n  exit 0"),
            1,
        )
        for mutated in (command_mktemp, command_substitution, nested_false):
            with self.subTest(mutated=mutated[:60]):
                self.assertTrue(MODULE.validate_updater_contract(TAURI, mutated))

    def test_transaction_ast_rejects_if_zero_dead_calls(self) -> None:
        source = read(ASSETS_PATH)
        mutated = hide_call(source, "select_commit_files(staging, version)", "0")
        self.assertTrue(MODULE.validate_publish_assets_contract(mutated))
        self.assertEqual(MODULE.validate_publish_assets_contract(source), [])
    def test_preflight_rejects_shell_wrapper_command(self) -> None:
        source = read(PUBLISH_PATH)
        preflight = (
            'if [ "${MACSlim_PUBLISH_PREFLIGHT_ONLY:-}" = "1" ]; then\n'
            '  echo "==> Updater 发布预检通过"\n'
            "  exit 0\n"
            "fi\n"
        )
        mutated = source.replace(
            preflight,
            preflight.replace('  echo "==> Updater 发布预检通过"', "  sh -c 'touch /tmp/preflight-side-effect'\n  echo \"==> Updater 发布预检通过\""),
            1,
        )
        self.assertTrue(MODULE.validate_updater_contract(TAURI, mutated))

    def test_preflight_rejects_mv_command(self) -> None:
        source = read(PUBLISH_PATH)
        preflight = (
            'if [ "${MACSlim_PUBLISH_PREFLIGHT_ONLY:-}" = "1" ]; then\n'
            '  echo "==> Updater 发布预检通过"\n'
            "  exit 0\n"
            "fi\n"
        )
        mutated = source.replace(
            preflight,
            preflight.replace('  echo "==> Updater 发布预检通过"', '  mv /tmp/a /tmp/b\n  echo "==> Updater 发布预检通过"'),
            1,
        )
        self.assertTrue(MODULE.validate_updater_contract(TAURI, mutated))

    def test_preflight_rejects_install_command(self) -> None:
        source = read(PUBLISH_PATH)
        preflight = (
            'if [ "${MACSlim_PUBLISH_PREFLIGHT_ONLY:-}" = "1" ]; then\n'
            '  echo "==> Updater 发布预检通过"\n'
            "  exit 0\n"
            "fi\n"
        )
        mutated = source.replace(
            preflight,
            preflight.replace('  echo "==> Updater 发布预检通过"', '  install /tmp/a /tmp/b\n  echo "==> Updater 发布预检通过"'),
            1,
        )
        self.assertTrue(MODULE.validate_updater_contract(TAURI, mutated))
    def test_preflight_rejects_unquoted_output_redirect(self) -> None:
        source = read(PUBLISH_PATH)
        preflight = (
            'if [ "${MACSlim_PUBLISH_PREFLIGHT_ONLY:-}" = "1" ]; then\n'
            '  echo "==> Updater 发布预检通过"\n'
            "  exit 0\n"
            "fi\n"
        )
        mutated = source.replace(
            preflight,
            preflight.replace('  echo "==> Updater 发布预检通过"', "  echo safe>/tmp/preflight-side-effect\n  echo \"==> Updater 发布预检通过\""),
            1,
        )
        self.assertTrue(MODULE.validate_updater_contract(TAURI, mutated))

    def test_preflight_rejects_attached_output_redirect_after_quote(self) -> None:
        source = read(PUBLISH_PATH)
        preflight = (
            'if [ "${MACSlim_PUBLISH_PREFLIGHT_ONLY:-}" = "1" ]; then\n'
            '  echo "==> Updater 发布预检通过"\n'
            "  exit 0\n"
            "fi\n"
        )
        mutated = source.replace(
            preflight,
            preflight.replace('  echo "==> Updater 发布预检通过"', '  echo "safe">/tmp/preflight-side-effect\n  echo "==> Updater 发布预检通过"'),
            1,
        )
        self.assertTrue(MODULE.validate_updater_contract(TAURI, mutated))


if __name__ == "__main__":
    unittest.main()
