from __future__ import annotations

import os
import pathlib
import tempfile
import unittest
from unittest import mock

from scripts import publish_assets


class PublishAssetsTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(temporary_directory.cleanup)
        self.root = pathlib.Path(temporary_directory.name)
        self.staging = self.root / "staging"
        self.live = self.root / "live"
        self.staging.mkdir()
        self.live.mkdir()

    def write_file(self, root: pathlib.Path, relative: str, content: bytes) -> pathlib.Path:
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return path

    def seed_version(self, root: pathlib.Path, marker: bytes) -> None:
        self.write_file(root, "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz", marker + b"-asset")
        self.write_file(root, "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz.build-stamp.json", marker + b"-stamp")
        self.write_file(root, "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz.sig", marker + b"-sig")
        self.write_file(root, "landing/updates/latest.json", marker + b"-latest")
        for platform in ("darwin-aarch64", "darwin-x86_64"):
            for compat in ("0.1.0.json", "0.2.2.json"):
                self.write_file(root, f"landing/updates/{platform}/{compat}", marker + f"-{platform}-{compat}".encode())

    def test_selects_all_manifests_and_only_direct_assets(self) -> None:
        self.seed_version(self.staging, b"new")
        nested = self.write_file(
            self.staging,
            "landing/downloads/nested/MacSlim_0.2.2_aarch64.app.tar.gz",
            b"nested",
        )
        selected = publish_assets.select_commit_files(self.staging, "0.2.2")
        expected = {
            pathlib.Path("landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz"),
            pathlib.Path("landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz.build-stamp.json"),
            pathlib.Path("landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz.sig"),
            pathlib.Path("landing/updates/latest.json"),
            pathlib.Path("landing/updates/darwin-aarch64/0.1.0.json"),
            pathlib.Path("landing/updates/darwin-aarch64/0.2.2.json"),
            pathlib.Path("landing/updates/darwin-x86_64/0.1.0.json"),
            pathlib.Path("landing/updates/darwin-x86_64/0.2.2.json"),
        }
        self.assertEqual(set(selected), expected)
        self.assertNotIn(nested.relative_to(self.staging), selected)

    def test_commit_replaces_current_version_and_preserves_unrelated_files(self) -> None:
        self.seed_version(self.live, b"old")
        self.seed_version(self.staging, b"new")
        self.write_file(self.live, "landing/downloads/MacSlim_0.1.0_aarch64.app.tar.gz", b"unrelated")
        self.write_file(self.staging, "landing/downloads/MacSlim_0.1.0_aarch64.app.tar.gz", b"new-unrelated")
        committed = publish_assets.commit_staged_assets(
            self.staging, self.live, "0.2.2"
        )
        expected = {
            "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz",
            "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz.build-stamp.json",
            "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz.sig",
            "landing/updates/latest.json",
            "landing/updates/darwin-aarch64/0.1.0.json",
            "landing/updates/darwin-aarch64/0.2.2.json",
            "landing/updates/darwin-x86_64/0.1.0.json",
            "landing/updates/darwin-x86_64/0.2.2.json",
        }
        self.assertEqual(set(committed), expected)
        compat_content = {
            relative: b"new-" + relative.removeprefix("landing/updates/").replace("/", "-").encode()
            for relative in expected
            if relative.startswith("landing/updates/")
            and relative != "landing/updates/latest.json"
        }
        expected_content = {
            "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz": b"new-asset",
            "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz.build-stamp.json": b"new-stamp",
            "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz.sig": b"new-sig",
            "landing/updates/latest.json": b"new-latest",
            **compat_content,
        }
        for relative in expected:
            with self.subTest(relative=relative):
                self.assertEqual((self.live / relative).read_bytes(), expected_content[relative])
        self.assertEqual(
            (self.live / "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz").read_bytes(),
            b"new-asset",
        )
        self.assertEqual(
            (self.live / "landing/updates/latest.json").read_bytes(),
            b"new-latest",
        )
        self.assertEqual(
            (self.live / "landing/downloads/MacSlim_0.1.0_aarch64.app.tar.gz").read_bytes(),
            b"unrelated",
        )

    def test_rejects_live_ancestor_symlink(self) -> None:
        self.seed_version(self.staging, b"new")
        outside = self.root / "outside-live"
        outside.mkdir()
        (self.live / "landing").symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink|越界"):
            publish_assets.commit_staged_assets(self.staging, self.live, "0.2.2")
        self.assertFalse((outside / "downloads").exists())
        self.assertFalse((outside / "updates").exists())

    def test_rejects_live_platform_symlink(self) -> None:
        self.seed_version(self.staging, b"new")
        updates = self.live / "landing/updates"
        updates.mkdir(parents=True)
        outside = self.root / "outside-platform"
        outside.mkdir()
        (updates / "darwin-aarch64").symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink|越界"):
            publish_assets.commit_staged_assets(self.staging, self.live, "0.2.2")
        self.assertEqual(list(outside.iterdir()), [])

    def test_rejects_destination_symlink(self) -> None:
        self.seed_version(self.staging, b"new")
        downloads = self.live / "landing/downloads"
        downloads.mkdir(parents=True)
        outside = self.root / "outside-file"
        outside.write_bytes(b"outside")
        (downloads / "MacSlim_0.2.2_aarch64.app.tar.gz").symlink_to(outside)
        with self.assertRaisesRegex(ValueError, "symlink|类型"):
            publish_assets.commit_staged_assets(self.staging, self.live, "0.2.2")
        self.assertEqual(outside.read_bytes(), b"outside")

    def test_rejects_staging_root_symlink(self) -> None:
        outside = self.root / "outside-staging"
        outside.mkdir()
        self.seed_version(outside, b"new")
        staging_link = self.root / "staging-link"
        staging_link.symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink|staging"):
            publish_assets.commit_staged_assets(staging_link, self.live, "0.2.2")
        self.assertFalse((self.live / "landing").exists())

    def test_rejects_staging_ancestor_symlink(self) -> None:
        self.write_file(
            self.staging,
            "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz",
            b"new",
        )
        outside = self.root / "outside-updates"
        outside.mkdir()
        (self.staging / "landing/updates").symlink_to(
            outside, target_is_directory=True
        )
        with self.assertRaisesRegex(ValueError, "symlink"):
            publish_assets.select_commit_files(self.staging, "0.2.2")

    def _assert_interrupt_rolls_back(self, exception_type: type[BaseException]) -> None:
        self.seed_version(self.live, b"old")
        self.seed_version(self.staging, b"new")
        old_files = {
            path.relative_to(self.live): path.read_bytes()
            for path in self.live.rglob("*")
            if path.is_file()
        }
        real_replace = os.replace
        calls = {"count": 0}

        def failing_replace(source, destination):
            calls["count"] += 1
            if calls["count"] == 2:
                raise exception_type("injected interrupt")
            return real_replace(source, destination)

        with self.assertRaises(exception_type):
            publish_assets.commit_staged_assets(
                self.staging, self.live, "0.2.2", replace_func=failing_replace
            )
        for relative, content in old_files.items():
            self.assertEqual((self.live / relative).read_bytes(), content)
        self.assertFalse(list(self.live.rglob(".publish-assets-*")))

    def test_keyboard_interrupt_restores_live_and_cleans_transaction(self) -> None:
        self._assert_interrupt_rolls_back(KeyboardInterrupt)

    def test_system_exit_restores_live_and_cleans_transaction(self) -> None:
        self._assert_interrupt_rolls_back(SystemExit)

    def test_commit_failure_restores_all_replaced_live_files(self) -> None:
        self.seed_version(self.live, b"old")
        self.seed_version(self.staging, b"new")
        real_replace = os.replace
        calls = {"count": 0}

        def failing_replace(source, destination):
            calls["count"] += 1
            if calls["count"] == 2:
                raise OSError("injected commit failure")
            return real_replace(source, destination)

        with self.assertRaises(OSError):
            publish_assets.commit_staged_assets(
                self.staging,
                self.live,
                "0.2.2",
                replace_func=failing_replace,
            )
        self.assertEqual(
            (self.live / "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz").read_bytes(),
            b"old-asset",
        )
        self.assertEqual(
            (self.live / "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz.build-stamp.json").read_bytes(),
            b"old-stamp",
        )
        self.assertEqual(
            (self.live / "landing/updates/latest.json").read_bytes(),
            b"old-latest",
        )
        self.assertFalse(list(self.live.rglob(".publish-assets-*")))

    def test_rollback_failure_keeps_transaction_backup(self) -> None:
        self.seed_version(self.live, b"old")
        self.seed_version(self.staging, b"new")

        def failing_replace(source, destination):
            raise OSError("injected commit failure")

        with mock.patch.object(publish_assets, "_restore", side_effect=OSError("injected rollback failure")):
            with self.assertRaisesRegex(ValueError, "备份保留"):
                publish_assets.commit_staged_assets(
                    self.staging,
                    self.live,
                    "0.2.2",
                    replace_func=failing_replace,
                )
        transactions = list(self.live.glob(".publish-assets-*"))
        self.assertEqual(len(transactions), 1)
        self.assertTrue(
            (transactions[0] / "landing/downloads/MacSlim_0.2.2_aarch64.app.tar.gz").is_file()
        )


if __name__ == "__main__":
    unittest.main()
