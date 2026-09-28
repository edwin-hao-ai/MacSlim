"""prepare → 用户确认 → execute 的顺序门禁。

后端 `PreparedOperation` 自带 `summary` / `estimated_bytes` / `expires_at_ms`，
只有把它们渲染给用户、拿到一次显式点击之后才 execute，AGENTS.md §4.1 的
「不可逆操作诚实表述」和 §4.5 的「>10GB 二次确认」才真正成立。

本文件锁住两件事：
1. 六个破坏性界面都必须复用 `OperationConfirm`；
2. 任何单个函数体内不得同时出现 `prepareOperation` 与 `executeOperation`
   —— 否则「准备完立刻执行」会重新长出来。
"""
from __future__ import annotations

import unittest

try: from scripts.tests.operation_contract_support import MODULE, frontend_sources
except ModuleNotFoundError: from operation_contract_support import MODULE, frontend_sources

CACHE_VIEW = "src/views/CacheView.tsx"


def sources() -> dict[str, str]:
    return frontend_sources()


def with_cache_view(mutated: str) -> dict[str, str]:
    files = sources()
    files[CACHE_VIEW] = mutated
    return files


class ConfirmationFlowTests(unittest.TestCase):
    def errors(self, files: dict[str, str]) -> list[str]:
        return MODULE.validate_frontend_confirmation_flow(files)

    def test_repository_requires_a_confirmation_before_execute(self):
        self.assertEqual(self.errors(sources()), [])

    def test_rejects_a_view_that_executes_in_the_same_function_as_prepare(self):
        original = """      setPending(
        await prepareOperation(cacheOperation(current.snapshot_id, keys)),
      );"""
        mutated_source = sources()[CACHE_VIEW]
        self.assertIn(original, mutated_source)
        mutated = mutated_source.replace(
            original,
            """      const prepared = await prepareOperation(
        cacheOperation(current.snapshot_id, keys),
      );
      await executeOperation(prepared.operation_id);""",
        )
        errors = self.errors(with_cache_view(mutated))
        joined = " ".join(errors)
        self.assertIn("prepare", joined)
        self.assertIn("requestClean", joined)

    def test_rejects_a_view_that_drops_the_shared_confirmation_component(self):
        mutated = sources()[CACHE_VIEW].replace("OperationConfirm", "LegacyDialog")
        errors = self.errors(with_cache_view(mutated))
        self.assertTrue(
            any("OperationConfirm" in error for error in errors), errors
        )

    def test_names_every_destructive_surface_it_checks(self):
        files = sources()
        for name in MODULE._surface.CONFIRM_REQUIRED_SOURCES:
            with self.subTest(name=name):
                files[name] = "const x = 1;\n"
                self.assertTrue(
                    any(name in error for error in self.errors(files)),
                    f"{name} 缺失时门禁没有报错",
                )

    def test_rejects_a_missing_destructive_surface(self):
        files = sources()
        files["src/components/DockerSection.tsx"] = ""
        errors = self.errors(files)
        self.assertTrue(
            any("DockerSection.tsx" in error for error in errors), errors
        )

    def test_accepts_a_view_that_splits_prepare_and_execute_across_functions(self):
        base = sources()[CACHE_VIEW]
        self.assertIn("const requestClean = async () => {", base)
        self.assertIn("const confirmClean = async () => {", base)
        self.assertNotIn(
            "prepareOperation", self._body(base, "confirmClean")
        )
        self.assertIn("executeOperation", self._body(base, "confirmClean"))

    def _body(self, source: str, name: str) -> str:
        import re

        masked = MODULE.mask_code(source, MODULE.TS_QUOTES)
        match = re.search(r"\b(?:const|let)\s+" + re.escape(name) + r"\b", masked)
        self.assertIsNotNone(match, f"找不到 {name}")
        brace = masked.find("{", match.end())
        return masked[brace : brace + 2000]


if __name__ == "__main__":
    unittest.main()
