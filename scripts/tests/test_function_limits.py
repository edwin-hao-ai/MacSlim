from __future__ import annotations

import ast
import pathlib
import re
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))
try: import source_structure as _structure
except ModuleNotFoundError: import source_structure as _structure

PYTHON_PATHS = sorted(SCRIPTS.glob("*.py")) + sorted(
    (SCRIPTS / "tests").glob("test_*.py")
)
SHELL_PATHS = (
    SCRIPTS / "release.sh",
    SCRIPTS / "sign.sh",
    SCRIPTS / "publish-update.sh",
)
FUNCTION_RE = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)\s*\(\s*\)\s*\{\s*$")
FUNCTION_LIMIT = 50
FILE_HARD_LIMIT = 800
RUST_BROKER_FILES = (
    "operation_commands.rs",
    "operation_executor.rs",
    "operation_registry.rs",
    "operation_types.rs",
    "operations.rs",
    "operations_prepare.rs",
    "cli_operations.rs",
)
TYPESCRIPT_BROKER_FILES = (
    "src/components/OperationConfirm.tsx",
    "src/components/DockerSection.tsx",
    "src/components/ProcessList.tsx",
    "src/lib/tauri.ts",
)
TS_ARROW_RE = _structure.TS_ARROW_RE


def rust_paths() -> list[pathlib.Path]:
    return [pathlib.Path(ROOT / "src-tauri" / "src" / name) for name in RUST_BROKER_FILES]


def typescript_paths() -> list[pathlib.Path]:
    return [ROOT / name for name in TYPESCRIPT_BROKER_FILES]


def _mask_shell_line(line: str, state: dict[str, object]) -> str:
    output = []
    index = 0
    while index < len(line):
        character = line[index]
        if state["escaped"]:
            state["escaped"] = False
            output.append(" ")
            index += 1
            continue
        if state["quote"] is not None:
            output.append(" ")
            if character == "\\" and state["quote"] == '"':
                state["escaped"] = True
            elif character == state["quote"]:
                state["quote"] = None
            index += 1
            continue
        if character in {"'", '"'}:
            state["quote"] = character
            output.append(" ")
            index += 1
            continue
        if character == "\\":
            state["escaped"] = True
            output.append(" ")
            index += 1
            continue
        if character == "$" and line[index + 1 : index + 2] == "{":
            state["expansion_depth"] += 1
            output.extend("  ")
            index += 2
            continue
        if state["expansion_depth"]:
            if character == "}":
                state["expansion_depth"] -= 1
            output.append(" ")
            index += 1
            continue
        if character == "#" and (not output or output[-1].isspace()):
            output.extend(" " * (len(line) - index))
            break
        output.append(character)
        index += 1
    state["escaped"] = False
    return "".join(output)


def _shell_functions(source: str) -> list[tuple[str, int, int]]:
    state = {"quote": None, "escaped": False, "expansion_depth": 0}
    functions = []
    current_name = None
    start_line = 0
    brace_depth = 0
    for line_number, line in enumerate(source.splitlines(), start=1):
        masked = _mask_shell_line(line, state)
        if brace_depth == 0:
            match = FUNCTION_RE.fullmatch(masked)
            if match:
                current_name = match.group(1)
                start_line = line_number
                brace_depth = 1
                continue
        for character in masked:
            if character == "{":
                brace_depth += 1
            elif character == "}" and brace_depth:
                brace_depth -= 1
        if brace_depth == 0 and current_name:
            functions.append((current_name, start_line, line_number))
            current_name = None
    if brace_depth:
        raise ValueError("存在未闭合的 shell 函数")
    return functions


class FunctionLimitTests(unittest.TestCase):
    def test_plan_python_functions_are_shorter_than_50_lines(self) -> None:
        violations = []
        for path in PYTHON_PATHS:
            tree = ast.parse(path.read_text(encoding="utf-8"))
            for node in ast.walk(tree):
                if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    continue
                length = node.end_lineno - node.lineno + 1
                if length >= FUNCTION_LIMIT:
                    violations.append(f"{path}:{node.lineno} {node.name} ({length} 行)")
        self.assertEqual(violations, [], "\n".join(violations))

    def test_release_shell_functions_are_shorter_than_50_lines(self) -> None:
        violations = []
        for path in SHELL_PATHS:
            for name, start, end in _shell_functions(
                path.read_text(encoding="utf-8")
            ):
                length = end - start + 1
                if length >= FUNCTION_LIMIT:
                    violations.append(f"{path}:{start} {name} ({length} 行)")
        self.assertEqual(violations, [], "\n".join(violations))

    def test_shell_parser_ignores_non_code_braces(self) -> None:
        source = r'''# fake() {
single='fake() {'
double="fake() {"
usage="用法: $0 <version> \"fake() {\" [arm]" >&2
escaped=fake\(\) \{
expanded="${value:-${nested}}"
expanded_brace=${value:-{}
real() {
  value="${value:-${nested}}"
  :
}
'''
        self.assertEqual(
            _shell_functions(source),
            [("real", 8, 11)],
        )

    def test_broker_rust_functions_are_shorter_than_50_lines(self) -> None:
        violations = []
        for path in rust_paths():
            source = path.read_text(encoding="utf-8")
            for name, (start, end) in sorted(
                _structure.rust_function_spans(source).items()
            ):
                length = source.count("\n", start, end) + 1
                if length >= FUNCTION_LIMIT:
                    violations.append(f"{path.name}:{name} ({length} 行)")
        self.assertEqual(violations, [], "\n".join(violations))

    def test_broker_typescript_functions_are_shorter_than_50_lines(self) -> None:
        violations = []
        for path in typescript_paths():
            source = path.read_text(encoding="utf-8")
            components = _structure.typescript_component_names(source)
            for name, (start, end) in sorted(
                _structure.typescript_arrow_function_spans(source).items()
            ):
                if name in components:
                    continue
                length = source.count("\n", start, end) + 1
                if length >= FUNCTION_LIMIT:
                    violations.append(f"{path.name}:{name} ({length} 行)")
        self.assertEqual(violations, [], "\n".join(violations))

    def test_broker_files_stay_under_the_800_line_hard_limit(self) -> None:
        violations = []
        for path in [*rust_paths(), *typescript_paths()]:
            length = len(path.read_text(encoding="utf-8").splitlines())
            if length > FILE_HARD_LIMIT:
                violations.append(f"{path.name} ({length} 行)")
        self.assertEqual(violations, [], "\n".join(violations))

    def test_the_broker_file_lists_are_real_files(self) -> None:
        self.assertGreaterEqual(len(RUST_BROKER_FILES), 5)
        self.assertGreaterEqual(len(TYPESCRIPT_BROKER_FILES), 3)
        for path in [*rust_paths(), *typescript_paths()]:
            self.assertTrue(path.is_file(), f"{path} 不存在")


if __name__ == "__main__":
    unittest.main()
