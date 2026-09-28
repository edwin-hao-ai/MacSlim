#!/usr/bin/env python3
"""源码结构解析原语（无领域知识）。

所有提取都是结构化的：先把注释与字符串字面量按**等长**空格屏蔽，再在屏蔽
后的文本上按括号配平与嵌套深度切分。因为屏蔽保持长度，返回的下标在原文件里
同样有效，可以直接切回原文取字面量值。

屏蔽层必须能报告自己失败了：未闭合的字符串字面量会抛 `SourceParseError`
（`ValueError` 子类），绝不允许「把剩余源码当字符串吞掉」后继续判定 ——
否则一行 JSX 文案里的撇号就能让整个文件在门禁视野里消失并静默通过。
调用方用 `parse_error` 把该异常转成一条中文错误。
"""
from __future__ import annotations
import re

RUST_QUOTES = '"'
TS_QUOTES = "\"'`"
OPEN_TO_CLOSE = {"(": ")", "[": "]", "{": "}"}
CLOSERS = "})]"
RUST_DEFINITION_RE = re.compile(
    r"\b(?:fn|struct|enum|trait|impl|mod|macro_rules|union)\s+[A-Za-z_]"
)
RUST_FUNCTION_RE = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")
TS_KEY_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*\s*:")
TS_OBJECT_RE = re.compile(r"export\s+const\s+\w+\s*(?::[^=]*?)?=\s*\{")
TS_ARROW_RE = re.compile(
    r"\b(?:const|let)\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?::[^=]*?)?=\s*(?:async\s*)?\("
)


class SourceParseError(ValueError):
    pass


def _blank(target: list[str], start: int, end: int) -> None:
    for offset in range(start, min(end, len(target))):
        target[offset] = " "


def _scan_quoted(source: str, index: int, quote: str) -> int:
    cursor = index + 1
    while cursor < len(source):
        if source[cursor] == "\\":
            cursor += 2
            continue
        if source[cursor] == quote:
            return cursor
        cursor += 1
    line = source.count("\n", 0, index) + 1
    raise SourceParseError(
        f"第 {line} 行出现未闭合的 {quote} 字符串字面量，源码无法被可靠屏蔽"
    )


def parse_error(name: str, source: str, quotes: str) -> str | None:
    try:
        for _kind, _start, _end in _scan_code(source, quotes):
            pass
    except SourceParseError as failure:
        return f"{name} 结构解析失败，静态门禁结果不可信: {failure}"
    return None


def _scan_code(source: str, quotes: str):
    index = 0
    while index < len(source):
        if source.startswith("//", index):
            end = source.find("\n", index)
            end = len(source) if end < 0 else end
            yield ("comment", index, end)
        elif source.startswith("/*", index):
            end = source.find("*/", index + 2)
            end = len(source) if end < 0 else end + 2
            yield ("comment", index, end)
        elif quotes and source[index] in quotes:
            end = _scan_quoted(source, index, source[index]) + 1
            yield ("string", index, end)
        else:
            index += 1
            continue
        index = end


def mask_code(source: str, quotes: str = "") -> str:
    target = list(source)
    for _kind, start, end in _scan_code(source, quotes):
        _blank(target, start, end)
    return "".join(target)


def string_spans(source: str, quotes: str) -> list[tuple[int, int, str]]:
    return [
        (start, end, source[start + 1 : end - 1])
        for kind, start, end in _scan_code(source, quotes)
        if kind == "string"
    ]


def balanced_slice(masked: str, open_index: int) -> str | None:
    if not 0 <= open_index < len(masked):
        return None
    closer = OPEN_TO_CLOSE.get(masked[open_index])
    if closer is None:
        return None
    stack = [closer]
    index = open_index + 1
    while index < len(masked) and stack:
        character = masked[index]
        if character in OPEN_TO_CLOSE:
            stack.append(OPEN_TO_CLOSE[character])
        elif character == stack[-1]:
            stack.pop()
        index += 1
    return None if stack else masked[open_index + 1 : index - 1]


def _depth_zero_index(masked: str, start: int, target: str) -> int:
    depth = 0
    for index in range(max(0, start), len(masked)):
        character = masked[index]
        if character in OPEN_TO_CLOSE:
            depth += 1
        elif character in CLOSERS:
            depth = max(0, depth - 1)
        elif character == target and depth == 0:
            return index
    return -1


def _top_level_ranges(
    masked: str, source: str, start: int, end: int, separator: str
) -> list[tuple[int, int]]:
    ranges: list[tuple[int, int]] = []
    depth = 0
    cursor = start
    for index in range(max(0, start), min(end, len(masked))):
        character = masked[index]
        if character in OPEN_TO_CLOSE:
            depth += 1
        elif character in CLOSERS:
            depth -= 1
        elif character == separator and depth == 0:
            ranges.append((cursor, index))
            cursor = index + 1
    ranges.append((cursor, end))
    return [(a, b) for a, b in ranges if source[a:b].strip()]


def _last_key_before(masked: str, floor: int, index: int) -> str:
    found = None
    for match in TS_KEY_RE.finditer(masked, floor, index):
        found = match.group(0)[:-1].strip()
    return found or ""


# ===== Rust 结构 =====


def rust_call_names(source: str) -> list[str]:
    masked = mask_code(source, RUST_QUOTES)
    names = []
    for match in re.finditer(
        r"\b([A-Za-z_][A-Za-z0-9_]*)\s*(?:::\s*([A-Za-z_][A-Za-z0-9_]*)\s*)?\(",
        masked,
    ):
        if RUST_DEFINITION_RE.match(masked, match.start()):
            continue
        names.append(match.group(2) or match.group(1))
    return names


def rust_use_paths(source: str) -> list[str]:
    masked = mask_code(source, RUST_QUOTES)
    paths = []
    for match in re.finditer(r"\buse\b", masked):
        end = masked.find(";", match.end())
        if end >= 0:
            paths.append(masked[match.end() : end].strip())
    return paths


def rust_function_spans(source: str) -> dict[str, tuple[int, int]]:
    masked = mask_code(source, RUST_QUOTES)
    spans: dict[str, tuple[int, int]] = {}
    for match in RUST_FUNCTION_RE.finditer(masked):
        brace = masked.find("{", match.end())
        body = balanced_slice(masked, brace)
        if body is None:
            continue
        spans.setdefault(match.group(1), (brace + 1, brace + 1 + len(body)))
    return spans


def rust_function_body(source: str, name: str) -> str | None:
    span = rust_function_spans(source).get(name)
    return None if span is None else source[span[0] : span[1]]


def rust_function_text(source: str, name: str) -> str:
    masked = mask_code(source, RUST_QUOTES)
    match = re.search(r"\bfn\s+" + re.escape(name) + r"\b", masked)
    if match is None:
        return ""
    brace = masked.find("{", match.end())
    body = balanced_slice(masked, brace)
    return "" if body is None else source[match.start() : brace + 1 + len(body)]


def macro_blocks(source: str, macro: str) -> list[str]:
    masked = mask_code(source, RUST_QUOTES)
    blocks: list[str] = []
    cursor = 0
    while True:
        index = masked.find(macro, cursor)
        if index < 0:
            return blocks
        cursor = index + 1
        body = balanced_slice(masked, masked.find("[", index))
        if body is not None:
            blocks.append(body)


def macro_entries(block: str) -> list[str]:
    return [
        entry.rsplit("::", 1)[-1].strip()
        for entry in (item.strip() for item in block.split(","))
        if entry.strip()
    ]


def rust_invoke_handler_commands(source: str) -> list[str]:
    blocks = macro_blocks(source, "generate_handler!")
    return macro_entries(blocks[0]) if blocks else []


def rust_declared_commands(source: str) -> list[str]:
    masked = mask_code(source, RUST_QUOTES)
    names = []
    for match in re.finditer(r"#\s*\[\s*(?:::)?tauri::command[^\]]*\]", masked):
        declaration = re.search(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)", masked[match.end() :])
        if declaration:
            names.append(declaration.group(1))
    return names


def rust_serde_directives(source: str, type_name: str) -> tuple[frozenset[str], frozenset[str]]:
    masked = mask_code(source, RUST_QUOTES)
    pattern = re.compile(r"#\s*\[\s*serde\s*\(([^\)]*)\)\s*\]")
    declaration = re.compile(r"\b(?:struct|enum)\s+" + re.escape(type_name) + r"\b")
    for match in pattern.finditer(masked):
        found = declaration.search(masked[match.end() :])
        if found is None:
            continue
        between = masked[match.end() : match.end() + found.start()]
        if re.search(r"\bfn\b|\bimpl\b|\benum\b|\bstruct\b", between):
            continue
        body = source[match.start(1) : match.end(1)]
        flags = frozenset(re.findall(r"[A-Za-z_][A-Za-z0-9_]*", body))
        values = frozenset(value for _s, _e, value in string_spans(body, RUST_QUOTES))
        return flags, values
    return frozenset(), frozenset()


def rust_operation_kind_labels(source: str) -> set[str]:
    body = rust_function_body(source, "label") or ""
    return {value for _s, _e, value in string_spans(body, RUST_QUOTES)}


def rust_if_conditions(masked_body: str) -> list[str]:
    return [match.group(1).strip() for match in re.finditer(r"\bif\b([^{;]+)\{", masked_body)]


def compares_owner(condition: str) -> bool:
    pattern = r"([A-Za-z_][A-Za-z0-9_.]*)\s*!=\s*([A-Za-z_][A-Za-z0-9_.]*)"
    for clause in condition.split("&&"):
        match = re.fullmatch(r"\s*" + pattern + r"\s*", clause)
        if match is None:
            continue
        left, right = match.group(1), match.group(2)
        if (left == "owner" and right.endswith(".owner")) or (
            right == "owner" and left.endswith(".owner")
        ):
            return True
    return False


def rust_struct_fields(body: str) -> list[str]:
    return re.findall(r"(?:^|[\{,])\s*([a-z_][a-z0-9_]*)\s*:", body)


def enum_variants(source: str, type_name: str) -> set[str]:
    masked = mask_code(source, RUST_QUOTES)
    match = re.search(r"\benum\s+" + re.escape(type_name) + r"\b", masked)
    if match is None:
        return set()
    body = balanced_slice(masked, masked.find("{", match.end()))
    if body is None:
        return set()
    return set(re.findall(r"(?:^|[\{,])\s*([A-Z][A-Za-z0-9_]*)", body))


# ===== TypeScript 结构 =====


def _object_fields(masked: str, source: str, open_index: int) -> list[str]:
    body = balanced_slice(masked, open_index)
    if body is None:
        return []
    start = open_index + 1
    fields = []
    for seg_start, seg_end in _top_level_ranges(
        masked, source, start, start + len(body), ";"
    ):
        found = TS_KEY_RE.search(masked[seg_start:seg_end])
        if found:
            fields.append(found.group(0)[:-1].strip())
    return fields


def typescript_type_fields(source: str, alias: str) -> list[str]:
    masked = mask_code(source, TS_QUOTES)
    match = re.search(r"\btype\s+" + re.escape(alias) + r"\s*=", masked)
    if match is None:
        return []
    return _object_fields(masked, source, masked.find("{", match.end()))


def typescript_request_variants(source: str) -> dict[str, tuple[str, ...]]:
    masked = mask_code(source, TS_QUOTES)
    match = re.search(r"\btype\s+PrepareOperationRequest\s*=", masked)
    if match is None:
        return {}
    semicolon = _depth_zero_index(masked, match.end(), ";")
    if semicolon < 0:
        return {}
    variants: dict[str, tuple[str, ...]] = {}
    for start, end in _top_level_ranges(masked, source, match.end(), semicolon, "|"):
        fields = _object_fields(masked, source, masked.find("{", start))
        tag = re.search(r'\btype\s*:\s*"([^"]+)"', source[start:end])
        variants[tag.group(1) if tag else f"<untagged@{start}>"] = tuple(fields)
    return variants


def typescript_invoke_commands(source: str) -> dict[int, str]:
    masked = mask_code(source, TS_QUOTES)
    commands: dict[int, str] = {}
    for match in re.finditer(r"\binvoke\s*\(", masked):
        open_paren = match.end() - 1
        body = balanced_slice(masked, open_paren)
        if body is None:
            commands[match.start()] = "<unterminated>"
            continue
        first = _top_level_ranges(
            masked, source, open_paren + 1, open_paren + 1 + len(body), ","
        )
        commands[match.start()] = source[first[0][0] : first[0][1]].strip() if first else ""
    return commands


def typescript_nested_keys(source: str) -> set[str]:
    masked = mask_code(source, TS_QUOTES)
    match = TS_OBJECT_RE.search(masked)
    if match is None:
        return set()
    opener = match.end() - 1
    keys: set[str] = set()
    path: list[str] = []
    index = opener + 1
    while index < len(masked):
        character = masked[index]
        if character == "{":
            path.append(_last_key_before(masked, opener, index))
        elif character == "}":
            if not path:
                break
            path.pop()
        else:
            found = TS_KEY_RE.match(masked, index)
            if found is not None:
                keys.add(
                    ".".join([part for part in path if part] + [found.group(0)[:-1].strip()])
                )
        index += 1
    return keys


def typescript_switch_arms(source: str, needle: str) -> dict[str, tuple[int, int]]:
    masked = mask_code(source, TS_QUOTES)
    index = masked.find(needle)
    if index < 0:
        return {}
    switch = masked.find("switch", index)
    if switch < 0:
        return {}
    body_start = masked.find("{", masked.find(")", switch))
    body = balanced_slice(masked, body_start)
    if body is None:
        return {}
    start = body_start + 1
    end = start + len(body)
    arms: dict[str, tuple[int, int]] = {}
    for seg_start, seg_end in _top_level_ranges(masked, source, start, end, ";"):
        if re.match(r"\s*case\s", masked[seg_start:seg_end]) is None:
            continue
        literal = re.match(r'\s*case\s*("[^"]*")', source[seg_start:seg_end])
        if literal:
            arms[literal.group(1)[1:-1]] = (seg_start + literal.end(), seg_end)
    return arms


def typescript_arrow_function_spans(source: str) -> dict[str, tuple[int, int]]:
    """`const name = (...) => { ... }` 的函数体区间（下标对原文件有效）。"""
    masked = mask_code(source, TS_QUOTES)
    spans: dict[str, tuple[int, int]] = {}
    for match in TS_ARROW_RE.finditer(masked):
        open_paren = masked.find("(", match.end() - 1)
        params = balanced_slice(masked, open_paren)
        if params is None:
            continue
        after_params = open_paren + 2 + len(params)
        arrow = _depth_zero_index(masked, after_params, "=")
        if arrow < 0 or masked[arrow : arrow + 2] != "=>":
            continue
        brace = arrow + 2
        while brace < len(masked) and masked[brace].isspace():
            brace += 1
        if brace >= len(masked) or masked[brace] != "{":
            continue
        body = balanced_slice(masked, brace)
        if body is None:
            continue
        spans.setdefault(match.group(1), (brace + 1, brace + 1 + len(body)))
    return spans


def typescript_component_names(source: str) -> set[str]:
    """Solid 组件（`const X: Component<...> = ...`）的函数名集合。

    组件体是声明式模板，不按 AGENTS.md §7.1 的 50 行函数上限衡量；
    规模由文件上限与 ledger 的组件清单负责。
    """
    masked = mask_code(source, TS_QUOTES)
    return {
        match.group(1)
        for match in re.finditer(
            r"\bconst\s+([A-Za-z_][A-Za-z0-9_]*)\s*:\s*Component\b", masked
        )
    }


