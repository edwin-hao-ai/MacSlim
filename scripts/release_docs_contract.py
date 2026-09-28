from __future__ import annotations
import ast
import json
import re
import shlex

README_SECTION = "## Updater 发布门禁"
EXPECTED_TARGETS = {
    "arm": ("aarch64-apple-darwin", "aarch64"),
    "intel": ("x86_64-apple-darwin", "x64"),
    "universal": ("universal-apple-darwin", "universal"),
}
README_MARKERS = (
    "build-stamp.json",
    "sidecar",
    "staging",
    "publish_assets.py commit",
    "src-tauri/tauri.conf.json",
    "版本参数必须",
    "不得手工复制旧 tar",
    "不得手工修改 manifest",
    "不得跨 target",
)
PYTHON_TEST_TOKENS = (
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
CONTROL_TOKENS = {"&&", "||", ";", "|", "&"}
_CONFIG_VERSION_RE = re.compile(
    r'^\s*CONFIGURED_VERSION="\$\(\s*python3\s+-c\s+(?P<quote>[\'"])(?P<code>.*?)(?P=quote)\s*\)"\s*$'
)

def _load_package(package: object) -> tuple[object, list[str]]:
    if isinstance(package, str):
        try:
            package = json.loads(package)
        except (TypeError, ValueError):
            return None, ["package.json 格式无效"]
    if not isinstance(package, dict):
        return None, ["package.json 必须是对象"]
    return package, []

def _tokens(command: str) -> list[str] | None:
    try:
        return shlex.split(command, comments=True, posix=True)
    except ValueError:
        return None

def _unwrap(tokens: list[str]) -> list[str]:
    wrappers = {"!", "if", "then", "do", "elif", "else", "while", "until"}
    index = 0
    while index < len(tokens) and tokens[index] in wrappers:
        index += 1
    return tokens[index:]

def _segments(tokens: list[str]) -> list[list[str]]:
    result = []
    current = []
    for token in tokens:
        if token in CONTROL_TOKENS:
            if current:
                result.append(current)
                current = []
        else:
            current.append(token)
    if current:
        result.append(current)
    return result

def _package_errors(package: object) -> list[str]:
    package, errors = _load_package(package)
    if errors:
        return errors
    scripts = package.get("scripts")
    if not isinstance(scripts, dict):
        return ["package.json 缺少 scripts"]
    python_test = scripts.get("test:python")
    python_tokens = _tokens(python_test) if isinstance(python_test, str) else None
    if python_tokens != list(PYTHON_TEST_TOKENS):
        errors.append("test:python 必须精确执行无缓存的 scripts/tests discovery")
    verify = scripts.get("verify")
    verify_tokens = _tokens(verify) if isinstance(verify, str) else None
    verify_segments = _segments(verify_tokens) if verify_tokens is not None else []
    test_segments = [
        segment
        for segment in verify_segments
        if _unwrap(segment) == ["bun", "run", "test:python"]
    ]
    if len(test_segments) != 1:
        errors.append("verify 必须恰好一次连续执行 bun run test:python")
    if verify_tokens is not None and any(
        token in {"||", ";", "|", "&"}
        or (
            token != "&&"
            and any(separator in token for separator in (";", "|", "&"))
        )
        for token in verify_tokens
    ):
        errors.append("verify 不得使用吞错或非 && 分隔符")
    if test_segments:
        index = next(
            index
            for index, segment in enumerate(verify_segments)
            if _unwrap(segment) == ["bun", "run", "test:python"]
        )
        if index > 0 and _unwrap(verify_segments[index - 1])[:1] == ["false"]:
            errors.append("verify 不得在 false && 后执行 test:python")
    return errors

def _documentation_section(readme: str) -> str | None:
    lines = readme.splitlines()
    start = next((index for index, line in enumerate(lines) if line.strip() == README_SECTION), None)
    if start is None:
        return None
    end = next(
        (index for index in range(start + 1, len(lines)) if lines[index].startswith("## ")),
        len(lines),
    )
    return "\n".join(lines[start + 1 : end])

def _bash_blocks(section: str) -> list[str]:
    return [
        match.group(1)
        for match in re.finditer(r"(?ms)^```bash[ \t]*\n(.*?)^```[ \t]*$", section)
    ]

def _code_statements(section: str) -> list[list[str]]:
    statements = []
    for block in _bash_blocks(section):
        for line in block.replace("\\\n", " ").splitlines():
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            tokens = _tokens(line)
            if tokens:
                statements.append(tokens)
    return statements

def _is_rebuild(tokens: list[str]) -> bool:
    tokens = _unwrap(tokens)
    if tokens[:3] != ["python3", "scripts/updater_artifact.py", "rebuild"]:
        return False
    return all(
        any(
            tokens[index] == option
            and index + 1 < len(tokens)
            and tokens[index + 1] not in {"--app", "--archive", "--target"}
            for index in range(len(tokens))
        )
        for option in ("--app", "--archive", "--target")
    )

def _is_release(tokens: list[str]) -> bool:
    return _unwrap(tokens)[:1] == ["./scripts/release.sh"]

def _is_publish(tokens: list[str]) -> bool:
    return _unwrap(tokens)[:1] == ["./scripts/publish-update.sh"]

def _is_python_interpreter(token: str) -> bool:
    name = token.rsplit("/", 1)[-1]
    return name in {"python", "python3"} or bool(re.fullmatch(r"python(?:\d+(?:\.\d+)?)?", name))

def _is_public_stamp_command(tokens: list[str]) -> bool:
    tokens = _unwrap(tokens)
    for index, token in enumerate(tokens):
        if not _is_python_interpreter(token):
            continue
        arguments = tokens[index + 1 :]
        if arguments[:2] == ["-m", "scripts.updater_artifact"] and arguments[2:3] == ["stamp"]:
            return True
        if arguments[:1] and arguments[0].lstrip("./") == "scripts/updater_artifact.py":
            if arguments[1:2] == ["stamp"]:
                return True
    return False

def _table_rows(section: str) -> list[tuple[str, ...]]:
    rows = []
    for line in section.splitlines():
        stripped = line.strip()
        if "|" not in stripped:
            continue
        cells = [cell.strip().strip("`").strip() for cell in stripped.strip("|").split("|")]
        if len(cells) < 2 or all(re.fullmatch(r":?-{3,}:?", cell) for cell in cells):
            continue
        rows.append(tuple(cells))
    return rows

def validate_release_documentation_contract(readme: object, package: object) -> list[str]:
    errors = []
    if not isinstance(readme, str):
        return ["scripts/README.md 内容必须是文本"]
    errors.extend(_package_errors(package))
    section = _documentation_section(readme)
    if section is None:
        errors.append("README 缺少 Updater 发布门禁章节")
        return errors
    code = _code_statements(section)
    if not code:
        errors.append("Updater 发布门禁缺少 bash code block")
    positions = {
        "release": [index for index, tokens in enumerate(code) if _is_release(tokens)],
        "rebuild": [index for index, tokens in enumerate(code) if _is_rebuild(tokens)],
        "publish": [index for index, tokens in enumerate(code) if _is_publish(tokens)],
    }
    if not all(positions.values()):
        errors.append("Updater 发布门禁必须包含真实 release、rebuild、publish 命令")
    elif not (
        min(positions["release"]) < min(positions["rebuild"]) < min(positions["publish"])
    ):
        errors.append("README 必须按 release → rebuild → publish 顺序发布")
    if "sign.sh" not in section or not any(word in section for word in ("二选一", "替代", "或")):
        errors.append("README 必须保留 sign 替代说明")
    if any(_is_public_stamp_command(tokens) for tokens in code):
        errors.append("README bash 门禁不得执行公开 stamp 命令")
    rows = set(_table_rows(section))
    for target, values in EXPECTED_TARGETS.items():
        if (target, values[0], values[1]) not in rows:
            errors.append(f"README 门禁表格缺少 {target} target 映射")
    evidence = "\n".join(" ".join(tokens) for tokens in code)
    evidence += "\n" + "\n".join(" ".join(row) for row in rows)
    for marker in README_MARKERS:
        if marker not in evidence:
            errors.append(f"README 门禁章节缺少 {marker} 契约")
    return errors

def _assignment_map(tree: ast.AST) -> dict[str, ast.AST]:
    assignments = {}
    for node in ast.walk(tree):
        if isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name):
                    assignments[target.id] = node.value
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            assignments[node.target.id] = node.value
    return assignments

def _target_names(target: ast.AST) -> set[str]:
    if isinstance(target, ast.Name):
        return {target.id}
    if isinstance(target, ast.Attribute) and target.attr == "write_stamp":
        return {"write_stamp"}
    if isinstance(target, ast.Subscript) and _string_value(target.slice, {}) == "write_stamp":
        return {"write_stamp"}
    if isinstance(target, (ast.Tuple, ast.List)):
        return {name for item in target.elts for name in _target_names(item)}
    return set()

def _string_value(
    node: ast.AST | None,
    assignments: dict[str, ast.AST],
    seen: set[str] | None = None,
) -> str | None:
    if node is None:
        return None
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    if hasattr(ast, "Index") and isinstance(node, ast.Index):
        return _string_value(node.value, assignments, seen)
    if isinstance(node, ast.Name):
        if seen is None:
            seen = set()
        if node.id in seen or node.id not in assignments:
            return None
        return _string_value(assignments[node.id], assignments, seen | {node.id})
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):
        left = _string_value(node.left, assignments, seen)
        right = _string_value(node.right, assignments, seen)
        if left is not None and right is not None:
            return left + right
    return None

def _is_attribute_call(node: ast.AST, module: str, attribute: str) -> bool:
    return (
        isinstance(node, ast.Call)
        and isinstance(node.func, ast.Attribute)
        and node.func.attr == attribute
        and isinstance(node.func.value, ast.Name)
        and node.func.value.id == module
    )

def validate_updater_artifact_contract(source: str) -> list[str]:
    try:
        tree = ast.parse(source)
    except SyntaxError:
        return ["updater_artifact.py 源码无法解析"]
    errors = []
    functions = {
        node.name
        for node in ast.walk(tree)
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }
    if "write_stamp" in functions:
        errors.append("updater_artifact 不得公开 write_stamp API")
    assignments = _assignment_map(tree)
    for node in ast.walk(tree):
        if isinstance(node, ast.Store) and _target_names(node) & {"write_stamp"}:
            errors.append("updater_artifact 不得公开 write_stamp 动态别名")
        if isinstance(node, (ast.Assign, ast.AnnAssign)):
            targets = node.targets if isinstance(node, ast.Assign) else [node.target]
            if any("write_stamp" in _target_names(target) for target in targets):
                errors.append("updater_artifact 不得公开 write_stamp 别名")
        if isinstance(node, ast.Call):
            if (
                isinstance(node.func, ast.Attribute)
                and node.func.attr == "add_parser"
            ):
                parser_name = node.args[0] if node.args else next(
                    (keyword.value for keyword in node.keywords if keyword.arg == "name"),
                    None,
                )
                if _string_value(parser_name, assignments) == "stamp":
                    errors.append("updater_artifact 不得公开 stamp CLI")
            if isinstance(node.func, ast.Name) and node.func.id == "setattr":
                setattr_name = node.args[1] if len(node.args) > 1 else next(
                    (keyword.value for keyword in node.keywords if keyword.arg == "name"),
                    None,
                )
                if _string_value(setattr_name, assignments) == "write_stamp":
                    errors.append("updater_artifact 不得公开 write_stamp 动态别名")
    if "_write_stamp" not in functions:
        errors.append("updater_artifact 缺少内部 _write_stamp")
    if "rebuild" not in functions:
        errors.append("updater_artifact 缺少 rebuild 函数")
    if not any(name == "verify" or name.startswith("verify_") for name in functions):
        errors.append("updater_artifact 缺少 verify 函数")
    return errors

def _module_functions(tree: ast.AST) -> dict[str, ast.FunctionDef]:
    return {
        node.name: node
        for node in getattr(tree, "body", [])
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
    }

def _call_name(node: ast.AST) -> str | None:
    if not isinstance(node, ast.Call):
        return None
    if isinstance(node.func, ast.Name):
        return node.func.id
    if isinstance(node.func, ast.Attribute) and isinstance(node.func.value, ast.Name):
        return f"{node.func.value.id}.{node.func.attr}"
    return None

def _is_dead_condition(node: ast.AST) -> bool:
    return isinstance(node, ast.Constant) and node.value in {False, 0, None, ""}

def _reachable_calls(function: ast.FunctionDef) -> set[str]:
    calls: set[str] = set()
    def visit(node: ast.AST) -> None:
        if isinstance(node, ast.If) and _is_dead_condition(node.test):
            return
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.Lambda)):
            return
        name = _call_name(node)
        if name is not None:
            calls.add(name)
        for child in ast.iter_child_nodes(node):
            visit(child)
    for statement in function.body:
        visit(statement)
    return calls

def _reachable_function_calls(
    root: str, functions: dict[str, ast.FunctionDef]
) -> set[str]:
    reachable: set[str] = set()
    pending = [root]
    visited: set[str] = set()
    while pending:
        name = pending.pop()
        if name in visited or name not in functions:
            continue
        visited.add(name)
        calls = _reachable_calls(functions[name])
        reachable.update(calls)
        pending.extend(call for call in calls if call in functions)
    return reachable

def validate_publish_assets_contract(source: str) -> list[str]:
    try:
        tree = ast.parse(source)
    except SyntaxError:
        return ["publish_assets.py 源码无法解析"]
    functions = _module_functions(tree)
    errors = []
    if "commit_staged_assets" not in functions:
        errors.append("publish_assets.py 缺少 module-level commit_staged_assets")
        return errors
    commit_function = functions["commit_staged_assets"]
    parameters = {argument.arg for argument in commit_function.args.args}
    if "replace_func" not in parameters:
        errors.append("commit_staged_assets 缺少 replace_func 注入参数")
    reachable = _reachable_function_calls("commit_staged_assets", functions)
    for name in (
        "select_commit_files",
        "_copy_to_sibling",
        "_backup_existing",
        "_restore",
        "replace_func",
    ):
        if name != "replace_func" and name not in functions:
            errors.append(f"publish_assets.py 缺少 {name} 函数")
        if name not in reachable:
            errors.append(f"commit_staged_assets 缺少可达 {name} 调用")
    for function_name, attribute in (("_restore", "os.replace"), ("_copy_to_sibling", "os.fsync")):
        if function_name not in functions or attribute not in _reachable_calls(functions[function_name]):
            errors.append(f"{function_name} 缺少可达 {attribute} 调用")
    return errors

def _shell_statements(content: str) -> list[list[str]]:
    statements = []
    for line in content.replace("\\\n", " ").splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        tokens = _tokens(line)
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

def _call_indexes(statements: list[list[str]], marker: str) -> list[int]:
    return [
        index
        for index, tokens in enumerate(statements)
        if tokens
        and tokens[0] not in {"def", f"{marker}()"}
        and any(marker in token for token in tokens)
    ]

def _statement_indexes(statements: list[list[str]], marker: str) -> list[int]:
    return [index for index, tokens in enumerate(statements) if any(marker in token for token in tokens)]

def _has_mismatch_exit(statements: list[list[str]], compare_index: int) -> bool:
    for tokens in statements[compare_index + 1 :]:
        if tokens[:2] == ["exit", "1"]:
            return True
        if tokens and tokens[0] in {"fi", "esac"}:
            break
    return False

def _is_preflight_condition(tokens: list[str]) -> bool:
    return tokens[:7] == [
        "if",
        "[",
        "${MACSlim_PUBLISH_PREFLIGHT_ONLY:-}",
        "=",
        "1",
        "];",
        "then",
    ]

def _top_level_preflight_blocks(publish: str) -> list[tuple[int, int, list[str]]]:
    lines = publish.splitlines()
    brace_depth = 0
    blocks = []
    index = 0
    while index < len(lines):
        line = lines[index].strip()
        if brace_depth == 0 and re.fullmatch(
            r"if\s+\[\s+\"\$\{MACSlim_PUBLISH_PREFLIGHT_ONLY:-\}\"\s*=\s*\"1\"\s*\]\s*;\s*then",
            line,
        ):
            body = []
            nested = 0
            end = None
            cursor = index + 1
            while cursor < len(lines):
                current = lines[cursor].strip()
                is_close = bool(re.match(r"^(?:fi|esac|done)\b", current))
                if current and not current.startswith("#") and not (is_close and nested == 0):
                    body.append(current)
                if re.match(r"^(?:if|for|while|until|case)\b", current):
                    nested += 1
                elif is_close:
                    if nested:
                        nested -= 1
                    else:
                        end = cursor
                        break
                cursor += 1
            if end is not None:
                blocks.append((index, end, body))
                index = end + 1
                continue
        if re.match(r"^[A-Za-z_][A-Za-z0-9_]*\s*\(\)\s*\{$", line):
            brace_depth += 1
        elif line == "}":
            brace_depth = max(0, brace_depth - 1)
        index += 1
    return blocks

def _has_unquoted_redirect(line: str) -> bool:
    quote = None
    escaped = False
    for character in line:
        if escaped:
            escaped = False
            continue
        if character == "\\" and quote != "'":
            escaped = True
            continue
        if character in {"'", '"'}:
            quote = None if quote == character else character if quote is None else quote
        elif quote is None and character in {"<", ">"}:
            return True
    return False

def _preflight_body_errors(body: list[str]) -> tuple[list[str], int]:
    errors = []
    exit_count = 0
    for line in body:
        if _has_unquoted_redirect(line):
            errors.append("publish-update.sh preflight 分支不得包含重定向")
            continue
        if re.match(r"^(?:if|then|elif|else|fi|for|while|until|case|esac|done)\b", line):
            errors.append("publish-update.sh preflight 分支不得包含嵌套控制流")
        tokens = _tokens(line)
        if not tokens:
            continue
        if tokens == ["exit", "0"]:
            exit_count += 1
            continue
        if (
            len(tokens) == 2
            and tokens[0] == "echo"
            and not any(character in tokens[1] for character in ("`", ";", "&", "|", "$("))
        ):
            continue
        errors.append("publish-update.sh preflight 分支只允许 echo 和 exit 0")
    return errors, exit_count

def _preflight_indexes(statements: list[list[str]]) -> list[int]:
    return [index for index, tokens in enumerate(statements) if _is_preflight_condition(tokens)]

def _has_zero_exit(statements: list[list[str]], start: int) -> bool:
    for tokens in statements[start + 1 :]:
        if tokens[:2] == ["exit", "0"]:
            return True
        if tokens and tokens[0] == "fi":
            break
    return False

def _config_reader_path(node: ast.AST) -> str | None:
    if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == "open":
        if node.args and isinstance(node.args[0], ast.Constant):
            return node.args[0].value if isinstance(node.args[0].value, str) else None
    if (
        isinstance(node, ast.Call)
        and isinstance(node.func, ast.Attribute)
        and node.func.attr == "read"
    ):
        return _config_reader_path(node.func.value)
    return None

def _config_code_is_real(code: str) -> bool:
    try:
        tree = ast.parse(code)
    except SyntaxError:
        return False
    prints = [
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == "print"
    ]
    if len(prints) != 1 or not prints[0].args:
        return False
    expression = prints[0].args[0]
    if not isinstance(expression, ast.Subscript) or _string_value(expression.slice, {}) != "version":
        return False
    reader = expression.value
    if not (
        isinstance(reader, ast.Call)
        and isinstance(reader.func, ast.Attribute)
        and isinstance(reader.func.value, ast.Name)
        and reader.func.value.id == "json"
        and reader.func.attr in {"load", "loads"}
        and reader.args
    ):
        return False
    return _config_reader_path(reader.args[0]) == "src-tauri/tauri.conf.json"

def _config_version_is_real(publish: str) -> bool:
    assignments = []
    for line in publish.replace("\\\n", " ").splitlines():
        if line.lstrip().startswith("#"):
            continue
        if re.match(r"^\s*(?:export\s+)?CONFIGURED_VERSION\s*=", line):
            assignments.append(line)
    if len(assignments) != 1:
        return False
    match = _CONFIG_VERSION_RE.fullmatch(assignments[0])
    return match is not None and _config_code_is_real(match.group("code"))

def _is_target_override_line(line: str) -> bool:
    if line.lstrip().startswith("#") or not re.search(r"\b(?:RUST_TARGET|ARCH)\b", line):
        return False
    if re.search(r"\b(?:eval|source)\b", line):
        return True
    if re.search(r"\bprintf\b[^\n]*\s-v\s+(?:RUST_TARGET|ARCH)\b", line):
        return True
    if re.search(r"\bread\b[^\n]*\b(?:RUST_TARGET|ARCH)\b", line):
        return True
    return bool(
        re.match(
            r"^\s*(?:(?:declare|local|export|readonly|typeset)\s+(?:-\w+\s+)*)?"
            r"(?:RUST_TARGET|ARCH)\s*(?:\+=|=)",
            line,
        )
    )

def _target_mapping_errors(publish: str) -> list[str]:
    case_match = re.search(r'(?m)^(?!\s*#)\s*case\s+"\$TARGET"\s+in\s*$', publish)
    if case_match is None:
        return ["publish-update.sh 缺少 target 映射"]
    end_match = re.search(r"(?m)^(?!\s*#)\s*esac\s*$", publish[case_match.end() :])
    if end_match is None:
        return ["publish-update.sh target case 缺少 esac"]
    case_end = case_match.end() + end_match.start()
    case_text = publish[case_match.end() : case_end]
    assignments: dict[str, dict[str, str]] = {}
    branch = ""
    errors = []
    for raw_line in case_text.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if line.endswith(")") and " " not in line:
            branch = line[:-1]
            assignments.setdefault(branch, {})
            continue
        if _is_target_override_line(line):
            if not re.match(r"^(?:RUST_TARGET|ARCH)\s*=", line):
                errors.append("publish-update.sh case 分支不得额外覆盖 RUST_TARGET/ARCH")
                continue
            key, value = line.split("=", 1)
            assignments[branch][key.strip()] = value.strip().strip('"')
    for target, values in EXPECTED_TARGETS.items():
        actual = assignments.get(target, {})
        if actual.get("RUST_TARGET") != values[0] or actual.get("ARCH") != values[1]:
            errors.append(f"publish-update.sh {target} target 映射无效")
    for line in publish[case_end:].splitlines():
        if _is_target_override_line(line):
            errors.append("publish-update.sh case 后不得覆盖 RUST_TARGET/ARCH")
    return errors

def _publish_boundaries(statements: list[list[str]], preflight_indexes: list[int]) -> dict[str, list[int]]:
    return {
        "preflight": preflight_indexes,
        "删除旧签名或 sidecar": _command_indices(statements, ("rm",), required=("-f",)),
        "mkdir": _command_indices(statements, ("mkdir",), required=("-p",)),
        "复制 updater archive": _command_indices(statements, ("cp",), required=("$UPDATER_SRC", "$UPDATER_PATH")),
        "复制 build stamp": _command_indices(statements, ("cp",), required=("$UPDATER_SRC_STAMP_PATH", "$UPDATER_STAMP_PATH")),
        "signer": _command_indices(statements, ("bun", "tauri", "signer", "sign")) + _call_indexes(statements, "sign_staged_assets"),
        "Python mkdir": _statement_indexes(statements, "path.parent.mkdir"),
        "Python write_text": _statement_indexes(statements, "path.write_text"),
        "manifest": sorted(set(_call_indexes(statements, "write_manifest")) | set(_statement_indexes(statements, "path.write_text"))),
        "commit": _command_indices(statements, ("python3", "scripts/publish_assets.py", "commit"), required=("--staging-root", "$STAGING_ROOT", "--live-root", ".", "--version", "$VERSION")),
    }
def _publish_source_contract(publish: str, statements: list[list[str]]) -> tuple[list[str], list[str]]:
    errors = []
    if re.search(r"PUBLISH_VERSION\s*=\s*[\"']9\.9\.9[\"']", publish):
        errors.append("updater 发布脚本不得固定使用 9.9.9 测试版本")
    config_valid = _config_version_is_real(publish)
    compare_indexes = _statement_indexes(statements, "$CONFIGURED_VERSION")
    if not config_valid:
        errors.append("publish-update.sh 必须真实从 tauri.conf.json 读取发布版本")
    if not compare_indexes:
        errors.append("publish-update.sh 必须比较 VERSION 与配置版本")
    elif not _has_mismatch_exit(statements, compare_indexes[0]):
        errors.append("publish-update.sh 版本不一致分支必须非零退出")
    source_verify_indexes = _command_indices(
        statements, ("python3", "scripts/updater_artifact.py", "verify"),
        required=("--archive", "$UPDATER_SRC", "--target", "$RUST_TARGET"))
    if len(source_verify_indexes) != 1:
        errors.append("publish-update.sh 必须且只能执行一次源 updater archive verify")
    order_errors = []
    boundaries = _publish_boundaries(statements, _preflight_indexes(statements))
    if len(source_verify_indexes) == 1:
        verify_index = source_verify_indexes[0]
        for label, indexes in boundaries.items():
            if indexes and min(indexes) <= verify_index:
                order_errors.append(f"publish-update.sh 必须在 {label} 前验证 updater archive")
        if compare_indexes and max(compare_indexes) > verify_index:
            order_errors.append("publish-update.sh 必须先比较发布版本再验证 updater archive")
    return errors, order_errors
def _publish_staging_contract(statements: list[list[str]]) -> tuple[list[str], list[str]]:
    errors = []
    verify_existing_indexes = _call_indexes(statements, "verify_existing_assets")
    if len(verify_existing_indexes) != 2:
        errors.append("publish-update.sh 必须在 preflight 和 staging 各验证一次既有资产")
    staging_indexes = _command_indices(statements, ("mktemp", "-d"))
    copy_existing_indexes = _call_indexes(statements, "copy_existing_assets")
    if len(copy_existing_indexes) != 1:
        errors.append("publish-update.sh 必须调用一次 staging asset copy")
    preflight_indexes = _preflight_indexes(statements)
    boundaries = _publish_boundaries(statements, preflight_indexes)
    if not boundaries["commit"]:
        errors.append("publish-update.sh 缺少事务 commit helper")
    order_errors = []
    if preflight_indexes and staging_indexes and min(preflight_indexes) > min(staging_indexes):
        order_errors.append("publish-update.sh preflight 分支必须位于 staging 之前")
    if len(verify_existing_indexes) == 2:
        first_existing, second_existing = verify_existing_indexes
        copy_indexes = boundaries["复制 updater archive"] + boundaries["复制 build stamp"]
        if staging_indexes and first_existing > min(staging_indexes):
            order_errors.append("第一次 verify_existing_assets 必须在 staging 创建前执行")
        if copy_indexes and second_existing < min(copy_indexes):
            order_errors.append("staging verify_existing_assets 必须在复制后执行")
        if copy_existing_indexes and second_existing < min(copy_existing_indexes):
            order_errors.append("staging verify_existing_assets 必须在 copy_existing_assets 后执行")
        for label in ("signer", "manifest", "commit"):
            indexes = boundaries[label]
            if indexes and second_existing > min(indexes):
                order_errors.append(f"verify_existing_assets 必须在 {label} 前执行")
    return errors, order_errors
def _publish_preflight_contract(publish: str, statements: list[list[str]]) -> list[str]:
    errors = []
    preflight_indexes = _preflight_indexes(statements)
    top_level_preflight = _top_level_preflight_blocks(publish)
    if len(top_level_preflight) != 1:
        errors.append("publish-update.sh 必须有唯一 top-level preflight 分支")
    else:
        body_errors, exit_count = _preflight_body_errors(top_level_preflight[0][2])
        errors.extend(body_errors)
        if exit_count != 1:
            errors.append("publish-update.sh preflight 分支必须唯一执行 exit 0")
    if len(preflight_indexes) != 1:
        errors.append("publish-update.sh 缺少唯一真实 preflight 分支")
    elif not any(_has_zero_exit(statements, index) for index in preflight_indexes):
        errors.append("publish-update.sh preflight 分支必须成功退出")
    return errors
def _publish_manifest_commit_contract(statements: list[list[str]]) -> list[str]:
    errors = []
    boundaries = _publish_boundaries(statements, _preflight_indexes(statements))
    if boundaries["commit"] and boundaries["manifest"] and min(boundaries["commit"]) <= max(boundaries["manifest"]):
        errors.append("publish-update.sh commit 必须在 manifest 写入之后")
    return errors
def _publish_marker_contract(statements: list[list[str]]) -> list[str]:
    errors = []
    for marker in (
        "verify_existing_assets", "UPDATER_SRC_STAMP_PATH", "UPDATER_STAMP_PATH",
        "sign_staged_assets", "STAGING_ROOT", "STAGING_DOWNLOAD_DIR", "mktemp",
        "subprocess.run", "verified.returncode", "sidecar",
    ):
        if not _statement_indexes(statements, marker):
            errors.append(f"publish-update.sh 缺少 {marker} 契约")
    if not _command_indices(statements, ("trap", "cleanup", "EXIT")):
        errors.append("publish-update.sh 缺少 trap cleanup EXIT 契约")
    return errors
def _publish_target_contract(publish: str) -> list[str]:
    return _target_mapping_errors(publish)
def validate_publish_gate_contract(publish: str) -> list[str]:
    errors = []
    statements = _shell_statements(publish)
    source_errors, source_order_errors = _publish_source_contract(publish, statements)
    errors.extend(source_errors)
    staging_errors, staging_order_errors = _publish_staging_contract(statements)
    errors.extend(staging_errors)
    errors.extend(_publish_preflight_contract(publish, statements))
    errors.extend(source_order_errors)
    errors.extend(staging_order_errors)
    errors.extend(_publish_manifest_commit_contract(statements))
    errors.extend(_publish_marker_contract(statements))
    errors.extend(_publish_target_contract(publish))
    return errors
