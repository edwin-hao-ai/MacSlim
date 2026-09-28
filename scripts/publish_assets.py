from __future__ import annotations

import argparse
import os
import re
import shutil
import sys
import tempfile
from pathlib import Path

VERSION_RE = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+")
ASSET_SUFFIXES = (
    ".app.tar.gz",
    ".app.tar.gz.build-stamp.json",
    ".app.tar.gz.sig",
)
COMPAT_NAMES = {"0.1.0.json", "0.2.2.json"}
COMPAT_PLATFORMS = {"darwin-aarch64", "darwin-x86_64"}


def _validate_version(version: str) -> str:
    if not isinstance(version, str) or VERSION_RE.fullmatch(version) is None:
        raise ValueError("发布版本无效")
    return version


def _is_selected(relative: Path, version: str) -> bool:
    parts = relative.parts
    if len(parts) != 3 or parts[:2] != ("landing", "downloads"):
        return False
    name = parts[-1]
    prefix = f"MacSlim_{version}_"
    return name.startswith(prefix) and name.endswith(ASSET_SUFFIXES)


def _is_manifest(relative: Path) -> bool:
    parts = relative.parts
    if len(parts) == 3 and parts[:2] == ("landing", "updates"):
        return parts[2] == "latest.json"
    return (
        len(parts) == 4
        and parts[:2] == ("landing", "updates")
        and parts[2] in COMPAT_PLATFORMS
        and parts[3] in COMPAT_NAMES
    )


def _absolute_path(path: Path) -> Path:
    return Path(os.path.abspath(os.fspath(path)))


def _fixed_directory(path: Path, label: str) -> Path:
    absolute = _absolute_path(path)
    if absolute.is_symlink():
        raise ValueError(f"{label} 根目录不得是 symlink")
    try:
        resolved = absolute.resolve(strict=True)
    except (OSError, RuntimeError):
        raise ValueError(f"{label} 根目录不存在或无法解析") from None
    if not resolved.is_dir():
        raise ValueError(f"{label} 根目录不是目录")
    return resolved


def _safe_relative_path(root: Path, relative: Path, label: str) -> Path:
    if relative.is_absolute() or ".." in relative.parts:
        raise ValueError(f"{label} 路径无效")
    candidate = root / relative
    current = root
    for part in relative.parts:
        current = current / part
        if current.is_symlink():
            raise ValueError(f"{label} 不得经过 symlink")
        if current != candidate and current.exists() and not current.is_dir():
            raise ValueError(f"{label} 祖先不是目录")
    try:
        resolved_root = root.resolve(strict=True)
        resolved_candidate = candidate.resolve(strict=False)
        inside = os.path.commonpath(
            (str(resolved_root), str(resolved_candidate))
        ) == str(resolved_root)
    except (OSError, RuntimeError, ValueError):
        raise ValueError(f"{label} 路径无法确认") from None
    if not inside:
        raise ValueError(f"{label} 路径越界")
    return candidate


def _prepare_destination(root: Path, relative: Path) -> Path:
    destination = _safe_relative_path(root, relative, "live destination")
    current = root
    for part in relative.parent.parts:
        current = current / part
        if current.is_symlink():
            raise ValueError("live destination 不得经过 symlink")
        if current.exists():
            if not current.is_dir():
                raise ValueError("live destination 祖先不是目录")
        else:
            current.mkdir()
    return _safe_relative_path(root, relative, "live destination")


def _safe_source(root: Path, relative: Path) -> Path:
    source = _safe_relative_path(root, relative, "staging source")
    if source.is_symlink() or not source.is_file():
        raise ValueError("staging source 必须是普通文件")
    return source


def _reject_staging_symlinks(root: Path) -> None:
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ValueError("staging 不得包含 symlink ancestor")


def select_commit_files(staging_root: Path, version: str) -> list[Path]:
    version = _validate_version(version)
    root = _fixed_directory(staging_root, "staging")
    _reject_staging_symlinks(root)
    selected = []
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ValueError("staging 文件不得是 symlink")
        if not path.is_file():
            continue
        relative = path.relative_to(root)
        if _is_selected(relative, version) or _is_manifest(relative):
            selected.append(relative)
    return sorted(selected, key=lambda item: item.as_posix())


def _copy_to_sibling(source: Path, destination: Path) -> Path:
    if destination.is_symlink():
        raise ValueError("live destination 不得是 symlink")
    destination.parent.mkdir(exist_ok=True)
    temporary = tempfile.NamedTemporaryFile(
        mode="wb",
        dir=destination.parent,
        prefix=f".publish-assets-{destination.name}.",
        suffix=".tmp",
        delete=False,
    )
    temporary_path = Path(temporary.name)
    try:
        with temporary:
            with source.open("rb") as reader:
                shutil.copyfileobj(reader, temporary)
            shutil.copymode(source, temporary_path)
            temporary.flush()
            os.fsync(temporary.fileno())
    except BaseException:
        try:
            temporary_path.unlink(missing_ok=True)
        except OSError:
            pass
        raise
    return temporary_path


def _fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _backup_existing(destination: Path, backup: Path) -> bool:
    if not destination.exists() and not destination.is_symlink():
        return False
    if destination.is_symlink() or not destination.is_file():
        raise ValueError("live 发布文件类型无效")
    backup.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(destination, backup)
    return True


def _restore(records: list[tuple[Path, Path | None, Path]]) -> None:
    first_error = None
    for destination, backup, temporary in reversed(records):
        try:
            if backup is not None and backup.exists():
                os.replace(backup, destination)
            else:
                destination.unlink(missing_ok=True)
            if temporary.exists():
                temporary.unlink(missing_ok=True)
        except OSError as error:
            first_error = first_error or error
    if first_error is not None:
        raise ValueError("无法恢复 live 发布文件") from first_error


def _commit_one(
    staging: Path,
    live: Path,
    transaction: Path,
    relative: Path,
    records: list[tuple[Path, Path | None, Path]],
    replace_func,
) -> None:
    source = _safe_source(staging, relative)
    destination = _prepare_destination(live, relative)
    backup = transaction / relative
    temporary = _copy_to_sibling(source, destination)
    try:
        existed = _backup_existing(destination, backup)
    except BaseException:
        try:
            temporary.unlink(missing_ok=True)
        except OSError:
            pass
        raise
    records.append((destination, backup if existed else None, temporary))
    replace_func(temporary, destination)
    _fsync_directory(destination.parent)


def commit_staged_assets(
    staging_root: Path,
    live_root: Path,
    version: str,
    replace_func=os.replace,
) -> tuple[str, ...]:
    version = _validate_version(version)
    staging = _fixed_directory(staging_root, "staging")
    live = _fixed_directory(live_root, "live")
    selected = select_commit_files(staging, version)
    if not selected:
        raise ValueError("staging 没有可提交文件")
    for relative in selected:
        _safe_relative_path(live, relative, "live destination")
    transaction = Path(tempfile.mkdtemp(prefix=".publish-assets-", dir=live))
    records: list[tuple[Path, Path | None, Path]] = []
    restore_failed = False
    try:
        for relative in selected:
            _commit_one(staging, live, transaction, relative, records, replace_func)
        return tuple(path.as_posix() for path in selected)
    except BaseException as publish_error:
        try:
            _restore(records)
        except BaseException as restore_error:
            restore_failed = True
            raise ValueError(
                f"发布失败且回滚失败，备份保留在 {transaction}"
            ) from restore_error
        raise publish_error
    finally:
        if not restore_failed:
            shutil.rmtree(transaction, ignore_errors=True)


def main(arguments: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="提交 updater staging 文件")
    commands = parser.add_subparsers(dest="command", required=True)
    commit = commands.add_parser("commit")
    commit.add_argument("--staging-root", type=Path, required=True)
    commit.add_argument("--live-root", type=Path, required=True)
    commit.add_argument("--version", required=True)
    options = parser.parse_args(arguments)
    try:
        committed = commit_staged_assets(
            options.staging_root, options.live_root, options.version
        )
    except (OSError, ValueError) as error:
        print(f"错误: {error}", file=sys.stderr)
        return 1
    print(f"已提交 {len(committed)} 个 updater 发布文件")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
