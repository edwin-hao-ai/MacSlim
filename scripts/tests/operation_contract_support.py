"""`test_operation_contract.py` 系列共用的加载与断言辅助。

不参与 `unittest discover`（文件名不匹配 `test_*.py`），只为三份契约测试
提供同一个模块实例与同一批内存字符串 helper，确保「负例只改内存字符串、
从不写回仓库」这件事在代码层面只有一个实现。
"""
from __future__ import annotations

import importlib.util
import pathlib
import sys

SCRIPTS_DIR = pathlib.Path(__file__).resolve().parents[1]
ROOT = SCRIPTS_DIR.parent
MODULE_PATH = SCRIPTS_DIR / "operation_contract.py"
SPEC = importlib.util.spec_from_file_location("operation_contract", MODULE_PATH)
assert SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)

REPOSITORY_FILES = MODULE.read_contract_sources(ROOT)
CACHE_VARIANT = '  | { type: "cache"; snapshot_id: string; item_keys: string[] }'
CLI_SESSION_BLOCK = (
    "    let session = CliSession::open(storage)?;\n"
    "    let whitelist = session.whitelist();\n"
    "    let mut observer = build_observer(Arc::clone(&session.storage));"
)
RAW_WRAPPER_CALL = '\nexport const legacy = () => killProcesses([1], ["x"]);\n'
UNCLOSED_APOSTROPHE = "<p>don't show raw commands</p>\n"
BALANCED_APOSTROPHE = "const NOTE = 'don\\'t show raw commands';\n"
APPENDED_RUST_QUOTE = '\nconst NOTE: &str = "oops;\n'


def add_handler_command(source: str, command: str) -> str:
    return source.replace(
        "            check_app_running,",
        f"            {command},\n            check_app_running,",
    )


def add_handler_entry(source: str, entry: str) -> str:
    return source.replace("        ])\n        .run(", f"            {entry},\n        ])\n        .run(")


def mark_anything(errors: list[str], *needles: str) -> bool:
    joined = " ".join(errors)
    return any(needle in joined for needle in needles)


def frontend_sources() -> dict[str, str]:
    return MODULE.frontend_sources(ROOT)


def package_json() -> dict:
    return __import__("json").loads((ROOT / "package.json").read_text(encoding="utf-8"))


def broken_quote_frontend() -> dict[str, str]:
    sources = frontend_sources()
    sources["src/views/CacheView.tsx"] = (
        UNCLOSED_APOSTROPHE
        + sources["src/views/CacheView.tsx"]
        + RAW_WRAPPER_CALL
    )
    return sources


def broken_quote_files() -> dict[str, str]:
    files = dict(REPOSITORY_FILES)
    files["src/views/CacheView.tsx"] = (
        UNCLOSED_APOSTROPHE
        + files["src/views/CacheView.tsx"]
        + RAW_WRAPPER_CALL
    )
    return files


def parse_failure_needle(name: str) -> tuple[str, ...]:
    return (name, "结构解析失败", "未闭合")
