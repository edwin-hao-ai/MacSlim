#!/usr/bin/env python3
from __future__ import annotations
import sys; sys.dont_write_bytecode = True
import json
import pathlib
import plistlib
import re
import shlex
import struct
from urllib.parse import urlsplit
from xml.parsers.expat import ExpatError
try: from scripts import release_docs_contract
except ModuleNotFoundError: import release_docs_contract
validate_release_documentation_contract = release_docs_contract.validate_release_documentation_contract
try: from scripts import operation_contract
except ModuleNotFoundError: import operation_contract
EXPECTED_CAPABILITIES = {"core:app:allow-version", "core:event:allow-listen", "core:event:allow-unlisten", "core:window:allow-start-dragging", "autostart:allow-enable", "autostart:allow-disable", "autostart:allow-is-enabled", "notification:allow-is-permission-granted", "notification:allow-request-permission", "notification:allow-notify", "process:allow-restart", "updater:allow-check", "updater:allow-download-and-install"}
EXPECTED_ENTITLEMENTS = {"com.apple.security.cs.allow-jit", "com.apple.security.network.client", "com.apple.security.automation.apple-events"}
ALLOWED_REMOTE_HOSTS = {"ipc.localhost", "asset.localhost"}
ALLOWED_SCRIPT_SOURCES = {"'self'", "ipc:"}
SCRIPT_DIRECTIVES = {"script-src", "script-src-elem", "script-src-attr"}
SCRIPT_INJECTION_PREFIXES = ("'nonce-", "'sha256-", "'sha384-", "'sha512-'")
SOURCE_REQUIRED_DIRECTIVES = {"default-src", "connect-src", "img-src", "style-src", "script-src", "script-src-elem", "script-src-attr", "object-src", "base-uri", "frame-src", "frame-ancestors"}
REMOTE_SOURCE_ERROR = "生产 CSP 不能加载远程脚本或资源"
SCRIPT_SOURCE_ERROR = "script-src 只能包含 'self' 与 Tauri 注入机制"
INVALID_URL_ERROR = "生产 CSP 包含无效 URL"
INVALID_DIRECTIVE_ERROR = "生产 CSP directive 格式无效"
def _is_tauri_script_source(source: str) -> bool:
    lowered = source.lower()
    if lowered in ALLOWED_SCRIPT_SOURCES:
        return True
    if not lowered.startswith("'") or not lowered.endswith("'"):
        return False
    return any(
        lowered.startswith(prefix) and len(lowered) > len(prefix)
        for prefix in SCRIPT_INJECTION_PREFIXES
    )
def _parse_csp_directives(value: str) -> tuple[list[tuple[str, list[str]]], list[str]]:
    directives = []
    errors = []
    seen = set()
    for raw_directive in value.split(";"):
        parts = raw_directive.strip().split()
        if not parts:
            continue
        name = parts[0].lower()
        if (
            not name[0].isalpha()
            or not name.isascii()
            or not name.replace("-", "").isalnum()
        ):
            errors.append(INVALID_DIRECTIVE_ERROR)
            continue
        if name in seen:
            errors.append("生产 CSP directive 重复")
            continue
        seen.add(name)
        sources = parts[1:]
        if name in SOURCE_REQUIRED_DIRECTIVES and not sources:
            errors.append("生产 CSP directive 缺少 source")
            continue
        directives.append((name, sources))
    return directives, errors
def _validate_http_source(source: str) -> str | None:
    try:
        parsed = urlsplit(source)
        hostname = parsed.hostname
        port = parsed.port
    except (UnicodeError, ValueError):
        return INVALID_URL_ERROR
    if (
        hostname not in ALLOWED_REMOTE_HOSTS
        or parsed.username is not None
        or parsed.password is not None
        or port is not None
        or parsed.path not in {"", "/"}
        or parsed.query
        or parsed.fragment
    ):
        return REMOTE_SOURCE_ERROR
    return None
def _validate_source(source: str, directive: str) -> str | None:
    lowered = source.lower()
    if directive in SCRIPT_DIRECTIVES:
        if lowered == "'unsafe-inline'":
            return "script-src 不能包含 unsafe-inline"
        if _is_tauri_script_source(source):
            return None
        return SCRIPT_SOURCE_ERROR
    if lowered.startswith("'"):
        return None
    if lowered in {"ipc:", "asset:"}:
        return None
    if lowered.startswith(("data:", "blob:")):
        return None
    if lowered.startswith("//"):
        return REMOTE_SOURCE_ERROR
    if lowered.startswith(("http://", "https://")):
        return _validate_http_source(source)
    return REMOTE_SOURCE_ERROR
def validate_csp(value: object) -> list[str]:
    if not isinstance(value, str) or not value.strip():
        return ["生产 CSP 不能为空"]
    errors = []
    lowered = value.lower()
    if "*" in value:
        errors.append("生产 CSP 不能包含通配符")
    if "unsafe-eval" in lowered:
        errors.append("生产 CSP 不能包含 unsafe-eval")
    directives, directive_errors = _parse_csp_directives(value)
    errors.extend(directive_errors)
    for directive, sources in directives:
        for source in sources:
            error = _validate_source(source, directive)
            if error is not None and error not in errors:
                errors.append(error)
    return errors
def validate_capabilities(value: object) -> list[str]:
    if not isinstance(value, dict) or not isinstance(value.get("permissions"), list):
        return ["capability permissions 必须是数组"]
    permissions = value["permissions"]
    if not all(isinstance(permission, str) for permission in permissions):
        return ["capability permissions 必须是字符串数组"]
    actual = set(permissions)
    missing = sorted(EXPECTED_CAPABILITIES - actual)
    extra = sorted(actual - EXPECTED_CAPABILITIES)
    errors = []
    if len(permissions) != len(actual):
        errors.append("capability permissions 存在重复项")
    if missing:
        errors.append(f"capability 缺少: {', '.join(missing)}")
    if extra:
        errors.append(f"capability 多余: {', '.join(extra)}")
    return errors
def _statement_indexes(statements: list[list[str]], marker: str) -> list[int]:
    return [
        index
        for index, tokens in enumerate(statements)
        if any(marker in token for token in tokens)
    ]
def _call_indexes(statements: list[list[str]], marker: str) -> list[int]:
    return [
        index
        for index, tokens in enumerate(statements)
        if tokens
        and tokens[0] not in {"def", f"{marker}()"}
        and any(marker in token for token in tokens)
    ]
def validate_updater_artifact_contract(source: str) -> list[str]:
    return release_docs_contract.validate_updater_artifact_contract(source)

def validate_publish_assets_contract(source: str) -> list[str]:
    return release_docs_contract.validate_publish_assets_contract(source)

def _validate_publish_updater_gate(publish: str) -> list[str]:
    return release_docs_contract.validate_publish_gate_contract(publish)
def validate_updater_contract(tauri: object, publish: str) -> list[str]:
    errors = []
    if not isinstance(tauri, dict):
        return ["tauri 配置必须是对象"]
    bundle = tauri.get("bundle", {})
    if not isinstance(bundle, dict) or bundle.get("createUpdaterArtifacts") is not True:
        errors.append("updater 必须启用 bundle.createUpdaterArtifacts")
    plugins = tauri.get("plugins", {})
    updater = plugins.get("updater", {}) if isinstance(plugins, dict) else {}
    endpoints = updater.get("endpoints") if isinstance(updater, dict) else None
    if not isinstance(endpoints, list) or len(endpoints) != 1:
        errors.append("updater endpoint 必须是唯一稳定地址")
    else:
        endpoint = endpoints[0]
        if not isinstance(endpoint, str) or not endpoint.endswith("/latest.json"):
            errors.append("updater endpoint 必须指向 latest.json")
        if isinstance(endpoint, str) and "{{current_version}}" in endpoint:
            errors.append("新客户端 updater endpoint 不得依赖 current_version")
    if not isinstance(publish, str):
        return [*errors, "publish-update.sh 内容必须是字符串"]
    errors.extend(_validate_publish_updater_gate(publish))
    for marker in (
        ".app.tar.gz",
        "MacSlim_${VERSION}_${ARCH}.app.tar.gz",
        "latest.json",
        "0.1.0",
        "0.2.2",
    ):
        if marker not in publish:
            errors.append(f"updater 发布脚本缺少 {marker}")
    if "DMG_SRC" in publish or 'signer sign "$DMG_SRC"' in publish:
        errors.append("updater 发布脚本不得使用 DMG 作为更新对象")
    return errors
def validate_info_plist_contract(tauri: object, info_plist: object) -> list[str]:
    errors = []
    if not isinstance(tauri, dict):
        return ["tauri 配置必须是对象"]
    bundle = tauri.get("bundle", {})
    macos = bundle.get("macOS", {}) if isinstance(bundle, dict) else {}
    if not isinstance(macos, dict) or macos.get("infoPlist") != "Info.plist":
        errors.append("bundle.macOS 必须显式使用受控 Info.plist")
    if not isinstance(info_plist, dict):
        return [*errors, "Info.plist 必须是字典"]
    description = info_plist.get("NSAppleEventsUsageDescription")
    if not isinstance(description, str) or not description.strip():
        errors.append("Info.plist 必须包含 NSAppleEventsUsageDescription")
    return errors
def validate_public_api_contract(tauri: object, cargo: str, source: str) -> list[str]:
    errors = []
    if not isinstance(tauri, dict):
        return ["tauri 配置必须是对象"]
    app = tauri.get("app", {})
    if not isinstance(app, dict):
        return ["app 配置必须是对象"]
    windows = app.get("windows")
    if not isinstance(windows, list) or not windows:
        errors.append("正式配置必须包含窗口")
    elif any(
        not isinstance(window, dict) or window.get("transparent") is not False
        for window in windows
    ):
        errors.append("正式窗口必须关闭 transparent")
    if app.get("macOSPrivateApi") is not False:
        errors.append("正式配置必须关闭 macOSPrivateApi")
    if "macos-private-api" in cargo or "macos-private-api" in source:
        errors.append("Cargo 不得启用 macos-private-api")
    forbidden_markers = (
        "window-vibrancy",
        "window_vibrancy",
        "apply_vibrancy",
        "NSVisualEffectMaterial",
    )
    if any(marker in cargo or marker in source for marker in forbidden_markers):
        errors.append("正式配置不得保留 window-vibrancy/apply_vibrancy")
    return errors
def validate_entitlements(value: dict) -> list[str]:
    if not isinstance(value, dict):
        return ["entitlements 必须是字典"]
    invalid_keys = [key for key in value if not isinstance(key, str)]
    if invalid_keys:
        return ["entitlement 名称必须是字符串"]
    actual = set(value)
    missing = sorted(EXPECTED_ENTITLEMENTS - {key for key, enabled in value.items() if enabled is True})
    extra = sorted(actual - EXPECTED_ENTITLEMENTS)
    errors = []
    if missing:
        errors.append(f"entitlement 缺少: {', '.join(missing)}")
    if extra:
        errors.append(f"entitlement 多余: {', '.join(extra)}")
    return errors
def validate_secret_fallbacks(files: dict[str, str]) -> list[str]:
    if not isinstance(files, dict):
        return ["凭据 fallback 文件列表必须是字典"]
    errors = []
    for name, content in files.items():
        if not isinstance(content, str):
            errors.append(f"{name} 文件内容必须是字符串")
            continue
        for number, line in enumerate(content.splitlines(), start=1):
            if "TAURI_SIGNING_PRIVATE_KEY_PASSWORD" in line and ":-" in line:
                errors.append(f"{name}:{number} 存在凭据 fallback")
    return errors
def _shell_statements(content: str) -> list[list[str]]:
    statements = []
    logical_lines = content.replace("\\\n", " ").splitlines()
    for line in logical_lines:
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        try:
            tokens = shlex.split(line, comments=True, posix=True)
        except ValueError:
            continue
        if tokens:
            statements.append(tokens)
    return statements
def _command_indices(
    statements: list[list[str]],
    prefix: tuple[str, ...],
    required: tuple[str, ...] = (),
    option_pairs: tuple[tuple[str, str], ...] = (),
) -> list[int]:
    matches = []
    wrappers = {"!", "do", "elif", "else", "if", "then"}
    for index, tokens in enumerate(statements):
        start = 0
        while start < len(tokens) and tokens[start] in wrappers:
            start += 1
        if tuple(tokens[start : start + len(prefix)]) != prefix:
            continue
        if not all(token in tokens for token in required):
            continue
        if any(
            not any(
                tokens[position] == option
                and position + 1 < len(tokens)
                and tokens[position + 1] == value
                for position in range(len(tokens) - 1)
            )
            for option, value in option_pairs
        ):
            continue
        matches.append(index)
    return matches
def _has_capture(
    statements: list[list[str]], assignment: str, required: tuple[str, ...]
) -> bool:
    return any(
        tokens
        and tokens[0].startswith(assignment)
        and all(any(required_text in token for token in tokens) for required_text in required)
        for tokens in statements
    )
def _has_pattern_assertion(
    statements: list[list[str]], variable: str, expected: str
) -> bool:
    return any(
        "[[" in tokens
        and variable in tokens
        and "!=" in tokens
        and any(expected in token for token in tokens)
        for tokens in statements
    )
def _logical_shell_lines(content: str) -> list[str]:
    lines = []
    logical_lines = content.replace("\\\n", " ").splitlines()
    for line in logical_lines:
        try:
            tokens = shlex.split(line, comments=True, posix=True)
        except ValueError:
            continue
        if tokens:
            lines.append(" ".join(tokens))
    return lines
def _has_history_sequence(tokens: list[str]) -> bool:
    prefix = ("xcrun", "notarytool", "history")
    return any(
        tuple(tokens[index : index + len(prefix)]) == prefix
        for index in range(len(tokens) - len(prefix) + 1)
    )
def _has_option_pair(tokens: list[str], option: str, value: str) -> bool:
    return any(
        tokens[index] == option
        and index + 1 < len(tokens)
        and tokens[index + 1] == value
        for index in range(len(tokens) - 1)
    )
def _has_forbidden_history_control(tokens: list[str]) -> bool:
    allowed_redirects = {"2>&1", "2>&2", "2>&-"}
    control_markers = ("&", ";", "|", "(", ")", "{", "}")
    return any(
        token not in allowed_redirects
        and any(marker in token for marker in control_markers)
        for token in tokens
    )
def _notary_profile(content: str) -> str:
    for line in content.splitlines():
        if line.startswith("PROFILE_NAME="):
            return line.split("=", 1)[1].strip().strip('"')
    return ""
def _validate_shell_safety(
    name: str,
    statements: list[list[str]],
    logical_lines: list[str],
    identity_variable: str,
) -> list[str]:
    errors = []
    strict_statement = ("set", "-euo", "pipefail")
    if not statements:
        errors.append(f"{name} 缺少 set -euo pipefail")
    elif tuple(statements[0]) != strict_statement:
        errors.append(
            f"{name} 首个可执行 logical statement 必须在首个关键命令之前精确为 set -euo pipefail"
        )
    if any("set +e" in line for line in logical_lines):
        errors.append(f"{name} 不能使用 set +e")
    key_markers = (
        "security",
        "codesign",
        "xcrun notarytool",
        "xcrun stapler",
        "spctl",
        "bun tauri build",
        "bun run tauri build",
        "python3 scripts/updater_artifact.py rebuild",
    )
    if any(
        "||" in line and any(marker in line for marker in key_markers)
        for line in logical_lines
    ):
        errors.append(f"{name} 关键命令不能通过 || 忽略命令失败")
    if any(
        any("security" in token or "codesign" in token for token in tokens)
        and "|" in tokens
        and "grep" in tokens
        and "-q" in tokens
        for tokens in statements
    ):
        errors.append(f"{name} 不得将 security/codesign 输出通过管道交给 grep -q")
    if not _has_capture(
        statements,
        "IDENTITIES=$(security",
        ("find-identity", "-v", "-p", "codesigning"),
    ) or not _has_pattern_assertion(statements, "$IDENTITIES", identity_variable):
        errors.append(f"{name} 缺少本机签名证书输出捕获")
    return errors
def _notary_history_errors(name: str, statements: list[list[str]]) -> list[str]:
    errors = []
    profile_pair = (("--keychain-profile", "$PROFILE_NAME"),)
    history_statements = [
        tokens for tokens in statements if _has_history_sequence(tokens)
    ]
    history_valid = False
    history_invalid = False
    history_prefix = ("xcrun", "notarytool", "history")
    for tokens in history_statements:
        is_direct = tuple(tokens[: len(history_prefix)]) == history_prefix
        has_profile = _has_option_pair(tokens, *profile_pair[0])
        if not is_direct or not has_profile:
            history_invalid = True
        elif _has_forbidden_history_control(tokens):
            history_invalid = True
        else:
            history_valid = True
    if not history_valid or history_invalid:
        errors.append(
            f"{name} notary history 预检必须是独立同步直接命令并依赖 set -euo pipefail"
        )
    return errors
def _notary_submission_contract(
    name: str, statements: list[list[str]]
) -> tuple[list[str], dict[str, list[int]]]:
    errors = []
    submission_indexes = {}
    profile_pair = (("--keychain-profile", "$PROFILE_NAME"),)
    for target, label in (("$APP_ARCHIVE_PATH", ".app"), ("$DMG_PATH", "DMG")):
        indexes = _command_indices(
            statements,
            ("xcrun", "notarytool", "submit"),
            required=(target, "--wait"),
            option_pairs=profile_pair,
        )
        submission_indexes[target] = indexes
        if not indexes:
            errors.append(f"{name} 缺少使用 $PROFILE_NAME 的 {label} 公证提交")
    return errors, submission_indexes
def _notary_ticket_contract(
    name: str, statements: list[list[str]]
) -> tuple[list[str], list[int]]:
    errors = []
    ticket_indexes = []
    for action in ("staple", "validate"):
        for target in ("$APP_PATH", "$DMG_PATH"):
            indexes = _command_indices(
                statements, ("xcrun", "stapler", action), required=(target,)
            )
            ticket_indexes.extend(indexes)
            if not indexes:
                errors.append(f"{name} 缺少 {target} stapler {action}")
    return errors, ticket_indexes
def _notary_order_errors(
    name: str,
    archive_indexes: list[int],
    submission_indexes: dict[str, list[int]],
    ticket_indexes: list[int],
) -> list[str]:
    errors = []
    app_submit_indexes = submission_indexes["$APP_ARCHIVE_PATH"]
    if archive_indexes and app_submit_indexes and min(app_submit_indexes) <= min(archive_indexes):
        errors.append(f"{name} 必须先用 ditto 归档 .app，再执行公证提交")
    if all(submission_indexes.values()):
        last_submit = max(min(indexes) for indexes in submission_indexes.values())
        if ticket_indexes and min(ticket_indexes) <= last_submit:
            errors.append(f"{name} 必须先完成 .app 和 DMG 公证提交，再执行 staple/validate")
    return errors
def _validate_notary_contract(name: str, statements: list[list[str]]) -> list[str]:
    errors = _notary_history_errors(name, statements)
    archive_indexes = _command_indices(
        statements,
        ("ditto",),
        required=(
            "-c",
            "-k",
            "--sequesterRsrc",
            "--keepParent",
            "$APP_PATH",
            "$APP_ARCHIVE_PATH",
        ),
    )
    if not archive_indexes:
        errors.append(f"{name} 缺少 .app 公证归档")
    submission_errors, submission_indexes = _notary_submission_contract(name, statements)
    errors.extend(submission_errors)
    ticket_errors, ticket_indexes = _notary_ticket_contract(name, statements)
    errors.extend(ticket_errors)
    errors.extend(
        _notary_order_errors(name, archive_indexes, submission_indexes, ticket_indexes)
    )
    return errors
def _validate_signature_contract(name: str, statements: list[list[str]]) -> list[str]:
    errors = []
    if not _command_indices(
        statements,
        ("codesign",),
        required=("--verify", "--deep", "--strict", "--verbose=2", "$APP_PATH"),
    ) or not _command_indices(
        statements, ("codesign",), required=("--verify", "--verbose=4", "$DMG_PATH")
    ):
        errors.append(f"{name} 缺少 .app 或 DMG codesign 验证")
    if not _has_capture(
        statements,
        "SIGNATURE_DETAILS=$(codesign",
        ("-dvvv", "$APP_PATH", "2>&1"),
    ) or not _has_pattern_assertion(statements, "$SIGNATURE_DETAILS", "Timestamp="):
        errors.append(f"{name} 缺少 positive secure timestamp 断言")
    for target_type, label in (("exec", ".app"), ("install", "DMG")):
        target = "$APP_PATH" if target_type == "exec" else "$DMG_PATH"
        if not _command_indices(
            statements,
            ("spctl",),
            required=("-a", "-t", target_type, "-vv", target),
        ):
            errors.append(f"{name} 缺少 {label} Gatekeeper 验证")
    return errors
def _updater_path_errors(name: str, content: str) -> list[str]:
    errors = []
    exact_path = (
        'UPDATER_PATH="src-tauri/target/${RUST_TARGET}/release/bundle/macos/'
        'MacSlim.app.tar.gz"'
    )
    if exact_path not in content:
        errors.append(f"{name} 必须精确选择同 target 的 updater archive")
    for marker in (
        'UPDATER_SIG_PATH="${UPDATER_PATH}.sig"',
        'UPDATER_STAMP_PATH="${UPDATER_PATH}.build-stamp.json"',
    ):
        if marker not in content:
            errors.append(f"{name} 缺少 updater artifact 路径契约")
    return errors
def _updater_invalidation_errors(
    name: str, statements: list[list[str]]
) -> list[str]:
    errors = []
    invalidation_indexes = _command_indices(
        statements,
        ("rm", "-f"),
        required=("$UPDATER_PATH", "$UPDATER_SIG_PATH", "$UPDATER_STAMP_PATH"),
    )
    if len(invalidation_indexes) != 1:
        errors.append(f"{name} 必须在流程开始失效旧 tar、sig 和 stamp")
    risky_indexes = []
    for marker in ("security", "xcrun notarytool history", "bun run tauri build", "codesign"):
        risky_indexes.extend(_statement_indexes(statements, marker))
    if invalidation_indexes and risky_indexes and min(invalidation_indexes) > min(risky_indexes):
        errors.append(f"{name} 必须在任何签名或公证操作前失效旧 updater artifact")
    return errors
def _updater_rebuild_errors(
    name: str, statements: list[list[str]]
) -> list[str]:
    errors = []
    rebuild_indexes = _command_indices(
        statements,
        ("python3", "scripts/updater_artifact.py", "rebuild"),
        required=("--app", "$APP_PATH", "--archive", "$UPDATER_PATH", "--target", "$RUST_TARGET"),
    )
    if len(rebuild_indexes) != 1:
        errors.append(f"{name} 必须且只能从当前 app 调用一次 updater rebuild")
    if _command_indices(
        statements,
        ("python3", "scripts/updater_artifact.py", "stamp"),
        required=("--archive", "$UPDATER_PATH", "--target", "$RUST_TARGET"),
    ):
        errors.append(f"{name} 不得只对旧 updater archive 写 stamp")
    completion_indexes = []
    for prefix, required in (
        (("xcrun", "notarytool", "submit"), ("$APP_ARCHIVE_PATH", "--wait")),
        (("xcrun", "notarytool", "submit"), ("$DMG_PATH", "--wait")),
        (("xcrun", "stapler", "validate"), ("$APP_PATH",)),
        (("xcrun", "stapler", "validate"), ("$DMG_PATH",)),
        (("spctl",), ("-a", "-t", "exec", "-vv", "$APP_PATH")),
        (("spctl",), ("-a", "-t", "install", "-vv", "$DMG_PATH")),
    ):
        completion_indexes.extend(
            _command_indices(statements, prefix, required=required)
        )
    if rebuild_indexes and completion_indexes and min(rebuild_indexes) <= max(completion_indexes):
        errors.append(f"{name} 必须在公证、staple 和 Gatekeeper 全部成功后 rebuild updater archive")
    return errors
def _updater_diagnostic_errors(name: str, content: str) -> list[str]:
    errors = []
    if "Apple notary 凭证预检失败" not in content:
        errors.append(f"{name} notary history 失败必须输出中文诊断")
    if name == "sign.sh" and 'echo "用法: $0 [arm|intel|universal]" >&2' not in content:
        errors.append("sign.sh 非法 target usage 必须写入 stderr")
    return errors
def _validate_updater_stamp_contract(
    name: str, content: str, statements: list[list[str]]
) -> list[str]:
    errors = _updater_path_errors(name, content)
    errors.extend(_updater_invalidation_errors(name, statements))
    errors.extend(_updater_rebuild_errors(name, statements))
    errors.extend(_updater_diagnostic_errors(name, content))
    return errors
def _validate_release_script(
    name: str, content: str, identity_variable: str
) -> list[str]:
    statements = _shell_statements(content)
    logical_lines = _logical_shell_lines(content)
    return [
        *_validate_shell_safety(
            name, statements, logical_lines, identity_variable
        ),
        *_validate_notary_contract(name, statements),
        *_validate_signature_contract(name, statements),
        *_validate_updater_stamp_contract(name, content, statements),
    ]
def validate_release_contracts(files: dict[str, str]) -> list[str]:
    errors = []
    sign = files.get("scripts/sign.sh", "")
    release = files.get("scripts/release.sh", "")
    if "--timestamp=none" in sign or "使用无时间戳签名" in sign:
        errors.append("签名脚本不能接受无 timestamp 成功路径")
    if "exit 0" in release and "notarytool 凭证未配置" in release:
        errors.append("缺少 notary 凭据时必须非零退出")
    if _notary_profile(release) != _notary_profile(sign):
        errors.append("release.sh 与 sign.sh 的 notary profile 不一致")
    for name, content in (("release.sh", release), ("sign.sh", sign)):
        if "src-tauri/tauri.conf.json" not in content:
            errors.append(f"{name} 必须从 tauri.conf.json 读取 version")
        if 'DMG_PATH="src-tauri/target/${RUST_TARGET}/release/bundle/dmg/MacSlim_${VERSION}_${ARCH}.dmg"' not in content:
            errors.append(f"{name} 必须精确选择 version/target 对应的 DMG")
        if "DMG_PATHS" in content or "MacSlim_*.dmg" in content or "${DMG_PATHS[0]}" in content:
            errors.append(f"{name} 不得通过 glob 或首项选择 DMG")
    if sign and "$TIMESTAMP_FLAG" not in sign:
        errors.append("sign.sh 必须引用 TIMESTAMP_FLAG")
    release_statements = _shell_statements(release)
    export_indexes = _command_indices(
        release_statements,
        ("export",),
        required=("APPLE_SIGNING_IDENTITY=$SIGNING_IDENTITY",),
    )
    build_indexes = _command_indices(
        release_statements, ("bun", "run", "tauri", "build")
    )
    if not export_indexes or not build_indexes or min(export_indexes) >= min(build_indexes):
        errors.append("release.sh 必须在 build 前导出 APPLE_SIGNING_IDENTITY")
    errors.extend(_validate_release_script("release.sh", release, "$SIGNING_IDENTITY"))
    errors.extend(_validate_release_script("sign.sh", sign, "$SIGNING_ID"))
    return errors
def _read_json(path: pathlib.Path) -> object:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError, RecursionError):
        raise ValueError(f"无法读取 JSON 配置: {path.name}") from None
def _read_plist(path: pathlib.Path) -> object:
    try:
        return plistlib.loads(path.read_bytes())
    except (
        OSError,
        UnicodeError,
        ValueError,
        TypeError,
        EOFError,
        OverflowError,
        struct.error,
        ExpatError,
        plistlib.InvalidFileException,
    ):
        raise ValueError(f"无法读取 plist 配置: {path.name}") from None
def _read_text(path: pathlib.Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeError):
        raise ValueError(f"无法读取文本配置: {path.name}") from None
def _release_paths(root: pathlib.Path) -> dict[str, pathlib.Path]:
    return {
        "scripts/publish-update.sh": root / "scripts/publish-update.sh",
        "scripts/release.sh": root / "scripts/release.sh",
        "scripts/sign.sh": root / "scripts/sign.sh",
        "scripts/README.md": root / "scripts/README.md",
        "package.json": root / "package.json",
    }
def _release_documentation_errors(release_files: dict[str, str]) -> list[str]:
    return validate_release_documentation_contract(
        release_files["scripts/README.md"], release_files["package.json"]
    )
def validate_operation_broker_contract(root: pathlib.Path) -> list[str]:
    try:
        return operation_contract.validate_operation_contract_files(root)
    except (OSError, UnicodeError, ValueError) as error:
        return [f"Operation Broker 静态契约无法完成校验: {error}"]
def main() -> int:
    root = pathlib.Path(__file__).resolve().parents[1]
    tauri_path = root / "src-tauri/tauri.conf.json"
    capability_path = root / "src-tauri/capabilities/default.json"
    entitlements_path = root / "src-tauri/entitlements.plist"
    info_plist_path = root / "src-tauri/Info.plist"
    cargo_path = root / "src-tauri/Cargo.toml"
    lib_path = root / "src-tauri/src/lib.rs"
    release_paths = _release_paths(root)
    try:
        tauri = _read_json(tauri_path)
        capabilities = _read_json(capability_path)
        entitlements = _read_plist(entitlements_path)
        info_plist = _read_plist(info_plist_path)
        cargo = _read_text(cargo_path)
        lib_source = _read_text(lib_path)
        updater_artifact_source = _read_text(root / "scripts/updater_artifact.py")
        publish_assets_source = _read_text(root / "scripts/publish_assets.py")
        release_files = {name: _read_text(path) for name, path in release_paths.items()}
    except (OSError, UnicodeError, ValueError) as error:
        print(f"错误: {error}", file=sys.stderr)
        return 1
    publish_update = release_files["scripts/publish-update.sh"]
    errors = []
    errors.extend(_release_documentation_errors(release_files))
    if not isinstance(tauri, dict):
        errors.append("tauri 配置必须是对象")
    else:
        app = tauri.get("app", {})
        security = app.get("security", {}) if isinstance(app, dict) else {}
        csp = security.get("csp") if isinstance(security, dict) else None
        errors.extend(validate_csp(csp))
    errors.extend(validate_capabilities(capabilities))
    errors.extend(validate_updater_contract(tauri, publish_update))
    errors.extend(validate_updater_artifact_contract(updater_artifact_source))
    errors.extend(validate_publish_assets_contract(publish_assets_source))
    errors.extend(validate_info_plist_contract(tauri, info_plist))
    errors.extend(validate_public_api_contract(tauri, cargo, lib_source))
    errors.extend(validate_entitlements(entitlements))
    errors.extend(validate_secret_fallbacks({"scripts/publish-update.sh": publish_update}))
    errors.extend(validate_release_contracts(release_files))
    errors.extend(validate_operation_broker_contract(root))
    if errors:
        for error in errors:
            print(f"错误: {error}", file=sys.stderr)
        return 1
    print("安全配置校验通过")
    return 0
if __name__ == "__main__":
    raise SystemExit(main())
