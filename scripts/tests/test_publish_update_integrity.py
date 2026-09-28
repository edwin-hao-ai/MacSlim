from __future__ import annotations

import json
import os
import pathlib
import subprocess
import tempfile
import unittest

from scripts import updater_artifact
from scripts.tests import test_updater_artifact as helpers

SCRIPT_PATH = pathlib.Path(__file__).resolve().parents[1] / "publish-update.sh"
CONFIG_PATH = pathlib.Path(__file__).resolve().parents[2] / "src-tauri/tauri.conf.json"
VERSION = json.loads(CONFIG_PATH.read_text(encoding="utf-8"))["version"]
PASSWORD = "publish-test-password-sentinel"
TARGETS = {
    "arm": ("aarch64-apple-darwin", "aarch64", "arm64"),
    "intel": ("x86_64-apple-darwin", "x64", "x86_64"),
    "universal": ("universal-apple-darwin", "universal", "universal"),
}


def write_fake_signer(bin_dir: pathlib.Path, log: pathlib.Path) -> None:
    bun_path = bin_dir / "bun"
    bun_path.write_text(
        "#!/usr/bin/env bash\n"
        'if [ "$1" = "tauri" ] && [ "$2" = "signer" ] && [ "$3" = "sign" ]; then\n'
        '  if [ "${MACSlim_TEST_SIGNER_FAIL:-}" = "1" ]; then\n'
        "    exit 7\n"
        "  fi\n"
        '  asset="$4"\n'
        '  name="${asset##*/}"\n'
        '  printf "fresh-%s" "$name" > "$asset.sig"\n'
        '  printf "%s\\n" "$asset" >> "$MACSlim_TEST_SIGNER_LOG"\n'
        "  exit 0\n"
        "fi\n"
        "exit 1\n",
        encoding="utf-8",
    )
    bun_path.chmod(0o755)


class PublishIntegrityTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(temporary_directory.cleanup)
        self.root = pathlib.Path(temporary_directory.name)

    def workspace(self, name: str):
        workspace = self.root / name
        home = workspace / "home"
        bin_dir = workspace / "bin"
        tmp_dir = workspace / "tmp"
        home.mkdir(parents=True)
        bin_dir.mkdir()
        tmp_dir.mkdir()
        config_path = workspace / "src-tauri/tauri.conf.json"
        config_path.parent.mkdir(parents=True)
        config_path.write_text(CONFIG_PATH.read_text(encoding="utf-8"), encoding="utf-8")
        scripts_path = workspace / "scripts"
        scripts_path.mkdir()
        (scripts_path / "updater_artifact.py").write_text(
            (pathlib.Path(__file__).resolve().parents[1] / "updater_artifact.py").read_text(
                encoding="utf-8"
            ),
            encoding="utf-8",
        )
        (scripts_path / "publish_assets.py").write_text(
            (pathlib.Path(__file__).resolve().parents[1] / "publish_assets.py").read_text(
                encoding="utf-8"
            ),
            encoding="utf-8",
        )
        log = workspace / "signer.log"
        write_fake_signer(bin_dir, log)
        env = os.environ.copy()
        for name in (
            "TAURI_SIGNING_PRIVATE_KEY",
            "TAURI_SIGNING_PRIVATE_KEY_PATH",
            "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
            "MACSlim_PUBLISH_PREFLIGHT_ONLY",
            "MACSlim_TEST_SIGNER_LOG",
            "MACSlim_TEST_SIGNER_FAIL",
        ):
            env.pop(name, None)
        env.update(
            {
                "HOME": str(home),
                "TMPDIR": str(tmp_dir),
                "MACSlim_TEST_SIGNER_LOG": str(log),
                "PATH": f"{bin_dir}{os.pathsep}{env.get('PATH', '')}",
                "TAURI_SIGNING_PRIVATE_KEY": "fake-publish-key",
                "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": PASSWORD,
            }
        )
        return workspace, env, log

    def source_archive(
        self, workspace: pathlib.Path, target: str, suffix: bytes = b""
    ) -> pathlib.Path:
        rust_target, _, architecture = TARGETS[target]
        path = workspace / "src-tauri/target" / rust_target / "release/bundle/macos/MacSlim.app.tar.gz"
        path.parent.mkdir(parents=True, exist_ok=True)
        app = helpers.make_app(
            path.parent / "MacSlim.app",
            version=VERSION,
            architecture=architecture,
            executable_suffix=suffix,
        )
        updater_artifact.rebuild_archive(app, path, rust_target)
        return path

    def run_publish(
        self, workspace, env, target: str, preflight: bool = False, version: str = VERSION
    ):
        if preflight:
            env["MACSlim_PUBLISH_PREFLIGHT_ONLY"] = "1"
        else:
            env.pop("MACSlim_PUBLISH_PREFLIGHT_ONLY", None)
        return subprocess.run(
            ["bash", str(SCRIPT_PATH), version, "integrity test", target],
            cwd=workspace,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_rejects_config_version_mismatch_before_side_effects(self) -> None:
        workspace, env, log = self.workspace("version-mismatch")
        self.source_archive(workspace, "arm")
        mismatched = "0.0.0" if VERSION != "0.0.0" else "0.0.1"
        completed = self.run_publish(workspace, env, "arm", version=mismatched)
        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0, output)
        self.assertIn("发布版本与 tauri.conf.json 不一致", output)
        self.assertFalse(log.exists())
        self.assertFalse((workspace / "landing").exists())

    def test_rejects_missing_stamp_before_preflight_side_effects(self) -> None:
        workspace, env, log = self.workspace("missing-stamp")
        source = self.source_archive(workspace, "arm")
        updater_artifact.stamp_path(source).unlink()
        completed = self.run_publish(workspace, env, "arm", preflight=True)
        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0, output)
        self.assertIn("缺少 updater build stamp", output)
        self.assertFalse(log.exists())
        self.assertFalse((workspace / "landing").exists())

    def test_rejects_wrong_bundle_version(self) -> None:
        workspace, env, log = self.workspace("wrong-version")
        source = self.source_archive(workspace, "arm")
        mismatched = "0.0.0" if VERSION != "0.0.0" else "0.0.1"
        helpers.make_archive(source, version=mismatched, architecture="arm64")
        completed = self.run_publish(workspace, env, "arm")
        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0, output)
        self.assertIn("版本", output)
        self.assertFalse(log.exists())

    def test_rejects_tampered_archive(self) -> None:
        workspace, env, log = self.workspace("tampered")
        source = self.source_archive(workspace, "arm")
        members = helpers.app_members(
            helpers.info_member(version=VERSION),
            helpers.make_member("MacSlim.app/Contents/Resources/tampered", b"tampered"),
            extra_directories=("MacSlim.app/Contents/Resources/",),
        )
        helpers.write_members(source, members)
        completed = self.run_publish(workspace, env, "arm")
        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0, output)
        self.assertIn("校验和", output)
        self.assertFalse(log.exists())

    def test_rejects_existing_asset_with_wrong_target(self) -> None:
        workspace, env, log = self.workspace("wrong-existing-target")
        self.source_archive(workspace, "arm")
        extra = workspace / "landing/downloads" / f"MacSlim_{VERSION}_x64.app.tar.gz"
        extra.parent.mkdir(parents=True)
        helpers.make_archive(extra, version=VERSION, architecture="arm64")
        helpers.write_test_stamp(extra, "aarch64-apple-darwin")
        completed = self.run_publish(workspace, env, "arm")
        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0, output)
        self.assertIn("架构", output)
        self.assertFalse(log.exists())
        self.assertFalse((workspace / "landing/updates/latest.json").exists())

    def test_rejects_zero_updater_assets(self) -> None:
        workspace, env, log = self.workspace("zero-assets")
        completed = self.run_publish(workspace, env, "arm")
        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0, output)
        self.assertIn("找不到 Tauri updater artifact", output)
        self.assertFalse(log.exists())
        self.assertFalse((workspace / "landing").exists())

    def test_preflight_has_no_publish_side_effects(self) -> None:
        workspace, env, log = self.workspace("preflight")
        self.source_archive(workspace, "arm")
        completed = self.run_publish(workspace, env, "arm", preflight=True)
        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertFalse(log.exists())
        self.assertFalse((workspace / "landing").exists())

    def test_rejects_existing_same_version_asset_without_sidecar(self) -> None:
        workspace, env, log = self.workspace("missing-sidecar")
        self.source_archive(workspace, "arm")
        extra = workspace / "landing/downloads" / f"MacSlim_{VERSION}_x64.app.tar.gz"
        extra.parent.mkdir(parents=True)
        helpers.make_archive(extra, version=VERSION, architecture="x86_64")
        pathlib.Path(f"{extra}.sig").write_text("stale-signature", encoding="utf-8")
        completed = self.run_publish(workspace, env, "arm")
        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0, output)
        self.assertIn("sidecar", output)
        self.assertFalse(log.exists())
        self.assertFalse((workspace / "landing/updates/latest.json").exists())

    def test_rewrites_every_same_version_signature_from_current_signer(self) -> None:
        workspace, env, log = self.workspace("rewrite-signatures")
        self.source_archive(workspace, "arm")
        extra = workspace / "landing/downloads" / f"MacSlim_{VERSION}_x64.app.tar.gz"
        extra.parent.mkdir(parents=True)
        helpers.make_archive(extra, version=VERSION, architecture="x86_64")
        helpers.write_test_stamp(extra, "x86_64-apple-darwin")
        pathlib.Path(f"{extra}.sig").write_text("stale-signature", encoding="utf-8")
        completed = self.run_publish(workspace, env, "arm")
        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        current = workspace / "landing/downloads" / f"MacSlim_{VERSION}_aarch64.app.tar.gz"
        self.assertEqual(pathlib.Path(f"{current}.sig").read_text(), f"fresh-{current.name}")
        self.assertEqual(pathlib.Path(f"{extra}.sig").read_text(), f"fresh-{extra.name}")
        self.assertTrue(pathlib.Path(f"{current}.build-stamp.json").is_file())
        self.assertTrue(pathlib.Path(f"{extra}.build-stamp.json").is_file())
        manifest = json.loads(
            (workspace / "landing/updates/latest.json").read_text(encoding="utf-8")
        )
        self.assertEqual(
            manifest["platforms"]["darwin-aarch64"]["signature"],
            f"fresh-{current.name}",
        )
        self.assertEqual(
            manifest["platforms"]["darwin-x86_64"]["signature"],
            f"fresh-{extra.name}",
        )
        self.assertEqual(len(log.read_text().splitlines()), 2)

    def test_signer_failure_keeps_previous_live_state(self) -> None:
        workspace, env, log = self.workspace("signer-rollback")
        self.source_archive(workspace, "arm", suffix=b"old")
        first = self.run_publish(workspace, env, "arm")
        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        asset = workspace / "landing/downloads" / f"MacSlim_{VERSION}_aarch64.app.tar.gz"
        signature = pathlib.Path(f"{asset}.sig")
        manifest = workspace / "landing/updates/latest.json"
        old_asset = asset.read_bytes()
        old_signature = signature.read_bytes()
        old_manifest = manifest.read_bytes()
        self.source_archive(workspace, "arm", suffix=b"new")
        env["MACSlim_TEST_SIGNER_FAIL"] = "1"
        second = self.run_publish(workspace, env, "arm")
        self.assertNotEqual(second.returncode, 0)
        self.assertEqual(asset.read_bytes(), old_asset)
        self.assertEqual(signature.read_bytes(), old_signature)
        self.assertEqual(manifest.read_bytes(), old_manifest)
        self.assertEqual(list((workspace / "tmp").iterdir()), [])

    def test_fresh_publish_commits_all_compat_manifests(self) -> None:
        workspace, env, log = self.workspace("fresh-compat")
        self.source_archive(workspace, "universal")
        completed = self.run_publish(workspace, env, "universal")
        output = completed.stdout + completed.stderr
        self.assertEqual(completed.returncode, 0, output)
        self.assertTrue((workspace / "landing/updates/latest.json").is_file())
        for platform in ("darwin-aarch64", "darwin-x86_64"):
            for compat in ("0.1.0.json", "0.2.2.json"):
                path = workspace / "landing/updates" / platform / compat
                self.assertTrue(path.is_file(), path)
        self.assertEqual(len(log.read_text().splitlines()), 1)

    def test_publish_maps_arm_intel_and_universal_targets(self) -> None:
        for target in TARGETS:
            with self.subTest(target=target):
                workspace, env, log = self.workspace(target)
                self.source_archive(workspace, target)
                completed = self.run_publish(workspace, env, target)
                output = completed.stdout + completed.stderr
                self.assertEqual(completed.returncode, 0, output)
                _, arch, _ = TARGETS[target]
                asset = workspace / "landing/downloads" / f"MacSlim_{VERSION}_{arch}.app.tar.gz"
                self.assertTrue(asset.is_file())
                self.assertTrue(pathlib.Path(f"{asset}.build-stamp.json").is_file())
                self.assertTrue(pathlib.Path(f"{asset}.sig").is_file())
                manifest = json.loads(
                    (workspace / "landing/updates/latest.json").read_text(encoding="utf-8")
                )
                expected_platforms = {
                    "arm": {"darwin-aarch64"},
                    "intel": {"darwin-x86_64"},
                    "universal": {"darwin-aarch64", "darwin-x86_64"},
                }[target]
                self.assertEqual(set(manifest["platforms"]), expected_platforms)
                self.assertEqual(len(log.read_text().splitlines()), 1)

    def test_preflight_rejects_mixed_universal_and_arm_platforms(self) -> None:
        workspace, env, log = self.workspace("mixed-platform-preflight")
        self.source_archive(workspace, "arm")
        extra = workspace / "landing/downloads" / f"MacSlim_{VERSION}_universal.app.tar.gz"
        extra.parent.mkdir(parents=True)
        helpers.make_archive(extra, version=VERSION, architecture="universal")
        helpers.write_test_stamp(extra, "universal-apple-darwin")
        completed = self.run_publish(workspace, env, "arm", preflight=True)
        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0, output)
        self.assertIn("平台", output)
        self.assertFalse(log.exists())
        self.assertFalse((workspace / "landing/updates").exists())

    def test_publish_script_keeps_versioned_tar_and_integrity_markers(self) -> None:
        content = SCRIPT_PATH.read_text(encoding="utf-8")
        for marker in (
            ".app.tar.gz",
            "MacSlim_${VERSION}_${ARCH}.app.tar.gz",
            "updater_artifact.py verify",
            "UPDATER_SRC_STAMP_PATH",
            "STAGING_ROOT",
            "STAGING_DOWNLOAD_DIR",
            "sign_staged_assets",
            "publish_assets.py commit",
            "mktemp",
            "trap cleanup EXIT",
            "latest.json",
            "0.1.0",
            "0.2.2",
        ):
            self.assertIn(marker, content)
        self.assertNotIn("DMG_SRC", content)
        self.assertNotIn('signer sign "$DMG_SRC"', content)


if __name__ == "__main__":
    unittest.main()
