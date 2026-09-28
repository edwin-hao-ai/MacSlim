#!/usr/bin/env python3
"""Trusted Operation Broker 静态契约门禁（组合入口）。

判定全部建立在 scripts/source_structure.py 的结构提取与
scripts/operation_surface_checks.py 的表面门禁之上：

* 16 个可信 IPC command，破坏性入口精确为 prepare/execute；
* 前端只提交 opaque key，没有 raw wrapper / raw request 字段；
* CLI 不再直接触碰领域执行函数，破坏性路径对本地存储 fail-closed；
* owner 绑定、deny_unknown_fields、history 不含 operation id；
* AppGracefulQuit 走 quit_app，拒绝被接回进程树 SIGTERM；
* 每个后端 operation kind 都有历史标签，locales 保持一致；
* 随机 ID 来自 getrandom 直接依赖；verify 链覆盖全部套件。

注释或字符串里出现旧 command / raw 字段名既不算违规、也不算通过。
"""
from __future__ import annotations
import json
import pathlib
import re
import shlex
import tomllib

try: from scripts import source_structure as _structure
except ModuleNotFoundError: import source_structure as _structure
try: from scripts import operation_surface_checks as _surface
except ModuleNotFoundError: import operation_surface_checks as _surface
mask_code = _structure.mask_code
rust_function_body = _structure.rust_function_body
rust_function_spans = _structure.rust_function_spans
rust_function_text = _structure.rust_function_text
rust_invoke_handler_commands = _structure.rust_invoke_handler_commands
rust_declared_commands = _structure.rust_declared_commands
rust_operation_kind_labels = _structure.rust_operation_kind_labels
rust_serde_directives = _structure.rust_serde_directives
enum_variants = _structure.enum_variants
rust_if_conditions = _structure.rust_if_conditions
compares_owner = _structure.compares_owner
balanced_slice = _structure.balanced_slice
rust_struct_fields = _structure.rust_struct_fields
string_spans = _structure.string_spans
parse_error = _structure.parse_error
typescript_nested_keys = _structure.typescript_nested_keys
typescript_switch_arms = _structure.typescript_switch_arms
typescript_invoke_commands = _structure.typescript_invoke_commands
typescript_request_variants = _structure.typescript_request_variants
typescript_type_fields = _structure.typescript_type_fields
RUST_QUOTES = _structure.RUST_QUOTES
TS_QUOTES = _structure.TS_QUOTES
EXPECTED_IPC_COMMANDS = _surface.EXPECTED_IPC_COMMANDS
DESTRUCTIVE_IPC_COMMANDS = _surface.DESTRUCTIVE_IPC_COMMANDS
RAW_IPC_COMMANDS = _surface.RAW_IPC_COMMANDS
RAW_FRONTEND_WRAPPERS = _surface.RAW_FRONTEND_WRAPPERS
RAW_FRONTEND_TYPES = _surface.RAW_FRONTEND_TYPES
RAW_REQUEST_FIELDS = _surface.RAW_REQUEST_FIELDS
EXPECTED_REQUEST_VARIANTS = _surface.EXPECTED_REQUEST_VARIANTS
ALLOWED_CLI_FLAGS = _surface.ALLOWED_CLI_FLAGS
RAW_CLI_FLAGS = _surface.RAW_CLI_FLAGS
RAW_CLI_FUNCTIONS = _surface.RAW_CLI_FUNCTIONS
ALLOWED_CLI_LIB_MODULES = _surface.ALLOWED_CLI_LIB_MODULES
CRATE_PRIVATE_MODULES = _surface.CRATE_PRIVATE_MODULES
CRATE_PRIVATE_PROCESS_ITEMS = _surface.CRATE_PRIVATE_PROCESS_ITEMS
CRATE_PRIVATE_TARGET_TYPES = _surface.CRATE_PRIVATE_TARGET_TYPES
CLI_SESSION_GATE = _surface.CLI_SESSION_GATE
CLI_SESSION_GATES = _surface.CLI_SESSION_GATES
FRONTEND_EXTENSIONS = _surface.FRONTEND_EXTENSIONS
TAURI_MODULE = _surface.TAURI_MODULE
validate_ipc_command_surface = _surface.validate_ipc_command_surface
validate_frontend_operation_surface = _surface.validate_frontend_operation_surface
validate_frontend_confirmation_flow = _surface.validate_frontend_confirmation_flow
validate_cli_operation_surface = _surface.validate_cli_operation_surface
validate_cli_fail_closed = _surface.validate_cli_fail_closed
validate_process_boundary = _surface.validate_process_boundary
frontend_parse_errors = _surface.frontend_parse_errors
SourceParseError = _structure.SourceParseError

REQUEST_VARIANT_TO_ENUM = {
    "cache": "Cache",
    "process": "Process",
    "app_terminate": "AppTerminate",
    "app_graceful_quit": "AppGracefulQuit",
    "uninstall": "Uninstall",
    "docker": "Docker",
}
HISTORY_BUILDERS = (
    "rejection_entry",
    "history_entry",
    "quit_entry",
    "process_entry",
    "record",
)
APP_LEVEL_PATHS = (
    "lib.rs",
    "operation_commands.rs",
    "operation_executor.rs",
    "applications.rs",
    "cli_rs",
    "cli_operations.rs",
)
GRACEFUL_QUIT_ROUTE = "GRACEFUL_QUIT_ROUTE"
PARSED_SUFFIXES = (".rs", ".ts", ".tsx")
RUST_ALIASED_KEYS = ("cli_rs",)
VERIFY_SEGMENTS = (
    "lint",
    "typecheck",
    "test",
    "test:python",
    "security:check",
    "test:rust",
    "build",
)
FRONTEND_GATE_SEGMENTS = (
    "lint",
    "typecheck",
    "test",
    "test:python",
    "security:check",
    "build",
)
FRONTEND_GATE_FORBIDDEN = ("test:rust",)
OPERATION_CONTRACT_FILES = {
    "lib.rs": "src-tauri/src/lib.rs",
    "operation_commands.rs": "src-tauri/src/operation_commands.rs",
    "operations.rs": "src-tauri/src/operations.rs",
    "operations_prepare.rs": "src-tauri/src/operations_prepare.rs",
    "operation_executor.rs": "src-tauri/src/operation_executor.rs",
    "operation_types.rs": "src-tauri/src/operation_types.rs",
    "operation_registry.rs": "src-tauri/src/operation_registry.rs",
    "applications.rs": "src-tauri/src/applications.rs",
    "uninstaller.rs": "src-tauri/src/uninstaller.rs",
    "process_ops.rs": "src-tauri/src/process_ops.rs",
    "scanner.rs": "src-tauri/src/scanner.rs",
    "docker.rs": "src-tauri/src/docker.rs",
    "cache_cleaner.rs": "src-tauri/src/cache_cleaner.rs",
    "cli_operations.rs": "src-tauri/src/cli_operations.rs",
    "cli_rs": "src-tauri/src/bin/cli.rs",
    "tauri.ts": "src/lib/tauri.ts",
    "HistoryView.tsx": "src/views/HistoryView.tsx",
    "en.ts": "src/i18n/en.ts",
    "zh-CN.ts": "src/i18n/zh-CN.ts",
    "Cargo.toml": "src-tauri/Cargo.toml",
    "package.json": "package.json",
}
FRONTEND_SKIP_PARTS = {"node_modules", "dist"}

def _request_container_errors(source: str) -> list[str]:
    flags, values = rust_serde_directives(source, "PrepareOperationRequest")
    errors = []
    if "deny_unknown_fields" not in flags:
        errors.append("PrepareOperationRequest 必须使用 serde deny_unknown_fields")
    if "type" not in values:
        errors.append('PrepareOperationRequest 必须使用 serde tag = "type"')
    if "snake_case" not in values:
        errors.append('PrepareOperationRequest 必须使用 serde rename_all = "snake_case"')
    for enum_name, expected in (
        ("PrepareOperationRequest", set(REQUEST_VARIANT_TO_ENUM.values())),
        ("OperationResult", {"Cache", "Process", "AppTerminate", "AppGracefulQuit", "Uninstall", "Docker"}),
    ):
        variants = enum_variants(source, enum_name)
        for variant in sorted(expected - variants):
            errors.append(f"{enum_name} 缺少 {variant} 变体")
        for variant in sorted(variants - expected):
            errors.append(f"{enum_name} 出现未登记变体 {variant}")
    return errors


def _owner_errors(source: str) -> list[str]:
    span = rust_function_spans(source).get("consume")
    if span is None:
        return ["OperationStore::consume 缺失，operation 无法单次原子消费"]
    body = mask_code(source[span[0] : span[1]], RUST_QUOTES)
    if not any(compares_owner(condition) for condition in rust_if_conditions(body)):
        return ["OperationStore::consume 必须按 owner 拒绝跨窗口/跨进程执行"]
    if re.search(r"\bErr\s*\(", body) is None:
        return ["OperationStore::consume 的 owner 不匹配必须返回错误而不是放行"]
    return []


def _history_entry_errors(source: str) -> list[str]:
    errors = []
    masked = mask_code(source, RUST_QUOTES)
    for name in HISTORY_BUILDERS:
        body = rust_function_body(source, name)
        if body is None:
            errors.append(f"history 构造路径 {name} 缺失，审计记录可能被绕过")
        elif re.search(r"\boperation_id\b", mask_code(body, RUST_QUOTES)):
            errors.append(f"history 构造路径 {name} 不得引用 operation_id")
    for match in re.finditer(r"OperationHistoryEntry\s*\{", masked):
        body = balanced_slice(masked, match.end() - 1)
        if body is not None and "operation_id" in rust_struct_fields(body):
            errors.append("history 条目结构不得包含 operation_id 字段")
    if re.search(r"operation\s*[:=][^;]*?\.label\(\)", masked) is None:
        errors.append("history 的 operation 必须来自 kind.label()，不得使用原始 operation id")
    return errors


def _graceful_route_errors(sources: dict[str, str]) -> list[str]:
    errors = []
    body = rust_function_body(sources["operations_prepare.rs"], "prepare_app_termination")
    masked = mask_code(body or "", RUST_QUOTES)
    gate = masked.find("ProcessMode::Graceful")
    route = masked.find(GRACEFUL_QUIT_ROUTE)
    selection = masked.find("self.snapshot(")
    if body is None or not 0 <= gate < route:
        errors.append(
            f"prepare_app_termination 必须在 mode == Graceful 时立刻返回 {GRACEFUL_QUIT_ROUTE}"
        )
    elif selection >= 0 and route > selection:
        errors.append(
            f"prepare_app_termination 的 {GRACEFUL_QUIT_ROUTE} 拒绝必须早于任何目标身份选择"
        )
    if "prepare_app_graceful_quit" not in sources["operations_prepare.rs"]:
        errors.append("缺少 prepare_app_graceful_quit，优雅退出没有独立计划")
    return errors


def _graceful_kill_visibility_errors(source: str) -> list[str]:
    masked = mask_code(source, RUST_QUOTES)
    pattern = (
        r"\bpub\b\s*(\([^)]*\))?\s*(?:async\s+)?(?:unsafe\s+)?fn\s+graceful_kill\b"
    )
    allowed = {"crate", "super", "self", "in crate", "in super", "in self"}
    errors = []
    for match in re.finditer(pattern, masked):
        restriction = match.group(1)
        scope = "pub" if restriction is None else restriction.strip("() ")
        if scope not in allowed:
            errors.append(
                f"process_ops::graceful_kill 当前是 {scope}，必须收窄为 pub(crate) 等受限可见性"
            )
    return errors


def _app_quit_route_errors(sources: dict[str, str]) -> list[str]:
    errors = [
        f"{name} 不得引用 process_ops::graceful_kill 作为应用级退出路径"
        for name in APP_LEVEL_PATHS
        if re.search(r"\bgraceful_kill\b", mask_code(sources[name], RUST_QUOTES))
    ]
    errors.extend(_graceful_kill_visibility_errors(sources["process_ops.rs"]))
    executor = rust_function_body(sources["operation_executor.rs"], "execute_domain_plan") or ""
    if re.search(r"AppGracefulQuit[\s\S]{0,200}?domains\.app_quit", executor) is None:
        errors.append("AppGracefulQuit 计划必须在 execute_domain_plan 里路由到 domains.app_quit")
    if "AppGracefulQuit" not in executor:
        errors.append("execute_domain_plan 必须处理 AppGracefulQuit 计划")
    if "quit_app" not in mask_code(sources["applications.rs"], RUST_QUOTES):
        errors.append("SystemAppQuitter 必须调用 uninstaller::quit_app（AppleScript 优雅退出）")
    if "quit_app" not in mask_code(sources["uninstaller.rs"], RUST_QUOTES):
        errors.append("uninstaller 必须提供 quit_app 优雅退出通道")
    return errors


def _execute_entry_errors(sources: dict[str, str]) -> list[str]:
    errors = []
    execute = rust_function_text(sources["lib.rs"], "execute_operation")
    if "operation_id" not in execute:
        errors.append("execute_operation 必须只接受 operation_id")
    collection = re.search(r"\bVec<[^>]*>", execute)
    if collection is not None:
        errors.append(f"execute_operation 不得接受客户端集合参数: {collection.group(0)}")
    prepare = rust_function_text(sources["lib.rs"], "prepare_operation")
    if "WebviewWindow" not in prepare or "label()" not in prepare:
        errors.append("prepare_operation 必须从 WebviewWindow 注入 owner")
    return errors


def _is_parsed_source(name: str) -> bool:
    return name.endswith(PARSED_SUFFIXES) or name in RUST_ALIASED_KEYS


def _quotes_for(name: str) -> str:
    return TS_QUOTES if name.endswith(FRONTEND_EXTENSIONS) else RUST_QUOTES


def parse_errors(files: dict[str, str]) -> list[str]:
    errors = []
    for name, source in sorted(files.items()):
        if not _is_parsed_source(name):
            continue
        error = parse_error(name, source, _quotes_for(name))
        if error is not None:
            errors.append(error)
    return errors


def validate_backend_operation_contract(sources: dict[str, str]) -> list[str]:
    unparseable = parse_errors(sources)
    if unparseable:
        return unparseable
    return [
        *_request_container_errors(sources["operation_commands.rs"]),
        *_owner_errors(sources["operations.rs"]),
        *_history_entry_errors(sources["operation_commands.rs"]),
        *_graceful_route_errors(sources),
        *_app_quit_route_errors(sources),
        *_execute_entry_errors(sources),
    ]


def validate_random_id_dependency(cargo_source: str) -> list[str]:
    try:
        cargo = tomllib.loads(cargo_source)
    except tomllib.TOMLDecodeError:
        return ["src-tauri/Cargo.toml 无法解析"]
    build = cargo.get("build-dependencies")
    errors = []
    if isinstance(build, dict) and "getrandom" in build:
        errors.append("getrandom 不能只出现在 build-dependencies")
    dependencies = cargo.get("dependencies")
    if not isinstance(dependencies, dict) or dependencies.get("getrandom") is None:
        return [*errors, "getrandom 必须是 [dependencies] 直接依赖，随机 ID 不能来自传递依赖"]
    spec = dependencies["getrandom"]
    version = spec if isinstance(spec, str) else spec.get("version") if isinstance(spec, dict) else None
    if version != "0.2":
        errors.append(f"getrandom 版本必须锁定 0.2，当前 {version!r}")
    return errors


def _rust_history_labels(source: str) -> set[str]:
    body = rust_function_body(source, "history_operation_label") or ""
    return {value for _s, _e, value in string_spans(body, RUST_QUOTES)}


def _cli_label_fallback_is_text(source: str) -> bool:
    span = rust_function_spans(source).get("history_operation_label")
    if span is None:
        return False
    body = source[span[0] : span[1]]
    match = re.search(r"_\s*=>\s*", mask_code(body, RUST_QUOTES))
    if match is None:
        return False
    start = match.start() + len(match.group(0).rstrip())
    while start < len(body) and body[start].isspace():
        start += 1
    comma = body.find(",", start)
    value = body[start : comma if comma >= 0 else len(body)].strip()
    return value.startswith('"')


def _history_locale_errors(sources: dict[str, str], arms: dict[str, tuple[int, int]]) -> list[str]:
    view = sources["HistoryView.tsx"]
    locales = {
        "en.ts": typescript_nested_keys(sources["en.ts"]),
        "zh-CN.ts": typescript_nested_keys(sources["zh-CN.ts"]),
    }
    errors = []
    for label, span in sorted(arms.items()):
        for key in re.findall(r't\(\s*"([^"]+)"', view[span[0] : span[1]]):
            errors.extend(
                f"{name} 缺少历史标签 {key}（{label}）"
                for name, keys in sorted(locales.items())
                if key not in keys
            )
    if locales["en.ts"] != locales["zh-CN.ts"]:
        errors.append("en.ts 与 zh-CN.ts 的 key 集合必须保持一致")
    return errors


def validate_operation_history_labels(sources: dict[str, str]) -> list[str]:
    unparseable = parse_errors(sources)
    if unparseable:
        return unparseable
    labels = rust_operation_kind_labels(sources["operation_types.rs"])
    if not labels:
        return ["operation_types.rs 无法解析 OperationKind::label() 取值"]
    arms = typescript_switch_arms(sources["HistoryView.tsx"], "const opLabel")
    cli_labels = _rust_history_labels(sources["cli.rs"])
    view = sources["HistoryView.tsx"]
    errors = []
    for label in sorted(labels):
        span = arms.get(label)
        if span is None:
            errors.append(f"HistoryView 缺少 {label} 的历史标签分支")
        elif "t(" not in view[span[0] : span[1]]:
            errors.append(f"HistoryView 的 {label} 分支必须返回 t(...) 本地化标签，不能回显裸 key")
        if label not in cli_labels:
            errors.append(f"CLI history_operation_label 缺少 {label}，会显示为“未知操作”")
    if not _cli_label_fallback_is_text(sources["cli.rs"]):
        errors.append("CLI history_operation_label 的兜底必须是固定中文文案，不能回显裸 key")
    errors.extend(_history_locale_errors(sources, arms))
    return errors


DECOY_COMMANDS = ("echo", "printf", "true", "false")
TOP_LEVEL_SEPARATORS = (";", "|", "&", "||", "|&")
BUN_PREFIX = ("bun", "run")
CARGO_SUBCOMMANDS = ("fmt", "test", "clippy")
CARGO_SUBCOMMAND_FLAGS = {
    "fmt": ("--", "--check"),
    "test": ("--all-targets",),
    "clippy": ("--all-targets", "-D", "warnings"),
}
CARGO_SUPPRESSING_FLAGS = (
    "--no-run",
    "--list",
    "-q",
    "--quiet",
    "--doc",
    "--test",
    "--examples",
    "--benches",
    "--package",
    "-p",
)
CARGO_UNFILTERABLE_SUBCOMMANDS = ("test",)
PYTHON_SUITE_CHAIN = (
    "PYTHONDONTWRITEBYTECODE=1",
    "python3",
    "-m",
    "unittest",
    "discover",
    "-s",
    "scripts/tests",
    "-p",
    "test_*.py",
)


def _chain_tokens(name: str, script: str) -> tuple[list[str], list[str]]:
    try:
        return shlex.split(script), []
    except ValueError as failure:
        return [], [f"package.json 的 {name} 无法按 shell 词法解析: {failure}"]


def _chain_segments(name: str, tokens: list[str]) -> tuple[list[list[str]], list[str]]:
    segments: list[list[str]] = []
    current: list[str] = []
    separators: list[str] = []
    for token in tokens:
        if token == "&&":
            segments.append(current)
            current = []
            continue
        if token in TOP_LEVEL_SEPARATORS:
            separators.append(token)
            continue
        current.append(token)
    segments.append(current)
    errors = [
        f"package.json 的 {name} 含顶层分隔符 {separator!r}，verify 链必须全部用 && 串联，"
        "否则前面的套件失败不会让整条命令返回非零"
        for separator in dict.fromkeys(separators)
    ]
    return [segment for segment in segments if segment], errors


def _decoy_error(name: str, segment: list[str]) -> str | None:
    if segment[0] in DECOY_COMMANDS:
        rendered = " ".join(segment)
        return f"package.json 的 {name} 的段 {rendered!r} 只用 {segment[0]} 提及脚本，不能代替真实执行"
    return None


def _bun_chain_errors(
    name: str,
    segments: list[list[str]],
    scripts: dict[str, str],
    required: tuple[str, ...],
    forbidden: tuple[str, ...],
) -> list[str]:
    errors: list[str] = []
    referenced: list[str] = []
    for segment in segments:
        decoy = _decoy_error(name, segment)
        if decoy is not None:
            errors.append(decoy)
            continue
        if tuple(segment[:2]) != BUN_PREFIX or len(segment) != 3:
            rendered = " ".join(segment)
            errors.append(
                f"package.json 的 {name} 的段 {rendered!r} 必须精确为 `bun run <script>`，"
                "不接受额外参数、前缀命令或管道"
            )
            continue
        target = segment[2]
        if target not in scripts:
            errors.append(f"package.json 的 {name} 引用了不存在的 script: {target}")
            continue
        referenced.append(target)
    errors.extend(
        f"package.json 的 {name} 缺少 {missing}"
        for missing in sorted(set(required) - set(referenced))
    )
    errors.extend(
        f"package.json 的 {name} 不得包含 {banned}"
        for banned in forbidden
        if banned in referenced
    )
    return errors


def _suppressing_cargo_flag(subcommand: str, segment: list[str]) -> str | None:
    for token in segment[2:]:
        if token == "--":
            break
        if token in CARGO_SUPPRESSING_FLAGS:
            return token
    return None


def _cargo_tail_block_error(name: str, subcommand: str, segment: list[str]) -> str | None:
    if subcommand not in CARGO_UNFILTERABLE_SUBCOMMANDS or "--" not in segment[2:]:
        return None
    return (
        f"package.json 的 {name} 的 `cargo {subcommand}` 段 `{' '.join(segment)}` "
        f"不得带 `--` 尾参块，测试名过滤会让套件一个用例都不跑（抑制执行）"
    )


def _cargo_chain_errors(name: str, segments: list[list[str]]) -> list[str]:
    errors: list[str] = []
    found: dict[str, list[str]] = {}
    for segment in segments:
        decoy = _decoy_error(name, segment)
        if decoy is not None:
            errors.append(decoy)
            continue
        if segment[0] != "cargo" or len(segment) < 2:
            rendered = " ".join(segment)
            errors.append(f"package.json 的 {name} 的段 {rendered!r} 必须以 cargo 开头")
            continue
        subcommand = segment[1]
        if subcommand not in CARGO_SUBCOMMANDS:
            errors.append(f"package.json 的 {name} 含未登记的 cargo 子命令: {subcommand}")
            continue
        found.setdefault(subcommand, segment)
        suppressing = _suppressing_cargo_flag(subcommand, segment)
        if suppressing is not None:
            errors.append(
                f"package.json 的 {name} 的 `cargo {subcommand}` 段含抑制执行参数 {suppressing}，"
                "它会让套件看起来跑过却没有真正执行测试"
            )
        tail = _cargo_tail_block_error(name, subcommand, segment)
        if tail is not None:
            errors.append(tail)
    errors.extend(
        f"package.json 的 {name} 缺少 cargo {missing}"
        for missing in sorted(set(CARGO_SUBCOMMANDS) - set(found))
    )
    errors.extend(
        f"package.json 的 {name} 的 `cargo {subcommand}` 段缺少 {flag}"
        for subcommand, flags in sorted(CARGO_SUBCOMMAND_FLAGS.items())
        if subcommand in found
        for flag in flags
        if flag not in found[subcommand]
    )
    return errors


def _exact_chain_errors(name: str, tokens: list[str], expected: tuple[str, ...]) -> list[str]:
    if tuple(tokens) == expected:
        return []
    return [
        f"package.json 的 {name} 必须是精确命令链 `{' '.join(expected)}`，"
        f"当前 `{' '.join(tokens)}`"
    ]


def _bun_script_errors(
    name: str,
    scripts: dict[str, str],
    required: tuple[str, ...],
    forbidden: tuple[str, ...],
) -> list[str]:
    tokens, errors = _chain_tokens(name, scripts.get(name, ""))
    if errors:
        return errors
    segments, separator_errors = _chain_segments(name, tokens)
    return [
        *errors,
        *separator_errors,
        *_bun_chain_errors(name, segments, scripts, required, forbidden),
    ]


def _cargo_script_errors(name: str, scripts: dict[str, str]) -> list[str]:
    tokens, errors = _chain_tokens(name, scripts.get(name, ""))
    if errors:
        return errors
    segments, separator_errors = _chain_segments(name, tokens)
    return [*errors, *separator_errors, *_cargo_chain_errors(name, segments)]


def validate_verify_chain(package: object) -> list[str]:
    if not isinstance(package, dict):
        return ["package.json 必须是对象"]
    scripts = package.get("scripts")
    if not isinstance(scripts, dict):
        return ["package.json 缺少 scripts"]
    python_tokens, python_errors = _chain_tokens("test:python", scripts.get("test:python", ""))
    return [
        *_bun_script_errors("verify", scripts, VERIFY_SEGMENTS, ()),
        *_bun_script_errors(
            "verify:frontend", scripts, FRONTEND_GATE_SEGMENTS, FRONTEND_GATE_FORBIDDEN
        ),
        *python_errors,
        *_exact_chain_errors("test:python", python_tokens, PYTHON_SUITE_CHAIN),
        *_cargo_script_errors("test:rust", scripts),
    ]


def operation_contract_files(root: pathlib.Path) -> dict[str, pathlib.Path]:
    return {name: root / relative for name, relative in OPERATION_CONTRACT_FILES.items()}




def frontend_sources(root: pathlib.Path) -> dict[str, str]:
    sources: dict[str, str] = {}
    for path in sorted((root / "src").rglob("*")):
        if path.suffix not in FRONTEND_EXTENSIONS or not path.is_file():
            continue
        if FRONTEND_SKIP_PARTS & set(path.relative_to(root).parts):
            continue
        sources[path.relative_to(root).as_posix()] = path.read_text(encoding="utf-8")
    return sources


def read_contract_sources(root: pathlib.Path) -> dict[str, str]:
    files: dict[str, str] = {}
    for name, path in operation_contract_files(root).items():
        files[name] = path.read_text(encoding="utf-8")
    files.update(frontend_sources(root))
    files["tauri.ts"] = files[TAURI_MODULE]
    return files


def validate_operation_contract_files(root: pathlib.Path) -> list[str]:
    try:
        return validate_operation_contract(read_contract_sources(root))
    except (OSError, UnicodeError) as error:
        return [f"无法读取 operation 契约源文件: {getattr(error, 'filename', error)}"]
    except ValueError as error:
        return [f"Operation Broker 静态契约无法完成校验: {error}"]


def _verify_chain_errors(files: dict[str, str]) -> list[str]:
    try:
        package = json.loads(files["package.json"])
    except (KeyError, ValueError) as failure:
        return [f"package.json 无法解析，verify 链不可校验: {failure}"]
    return validate_verify_chain(package)


def validate_operation_contract(files: dict[str, str]) -> list[str]:
    unparseable = parse_errors(files)
    if unparseable:
        return unparseable
    return [
        *validate_ipc_command_surface(files["lib.rs"]),
        *validate_frontend_operation_surface(files),
        *validate_frontend_confirmation_flow(files),
        *validate_cli_operation_surface(files["cli_rs"]),
        *validate_cli_fail_closed(files["cli_operations.rs"]),
        *validate_process_boundary(files),
        *validate_backend_operation_contract(files),
        *validate_operation_history_labels(
            {
                "operation_types.rs": files["operation_types.rs"],
                "HistoryView.tsx": files["HistoryView.tsx"],
                "cli.rs": files["cli_rs"],
                "en.ts": files["en.ts"],
                "zh-CN.ts": files["zh-CN.ts"],
            }
        ),
        *validate_random_id_dependency(files["Cargo.toml"]),
        *_verify_chain_errors(files),
    ]
