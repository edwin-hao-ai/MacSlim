from __future__ import annotations

import argparse
import hashlib
import json
import os
import plistlib
import posixpath
import re
import struct
import sys
import tarfile
import tempfile
import unicodedata
from datetime import datetime, timezone
from pathlib import Path
from xml.parsers.expat import ExpatError


APP_NAME = "MacSlim.app"
INFO_NAME = f"{APP_NAME}/Contents/Info.plist"
SCHEMA_VERSION = 2
HASH_CHUNK_SIZE = 1024 * 1024
CPU_TYPE_X86_64 = 0x01000007
CPU_TYPE_ARM64 = 0x0100000C
FAT_MAGIC = 0xCAFEBABE
FAT_CIGAM = 0xBEBAFECA
FAT_MAGIC_64 = 0xCAFEBABF
FAT_CIGAM_64 = 0xBFBAFECA
TARGET_ARCHITECTURES = {
    "aarch64-apple-darwin": ("arm64",),
    "x86_64-apple-darwin": ("x86_64",),
    "universal-apple-darwin": ("arm64", "x86_64"),
}
TAURI_CONFIG_PATH = Path(__file__).resolve().parents[1] / "src-tauri/tauri.conf.json"
ISO_DATETIME_RE = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?(?:Z|[+-]\d{2}:\d{2})$"
)
PLIST_ERRORS = (
    ValueError,
    TypeError,
    OverflowError,
    struct.error,
    ExpatError,
    plistlib.InvalidFileException,
)


def _canonical_key(value: str) -> str:
    return unicodedata.normalize("NFC", value).casefold()


APP_KEY = _canonical_key(APP_NAME)
INFO_KEY = _canonical_key(INFO_NAME)


def _read_tauri_config() -> dict[str, str]:
    try:
        config = json.loads(TAURI_CONFIG_PATH.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError, RecursionError):
        raise ValueError("无法读取 Tauri 配置") from None
    if not isinstance(config, dict):
        raise ValueError("Tauri 配置格式无效")
    values = {
        "product": config.get("productName"),
        "version": config.get("version"),
        "identifier": config.get("identifier"),
    }
    if not all(isinstance(value, str) and value.strip() for value in values.values()):
        raise ValueError("Tauri 配置缺少有效的产品、版本或标识符")
    return {key: value for key, value in values.items() if isinstance(value, str)}


def _validate_expected_value(value: str, label: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"期望{label}不能为空")
    return value


def _archive_sha256(archive: Path) -> str:
    digest = hashlib.sha256()
    try:
        with archive.open("rb") as source:
            for chunk in iter(lambda: source.read(HASH_CHUNK_SIZE), b""):
                digest.update(chunk)
    except OSError:
        raise ValueError("无法计算 updater archive 校验和") from None
    return digest.hexdigest()


def _read_info_plist(archive: tarfile.TarFile, member: tarfile.TarInfo) -> object:
    try:
        source = archive.extractfile(member)
        if source is None:
            raise ValueError("无法读取 app Info.plist")
        with source:
            return plistlib.loads(source.read())
    except PLIST_ERRORS:
        raise ValueError("updater archive 的 Info.plist 无法解析") from None
    except (OSError, EOFError, KeyError):
        raise ValueError("无法读取 app Info.plist") from None


def _validated_info(
    info: object,
    expected_version: str,
    expected_identifier: str,
) -> tuple[str, str, str, str]:
    if not isinstance(info, dict):
        raise ValueError("updater archive 的 Info.plist 格式无效")
    identifier = info.get("CFBundleIdentifier")
    if identifier != expected_identifier:
        raise ValueError("updater archive 标识符与 Tauri 配置不一致")
    version = info.get("CFBundleShortVersionString")
    if version != expected_version:
        raise ValueError("updater archive 版本与 Tauri 配置不一致")
    bundle_version = info.get("CFBundleVersion")
    if not isinstance(bundle_version, str) or not bundle_version.strip():
        raise ValueError("updater archive 的 CFBundleVersion 不能为空")
    executable = info.get("CFBundleExecutable")
    if (
        not isinstance(executable, str)
        or not executable
        or executable in {".", ".."}
        or "/" in executable
        or "\\" in executable
        or "\x00" in executable
        or any(ord(character) < 32 for character in executable)
    ):
        raise ValueError("updater archive 的 CFBundleExecutable 无效")
    return version, identifier, bundle_version, executable


def _member_kind(member: tarfile.TarInfo) -> str:
    if member.isdir():
        return "directory"
    if member.issym():
        return "symlink"
    if member.isfile() and getattr(member, "sparse", None) is None:
        return "file"
    raise ValueError("updater archive 包含不支持的 member 类型")


def _safe_member_name(member: tarfile.TarInfo) -> str:
    name = member.name
    if not isinstance(name, str) or not name or "\x00" in name:
        raise ValueError("updater archive 包含不安全路径")
    if name.startswith("/") or any(ord(character) < 32 for character in name):
        raise ValueError("updater archive 包含不安全路径")
    canonical = name[:-1] if name.endswith("/") else name
    parts = canonical.split("/")
    if any(part in {"", ".", ".."} for part in parts):
        raise ValueError("updater archive 包含不安全路径")
    nested_app = any(part.casefold().endswith(".app") for part in parts[1:])
    if parts[0] != APP_NAME or nested_app:
        raise ValueError("updater archive 必须包含且只包含一个 MacSlim.app")
    if canonical == APP_NAME and not member.isdir():
        raise ValueError("updater archive 包含不安全路径")
    if name.endswith("/") and not member.isdir():
        raise ValueError("updater archive 包含不安全路径")
    return canonical


def _validate_symlink(member_name: str, linkname: str) -> None:
    if not linkname or not isinstance(linkname, str) or linkname.startswith("/"):
        raise ValueError("updater archive 包含不安全的 symlink")
    if "\x00" in linkname or any(part in {"", "."} for part in linkname.split("/")):
        raise ValueError("updater archive 包含不安全的 symlink")
    resolved = posixpath.normpath(
        posixpath.join(posixpath.dirname(member_name), linkname)
    )
    if resolved != APP_NAME and not resolved.startswith(f"{APP_NAME}/"):
        raise ValueError("updater archive 包含不安全的 symlink")


def _validate_ancestors(
    member_name: str,
    entries: dict[str, tuple[str, str, tarfile.TarInfo]],
) -> None:
    parts = member_name.split("/")
    for depth in range(1, len(parts)):
        ancestor_name = "/".join(parts[:depth])
        ancestor = entries.get(_canonical_key(ancestor_name))
        if ancestor is None or ancestor[1] != "directory":
            raise ValueError("updater archive 的每个祖先必须是显式目录")


def _validated_info_member(members: list[tarfile.TarInfo]) -> tarfile.TarInfo:
    entries: dict[str, tuple[str, str, tarfile.TarInfo]] = {}
    info_members: list[tarfile.TarInfo] = []
    for member in members:
        kind = _member_kind(member)
        member_name = _safe_member_name(member)
        member_key = _canonical_key(member_name)
        if member_key in entries:
            raise ValueError("updater archive 包含重复 member")
        entries[member_key] = (member_name, kind, member)
        if kind == "symlink":
            _validate_symlink(member_name, member.linkname)
        if kind == "file" and member_key == INFO_KEY:
            info_members.append(member)
    root = entries.get(APP_KEY)
    if root is None or root[0] != APP_NAME or root[1] != "directory":
        raise ValueError("updater archive 必须包含且只包含一个 MacSlim.app")
    for member_name, _, _ in entries.values():
        if member_name != APP_NAME:
            _validate_ancestors(member_name, entries)
    if len(info_members) != 1:
        raise ValueError("updater archive 必须包含且只包含一个 app Info.plist")
    return info_members[0]


def _executable_member(
    members: list[tarfile.TarInfo], executable: str
) -> tarfile.TarInfo:
    expected_name = f"{APP_NAME}/Contents/MacOS/{executable}"
    expected_key = _canonical_key(expected_name)
    matches = []
    for member in members:
        member_name = member.name[:-1] if member.name.endswith("/") else member.name
        if _canonical_key(member_name) != expected_key:
            continue
        if not member.isfile() or getattr(member, "sparse", None) is not None:
            raise ValueError("updater archive 的 CFBundleExecutable 不是普通文件")
        matches.append(member)
    if len(matches) != 1:
        raise ValueError("updater archive 的 CFBundleExecutable 不存在或不唯一")
    return matches[0]


def _read_member_bytes(
    archive: tarfile.TarFile, member: tarfile.TarInfo, label: str
) -> bytes:
    try:
        source = archive.extractfile(member)
        if source is None:
            raise ValueError(f"无法读取 {label}")
        with source:
            return source.read()
    except ValueError:
        raise
    except (OSError, EOFError, KeyError):
        raise ValueError(f"无法读取 {label}") from None


def _cpu_type_architecture(cpu_type: int) -> str | None:
    if cpu_type == CPU_TYPE_ARM64:
        return "arm64"
    if cpu_type == CPU_TYPE_X86_64:
        return "x86_64"
    return None


def _parse_thin_macho(data: bytes) -> tuple[str, ...]:
    formats = {
        b"\xce\xfa\xed\xfe": ("<", 32),
        b"\xcf\xfa\xed\xfe": ("<", 64),
        b"\xfe\xed\xfa\xce": (">", 32),
        b"\xfe\xed\xfa\xcf": (">", 64),
    }
    format_info = formats.get(data[:4])
    if format_info is None or len(data) < 8:
        raise ValueError("CFBundleExecutable 不是受支持的 Mach-O 文件")
    endian, bits = format_info
    try:
        cpu_type = struct.unpack_from(f"{endian}I", data, 4)[0]
    except struct.error:
        raise ValueError("Mach-O header 无效") from None
    architecture = _cpu_type_architecture(cpu_type)
    if architecture is None:
        raise ValueError("Mach-O CPU 类型不受支持")
    if len(data) < (32 if bits == 64 else 28):
        raise ValueError("Mach-O header 无效")
    return (architecture,)


def _parse_fat_macho(data: bytes) -> tuple[str, ...]:
    if len(data) < 8:
        raise ValueError("fat Mach-O header 无效")
    magic = struct.unpack_from(">I", data, 0)[0]
    if magic in {FAT_MAGIC, FAT_MAGIC_64}:
        endian = ">"
    elif magic in {FAT_CIGAM, FAT_CIGAM_64}:
        endian = "<"
    else:
        raise ValueError("fat Mach-O header 无效")
    bits = 64 if magic in {FAT_MAGIC_64, FAT_CIGAM_64} else 32
    entry_size = 32 if bits == 64 else 20
    nfat_arch = struct.unpack_from(f"{endian}I", data, 4)[0]
    header_size = 8 + nfat_arch * entry_size
    if nfat_arch == 0 or header_size > len(data):
        raise ValueError("fat Mach-O header 无效")
    architectures: set[str] = set()
    for index in range(nfat_arch):
        entry_offset = 8 + index * entry_size
        if bits == 64:
            cpu_type, _, offset, size, _, _ = struct.unpack_from(
                f"{endian}IIQQII", data, entry_offset
            )
        else:
            cpu_type, _, offset, size, _ = struct.unpack_from(
                f"{endian}IIIII", data, entry_offset
            )
        if size == 0 or offset > len(data) or size > len(data) - offset:
            raise ValueError("fat Mach-O slice 无效")
        declared = _cpu_type_architecture(cpu_type)
        actual = set(_parse_macho(data[offset : offset + size]))
        if declared is None or declared not in actual:
            raise ValueError("fat Mach-O CPU type 与 slice 不一致")
        architectures.update(actual)
    order = {"arm64": 0, "x86_64": 1}
    return tuple(sorted(architectures, key=order.__getitem__))


def _parse_macho(data: bytes) -> tuple[str, ...]:
    if data[:4] in {
        b"\xce\xfa\xed\xfe",
        b"\xcf\xfa\xed\xfe",
        b"\xfe\xed\xfa\xce",
        b"\xfe\xed\xfa\xcf",
    }:
        return _parse_thin_macho(data)
    return _parse_fat_macho(data)


def _expected_architectures(target: str) -> tuple[str, ...]:
    try:
        return TARGET_ARCHITECTURES[target]
    except KeyError:
        raise ValueError("target 不受支持") from None


def _validate_architecture(actual: tuple[str, ...], target: str) -> None:
    expected = _expected_architectures(target)
    if set(actual) != set(expected):
        raise ValueError("updater archive 架构与 target 不一致")


def inspect_archive(
    archive: Path,
    expected_version: str,
    expected_identifier: str,
    expected_target: str | None = None,
) -> dict[str, object]:
    _validate_expected_value(expected_version, "版本")
    _validate_expected_value(expected_identifier, "标识符")
    if expected_target is not None:
        _expected_architectures(expected_target)
    archive_path = Path(archive)
    if not archive_path.is_file():
        raise ValueError("updater archive 不存在")
    try:
        with tarfile.open(archive_path, mode="r:gz") as opened_archive:
            members = opened_archive.getmembers()
            info_member = _validated_info_member(members)
            info = _read_info_plist(opened_archive, info_member)
            version, identifier, bundle_version, executable = _validated_info(
                info,
                expected_version,
                expected_identifier,
            )
            executable_member = _executable_member(members, executable)
            executable_data = _read_member_bytes(
                opened_archive, executable_member, "CFBundleExecutable"
            )
            architectures = _parse_macho(executable_data)
    except ValueError:
        raise
    except (OSError, EOFError, tarfile.TarError, struct.error):
        raise ValueError("无法读取 updater archive") from None
    if expected_target is not None:
        _validate_architecture(architectures, expected_target)
    return {
        "version": version,
        "identifier": identifier,
        "bundle_version": bundle_version,
        "architectures": architectures,
        "archive_sha256": _archive_sha256(archive_path),
    }


def stamp_path(archive: Path) -> Path:
    return Path(f"{archive}.build-stamp.json")


def _write_json(path: Path, payload: dict[str, object]) -> None:
    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="w",
            encoding="utf-8",
            dir=path.parent,
            prefix=f".{path.name}.",
            delete=False,
        ) as destination:
            temporary_path = Path(destination.name)
            json.dump(payload, destination, ensure_ascii=False, sort_keys=True)
            destination.write("\n")
            destination.flush()
            os.fsync(destination.fileno())
        temporary_path.replace(path)
    except (OSError, TypeError, ValueError):
        raise ValueError("无法写入 updater build stamp") from None
    finally:
        if temporary_path is not None:
            try:
                temporary_path.unlink(missing_ok=True)
            except OSError:
                pass


def _write_stamp(archive: Path, target: str) -> Path:
    target = _validate_expected_value(target, "target")
    archive = Path(archive)
    config = _read_tauri_config()
    details = inspect_archive(
        archive,
        expected_version=config["version"],
        expected_identifier=config["identifier"],
        expected_target=target,
    )
    destination = stamp_path(archive)
    payload = {
        "schema_version": SCHEMA_VERSION,
        "product": config["product"],
        "version": details["version"],
        "target": target,
        "identifier": details["identifier"],
        "bundle_version": details["bundle_version"],
        "architectures": list(details["architectures"]),
        "archive_sha256": details["archive_sha256"],
        "created_at": datetime.now(timezone.utc)
        .isoformat(timespec="seconds")
        .replace("+00:00", "Z"),
    }
    _write_json(destination, payload)
    return destination


def _remove_artifact(path: Path) -> None:
    try:
        path.unlink()
    except FileNotFoundError:
        return
    except OSError:
        raise ValueError("无法失效旧 updater artifact") from None


def _invalidate_artifacts(archive: Path) -> None:
    for path in (archive, Path(f"{archive}.sig"), stamp_path(archive)):
        _remove_artifact(path)


def _validate_app_tree(app_path: Path) -> None:
    if app_path.name != APP_NAME or app_path.is_symlink() or not app_path.is_dir():
        raise ValueError("rebuild 的 app 根目录必须是 MacSlim.app")
    pending = [app_path]
    while pending:
        directory = pending.pop()
        try:
            entries = list(os.scandir(directory))
        except OSError:
            raise ValueError("无法读取 app 目录") from None
        for entry in entries:
            relative = Path(entry.path).relative_to(app_path)
            name = relative.as_posix()
            if (
                not name
                or name.startswith("/")
                or "\\" in name
                or "\x00" in name
                or any(ord(character) < 32 for character in name)
                or any(part in {"", ".", ".."} for part in name.split("/"))
                or any(part.casefold().endswith(".app") for part in name.split("/"))
            ):
                raise ValueError("app 包含不安全路径")
            if entry.is_symlink():
                try:
                    linkname = os.readlink(entry.path)
                except OSError:
                    raise ValueError("无法读取 app symlink") from None
                _validate_symlink(f"{APP_NAME}/{name}", linkname)
            elif entry.is_dir(follow_symlinks=False):
                pending.append(Path(entry.path))
            elif not entry.is_file(follow_symlinks=False):
                raise ValueError("app 包含不支持的文件类型")


def _read_app_info(app_path: Path, config: dict[str, str]) -> str:
    info_path = app_path / "Contents" / "Info.plist"
    if not info_path.is_file():
        raise ValueError("app 缺少 Info.plist")
    try:
        info = plistlib.loads(info_path.read_bytes())
    except PLIST_ERRORS:
        raise ValueError("app Info.plist 无法解析") from None
    return _validated_info(
        info,
        config["version"],
        config["identifier"],
    )[3]


def rebuild_archive(app_path: Path, archive: Path, target: str) -> Path:
    target = _validate_expected_value(target, "target")
    app_path = Path(app_path)
    archive = Path(archive)
    _invalidate_artifacts(archive)
    config = _read_tauri_config()
    _validate_app_tree(app_path)
    _read_app_info(app_path, config)
    temporary_path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile(
            mode="wb",
            dir=archive.parent,
            prefix=f".{archive.name}.",
            delete=False,
        ) as destination:
            temporary_path = Path(destination.name)
        with tarfile.open(temporary_path, mode="w:gz", dereference=False) as opened:
            opened.add(app_path, arcname=APP_NAME, recursive=True)
        inspect_archive(
            temporary_path,
            expected_version=config["version"],
            expected_identifier=config["identifier"],
            expected_target=target,
        )
        with temporary_path.open("rb") as source:
            os.fsync(source.fileno())
        os.replace(temporary_path, archive)
        temporary_path = None
        _write_stamp(archive, target)
    except ValueError:
        raise
    except (OSError, EOFError, tarfile.TarError, struct.error):
        raise ValueError("无法重建 updater archive") from None
    finally:
        if temporary_path is not None:
            try:
                temporary_path.unlink(missing_ok=True)
            except OSError:
                pass
    return archive


def rebuild(app_path: Path, archive: Path, target: str) -> Path:
    return rebuild_archive(app_path, archive, target)


def _load_stamp(path: Path) -> dict[str, object]:
    if not path.is_file():
        raise ValueError("缺少 updater build stamp")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError, RecursionError):
        raise ValueError("无法读取 updater build stamp") from None
    if not isinstance(payload, dict):
        raise ValueError("updater build stamp 格式无效")
    return payload


def _is_timezone_aware_iso8601(value: object) -> bool:
    if not isinstance(value, str) or ISO_DATETIME_RE.fullmatch(value) is None:
        return False
    normalized = value[:-1] + "+00:00" if value.endswith("Z") else value
    try:
        parsed = datetime.fromisoformat(normalized)
    except ValueError:
        return False
    return parsed.tzinfo is not None and parsed.utcoffset() is not None


def _verify_fields(
    payload: dict[str, object],
    expected: dict[str, object],
) -> None:
    schema_version = payload.get("schema_version")
    if type(schema_version) is not int or schema_version != SCHEMA_VERSION:
        raise ValueError("updater build stamp 的 schema_version 无效")
    if not _is_timezone_aware_iso8601(payload.get("created_at")):
        raise ValueError("updater build stamp 的创建时间无效")
    labels = {
        "schema_version": "schema_version",
        "product": "产品",
        "version": "版本",
        "target": "target",
        "identifier": "标识符",
        "bundle_version": "bundle version",
        "architectures": "架构",
        "archive_sha256": "校验和",
    }
    for field, expected_value in expected.items():
        if payload.get(field) != expected_value:
            label = labels.get(field, field)
            raise ValueError(f"updater build stamp 的{label}不一致")


def _verify_stamp(archive: Path, target: str, selected_stamp: Path) -> None:
    target = _validate_expected_value(target, "target")
    archive = Path(archive)
    config = _read_tauri_config()
    details = inspect_archive(
        archive,
        expected_version=config["version"],
        expected_identifier=config["identifier"],
        expected_target=target,
    )
    payload = _load_stamp(selected_stamp)
    expected = {
        "schema_version": SCHEMA_VERSION,
        "product": config["product"],
        "version": details["version"],
        "target": target,
        "identifier": details["identifier"],
        "bundle_version": details["bundle_version"],
        "architectures": list(details["architectures"]),
        "archive_sha256": details["archive_sha256"],
    }
    _verify_fields(payload, expected)


def verify_stamp(archive: Path, target: str) -> None:
    archive = Path(archive)
    _verify_stamp(archive, target, stamp_path(archive))


class ChineseArgumentParser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        self.exit(2, "错误: 命令行参数无效\n")


def _build_parser() -> argparse.ArgumentParser:
    parser = ChineseArgumentParser(description="校验 updater archive build stamp")
    commands = parser.add_subparsers(dest="command", required=True)
    verify = commands.add_parser("verify", help="校验 build stamp")
    verify.add_argument("--archive", type=Path, required=True, help="gzip tar archive 路径")
    verify.add_argument("--target", required=True, help="target triple")
    verify.add_argument("--stamp", type=Path, help="build stamp 路径")
    rebuild_command = commands.add_parser("rebuild", help="从 app 重建 archive")
    rebuild_command.add_argument("--app", type=Path, required=True, help="MacSlim.app 路径")
    rebuild_command.add_argument("--archive", type=Path, required=True, help="gzip tar archive 路径")
    rebuild_command.add_argument("--target", required=True, help="target triple")
    return parser


def main(arguments: list[str] | None = None) -> int:
    parser = _build_parser()
    options = parser.parse_args(arguments)
    try:
        if options.command == "verify":
            selected_stamp = options.stamp or stamp_path(options.archive)
            _verify_stamp(options.archive, options.target, selected_stamp)
            print("updater archive build stamp 校验通过")
        else:
            destination = rebuild_archive(options.app, options.archive, options.target)
            print(f"updater archive 已重建: {destination}")

    except (OSError, ValueError) as error:
        print(f"错误: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
