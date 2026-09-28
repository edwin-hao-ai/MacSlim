from __future__ import annotations

import datetime
import hashlib
import io
import json
import os
import pathlib
import plistlib
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest

from scripts import updater_artifact


SCRIPT_PATH = pathlib.Path(__file__).resolve().parents[1] / "updater_artifact.py"
SYSTEM_PYTHON = "/usr/bin/python3"
APP_NAME = "MacSlim.app"
INFO_NAME = "MacSlim.app/Contents/Info.plist"
TARGET = "aarch64-apple-darwin"
SENTINEL = "archive-content-sentinel"
CPU_TYPE_X86_64 = 0x01000007
CPU_TYPE_ARM64 = 0x0100000C
FAT_MAGIC = 0xCAFEBABE


def thin_macho(architecture: str) -> bytes:
    cpu_type = CPU_TYPE_ARM64 if architecture == "arm64" else CPU_TYPE_X86_64
    return struct.pack(
        "<IIIIIIII",
        0xFEEDFACF,
        cpu_type,
        0,
        2,
        0,
        0,
        0,
        0,
    )


def fat_macho(architectures: tuple[str, ...]) -> bytes:
    slices = [(CPU_TYPE_ARM64, thin_macho("arm64"))] if architectures == ("arm64",) else []
    if architectures == ("x86_64",):
        slices = [(CPU_TYPE_X86_64, thin_macho("x86_64"))]
    elif architectures == ("arm64", "x86_64"):
        slices = [
            (CPU_TYPE_ARM64, thin_macho("arm64")),
            (CPU_TYPE_X86_64, thin_macho("x86_64")),
        ]
    if not slices:
        raise ValueError("unsupported fat fixture")
    offset = 0x1000
    result = bytearray(offset)
    struct.pack_into(">II", result, 0, FAT_MAGIC, len(slices))
    for index, (cpu_type, data) in enumerate(slices):
        entry_offset = 8 + index * 20
        struct.pack_into(
            ">IIIII",
            result,
            entry_offset,
            cpu_type,
            3,
            offset,
            len(data),
            14,
        )
        result.extend(b"\0" * (offset - len(result)))
        result.extend(data)
        offset += len(data)
    return bytes(result)


# 夹具版本必须等于项目真实版本 —— `updater_artifact.inspect_archive` 会强制
# 校验 archive 版本与 tauri.conf.json 一致（这是防错投的安全检查，不是缺陷）。
# 因此从配置读取单一真源，改版本号时本测试无需任何改动。
TAURI_CONFIG = pathlib.Path(__file__).resolve().parents[2] / "src-tauri" / "tauri.conf.json"
FIXTURE_VERSION = json.loads(TAURI_CONFIG.read_text())["version"]


def architecture_fixture(architecture: str) -> bytes:
    if architecture == "universal":
        return fat_macho(("arm64", "x86_64"))
    return thin_macho(architecture)


def info_data(
    version: str = FIXTURE_VERSION,
    identifier: str = "com.vgoapp.macslim",
    bundle_version: str | None = None,
) -> bytes:
    return plistlib.dumps(
        {
            "CFBundleIdentifier": identifier,
            "CFBundleShortVersionString": version,
            "CFBundleVersion": version if bundle_version is None else bundle_version,
            "CFBundleExecutable": "MacSlim",
        }
    )


def make_member(
    name: str,
    data: bytes = b"",
    member_type: bytes = tarfile.REGTYPE,
    linkname: str = "",
) -> tuple[tarfile.TarInfo, bytes]:
    item = tarfile.TarInfo(name)
    item.type = member_type
    item.linkname = linkname
    item.size = len(data) if item.isfile() else 0
    return item, data


def info_member(
    version: str = FIXTURE_VERSION,
    identifier: str = "com.vgoapp.macslim",
) -> tuple[tarfile.TarInfo, bytes]:
    return make_member(INFO_NAME, info_data(version, identifier))


def app_members(
    *members: tuple[tarfile.TarInfo, bytes],
    extra_directories: tuple[str, ...] = (),
    architecture: str = "arm64",
) -> list[tuple[tarfile.TarInfo, bytes]]:
    directories = (
        f"{APP_NAME}/",
        f"{APP_NAME}/Contents/",
        f"{APP_NAME}/Contents/MacOS/",
        *extra_directories,
    )
    executable = make_member(
        f"{APP_NAME}/Contents/MacOS/MacSlim",
        architecture_fixture(architecture),
    )
    return [
        *(make_member(name, member_type=tarfile.DIRTYPE) for name in directories),
        executable,
        *members,
    ]


def write_members(
    path: pathlib.Path,
    members: list[tuple[tarfile.TarInfo, bytes]],
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(path, "w:gz") as archive:
        for item, data in members:
            source = io.BytesIO(data) if item.isfile() else None
            archive.addfile(item, source)


def make_archive(
    path: pathlib.Path,
    version: str = FIXTURE_VERSION,
    identifier: str = "com.vgoapp.macslim",
    architecture: str = "arm64",
) -> None:
    write_members(
        path,
        app_members(
            info_member(version, identifier),
            architecture=architecture,
        ),
    )


def make_app(
    path: pathlib.Path,
    version: str = FIXTURE_VERSION,
    identifier: str = "com.vgoapp.macslim",
    architecture: str = "arm64",
    executable_suffix: bytes = b"",
    symlink_target: str | None = None,
) -> pathlib.Path:
    contents = path / "Contents"
    macos = contents / "MacOS"
    macos.mkdir(parents=True, exist_ok=True)
    (contents / "Info.plist").write_bytes(
        info_data(version=version, identifier=identifier)
    )
    (macos / "MacSlim").write_bytes(
        architecture_fixture(architecture) + executable_suffix
    )
    if symlink_target is not None:
        resources = contents / "Resources"
        resources.mkdir()
        (resources / "Shared").symlink_to(
            symlink_target,
            target_is_directory=True,
        )
        if symlink_target == "../Frameworks/Shared":
            shared = contents / "Frameworks" / "Shared"
            shared.mkdir(parents=True)
    return path


def write_test_stamp(path: pathlib.Path, target: str) -> pathlib.Path:
    return updater_artifact._write_stamp(path, target)


def file_sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run_cli(
    *arguments: str,
    executable: str = sys.executable,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    child_env = os.environ.copy() if env is None else env.copy()
    child_env["PYTHONDONTWRITEBYTECODE"] = "1"
    return subprocess.run(
        [executable, str(SCRIPT_PATH), *arguments],
        cwd=SCRIPT_PATH.parents[1],
        env=child_env,
        capture_output=True,
        text=True,
        check=False,
    )


class UpdaterArtifactTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(temporary_directory.cleanup)
        self.root = pathlib.Path(temporary_directory.name)
        self.archive = self.root / "MacSlim.app.tar.gz"

    def inspect(self, target=None) -> dict[str, object]:
        arguments = {
            "expected_version": FIXTURE_VERSION,
            "expected_identifier": "com.vgoapp.macslim",
        }
        if target is not None:
            arguments["expected_target"] = target
        return updater_artifact.inspect_archive(self.archive, **arguments)

    def rejection(self, pattern: str) -> str:
        with self.assertRaises(ValueError) as context:
            self.inspect()
        message = str(context.exception)
        self.assertRegex(message, pattern)
        return message

    def stamp_with(self, **updates: object) -> pathlib.Path:
        make_archive(self.archive)
        stamp = write_test_stamp(self.archive, TARGET)
        payload = json.loads(stamp.read_text(encoding="utf-8"))
        payload.update(updates)
        stamp.write_text(json.dumps(payload), encoding="utf-8")
        return stamp

    def test_inspects_valid_bundle(self) -> None:
        make_archive(self.archive)
        result = self.inspect()
        self.assertEqual(result["version"], FIXTURE_VERSION)
        self.assertEqual(result["identifier"], "com.vgoapp.macslim")
        self.assertEqual(result["bundle_version"], FIXTURE_VERSION)
        self.assertEqual(result["archive_sha256"], file_sha256(self.archive))

    def test_inspects_thin_macho_architectures(self) -> None:
        cases = (("arm64", "aarch64-apple-darwin"), ("x86_64", "x86_64-apple-darwin"))
        for architecture, target in cases:
            with self.subTest(architecture=architecture):
                make_archive(self.archive, architecture=architecture)
                result = self.inspect(target)
                self.assertEqual(result["architectures"], (architecture,))

    def test_inspects_universal_fat_macho(self) -> None:
        make_archive(self.archive, architecture="universal")
        result = self.inspect("universal-apple-darwin")
        self.assertEqual(result["architectures"], ("arm64", "x86_64"))

    def test_rejects_self_reported_target_for_wrong_macho(self) -> None:
        app = make_app(self.root / APP_NAME, architecture="x86_64")
        with self.assertRaisesRegex(ValueError, "架构|target"):
            updater_artifact.rebuild_archive(app, self.archive, TARGET)

    def test_rebuild_replaces_stale_archive_and_writes_stamp(self) -> None:
        make_archive(self.archive)
        old_hash = file_sha256(self.archive)
        write_test_stamp(self.archive, TARGET)
        app = make_app(self.root / APP_NAME, executable_suffix=b"current")
        rebuilt = updater_artifact.rebuild_archive(app, self.archive, TARGET)
        self.assertEqual(rebuilt, self.archive)
        self.assertNotEqual(file_sha256(self.archive), old_hash)
        updater_artifact.verify_stamp(self.archive, TARGET)
        with tarfile.open(self.archive, "r:gz") as archive:
            member = archive.getmember(f"{APP_NAME}/Contents/MacOS/MacSlim")
            self.assertTrue(archive.extractfile(member).read().endswith(b"current"))

    def test_rebuild_failure_removes_old_artifacts(self) -> None:
        make_archive(self.archive)
        write_test_stamp(self.archive, TARGET)
        signature = pathlib.Path(f"{self.archive}.sig")
        signature.write_text("stale-signature", encoding="utf-8")
        with self.assertRaises(ValueError):
            updater_artifact.rebuild_archive(
                self.root / "Missing.app", self.archive, TARGET
            )
        self.assertFalse(self.archive.exists())
        self.assertFalse(updater_artifact.stamp_path(self.archive).exists())
        self.assertFalse(signature.exists())

    def test_rebuild_preserves_safe_symlink_and_rejects_escape(self) -> None:
        app = make_app(
            self.root / APP_NAME,
            symlink_target="../Frameworks/Shared",
        )
        updater_artifact.rebuild_archive(app, self.archive, TARGET)
        with tarfile.open(self.archive, "r:gz") as archive:
            symlink = archive.getmember(f"{APP_NAME}/Contents/Resources/Shared")
            self.assertTrue(symlink.issym())
            self.assertEqual(symlink.linkname, "../Frameworks/Shared")
        unsafe_app = make_app(
            self.root / "unsafe" / APP_NAME,
            symlink_target="../../../outside",
        )
        with self.assertRaises(ValueError):
            updater_artifact.rebuild_archive(unsafe_app, self.archive, TARGET)
        self.assertFalse(self.archive.exists())

    def test_cli_rebuild_and_verify(self) -> None:
        app = make_app(self.root / APP_NAME)
        rebuilt = run_cli(
            "rebuild",
            "--app",
            str(app),
            "--archive",
            str(self.archive),
            "--target",
            TARGET,
        )
        verified = run_cli(
            "verify",
            "--archive",
            str(self.archive),
            "--target",
            TARGET,
        )
        self.assertEqual(rebuilt.returncode, 0, rebuilt.stdout + rebuilt.stderr)
        self.assertEqual(verified.returncode, 0, verified.stdout + verified.stderr)

    def test_rejects_wrong_version_and_identifier(self) -> None:
        cases = (("9.9.9", "com.vgoapp.macslim", "版本"), (FIXTURE_VERSION, SENTINEL, "标识符"))
        for version, identifier, pattern in cases:
            with self.subTest(version=version, identifier=identifier):
                make_archive(self.archive, version=version, identifier=identifier)
                self.rejection(pattern)

    def test_rejects_missing_multiple_and_symlink_info_plists(self) -> None:
        cases = (
            app_members(
                make_member("MacSlim.app/Contents/Resources/placeholder"),
                extra_directories=(f"{APP_NAME}/Contents/Resources/",),
            ),
            [make_member("First.app/Contents/Info.plist", info_data()), make_member("Second.app/Contents/Info.plist", info_data())],
            app_members(make_member(INFO_NAME, member_type=tarfile.SYMTYPE, linkname="Resources")),
        )
        for members in cases:
            with self.subTest(names=[str(item.name) for item, _ in members]):
                write_members(self.archive, members)
                self.rejection("MacSlim.app|必须包含且只包含一个 app Info.plist")

    def test_rejects_malformed_plist_without_leaking_content(self) -> None:
        write_members(self.archive, app_members(make_member(INFO_NAME, b"<plist>" + SENTINEL.encode())))
        self.assertNotIn(SENTINEL, self.rejection("Info.plist"))

    def test_rejects_empty_bundle_version(self) -> None:
        write_members(self.archive, app_members(make_member(INFO_NAME, info_data(bundle_version=""))))
        self.rejection("CFBundleVersion")

    def test_rejects_wrong_bundle_root(self) -> None:
        write_members(self.archive, [make_member("Evil.app/Contents/Info.plist", info_data())])
        self.assertNotIn("Evil.app", self.rejection("MacSlim.app"))

    def test_rejects_other_app_members(self) -> None:
        names = ("Evil.app/Contents/Resources/payload", "MacSlim.app/Contents/Helper.app/Resources/payload")
        for name in names:
            with self.subTest(name=name):
                write_members(self.archive, app_members(info_member(), make_member(name)))
                self.rejection("MacSlim.app")

    def test_rejects_unsafe_member_paths(self) -> None:
        names = (f"../{SENTINEL}", f"/{SENTINEL}", f"MacSlim.app/../{SENTINEL}")
        for name in names:
            with self.subTest(name=name):
                write_members(self.archive, app_members(info_member(), make_member(name)))
                self.assertNotIn(SENTINEL, self.rejection("不安全"))

    def test_rejects_duplicate_and_casefold_members(self) -> None:
        duplicate_info = make_member(INFO_NAME, info_data())
        variant_info = make_member("MacSlim.app/Contents/info.plist", info_data())
        cases = (
            ("exact", app_members(info_member(), duplicate_info)),
            ("regular-symlink", app_members(info_member(), make_member(INFO_NAME, member_type=tarfile.SYMTYPE, linkname="Info.plist"))),
            ("casefold", app_members(info_member(), variant_info)),
            ("unicode", app_members(info_member(), make_member("MacSlim.app/Contents/Café"), make_member("MacSlim.app/Contents/Cafe\u0301"))),
        )
        for label, members in cases:
            with self.subTest(label=label):
                write_members(self.archive, members)
                self.rejection("重复")

    def test_rejects_parent_regular_with_child(self) -> None:
        members = [make_member(f"{APP_NAME}/", member_type=tarfile.DIRTYPE), make_member(f"{APP_NAME}/Contents", b"parent"), info_member()]
        write_members(self.archive, members)
        self.rejection("祖先")

    def test_rejects_parent_symlink_with_child(self) -> None:
        parent = make_member(f"{APP_NAME}/Contents", member_type=tarfile.SYMTYPE, linkname="Frameworks")
        members = [make_member(f"{APP_NAME}/", member_type=tarfile.DIRTYPE), parent, info_member()]
        write_members(self.archive, members)
        self.rejection("祖先")

    def test_rejects_missing_explicit_root_or_ancestor(self) -> None:
        root = make_member(f"{APP_NAME}/", member_type=tarfile.DIRTYPE)
        for members in ([info_member()], [root, info_member()]):
            with self.subTest(names=[str(item.name) for item, _ in members]):
                write_members(self.archive, members)
                self.rejection("MacSlim.app|祖先")

    def test_rejects_unsafe_symlinks(self) -> None:
        links = ("", f"/{SENTINEL}", f"../../../../{SENTINEL}")
        for linkname in links:
            with self.subTest(linkname=linkname):
                name = f"MacSlim.app/Contents/Resources/link-{len(linkname)}"
                member = make_member(name, member_type=tarfile.SYMTYPE, linkname=linkname)
                directories = (f"{APP_NAME}/Contents/Resources/",)
                write_members(self.archive, app_members(info_member(), member, extra_directories=directories))
                self.assertNotIn(SENTINEL, self.rejection("symlink"))

    def test_accepts_valid_directory_chain_and_safe_symlink(self) -> None:
        directories = (f"{APP_NAME}/Contents/Resources/", f"{APP_NAME}/Contents/Frameworks/", f"{APP_NAME}/Contents/Frameworks/Shared/")
        symlink = make_member("MacSlim.app/Contents/Resources/Shared", member_type=tarfile.SYMTYPE, linkname="../Frameworks/Shared")
        write_members(self.archive, app_members(info_member(), symlink, extra_directories=directories))
        self.inspect()

    def test_rejects_unsupported_member_type(self) -> None:
        pipe = make_member("MacSlim.app/Contents/Resources/pipe", member_type=tarfile.FIFOTYPE)
        write_members(self.archive, app_members(info_member(), pipe, extra_directories=(f"{APP_NAME}/Contents/Resources/",)))
        self.rejection("不支持")

    def test_writes_stamp_with_artifact_identity(self) -> None:
        app = make_app(self.root / APP_NAME)
        updater_artifact.rebuild_archive(app, self.archive, TARGET)
        stamp = updater_artifact.stamp_path(self.archive)
        payload = json.loads(stamp.read_text(encoding="utf-8"))
        self.assertEqual(stamp, updater_artifact.stamp_path(self.archive))
        self.assertIs(type(payload["schema_version"]), int)
        self.assertEqual(payload["product"], "MacSlim")
        self.assertEqual(payload["version"], FIXTURE_VERSION)
        self.assertEqual(payload["target"], TARGET)
        self.assertEqual(payload["identifier"], "com.vgoapp.macslim")
        self.assertEqual(payload["bundle_version"], FIXTURE_VERSION)
        self.assertEqual(payload["architectures"], ["arm64"])
        self.assertEqual(payload["archive_sha256"], file_sha256(self.archive))
        created_at = datetime.datetime.fromisoformat(payload["created_at"].replace("Z", "+00:00"))
        self.assertIsNotNone(created_at.tzinfo)

    def test_verifies_matching_stamp(self) -> None:
        make_archive(self.archive)
        write_test_stamp(self.archive, TARGET)
        updater_artifact.verify_stamp(self.archive, TARGET)

    def test_rejects_tamper_after_stamp(self) -> None:
        make_archive(self.archive)
        write_test_stamp(self.archive, TARGET)
        tampered = make_member(f"MacSlim.app/Contents/{SENTINEL}")
        write_members(self.archive, app_members(info_member(), tampered))
        with self.assertRaisesRegex(ValueError, "校验和"):
            updater_artifact.verify_stamp(self.archive, TARGET)

    def test_rejects_wrong_target_and_missing_stamp(self) -> None:
        make_archive(self.archive)
        with self.assertRaisesRegex(ValueError, "缺少"):
            updater_artifact.verify_stamp(self.archive, TARGET)
        write_test_stamp(self.archive, TARGET)
        with self.assertRaisesRegex(ValueError, "target"):
            updater_artifact.verify_stamp(self.archive, "x86_64-apple-darwin")

    def test_rejects_non_integer_schema_version(self) -> None:
        for value in (True, 1.0, "1"):
            with self.subTest(value=value):
                self.stamp_with(schema_version=value)
                with self.assertRaisesRegex(ValueError, "schema_version"):
                    updater_artifact.verify_stamp(self.archive, TARGET)

    def test_rejects_invalid_created_at(self) -> None:
        for value in ("not-a-time", "2026-09-25T12:34:56", "2026-09-25T12:34:56+25:00"):
            with self.subTest(value=value):
                self.stamp_with(created_at=value)
                with self.assertRaisesRegex(ValueError, "创建时间"):
                    updater_artifact.verify_stamp(self.archive, TARGET)

    def test_accepts_timezone_aware_created_at(self) -> None:
        self.stamp_with(created_at="2026-09-25T12:34:56+08:00")
        updater_artifact.verify_stamp(self.archive, TARGET)

    def test_cli_stamp_is_rejected_and_verify_remains_available(self) -> None:
        app = make_app(self.root / APP_NAME)
        updater_artifact.rebuild_archive(app, self.archive, TARGET)
        default_stamp = updater_artifact.stamp_path(self.archive)
        stamp_result = run_cli(
            "stamp", "--archive", str(self.archive), "--target", TARGET
        )
        self.assertNotEqual(stamp_result.returncode, 0)
        self.assertNotIn("build stamp 已写入", stamp_result.stdout)
        verified = run_cli("verify", "--archive", str(self.archive), "--target", TARGET)
        self.assertEqual(verified.returncode, 0, verified.stdout + verified.stderr)
        custom_stamp = self.root / "custom" / "artifact.stamp.json"
        custom_stamp.parent.mkdir()
        custom_stamp.write_bytes(default_stamp.read_bytes())
        default_stamp.unlink()
        completed = run_cli(
            "verify",
            "--archive",
            str(self.archive),
            "--target",
            TARGET,
            "--stamp",
            str(custom_stamp),
        )
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
        self.assertFalse(hasattr(updater_artifact, "write_stamp"))

    def test_system_python39_cli_compatibility(self) -> None:
        version = subprocess.run([SYSTEM_PYTHON, "--version"], capture_output=True, text=True, check=False)
        self.assertIn("Python 3.9.", version.stdout + version.stderr)
        app = make_app(self.root / APP_NAME)
        rebuilt = run_cli(
            "rebuild",
            "--app",
            str(app),
            "--archive",
            str(self.archive),
            "--target",
            TARGET,
            executable=SYSTEM_PYTHON,
        )
        verified = run_cli("verify", "--archive", str(self.archive), "--target", TARGET, executable=SYSTEM_PYTHON)
        self.assertEqual(rebuilt.returncode, 0, rebuilt.stdout + rebuilt.stderr)
        self.assertEqual(verified.returncode, 0, verified.stdout + verified.stderr)

    def test_cli_error_is_chinese_and_sanitized(self) -> None:
        app = make_app(self.root / APP_NAME, identifier=SENTINEL)
        env = os.environ.copy()
        env["TAURI_SIGNING_PRIVATE_KEY"] = "fake-private-key-sentinel"
        env["TAURI_SIGNING_PRIVATE_KEY_PASSWORD"] = "fake-password-sentinel"
        completed = run_cli(
            "rebuild",
            "--app",
            str(app),
            "--archive",
            str(self.archive),
            "--target",
            TARGET,
            env=env,
        )
        output = completed.stdout + completed.stderr
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("错误", output)
        self.assertIn("标识符", output)
        self.assertNotIn(SENTINEL, output)
        self.assertNotIn("fake-private-key-sentinel", output)
        self.assertNotIn("fake-password-sentinel", output)


if __name__ == "__main__":
    unittest.main()
