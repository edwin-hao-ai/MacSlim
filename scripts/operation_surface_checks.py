#!/usr/bin/env python3
"""Operation Broker 的 IPC / 前端 / CLI 表面门禁。

每一项检查都建立在 scripts/source_structure.py 的结构提取之上：判定必须落在
invoke handler、TS 类型联合、switch/match 分支、函数体这些真实结构上。
"""
from __future__ import annotations
import re

try: from scripts import source_structure as _structure
except ModuleNotFoundError: import source_structure as _structure
mask_code = _structure.mask_code
balanced_slice = _structure.balanced_slice
parse_error = _structure.parse_error
rust_call_names = _structure.rust_call_names
rust_declared_commands = _structure.rust_declared_commands
rust_function_body = _structure.rust_function_body
rust_function_spans = _structure.rust_function_spans
rust_invoke_handler_commands = _structure.rust_invoke_handler_commands
rust_use_paths = _structure.rust_use_paths
_macro_blocks = _structure.macro_blocks
_macro_entries = _structure.macro_entries
_rust_struct_fields = _structure.rust_struct_fields
string_spans = _structure.string_spans
typescript_arrow_function_spans = _structure.typescript_arrow_function_spans
typescript_invoke_commands = _structure.typescript_invoke_commands
typescript_type_fields = _structure.typescript_type_fields
typescript_request_variants = _structure.typescript_request_variants
RUST_QUOTES = _structure.RUST_QUOTES
TS_QUOTES = _structure.TS_QUOTES

EXPECTED_IPC_COMMANDS = (
    # 纯元数据：下发当前构建形态（developer_id / mas），前端据此决定
    # 「终止进程」这类沙箱里做不到的入口要不要出现。零副作用、不碰用户数据。
    "get_build_flavor",
    "get_system_health",
    "scan_all",
    "list_all_processes",
    "scan_cache",
    "get_history",
    "get_whitelist",
    "add_whitelist",
    "remove_whitelist",
    "list_applications",
    "docker_available",
    "docker_inventory",
    "scan_installed_apps",
    "scan_app_residues_batch",
    "check_app_running",
    "prepare_operation",
    "execute_operation",
)
DESTRUCTIVE_IPC_COMMANDS = ("prepare_operation", "execute_operation")
RAW_IPC_COMMANDS = frozenset(
    {
        "kill_processes",
        "clean_cache",
        "quit_application",
        "force_quit_application",
        "uninstall_apps",
        "quit_and_uninstall",
        "scan_app_residues",
        "docker_remove_image",
        "docker_remove_container",
        "docker_remove_volume",
        "docker_prune_all",
    }
)
RAW_FRONTEND_WRAPPERS = frozenset(
    {
        "killProcesses",
        "cleanCache",
        "quitApplication",
        "forceQuitApplication",
        "uninstallApps",
        "quitAndUninstall",
        "scanAppResidues",
        "dockerRemoveImage",
        "dockerRemoveContainer",
        "dockerRemoveVolume",
        "dockerPruneAll",
    }
)
RAW_FRONTEND_TYPES = frozenset(
    {"UninstallTarget", "KillResult", "KillReport", "CacheScanResult"}
)
RAW_REQUEST_FIELDS = frozenset(
    {
        "pid",
        "pids",
        "name",
        "names",
        "path",
        "paths",
        "command",
        "items",
        "targets",
        "args",
        "bundle_path",
        "bundle_id",
        "app_name",
        "residue_paths",
        "residue_items",
        "docker_args",
        "uninstall_targets",
    }
)
ALLOWED_REQUEST_FIELDS = frozenset(
    {
        "type",
        "snapshot_id",
        "app_snapshot_id",
        "residue_snapshot_id",
        "item_keys",
        "process_keys",
        "app_keys",
        "residue_keys",
        "mode",
        "quit_running",
        "action",
        "target_keys",
    }
)
EXPECTED_REQUEST_VARIANTS = {
    "cache": ("type", "snapshot_id", "item_keys"),
    "process": ("type", "snapshot_id", "process_keys", "mode"),
    "app_terminate": ("type", "snapshot_id", "app_keys", "mode"),
    "app_graceful_quit": ("type", "snapshot_id", "app_keys"),
    "uninstall": (
        "type",
        "app_snapshot_id",
        "residue_snapshot_id",
        "app_keys",
        "residue_keys",
        "quit_running",
    ),
    "docker": ("type", "snapshot_id", "action", "target_keys"),
}
ALLOWED_CLI_FLAGS = frozenset(
    {
        "--help",
        "-h",
        "--version",
        "-V",
        "--scan",
        "--cache",
        "--disk",
        "--npm",
        "--xcode",
        "--process",
        "--list",
        "--docker",
        "--history",
        "--whitelist",
    }
)
RAW_CLI_FLAGS = frozenset(
    {
        "--pid",
        "--pids",
        "--path",
        "--paths",
        "--target",
        "--targets",
        "--bundle",
        "--bundle-path",
        "--app",
        "--item",
        "--items",
        "--kill",
        "--force",
        "--key",
        "--command",
        "--image",
        "--container",
        "--volume",
    }
)
RAW_CLI_FUNCTIONS = frozenset(
    {
        "graceful_kill",
        "force_kill_tree",
        "clean_snapshot_items",
        "clean_snapshot_with",
        "uninstall_app",
        "quit_app",
        "remove_image",
        "remove_container",
        "remove_volume",
        "prune_all",
    }
)
ALLOWED_CLI_LIB_MODULES = frozenset(
    {
        "cache_cleaner",
        "cache_scanner",
        "cli_operations",
        "operations",
        "run_tauri",
        "scanner",
        "scanner_read_health",
        "storage",
    }
)
CRATE_PRIVATE_MODULES = ("process_ops",)
CRATE_PRIVATE_PROCESS_ITEMS = (
    ("trait", "ProcessSignaller"),
    ("trait", "ProcessObserver"),
    ("struct", "SystemProcessSignaller"),
    ("struct", "SystemProcessObserver"),
    ("struct", "LiveProcess"),
    ("enum", "KillOutcome"),
    ("type", "WhitelistPolicy"),
)
CRATE_PRIVATE_TARGET_TYPES = (("struct", "ProcessTarget"),)
RESTRICTED_VISIBILITIES = frozenset(
    {"crate", "super", "self", "in crate", "in super", "in self"}
)
CLI_SESSION_GATE = "CliSession::open"
CLI_SESSION_GATES = (
    ("clean_cache_in_session", "clean_snapshot_with"),
    ("clean_processes_in_session", "build_observer"),
)
FRONTEND_EXTENSIONS = (".ts", ".tsx")
TAURI_MODULE = "src/lib/tauri.ts"
CONFIRM_REQUIRED_SOURCES = (
    "src/views/CacheView.tsx",
    "src/views/ScanView.tsx",
    "src/views/ProcessView.tsx",
    "src/views/ApplicationsView.tsx",
    "src/views/UninstallerView.tsx",
    "src/components/DockerSection.tsx",
)



def _handler_errors(source: str) -> list[str]:
    masked = mask_code(source, RUST_QUOTES)
    errors = []
    if masked.count("invoke_handler") != 1:
        errors.append("lib.rs 必须恰好有一个 invoke_handler")
    blocks = _macro_blocks(source, "generate_handler!")
    if len(blocks) != 1:
        errors.append(f"lib.rs 必须恰好有一个 generate_handler!，当前 {len(blocks)} 个")
    if not blocks:
        errors.append("invoke_handler 必须注册 tauri::generate_handler! command 列表")
        return errors
    entries = [name for block in blocks for name in _macro_entries(block)]
    if not entries:
        errors.append("generate_handler! 不得为空")
        return errors
    if len(entries) != len(set(entries)):
        errors.append("generate_handler! 存在重复 command")
    for raw in sorted(RAW_IPC_COMMANDS & set(entries)):
        errors.append(f"generate_handler! 仍注册 raw 破坏性 command: {raw}")
    if set(entries) != set(EXPECTED_IPC_COMMANDS):
        missing = sorted(set(EXPECTED_IPC_COMMANDS) - set(entries))
        extra = sorted(set(entries) - set(EXPECTED_IPC_COMMANDS))
        errors.append(
            f"IPC command 面必须是 {len(EXPECTED_IPC_COMMANDS)} 个可信 command"
            f"（缺少 {missing}，多余 {extra}）"
        )
    destructive = [name for name in entries if name in DESTRUCTIVE_IPC_COMMANDS]
    if destructive != list(DESTRUCTIVE_IPC_COMMANDS):
        errors.append(
            f"破坏性 command 必须精确为 {list(DESTRUCTIVE_IPC_COMMANDS)}，当前 {destructive}"
        )
    return errors


def _declaration_errors(source: str) -> list[str]:
    declared = rust_declared_commands(source)
    registered = set(rust_invoke_handler_commands(source))
    errors = [
        f"声明了 #[tauri::command] {name} 但没有注册，属于残留 raw handler"
        for name in sorted(set(declared) - registered)
    ]
    errors.extend(
        f"invoke_handler 注册了 {name} 但没有对应的 #[tauri::command] 声明"
        for name in sorted(registered - set(declared))
    )
    if sorted(declared) != sorted(registered):
        errors.append("#[tauri::command] 声明集合与 invoke_handler 注册集合不一致")
    return errors


def _parse_gate(name: str, source: str) -> list[str]:
    error = parse_error(name, source, RUST_QUOTES)
    return [error] if error is not None else []


def validate_ipc_command_surface(source: str) -> list[str]:
    unparseable = _parse_gate("src-tauri/src/lib.rs", source)
    if unparseable:
        return unparseable
    return [*_handler_errors(source), *_declaration_errors(source)]


def _tauri_module_source(sources: dict[str, str]) -> str | None:
    for name in ("tauri.ts", TAURI_MODULE):
        if name in sources:
            return sources[name]
    return None


def _is_tauri_module(name: str) -> bool:
    return name in {"tauri.ts", TAURI_MODULE}


def _is_frontend_test(name: str) -> bool:
    return name.endswith(".test.ts") or name.endswith(".test.tsx")


def _frontend_wrapper_errors(sources: dict[str, str]) -> list[str]:
    errors = []
    for name, source in sorted(_frontend_files(sources).items()):
        masked = mask_code(source, TS_QUOTES)
        for wrapper in sorted(RAW_FRONTEND_WRAPPERS):
            if re.search(rf"\b{wrapper}\s*(?:<[^;()]*>)?\s*\(", masked):
                errors.append(f"{name} 仍调用 raw 破坏性封装 {wrapper}")
        errors.extend(
            f"{name} 仍引用已删除的 raw 类型 {alias}"
            for alias in sorted(RAW_FRONTEND_TYPES)
            if re.search(rf"\b{alias}\b", masked)
        )
    return errors


def _frontend_files(sources: dict[str, str]) -> dict[str, str]:
    return {
        name: source
        for name, source in sources.items()
        if name.endswith(FRONTEND_EXTENSIONS)
    }


def _frontend_invoke_errors(sources: dict[str, str]) -> list[str]:
    module = _tauri_module_source(sources)
    errors = []
    for name, source in sorted(_frontend_files(sources).items()):
        if _is_tauri_module(name) or _is_frontend_test(name):
            continue
        if re.search(r"\binvoke\s*\(", mask_code(source, TS_QUOTES)):
            errors.append(f"{name} 直接调用 invoke，破坏性入口必须收敛到 {TAURI_MODULE}")
    if module is None:
        return [*errors, f"缺少 {TAURI_MODULE}，无法校验前端操作面"]
    literals = typescript_invoke_commands(module)
    for position, literal in sorted(literals.items()):
        if re.fullmatch(r'"[a-z0-9_]+"', literal) is None:
            line = module.count("\n", 0, position) + 1
            errors.append(
                f"{TAURI_MODULE}:{line} invoke 的 command 必须是静态字符串字面量，"
                f"当前 {literal!r}"
            )
    names = [value.strip('"') for value in literals.values() if re.fullmatch(r'"[a-z0-9_]+"', value)]
    if set(names) != set(EXPECTED_IPC_COMMANDS):
        missing = sorted(set(EXPECTED_IPC_COMMANDS) - set(names))
        extra = sorted(set(names) - set(EXPECTED_IPC_COMMANDS))
        errors.append(f"{TAURI_MODULE} 封装集合与注册面不一致（缺少 {missing}，多余 {extra}）")
    if len(names) != len(set(names)):
        errors.append(f"{TAURI_MODULE} 存在重复的 invoke command 封装")
    errors.extend(
        f"{TAURI_MODULE} 仍封装 raw 破坏性 command: {raw}"
        for raw in sorted(RAW_IPC_COMMANDS & set(names))
    )
    return errors


def _frontend_request_errors(source: str) -> list[str]:
    variants = typescript_request_variants(source)
    errors = []
    for tag, fields in sorted(variants.items()):
        raw = sorted(RAW_REQUEST_FIELDS & set(fields))
        if raw:
            errors.append(f"PrepareOperationRequest.{tag} 携带 raw 目标字段 {raw}")
        extra = sorted(set(fields) - ALLOWED_REQUEST_FIELDS)
        if extra:
            errors.append(f"PrepareOperationRequest.{tag} 出现未授权字段 {extra}")
        if tag not in EXPECTED_REQUEST_VARIANTS:
            errors.append(f"PrepareOperationRequest 出现未登记的变体 {tag}")
        elif sorted(fields) != sorted(EXPECTED_REQUEST_VARIANTS[tag]):
            errors.append(
                f"PrepareOperationRequest.{tag} 字段集必须是 "
                f"{sorted(EXPECTED_REQUEST_VARIANTS[tag])}，当前 {sorted(fields)}"
            )
    if set(variants) != set(EXPECTED_REQUEST_VARIANTS):
        errors.append(
            "PrepareOperationRequest 变体集合必须精确等于 "
            f"{sorted(EXPECTED_REQUEST_VARIANTS)}，当前 {sorted(variants)}"
        )
    cache_fields = typescript_type_fields(source, "CacheItem")
    if "command" in cache_fields:
        errors.append("CacheItem 不能携带可执行 shell command 字段")
    if "path" not in cache_fields:
        errors.append("CacheItem 必须保留 path 作为只读展示字段")
    return errors


def frontend_parse_errors(sources: dict[str, str]) -> list[str]:
    errors = []
    for name, source in sorted(_frontend_files(sources).items()):
        error = parse_error(name, source, TS_QUOTES)
        if error is not None:
            errors.append(error)
    return errors


def validate_frontend_operation_surface(sources: dict[str, str]) -> list[str]:
    unparseable = frontend_parse_errors(sources)
    if unparseable:
        return unparseable
    module = _tauri_module_source(sources)
    errors = [*_frontend_wrapper_errors(sources), *_frontend_invoke_errors(sources)]
    if module is not None:
        errors.extend(_frontend_request_errors(module))
    return errors


def _own_body(masked: str, spans: dict[str, tuple[int, int]], name: str) -> str:
    """某个箭头函数**自身**的函数体：先挖掉所有嵌套箭头函数的区间。"""
    start, end = spans[name]
    body = list(masked[start:end])
    for other, (inner_start, inner_end) in spans.items():
        if other == name or not (start <= inner_start and inner_end <= end):
            continue
        for offset in range(inner_start - start, inner_end - start):
            body[offset] = " "
    return "".join(body)


def _confirmation_gate_errors(sources: dict[str, str]) -> list[str]:
    errors: list[str] = []
    for name in CONFIRM_REQUIRED_SOURCES:
        source = sources.get(name)
        if source is None:
            errors.append(f"{name} 不存在，破坏性确认流程无法校验")
            continue
        masked = mask_code(source, TS_QUOTES)
        if "OperationConfirm" not in masked:
            errors.append(
                f"{name} 必须复用 OperationConfirm 展示后端 summary/estimated_bytes/expires_at_ms "
                "之后才允许 execute"
            )
        spans = typescript_arrow_function_spans(source)
        for function in sorted(spans):
            body = _own_body(masked, spans, function)
            if "prepareOperation" in body and "executeOperation" in body:
                errors.append(
                    f"{name} 的 {function} 在同一个函数里 prepare 后立刻 execute，"
                    "用户没有机会确认后端摘要"
                )
    return errors


def validate_frontend_confirmation_flow(sources: dict[str, str]) -> list[str]:
    unparseable = frontend_parse_errors(sources)
    if unparseable:
        return unparseable
    return _confirmation_gate_errors(sources)


def _cli_flag_errors(source: str) -> list[str]:
    body = rust_function_body(source, "parse_args")
    if body is None:
        return ["bin/cli.rs 缺少 parse_args，无法校验命令行参数面"]
    flags = {value for _s, _e, value in string_spans(body, RUST_QUOTES) if value.startswith("-")}
    errors = [f"CLI parse_args 仍接受 raw 目标参数 {flag}" for flag in sorted(RAW_CLI_FLAGS & flags)]
    errors.extend(f"CLI parse_args 接受未登记参数 {flag}" for flag in sorted(flags - ALLOWED_CLI_FLAGS))
    return errors


def _cli_bypass_errors(source: str) -> list[str]:
    calls = set(rust_call_names(source))
    paths = rust_use_paths(source)
    errors = [
        f"CLI 直接调用领域执行函数 {name}，必须走 Operation Broker"
        for name in sorted(RAW_CLI_FUNCTIONS & calls)
    ]
    for name in sorted(RAW_CLI_FUNCTIONS):
        errors.extend(
            f"CLI 通过 use 重新引入迁移桥 {name}（{path}）"
            for path in paths
            if f"::{name}" in path or path.rsplit("::", 1)[-1].strip("{} ") == name
        )
    return errors


def _cli_library_modules(masked: str) -> set[str]:
    modules = set()
    for match in re.finditer(r"\bmacslim_lib\b\s*::\s*", masked):
        head = re.match(r"[A-Za-z_][A-Za-z0-9_]*", masked[match.end() :])
        if head is not None:
            modules.add(head.group(0))
    return modules


def _cli_library_errors(source: str) -> list[str]:
    masked = mask_code(source, RUST_QUOTES)
    errors = [
        f"bin/cli.rs 不得引用 process_ops 模块（{context}）："
        "可执行信号能力必须留在 crate 内部，CLI 只能拿 operations 的结果 DTO"
        for context in sorted(set(re.findall(r"[^\s;]*process_ops[^\s;]*", masked)))
    ]
    modules = _cli_library_modules(masked)
    errors.extend(
        f"bin/cli.rs 依赖了未授权的 macslim_lib 模块 {module}，"
        f"CLI 可用的库模块只能是 {sorted(ALLOWED_CLI_LIB_MODULES)}"
        for module in sorted(modules - ALLOWED_CLI_LIB_MODULES)
    )
    return errors


def validate_cli_operation_surface(source: str) -> list[str]:
    unparseable = _parse_gate("src-tauri/src/bin/cli.rs", source)
    if unparseable:
        return unparseable
    return [
        *_cli_flag_errors(source),
        *_cli_bypass_errors(source),
        *_cli_library_errors(source),
    ]


def _declaration_visibilities(masked: str, keyword: str, name: str) -> set[str]:
    pattern = re.compile(
        r"\bpub\s*(?:\(([^)]*)\))?\s+" + keyword + r"\s+" + re.escape(name) + r"\b"
    )
    found = set()
    for match in pattern.finditer(masked):
        restriction = match.group(1)
        found.add("pub" if restriction is None else restriction.strip())
    return found


def _module_visibility_errors(lib_source: str, module: str) -> list[str]:
    masked = mask_code(lib_source, RUST_QUOTES)
    if re.search(r"\bmod\s+" + re.escape(module) + r"\b", masked) is None:
        return [f"lib.rs 缺少 process_ops 模块声明，进程域边界无法校验"]
    return [
        f"lib.rs 的 {module} 模块当前是 {scope}，"
        f"它必须收窄为 pub(crate) 等受限可见性，否则 library 外的 crate 能直接构造信号器"
        for scope in sorted(
            _declaration_visibilities(masked, "mod", module) - RESTRICTED_VISIBILITIES
        )
    ]


def _crate_private_item_errors(source: str, name: str, items) -> list[str]:
    masked = mask_code(source, RUST_QUOTES)
    errors = []
    for keyword, item in items:
        for scope in sorted(
            _declaration_visibilities(masked, keyword, item) - RESTRICTED_VISIBILITIES
        ):
            errors.append(
                f"{name} 的 {keyword} {item} 当前是 {scope}，"
                "它必须收窄为 pub(crate) 等受限可见性，"
                "否则 library 外的 crate 能绕过 Store/owner/TTL/protected gate"
            )
    return errors


def _public_field_names(masked: str, struct_name: str) -> set[str]:
    match = re.search(r"\bstruct\s+" + re.escape(struct_name) + r"\b", masked)
    if match is None:
        return set()
    body = balanced_slice(masked, masked.find("{", match.end()))
    return set() if body is None else set(re.findall(r"\bpub\s+([a-z_][a-z0-9_]*)\s*:", body))


def _process_boundary_errors(sources: dict[str, str]) -> list[str]:
    errors: list[str] = []
    for module in CRATE_PRIVATE_MODULES:
        errors.extend(_module_visibility_errors(sources["lib.rs"], module))
        errors.extend(
            f"lib.rs 用 pub 重新导出了 {module}::{tail}，crate 边界被绕过"
            for match in re.finditer(
                r"\bpub\s+use\b[^;]*", mask_code(sources["lib.rs"], RUST_QUOTES)
            )
            if f"{module}::" in match.group(0)
            for tail in [match.group(0).strip()]
        )
    errors.extend(
        _crate_private_item_errors(
            sources["process_ops.rs"], "process_ops.rs", CRATE_PRIVATE_PROCESS_ITEMS
        )
    )
    errors.extend(
        _crate_private_item_errors(
            sources["operation_types.rs"],
            "operation_types.rs",
            CRATE_PRIVATE_TARGET_TYPES,
        )
    )
    types_masked = mask_code(sources["operation_types.rs"], RUST_QUOTES)
    for _keyword, target in CRATE_PRIVATE_TARGET_TYPES:
        errors.extend(
            f"operation_types.rs 的 {target} 字段 {field} 仍是 pub，"
            "外部 crate 可以自行拼装可执行目标"
            for field in sorted(_public_field_names(types_masked, target))
        )
    cli_source = sources["cli.rs"]
    errors.extend(_cli_library_errors(cli_source))
    cli_calls = set(rust_call_names(cli_source))
    errors.extend(
        f"bin/cli.rs 直接构造或驱动 crate 私有的 {item}，"
        "信号能力不得离开 crate 内部"
        for keyword, item in CRATE_PRIVATE_PROCESS_ITEMS
        if keyword in ("struct", "enum")
        for name in (item, f"{item}::new")
        if name in cli_calls
    )
    return errors


def validate_process_boundary(sources: dict[str, str]) -> list[str]:
    files = {**sources, "cli.rs": sources["cli_rs"]}
    unparseable = []
    for name in ("lib.rs", "process_ops.rs", "operation_types.rs", "cli.rs"):
        error = parse_error(name, files[name], RUST_QUOTES)
        if error is not None:
            unparseable.append(error)
    if unparseable:
        return unparseable
    return _process_boundary_errors(files)


def _cli_storage_open_errors(source: str) -> list[str]:
    masked = mask_code(source, RUST_QUOTES)
    errors = [
        f"CLI 对 Storage::open() 使用 .{method}()，破坏性路径必须 fail-closed"
        for method in sorted(
            set(re.findall(r"Storage::open\(\)\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(", masked))
        )
    ]
    if CLI_SESSION_GATE not in masked:
        errors.append(f"CLI 破坏性路径必须经过 {CLI_SESSION_GATE} 前置检查")
    return errors


def _cli_session_order_errors(source: str) -> list[str]:
    errors = []
    for name, gate in CLI_SESSION_GATES:
        body = rust_function_body(source, name)
        if body is None:
            errors.append(f"CLI 缺少 {name}")
            continue
        masked = mask_code(body, RUST_QUOTES)
        session = masked.find(CLI_SESSION_GATE)
        domain = masked.find(gate)
        if session < 0:
            errors.append(
                f"{name} 必须在构造任何领域执行器之前调用 {CLI_SESSION_GATE}"
            )
        elif domain < 0:
            errors.append(
                f"{name} 必须真正调用 {gate}，注释或字符串里的同名标记不算执行"
            )
        elif domain < session:
            errors.append(f"{name} 必须在 {gate} 之前打开存储会话（存储不可读时直接失败）")
    return errors


def validate_cli_fail_closed(source: str) -> list[str]:
    unparseable = _parse_gate("src-tauri/src/cli_operations.rs", source)
    if unparseable:
        return unparseable
    return [*_cli_storage_open_errors(source), *_cli_session_order_errors(source)]


