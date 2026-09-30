from __future__ import annotations

import ast
import importlib.util
import json
import os
import pathlib
import plistlib
import re
import subprocess
import sys
import tempfile
import unittest

SCRIPT_PATH = pathlib.Path(__file__).resolve().parents[1] / "verify-security-config.py"
SCRIPTS_DIR = SCRIPT_PATH.parent
SPEC = importlib.util.spec_from_file_location("verify_security_config", SCRIPT_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


VALID_CSP = (
    "default-src 'self' ipc: http://ipc.localhost; "
    "connect-src ipc: http://ipc.localhost; "
    "img-src 'self' asset: http://asset.localhost blob: data:; "
    "style-src 'self' 'unsafe-inline'; script-src 'self'; "
    "object-src 'none'; base-uri 'self'; frame-src 'none'; frame-ancestors 'none'"
)


def contract_module():
    return MODULE.operation_contract


def contract_sources(root):
    return contract_module().read_contract_sources(root)


class SecurityConfigTests(unittest.TestCase):
    def test_accepts_production_csp(self):
        self.assertEqual(MODULE.validate_csp(VALID_CSP), [])

    def test_rejects_unsafe_eval(self):
        errors = MODULE.validate_csp("default-src 'self'; script-src 'self' 'unsafe-eval'")
        self.assertTrue(any("unsafe-eval" in error for error in errors))

    def test_rejects_capability_drift(self):
        errors = MODULE.validate_capabilities({"permissions": ["core:default"]})
        self.assertTrue(any("capability" in error for error in errors))

    def test_current_capability_allowlist_has_exactly_fourteen_unique_permissions(self):
        # 14 = 原 13 + `opener:allow-open-url`。多这一条只为 FDA 引导：
        # 点「打开系统设置」要跳 `x-apple.systempreferences:` 深链，MAS 版
        # 没有它用户就只能自己去找设置页。**只允许 open-url**，不放宽成整组
        # opener 权限。
        capability_path = SCRIPT_PATH.parents[1] / "src-tauri/capabilities/default.json"
        capabilities = json.loads(capability_path.read_text(encoding="utf-8"))
        permissions = capabilities["permissions"]
        self.assertEqual(len(MODULE.EXPECTED_CAPABILITIES), 14)
        self.assertEqual(len(permissions), 14)
        self.assertEqual(len(permissions), len(set(permissions)))
        self.assertEqual(MODULE.validate_capabilities(capabilities), [])

    def test_rejects_duplicate_capability(self):
        permissions = sorted(MODULE.EXPECTED_CAPABILITIES)
        errors = MODULE.validate_capabilities({"permissions": [*permissions, permissions[0]]})
        self.assertTrue(any("重复" in error for error in errors))

    def test_updater_config_uses_latest_endpoint_and_artifact_generation(self):
        config_path = SCRIPT_PATH.parents[1] / "src-tauri/tauri.conf.json"
        config = json.loads(config_path.read_text(encoding="utf-8"))
        endpoints = config["plugins"]["updater"]["endpoints"]
        self.assertIs(config["bundle"]["createUpdaterArtifacts"], True)
        self.assertEqual(len(endpoints), 1)
        self.assertTrue(endpoints[0].endswith("/latest.json"))
        self.assertNotIn("{{current_version}}", endpoints[0])

    def test_release_and_sign_select_exact_versioned_dmg(self):
        for name in ("release.sh", "sign.sh"):
            with self.subTest(script=name):
                content = (SCRIPTS_DIR / name).read_text(encoding="utf-8")
                self.assertIn("src-tauri/tauri.conf.json", content)
                self.assertIn('RUST_TARGET="aarch64-apple-darwin"', content)
                self.assertIn('ARCH="aarch64"', content)
                self.assertIn('RUST_TARGET="x86_64-apple-darwin"', content)
                self.assertIn('ARCH="x64"', content)
                self.assertIn('RUST_TARGET="universal-apple-darwin"', content)
                self.assertIn('ARCH="universal"', content)
                self.assertIn('DMG_PATH="src-tauri/target/${RUST_TARGET}/release/bundle/dmg/MacSlim_${VERSION}_${ARCH}.dmg"', content)
                self.assertNotIn("DMG_PATHS", content)
                self.assertNotIn("MacSlim_*.dmg", content)
                self.assertNotIn("head", content)
                self.assertIn('$TIMESTAMP_FLAG', (SCRIPTS_DIR / "sign.sh").read_text(encoding="utf-8"))

    def test_release_contract_rejects_dmg_glob(self):
        for name in ("release.sh", "sign.sh"):
            with self.subTest(script=name):
                content = (SCRIPTS_DIR / name).read_text(encoding="utf-8")
                content = content.replace(
                    'DMG_PATH="src-tauri/target/${RUST_TARGET}/release/bundle/dmg/MacSlim_${VERSION}_${ARCH}.dmg"',
                    'DMG_PATHS=(src-tauri/target/${RUST_TARGET}/release/bundle/dmg/MacSlim_*.dmg)\nDMG_PATH="${DMG_PATHS[0]}"',
                )
                errors = MODULE.validate_release_contracts({f"scripts/{name}": content})
                self.assertTrue(any("DMG" in error or "精确" in error for error in errors))

    def test_controlled_info_plist_declares_apple_events_usage(self):
        info_path = SCRIPT_PATH.parents[1] / "src-tauri/Info.plist"
        self.assertTrue(info_path.is_file())
        info = plistlib.loads(info_path.read_bytes())
        self.assertIsInstance(info, dict)
        self.assertIsInstance(info.get("NSAppleEventsUsageDescription"), str)
        self.assertTrue(info["NSAppleEventsUsageDescription"].strip())

    def test_final_debug_app_info_plist(self):
        app_path = os.environ.get("MACSlim_DEBUG_APP_PATH")
        if not app_path:
            self.skipTest("设置 MACSlim_DEBUG_APP_PATH 后检查最终 debug app")
        info_path = pathlib.Path(app_path) / "Contents/Info.plist"
        self.assertTrue(info_path.is_file())
        info = plistlib.loads(info_path.read_bytes())
        self.assertIsInstance(info, dict)
        self.assertTrue(info.get("NSAppleEventsUsageDescription", "").strip())
        config = json.loads(
            (SCRIPT_PATH.parents[1] / "src-tauri/tauri.conf.json").read_text(encoding="utf-8")
        )
        self.assertEqual(info.get("CFBundleShortVersionString"), config["version"])

    def test_tauri_info_plist_binding_removes_sign_mutation(self):
        config_path = SCRIPT_PATH.parents[1] / "src-tauri/tauri.conf.json"
        config = json.loads(config_path.read_text(encoding="utf-8"))
        macos = config["bundle"]["macOS"]
        self.assertEqual(macos.get("infoPlist"), "Info.plist")
        sign = (SCRIPTS_DIR / "sign.sh").read_text(encoding="utf-8")
        self.assertNotIn("PlistBuddy", sign)
        self.assertNotIn("注入 Info.plist", sign)

    def test_unsigned_ci_build_skips_updater_signing(self):
        ci = (SCRIPT_PATH.parents[1] / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.assertIn(
            "bun run tauri build --target aarch64-apple-darwin --no-sign",
            ci,
        )

    def test_frontend_gate_excludes_rust_and_ci_uses_it(self):
        root = SCRIPT_PATH.parents[1]
        package = json.loads((root / "package.json").read_text(encoding="utf-8"))
        frontend = package["scripts"]["verify:frontend"]
        self.assertNotIn("test:rust", frontend)
        self.assertIn("bun run lint", frontend)
        self.assertIn("bun run security:check", frontend)
        ci = (root / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.assertIn("run: bun run verify:frontend", ci)
        self.assertNotIn("run: bun run verify\n", ci)

    def test_scanner_documentation_matches_dynamic_idle_threshold(self):
        source = (SCRIPT_PATH.parents[1] / "src-tauri/src/scanner.rs").read_text(encoding="utf-8")
        self.assertNotIn("100MB", source)
        self.assertIn("idle_memory_threshold(total_mem_mb)", source)

    def test_release_readme_has_continuous_workflow_numbers(self):
        content = (SCRIPTS_DIR / "README.md").read_text(encoding="utf-8")
        numbers = [int(value) for value in re.findall(r"^# (\d+)\.", content, re.MULTILINE)]
        self.assertEqual(numbers, [1, 2, 3, 4])

    def test_task3_release_documentation_contract(self):
        readme = (SCRIPTS_DIR / "README.md").read_text(encoding="utf-8")
        package = json.loads(
            (SCRIPT_PATH.parents[1] / "package.json").read_text(encoding="utf-8")
        )
        self.assertEqual(MODULE.validate_release_documentation_contract(readme, package), [])

    def test_task3_documentation_rejects_public_stamp_and_manual_artifact_changes(self):
        readme = (SCRIPTS_DIR / "README.md").read_text(encoding="utf-8")
        package = json.loads(
            (SCRIPT_PATH.parents[1] / "package.json").read_text(encoding="utf-8")
        )
        mutations = (
            readme.replace(
                "updater_artifact.py rebuild --app",
                "updater_artifact.py stamp --app",
            ),
            readme.replace("不得手工复制旧 tar", "可以复制旧 tar"),
            readme.replace("不得手工修改 manifest", "可以修改 manifest"),
        )
        for mutated in mutations:
            with self.subTest(mutated=mutated[:40]):
                self.assertTrue(
                    MODULE.validate_release_documentation_contract(mutated, package)
                )

    def test_task3_documentation_requires_staging_and_python_discovery(self):
        readme = (SCRIPTS_DIR / "README.md").read_text(encoding="utf-8")
        package = json.loads(
            (SCRIPT_PATH.parents[1] / "package.json").read_text(encoding="utf-8")
        )
        for marker in (
            "build-stamp.json",
            "staging",
            "publish_assets.py commit",
            "tauri.conf.json",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "universal-apple-darwin",
        ):
            with self.subTest(marker=marker):
                self.assertTrue(
                    MODULE.validate_release_documentation_contract(
                        readme.replace(marker, ""), package
                    ),
                    marker,
                )
        scripts = dict(package["scripts"])
        scripts["test:python"] = scripts["test:python"].replace(
            "PYTHONDONTWRITEBYTECODE=1 ", ""
        )
        scripts["verify"] = scripts["verify"].replace(" && bun run test:python", "")
        broken = dict(package)
        broken["scripts"] = scripts
        errors = MODULE.validate_release_documentation_contract(readme, broken)
        self.assertTrue(any("缓存" in error for error in errors))
        self.assertTrue(any("verify" in error for error in errors))

    def test_opaque_public_window_contract_has_no_native_vibrancy(self):
        root = SCRIPT_PATH.parents[1]
        config = json.loads((root / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
        cargo = (root / "src-tauri/Cargo.toml").read_text(encoding="utf-8")
        lib = (root / "src-tauri/src/lib.rs").read_text(encoding="utf-8")
        self.assertIs(config["app"]["windows"][0].get("transparent"), False)
        self.assertIs(config["app"].get("macOSPrivateApi"), False)
        self.assertNotIn("macos-private-api", cargo)
        self.assertNotIn("window-vibrancy", cargo)
        self.assertNotIn("window-vibrancy", lib)
        self.assertNotIn("apply_vibrancy", lib)
        self.assertNotIn("NSVisualEffectMaterial::Sidebar", lib)
        self.assertEqual(MODULE.validate_public_api_contract(config, cargo, lib), [])

    def test_public_window_material_stays_opaque_and_grant_free(self):
        root = SCRIPT_PATH.parents[1]
        config = json.loads((root / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
        cargo = (root / "src-tauri/Cargo.toml").read_text(encoding="utf-8")
        lib = (root / "src-tauri/src/lib.rs").read_text(encoding="utf-8")
        for window in config["app"]["windows"]:
            self.assertIs(window.get("transparent"), False)
            self.assertNotIn("windowEffects", window)
            self.assertNotIn("window-vibrancy", window)
        self.assertIs(config["app"].get("macOSPrivateApi"), False)
        for source in (cargo, lib):
            for marker in (
                "macos-private-api",
                "window-vibrancy",
                "window_vibrancy",
                "apply_vibrancy",
                "NSVisualEffectMaterial",
            ):
                self.assertNotIn(marker, source)

    def test_capability_allowlist_is_not_widened_by_the_operation_broker(self):
        root = SCRIPT_PATH.parents[1]
        capabilities = json.loads(
            (root / "src-tauri/capabilities/default.json").read_text(encoding="utf-8")
        )
        for permission in MODULE.operation_contract.EXPECTED_IPC_COMMANDS:
            with self.subTest(command=permission):
                self.assertNotIn(f"allow-{permission}", capabilities["permissions"])
        self.assertEqual(len(capabilities["permissions"]), 14)
        self.assertEqual(MODULE.validate_capabilities(capabilities), [])

    def test_operation_broker_ipc_surface_is_exactly_eighteen_trusted_commands(self):
        # 17 = 原 16 + `get_build_flavor`（纯元数据：下发 developer_id / mas，
        # 让前端在 App Store 版里藏掉沙箱里做不到的「终止进程」入口）。
        # 破坏性面仍必须精确等于 prepare + execute 两条 —— 下面单独断言。
        contract = MODULE.operation_contract
        root = SCRIPT_PATH.parents[1]
        lib = (root / "src-tauri/src/lib.rs").read_text(encoding="utf-8")
        commands = contract.rust_invoke_handler_commands(lib)
        self.assertEqual(commands, list(contract.EXPECTED_IPC_COMMANDS))
        self.assertEqual(len(commands), 18)
        self.assertEqual(sorted(contract.rust_declared_commands(lib)), sorted(commands))
        self.assertEqual(
            [name for name in commands if name in contract.DESTRUCTIVE_IPC_COMMANDS],
            ["prepare_operation", "execute_operation"],
        )
        self.assertEqual(contract.validate_ipc_command_surface(lib), [])
        self.assertFalse(contract.RAW_IPC_COMMANDS & set(commands))

    def test_operation_broker_contract_covers_frontend_cli_and_history_labels(self):
        root = SCRIPT_PATH.parents[1]
        self.assertEqual(MODULE.validate_operation_broker_contract(root), [])
        sources = contract_sources(root)
        self.assertEqual(contract_module().validate_frontend_operation_surface(sources), [])
        self.assertEqual(
            contract_module().validate_cli_operation_surface(sources["cli_rs"]), []
        )
        self.assertEqual(
            contract_module().validate_cli_fail_closed(sources["cli_operations.rs"]), []
        )
        self.assertEqual(
            contract_module().validate_random_id_dependency(sources["Cargo.toml"]), []
        )

    def test_public_window_contract_rejects_transparency_and_vibrancy(self):
        base = {"app": {"windows": [{"transparent": False}], "macOSPrivateApi": False}}
        self.assertEqual(
            MODULE.validate_public_api_contract(base, "[dependencies]", "fn run() {}"),
            [],
        )
        invalid_cases = (
            ({"app": {"windows": [{"transparent": True}], "macOSPrivateApi": False}}, "[dependencies]", "fn run() {}", "transparent"),
            ({"app": {"windows": [{"transparent": False}], "macOSPrivateApi": True}}, "[dependencies]", "fn run() {}", "macOSPrivateApi"),
            (base, "[dependencies]\nwindow-vibrancy = \"0.6\"", "fn run() {}", "window-vibrancy"),
            (base, "[dependencies]", "fn run() { apply_vibrancy(); }", "apply_vibrancy"),
        )
        for tauri, cargo, source, marker in invalid_cases:
            with self.subTest(marker=marker):
                errors = MODULE.validate_public_api_contract(tauri, cargo, source)
                self.assertTrue(any(marker in error for error in errors), errors)

    def test_rejects_updater_contract_drift(self):
        config = {
            "bundle": {},
            "plugins": {
                "updater": {
                    "endpoints": [
                        "https://example.test/updates/{{target}}/{{current_version}}.json"
                    ]
                }
            },
        }
        errors = MODULE.validate_updater_contract(config, "DMG")
        self.assertTrue(any("updater" in error for error in errors))

    def test_rejects_extra_entitlement(self):
        errors = MODULE.validate_entitlements(
            {"com.apple.security.cs.disable-library-validation": True}
        )
        self.assertTrue(any("entitlement" in error for error in errors))

    def test_rejects_inline_secret_fallback(self):
        errors = MODULE.validate_secret_fallbacks(
            {
                "scripts/publish-update.sh": (
                    'export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-fallback}"\n'
                )
            }
        )
        self.assertTrue(any("publish-update.sh:1" in error for error in errors))

    def test_rejects_no_scheme_remote_script(self):
        errors = MODULE.validate_csp("default-src 'self'; script-src 'self' evil.example")
        self.assertTrue(any("script-src" in error for error in errors))

    def test_rejects_protocol_relative_source(self):
        errors = MODULE.validate_csp("default-src 'self'; img-src 'self' //evil.example")
        self.assertTrue(any("远程" in error for error in errors))

    def test_rejects_allowed_host_in_script_src(self):
        errors = MODULE.validate_csp(
            "default-src 'self'; script-src 'self' http://ipc.localhost"
        )
        self.assertTrue(any("script-src" in error for error in errors))

    def test_rejects_wildcard(self):
        errors = MODULE.validate_csp("default-src 'self' *; script-src 'self'")
        self.assertTrue(any("通配符" in error for error in errors))

    def test_rejects_script_src_unsafe_inline(self):
        errors = MODULE.validate_csp("default-src 'self'; script-src 'self' 'unsafe-inline'")
        self.assertTrue(any("unsafe-inline" in error for error in errors))

    def test_accepts_tauri_script_mechanism(self):
        self.assertEqual(MODULE.validate_csp("default-src 'self'; script-src 'self' ipc:"), [])

    def test_malformed_json_error_is_sanitized(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "malformed.json"
            path.write_text('{"secret": "DO_NOT_PRINT"', encoding="utf-8")
            with self.assertRaises(ValueError) as context:
                MODULE._read_json(path)
        message = str(context.exception)
        self.assertEqual(message, "无法读取 JSON 配置: malformed.json")
        self.assertNotIn("DO_NOT_PRINT", message)

    def test_malformed_plist_error_is_sanitized(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "malformed.plist"
            path.write_text("<plist><dict><key>DO_NOT_PRINT</key>", encoding="utf-8")
            with self.assertRaises(ValueError) as context:
                MODULE._read_plist(path)
        message = str(context.exception)
        self.assertEqual(message, "无法读取 plist 配置: malformed.plist")
        self.assertNotIn("DO_NOT_PRINT", message)

    def test_malformed_url_error_is_sanitized(self):
        errors = MODULE.validate_csp(
            "default-src 'self'; img-src 'self' http://ipc.localhost:bad"
        )
        message = " ".join(errors)
        self.assertTrue(any("URL" in error for error in errors))
        self.assertNotIn("ipc.localhost", message)

    def test_release_contract_validator_is_defined_once(self):
        tree = ast.parse(SCRIPT_PATH.read_text(encoding="utf-8"))
        definitions = [
            node.name
            for node in tree.body
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        ]
        self.assertEqual(definitions.count("validate_release_contracts"), 1)

    def test_rejects_stapling_before_notary_submission(self):
        content = """PROFILE_NAME="macslim-notary"
ditto -c -k --sequesterRsrc --keepParent "$APP_PATH" "$APP_ARCHIVE_PATH"
xcrun stapler staple "$APP_PATH"
xcrun stapler staple "$DMG_PATH"
xcrun notarytool submit "$APP_ARCHIVE_PATH" --keychain-profile "$PROFILE_NAME" --wait
xcrun notarytool submit "$DMG_PATH" --keychain-profile "$PROFILE_NAME" --wait
xcrun stapler validate "$APP_PATH"
xcrun stapler validate "$DMG_PATH"
"""
        for script_name in ("release.sh", "sign.sh"):
            with self.subTest(script=script_name):
                errors = MODULE.validate_release_contracts(
                    {f"scripts/{script_name}": content}
                )
                self.assertTrue(
                    any("公证提交" in error and "staple" in error for error in errors)
                )

    def test_current_release_and_sign_satisfy_load_bearing_contract(self):
        files = {
            f"scripts/{name}": (SCRIPTS_DIR / name).read_text(encoding="utf-8")
            for name in ("release.sh", "sign.sh")
        }
        self.assertEqual(MODULE.validate_release_contracts(files), [])

    def test_release_and_sign_require_post_notarization_rebuild(self):
        rebuild = (
            'python3 scripts/updater_artifact.py rebuild \\\n'
            '  --app "$APP_PATH" \\\n'
            '  --archive "$UPDATER_PATH" \\\n'
            '  --target "$RUST_TARGET"\n'
        )
        final_check = 'spctl -a -t exec -vv "$APP_PATH"\n'
        for script_name in ("release.sh", "sign.sh"):
            content = (SCRIPTS_DIR / script_name).read_text(encoding="utf-8")
            missing = content.replace(rebuild, "")
            errors = MODULE.validate_release_contracts(
                {f"scripts/{script_name}": missing}
            )
            self.assertTrue(any("rebuild" in error for error in errors))
            early = missing.replace(final_check, rebuild + final_check)
            errors = MODULE.validate_release_contracts(
                {f"scripts/{script_name}": early}
            )
            self.assertTrue(any("rebuild" in error for error in errors))

    def test_requires_signing_identity_export_before_release_build(self):
        files = {
            f"scripts/{name}": (SCRIPTS_DIR / name).read_text(encoding="utf-8")
            for name in ("release.sh", "sign.sh")
        }
        release = files["scripts/release.sh"]
        export_line = 'export APPLE_SIGNING_IDENTITY="$SIGNING_IDENTITY"'
        build_line = 'bun run tauri build --target "$RUST_TARGET"'
        if export_line in release and build_line in release:
            release = release.replace(f"{export_line}\n", "", 1)
            release = release.replace(build_line, f"{build_line}\n{export_line}", 1)
        files["scripts/release.sh"] = release
        errors = MODULE.validate_release_contracts(files)
        self.assertTrue(
            any("build 前" in error and "APPLE_SIGNING_IDENTITY" in error for error in errors)
        )

    def test_requires_profile_bound_app_and_dmg_notary_submission(self):
        app_submit = (
            'xcrun notarytool submit "$APP_ARCHIVE_PATH" '
            '--keychain-profile "$PROFILE_NAME" --wait'
        )
        dmg_submit = (
            'xcrun notarytool submit "$DMG_PATH" '
            '--keychain-profile "$PROFILE_NAME" --wait'
        )
        archive = (
            'ditto -c -k --sequesterRsrc --keepParent "$APP_PATH" '
            '"$APP_ARCHIVE_PATH"'
        )
        for script_name in ("release.sh", "sign.sh"):
            for missing_label, content in (
                (".app", f'PROFILE_NAME="macslim-notary"\n{archive}\n{dmg_submit}\n'),
                ("DMG", f'PROFILE_NAME="macslim-notary"\n{archive}\n{app_submit}\n'),
            ):
                with self.subTest(script=script_name, missing=missing_label):
                    errors = MODULE.validate_release_contracts(
                        {f"scripts/{script_name}": content}
                    )
                    self.assertTrue(
                        any(
                            missing_label in error and "公证提交" in error
                            for error in errors
                        )
                    )

    def test_rejects_hardcoded_notary_profile(self):
        content = (
            'PROFILE_NAME="macslim-notary"\n'
            'xcrun notarytool submit "$APP_ARCHIVE_PATH" '
            '--keychain-profile hardcoded --wait\n'
            'xcrun notarytool submit "$DMG_PATH" '
            '--keychain-profile hardcoded --wait\n'
        )
        errors = MODULE.validate_release_contracts({"scripts/release.sh": content})
        self.assertTrue(any("$PROFILE_NAME" in error for error in errors))

    def test_rejects_wrapped_or_compact_notary_history(self):
        variants = (
            """if ! xcrun notarytool history --keychain-profile "$PROFILE_NAME"; then
exit 1
fi
""",
            'if ! xcrun notarytool history --keychain-profile "$PROFILE_NAME";then exit 1;fi\n',
            'if xcrun notarytool history --keychain-profile "$PROFILE_NAME"; then exit 1; fi\n',
            'true; xcrun notarytool history --keychain-profile "$PROFILE_NAME"\n',
        )
        for script_name in ("release.sh", "sign.sh"):
            for content in variants:
                with self.subTest(script=script_name, content=content):
                    errors = MODULE.validate_release_contracts(
                        {f"scripts/{script_name}": f"set -euo pipefail\n{content}"}
                    )
                    self.assertTrue(
                        any(
                            "notary history" in error and "直接命令" in error
                            for error in errors
                        )
                    )

    def test_rejects_any_key_command_success_fallback(self):
        commands = (
            'security find-identity -v -p codesigning',
            'SIGNATURE_DETAILS="$(codesign -dvvv "$APP_PATH" 2>&1)"',
            'xcrun notarytool history --keychain-profile "$PROFILE_NAME"',
            'xcrun stapler staple "$APP_PATH"',
            'spctl -a -t exec -vv "$APP_PATH"',
            'bun run tauri build --target "$RUST_TARGET"',
        )
        for script_name in ("release.sh", "sign.sh"):
            for command in commands:
                with self.subTest(script=script_name, command=command):
                    errors = MODULE.validate_release_contracts(
                        {
                            f"scripts/{script_name}": (
                                f"set -euo pipefail\n{command} || echo warning\n"
                            )
                        }
                    )
                    self.assertTrue(
                        any(
                            "||" in error and "忽略命令失败" in error
                            for error in errors
                        )
                    )

    def test_rejects_compact_spctl_fallback(self):
        content = (
            "set -euo pipefail\n"
            'spctl -a -t exec -vv "$APP_PATH"||echo warning\n'
        )
        for script_name in ("release.sh", "sign.sh"):
            with self.subTest(script=script_name):
                errors = MODULE.validate_release_contracts(
                    {f"scripts/{script_name}": content}
                )
                self.assertTrue(
                    any(
                        "||" in error and "忽略命令失败" in error
                        for error in errors
                    )
                )

    def test_requires_strict_shell_mode(self):
        for script_name in ("release.sh", "sign.sh"):
            with self.subTest(script=script_name):
                errors = MODULE.validate_release_contracts(
                    {f"scripts/{script_name}": "set -eu\n"}
                )
                self.assertTrue(
                    any("set -euo pipefail" in error for error in errors)
                )

    def test_rejects_disabling_errexit(self):
        variants = (
            "set +e\n",
            "set +e;\n",
            "if true; then set +e; fi\n",
        )
        for script_name in ("release.sh", "sign.sh"):
            for disabling_line in variants:
                with self.subTest(script=script_name, line=disabling_line):
                    errors = MODULE.validate_release_contracts(
                        {
                            f"scripts/{script_name}": (
                                f"set -euo pipefail\n{disabling_line}"
                            )
                        }
                    )
                    self.assertTrue(any("set +e" in error for error in errors))

    def test_rejects_strict_mode_after_first_key_command(self):
        content = """bun run tauri build --target "$RUST_TARGET"
set -euo pipefail
"""
        for script_name in ("release.sh", "sign.sh"):
            with self.subTest(script=script_name):
                errors = MODULE.validate_release_contracts(
                    {f"scripts/{script_name}": content}
                )
                self.assertTrue(
                    any(
                        "首个关键命令之前" in error
                        and "set -euo pipefail" in error
                        for error in errors
                    )
                )

    def test_rejects_early_composite_spctl_before_strict_mode(self):
        content = """if true; then spctl -a -t exec -vv "$APP_PATH"; fi
set -euo pipefail
"""
        for script_name in ("release.sh", "sign.sh"):
            with self.subTest(script=script_name):
                errors = MODULE.validate_release_contracts(
                    {f"scripts/{script_name}": content}
                )
                self.assertTrue(
                    any(
                        "首个可执行 logical statement" in error
                        and "set -euo pipefail" in error
                        for error in errors
                    )
                )

    def test_rejects_async_notary_history(self):
        content = (
            "set -euo pipefail\n"
            'xcrun notarytool history --keychain-profile "$PROFILE_NAME" &\n'
        )
        for script_name in ("release.sh", "sign.sh"):
            with self.subTest(script=script_name):
                errors = MODULE.validate_release_contracts(
                    {f"scripts/{script_name}": content}
                )
                self.assertTrue(
                    any(
                        "notary history" in error and "独立同步" in error
                        for error in errors
                    )
                )

    def test_rejects_notary_history_and_list(self):
        content = (
            "set -euo pipefail\n"
            'xcrun notarytool history --keychain-profile "$PROFILE_NAME" && true\n'
        )
        for script_name in ("release.sh", "sign.sh"):
            with self.subTest(script=script_name):
                errors = MODULE.validate_release_contracts(
                    {f"scripts/{script_name}": content}
                )
                self.assertTrue(
                    any(
                        "notary history" in error and "独立同步" in error
                        for error in errors
                    )
                )

    def test_rejects_strict_mode_not_first_executable_statement(self):
        variants = (
            """true
set -euo pipefail
""",
            "set -euo pipefail; true\n",
        )
        for script_name in ("release.sh", "sign.sh"):
            for content in variants:
                with self.subTest(script=script_name, content=content):
                    errors = MODULE.validate_release_contracts(
                        {f"scripts/{script_name}": content}
                    )
                    self.assertTrue(
                        any(
                            "首个可执行 logical statement" in error
                            and "set -euo pipefail" in error
                            for error in errors
                        )
                    )

    def test_rejects_security_and_codesign_grep_pipelines(self):
        for command in (
            'security find-identity -v -p codesigning | grep -q "$SIGNING_IDENTITY"',
            'codesign -dvvv "$APP_PATH" 2>&1 | grep -q "Timestamp="',
        ):
            with self.subTest(command=command):
                errors = MODULE.validate_release_contracts(
                    {"scripts/release.sh": f"{command}\n"}
                )
                self.assertTrue(any("grep -q" in error for error in errors))

    def test_requires_positive_secure_timestamp_assertion(self):
        content = (
            'PROFILE_NAME="macslim-notary"\n'
            'SIGNATURE_DETAILS="$(codesign -dvvv "$APP_PATH" 2>&1)"\n'
        )
        errors = MODULE.validate_release_contracts({"scripts/sign.sh": content})
        self.assertTrue(any("secure timestamp" in error for error in errors))

    def test_contract_commands_in_comments_are_ignored(self):
        content = """# security find-identity -v -p codesigning
# xcrun stapler validate \"$APP_PATH\"
# xcrun stapler validate \"$DMG_PATH\"
# spctl -a -t exec -vv \"$APP_PATH\"
# spctl -a -t install -vv \"$DMG_PATH\"
"""
        errors = MODULE.validate_release_contracts({"scripts/release.sh": content})
        self.assertTrue(any("签名证书" in error for error in errors))

    def test_rejects_timestamp_fallback(self):
        errors = MODULE.validate_release_contracts(
            {"scripts/sign.sh": 'TIMESTAMP_FLAG="--timestamp=none"\nexit 0\n'}
        )
        self.assertTrue(any("timestamp" in error for error in errors))

    def test_rejects_ignored_gatekeeper_failure(self):
        errors = MODULE.validate_release_contracts(
            {"scripts/release.sh": 'spctl -a -vv -t install "$DMG_PATH" || true\n'}
        )
        self.assertTrue(any("Gatekeeper" in error for error in errors))

    def test_requires_consistent_notary_profile(self):
        errors = MODULE.validate_release_contracts(
            {
                "scripts/release.sh": 'PROFILE_NAME="macslim-notary"\n',
                "scripts/sign.sh": 'PROFILE_NAME="macflow-notary"\n',
            }
        )
        self.assertTrue(any("notary" in error for error in errors))

    def test_requires_local_certificate_preflight(self):
        errors = MODULE.validate_release_contracts(
            {
                "scripts/release.sh": 'PROFILE_NAME="macslim-notary"\n',
                "scripts/sign.sh": 'PROFILE_NAME="macslim-notary"\n',
            }
        )
        self.assertTrue(any("签名证书" in error for error in errors))

    def test_requires_app_and_dmg_notarization_checks(self):
        errors = MODULE.validate_release_contracts(
            {
                "scripts/release.sh": 'PROFILE_NAME="macslim-notary"\n',
                "scripts/sign.sh": 'PROFILE_NAME="macslim-notary"\n',
            }
        )
        self.assertTrue(any("stapler" in error for error in errors))
        self.assertTrue(any("Gatekeeper" in error for error in errors))

    def test_cli_accepts_explicit_updater_credentials(self):
        completed = subprocess.run(
            [sys.executable, str(SCRIPT_PATH)],
            cwd=SCRIPT_PATH.parents[2],
            capture_output=True,
            text=True,
            check=False,
        )
        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertIn("安全配置校验通过", output)
        self.assertNotIn("macslim-dev-pw", output)

    def test_validator_entrypoint_does_not_write_bytecode(self):
        with tempfile.TemporaryDirectory() as cache_directory:
            environment = os.environ.copy()
            environment.pop("PYTHONDONTWRITEBYTECODE", None)
            environment["PYTHONPYCACHEPREFIX"] = cache_directory
            completed = subprocess.run(
                [sys.executable, str(SCRIPT_PATH)],
                cwd=SCRIPT_PATH.parents[2],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )
            output = completed.stdout + completed.stderr
            self.assertEqual(completed.returncode, 0, output)
            local_bytecode = [
                path
                for path in pathlib.Path(cache_directory).rglob("*.pyc")
                if "release_docs_contract" in path.name
            ]
            self.assertEqual(local_bytecode, [])


if __name__ == "__main__":
    unittest.main()
