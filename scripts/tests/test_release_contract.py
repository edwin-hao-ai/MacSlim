from __future__ import annotations

import importlib.util
import os
import pathlib
import subprocess
import tempfile
import unittest

SCRIPT_PATH = pathlib.Path(__file__).resolve().parents[1] / "verify-security-config.py"
SCRIPTS_DIR = SCRIPT_PATH.parent
PUBLISH_PATH = SCRIPTS_DIR / "publish-update.sh"
RELEASE_PATH = SCRIPTS_DIR / "release.sh"
SIGN_PATH = SCRIPTS_DIR / "sign.sh"
SPEC = importlib.util.spec_from_file_location("verify_security_config_contracts", SCRIPT_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class ReleaseContractTests(unittest.TestCase):
    def files(self) -> dict[str, str]:
        return {
            "scripts/release.sh": RELEASE_PATH.read_text(encoding="utf-8"),
            "scripts/sign.sh": SIGN_PATH.read_text(encoding="utf-8"),
        }

    def test_current_scripts_invalidate_old_assets_and_rebuild(self) -> None:
        for path in (RELEASE_PATH, SIGN_PATH):
            content = path.read_text(encoding="utf-8")
            with self.subTest(script=path.name):
                self.assertIn('UPDATER_SIG_PATH="${UPDATER_PATH}.sig"', content)
                self.assertIn('UPDATER_STAMP_PATH="${UPDATER_PATH}.build-stamp.json"', content)
                self.assertIn('rm -f "$UPDATER_PATH" "$UPDATER_SIG_PATH" "$UPDATER_STAMP_PATH"', content)
                self.assertIn("updater_artifact.py rebuild", content)
                self.assertIn('--app "$APP_PATH"', content)

    def test_validator_rejects_missing_or_late_invalidation(self) -> None:
        files = self.files()
        for name in files:
            content = files[name]
            missing = content.replace(
                'rm -f "$UPDATER_PATH" "$UPDATER_SIG_PATH" "$UPDATER_STAMP_PATH"\n',
                "",
            )
            errors = MODULE.validate_release_contracts({name: missing})
            self.assertTrue(any("失效" in error or "旧" in error for error in errors))
            build = "bun run tauri build --target \"$RUST_TARGET\"\n"
            if build in content:
                late = content.replace(
                    build,
                    build
                    + 'rm -f "$UPDATER_PATH" "$UPDATER_SIG_PATH" "$UPDATER_STAMP_PATH"\n',
                )
            else:
                late = content.replace(
                    'IDENTITIES="$(security find-identity -v -p codesigning)"\n',
                    'rm -f "$UPDATER_PATH" "$UPDATER_SIG_PATH" "$UPDATER_STAMP_PATH"\n'
                    + 'IDENTITIES="$(security find-identity -v -p codesigning)"\n',
                )
            errors = MODULE.validate_release_contracts({name: late})
            self.assertTrue(any("失效" in error or "之前" in error for error in errors))

    def test_validator_rejects_each_publish_side_effect_before_verify(self) -> None:
        content = PUBLISH_PATH.read_text(encoding="utf-8")
        verify = (
            'python3 scripts/updater_artifact.py verify \\\n'
            '  --archive "$UPDATER_SRC" \\\n'
            '  --target "$RUST_TARGET"\n'
        )
        markers = (
            'mkdir -p "$DOWNLOAD_DIR" "$UPDATE_DIR"\n',
            'cp "$UPDATER_SRC" "$UPDATER_PATH"\n',
            'cp "$UPDATER_SRC_STAMP_PATH" "$UPDATER_STAMP_PATH"\n',
            'bun tauri signer sign "$asset"\n',
            "path.parent.mkdir(parents=True, exist_ok=True)\n",
            "path.write_text(json.dumps(manifest, indent=2, ensure_ascii=False), encoding=\"utf-8\")\n",
            'write_manifest(update_dir / "latest.json")\n',
        )
        for marker in markers:
            with self.subTest(marker=marker.strip()):
                mutated = content.replace(marker, "")
                mutated = mutated.replace(verify, marker + verify, 1)
                errors = MODULE.validate_updater_contract(
                    {
                        "bundle": {"createUpdaterArtifacts": True},
                        "plugins": {
                            "updater": {
                                "endpoints": ["https://example.test/latest.json"]
                            }
                        },
                    },
                    mutated,
                )
                self.assertTrue(errors)

    def test_validator_requires_staging_and_transaction_commit(self) -> None:
        content = PUBLISH_PATH.read_text(encoding="utf-8")
        for marker in (
            "STAGING_ROOT",
            "STAGING_DOWNLOAD_DIR",
            "mktemp",
            "trap cleanup EXIT",
            "publish_assets.py commit",
        ):
            mutated = content.replace(marker, "")
            errors = MODULE.validate_updater_contract(
                {
                    "bundle": {"createUpdaterArtifacts": True},
                    "plugins": {
                        "updater": {
                            "endpoints": ["https://example.test/latest.json"]
                        }
                    },
                },
                mutated,
            )
            self.assertTrue(errors, marker)

    def test_validator_rejects_verify_existing_assets_call_drift(self) -> None:
        content = PUBLISH_PATH.read_text(encoding="utf-8")
        without_calls = content.replace('verify_existing_assets "." "$ARCH"\n', "", 1)
        without_calls = without_calls.replace(
            'verify_existing_assets "$STAGING_ROOT" "$ARCH"\n', "", 1
        )
        for mutated in (
            without_calls,
            content.replace(
                'verify_existing_assets "." "$ARCH"\n',
                'verify_existing_assets "." "$ARCH"\nverify_existing_assets "." "$ARCH"\n',
                1,
            ),
        ):
            errors = MODULE.validate_updater_contract(
                {
                    "bundle": {"createUpdaterArtifacts": True},
                    "plugins": {
                        "updater": {
                            "endpoints": ["https://example.test/latest.json"]
                        }
                    },
                },
                mutated,
            )
            self.assertTrue(errors)

    def test_validator_rejects_copy_after_staging_verify(self) -> None:
        content = PUBLISH_PATH.read_text(encoding="utf-8")
        call = "copy_existing_assets\n"
        staging_verify = 'verify_existing_assets "$STAGING_ROOT" "$ARCH"\n'
        mutated = content.replace(call, "", 1).replace(
            staging_verify,
            staging_verify + call,
            1,
        )
        errors = MODULE.validate_updater_contract(
            {
                "bundle": {"createUpdaterArtifacts": True},
                "plugins": {
                    "updater": {
                        "endpoints": ["https://example.test/latest.json"]
                    }
                },
            },
            mutated,
        )
        self.assertTrue(any("copy_existing_assets" in error for error in errors))

    def test_validator_rejects_incomplete_publish_assets_helper(self) -> None:
        errors = MODULE.validate_publish_assets_contract(
            "def commit_staged_assets(staging_root, live_root, version):\n    return ()\n"
        )
        self.assertTrue(errors)

    def test_validator_rejects_public_stamp_contract(self) -> None:
        self.assertTrue(
            MODULE.validate_updater_artifact_contract(
                'def write_stamp(archive, target):\n    pass\n'
            )
        )
        self.assertEqual(
            MODULE.validate_updater_artifact_contract(
                'def _write_stamp(archive, target):\n    pass\n'
                'def rebuild(app, archive, target):\n    pass\n'
                'def verify_stamp(archive, target):\n    pass\n'
                'commands.add_parser("verify")\n'
                'commands.add_parser("rebuild")\n'
            ),
            [],
        )

    def test_validator_rejects_stamp_instead_of_rebuild(self) -> None:
        files = self.files()
        for name, content in files.items():
            mutated = content.replace(
                "python3 scripts/updater_artifact.py rebuild",
                "python3 scripts/updater_artifact.py stamp",
            )
            errors = MODULE.validate_release_contracts({name: mutated})
            self.assertTrue(any("rebuild" in error for error in errors))

    def test_validator_rejects_publish_side_effect_before_verify(self) -> None:
        content = PUBLISH_PATH.read_text(encoding="utf-8")
        verify = (
            'python3 scripts/updater_artifact.py verify \\\n'
            '  --archive "$UPDATER_SRC" \\\n'
            '  --target "$RUST_TARGET"\n'
        )
        deletion = 'rm -f "${asset}.sig"\n'
        content_without_deletion = content.replace(deletion, "", 1)
        mutated = content_without_deletion.replace(
            verify,
            deletion + verify,
            1,
        )
        errors = MODULE.validate_updater_contract(
            {
                "bundle": {"createUpdaterArtifacts": True},
                "plugins": {"updater": {"endpoints": ["https://example.test/latest.json"]}},
            },
            mutated,
        )
        self.assertTrue(any("删除" in error or "验证" in error for error in errors))

    def test_validator_rejects_version_mismatch_without_nonzero_exit(self) -> None:
        content = PUBLISH_PATH.read_text(encoding="utf-8")
        mutated = content.replace(
            '  echo "错误: 发布版本与 tauri.conf.json 不一致" >&2\n  exit 1\n',
            '  echo "错误: 发布版本与 tauri.conf.json 不一致" >&2\n',
        )
        errors = MODULE.validate_updater_contract(
            {
                "bundle": {"createUpdaterArtifacts": True},
                "plugins": {"updater": {"endpoints": ["https://example.test/latest.json"]}},
            },
            mutated,
        )
        self.assertTrue(any("非零" in error or "退出" in error for error in errors))

    def test_validator_rejects_target_mapping_drift(self) -> None:
        content = PUBLISH_PATH.read_text(encoding="utf-8").replace(
            '    ARCH="aarch64"\n', '    ARCH="arm64"\n', 1
        )
        errors = MODULE.validate_updater_contract(
            {
                "bundle": {"createUpdaterArtifacts": True},
                "plugins": {"updater": {"endpoints": ["https://example.test/latest.json"]}},
            },
            content,
        )
        self.assertTrue(any("arm" in error or "架构" in error for error in errors))

    def test_sign_usage_is_sent_to_stderr(self) -> None:
        completed = subprocess.run(
            ["bash", str(SIGN_PATH), "invalid-target"],
            cwd=SIGN_PATH.parents[1],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("用法", completed.stderr)
        self.assertNotIn("用法", completed.stdout)

    def test_release_and_sign_failure_removes_stale_updater_artifacts(self) -> None:
        for script_path in (RELEASE_PATH, SIGN_PATH):
            with self.subTest(script=script_path.name), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                workspace = root / "workspace"
                bin_dir = root / "bin"
                workspace.mkdir()
                bin_dir.mkdir()
                config = workspace / "src-tauri/tauri.conf.json"
                config.parent.mkdir()
                config.write_text(
                    (pathlib.Path(__file__).resolve().parents[2] / "src-tauri/tauri.conf.json").read_text(
                        encoding="utf-8"
                    ),
                    encoding="utf-8",
                )
                target_root = workspace / "src-tauri/target/aarch64-apple-darwin/release/bundle"
                archive = target_root / "macos/MacSlim.app.tar.gz"
                archive.parent.mkdir(parents=True)
                archive.write_bytes(b"stale-archive")
                pathlib.Path(f"{archive}.sig").write_text("stale-signature", encoding="utf-8")
                pathlib.Path(f"{archive}.build-stamp.json").write_text("{}", encoding="utf-8")
                (target_root / "macos/MacSlim.app").mkdir()
                (target_root / "dmg").mkdir()
                (target_root / "dmg/MacSlim_0.2.2_aarch64.dmg").write_bytes(b"dmg")
                security = bin_dir / "security"
                security.write_text(
                    "#!/usr/bin/env bash\n"
                    "printf '%s\\n' 'Developer ID Application: Beijing VGO Co;Ltd (5XNDF727Y6)'\n",
                    encoding="utf-8",
                )
                xcrun = bin_dir / "xcrun"
                xcrun.write_text("#!/usr/bin/env bash\nexit 1\n", encoding="utf-8")
                security.chmod(0o755)
                xcrun.chmod(0o755)
                env = os.environ.copy()
                env["PATH"] = f"{bin_dir}{os.pathsep}{env.get('PATH', '')}"
                completed = subprocess.run(
                    ["bash", str(script_path), "arm"],
                    cwd=workspace,
                    env=env,
                    capture_output=True,
                    text=True,
                    check=False,
                )
                self.assertNotEqual(completed.returncode, 0)
                self.assertFalse(archive.exists())
                self.assertFalse(pathlib.Path(f"{archive}.sig").exists())
                self.assertFalse(pathlib.Path(f"{archive}.build-stamp.json").exists())

    def test_notary_history_failure_has_chinese_diagnostic(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            workspace = root / "workspace"
            bin_dir = root / "bin"
            workspace.mkdir()
            bin_dir.mkdir()
            config = workspace / "src-tauri/tauri.conf.json"
            config.parent.mkdir()
            config.write_text(
                (pathlib.Path(__file__).resolve().parents[2] / "src-tauri/tauri.conf.json").read_text(
                    encoding="utf-8"
                ),
                encoding="utf-8",
            )
            security = bin_dir / "security"
            security.write_text(
                "#!/usr/bin/env bash\n"
                "printf '%s\\n' 'Developer ID Application: Beijing VGO Co;Ltd (5XNDF727Y6)'\n",
                encoding="utf-8",
            )
            xcrun = bin_dir / "xcrun"
            xcrun.write_text("#!/usr/bin/env bash\nexit 1\n", encoding="utf-8")
            security.chmod(0o755)
            xcrun.chmod(0o755)
            env = os.environ.copy()
            env["PATH"] = f"{bin_dir}{os.pathsep}{env.get('PATH', '')}"
            completed = subprocess.run(
                ["bash", str(RELEASE_PATH), "arm"],
                cwd=workspace,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            output = completed.stdout + completed.stderr
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("notary", output)
            self.assertNotIn("macslim-notary", output)


if __name__ == "__main__":
    unittest.main()
