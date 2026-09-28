"""斜线网格「可清理空间」标记的静态门禁。

判定逻辑（`src/lib/format.ts` 的 `isCategoryReclaimable`）与渲染
（`src/views/CacheView.tsx` 的分类徽章）由 vitest 守住；**样式本体**
放在 Python 侧守，原因是 TS 侧读不到未经 Tailwind 处理的 `styles.css`
原文 —— `?raw` 导入拿到的是空串，断言会静默空转成「全过」。

三条门禁对应三条硬约束：
1. 纯 CSS（`repeating-linear-gradient`），不引图片资源、不加新依赖；
2. 深浅两套配色都给了（深色下黑色描线会看不见），且**不含任何动画**
   —— 静态渐变在 `prefers-reduced-motion: reduce` 下表现一致；
3. 样式里不含任何分类名单：能不能标由 `CacheItem.safety` 决定，
   名单一漂移就会把纹理标到不该标的地方。
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STYLES = ROOT / "src" / "styles.css"
FORMAT_TS = ROOT / "src" / "lib" / "format.ts"
CACHE_VIEW = ROOT / "src" / "views" / "CacheView.tsx"

#: 11 个 `CacheCategory` 的 wire 名。出现在样式里就说明判定被硬编码进 CSS 了。
CATEGORIES = (
    "npm",
    "pnpm",
    "yarn",
    "docker",
    "homebrew",
    "xcode",
    "cocoapods",
    "cargo",
    "pip",
    "go",
    "system",
)

HATCH_SELECTOR = ".reclaimable-hatch"


def _strip_css_comments(text: str) -> str:
    return re.sub(r"/\*[\s\S]*?\*/", "", text)


def _hatch_declarations(styles: str) -> str:
    """抽出 ``.reclaimable-hatch`` 的两段声明（浅色 + 深色），并剥掉注释。

    不剥注释的话，注释里出现 ``transition`` 这类词会让下面的断言误报。
    """
    start = styles.find(HATCH_SELECTOR)
    assert start >= 0, "styles.css 里必须有 .reclaimable-hatch"
    window = styles[start : start + 900]
    end = window.find("@media (prefers-reduced-motion")
    if end > 0:
        window = window[:end]
    return _strip_css_comments(window).strip()


class ReclaimableHatchStyleTest(unittest.TestCase):
    def setUp(self) -> None:
        self.styles = STYLES.read_text(encoding="utf-8")
        self.declarations = _hatch_declarations(self.styles)

    def test_selector_exists_and_is_pure_css(self) -> None:
        self.assertIn(HATCH_SELECTOR, self.styles)
        self.assertIn("repeating-linear-gradient", self.declarations)
        # 不许退化成 url() / 图片 / base64
        self.assertNotIn("url(", self.declarations)
        self.assertNotIn("data:", self.declarations)
        self.assertNotIn("image-set(", self.declarations)

    def test_dark_mode_variant_is_present(self) -> None:
        # 深色下黑色描线会完全看不见，所以必须给深色单独一套
        self.assertIn("prefers-color-scheme: dark", self.declarations)
        self.assertGreaterEqual(
            self.declarations.count("repeating-linear-gradient"),
            2,
            "浅色 + 深色两段都要有渐变",
        )

    def test_is_static_so_reduced_motion_needs_no_media_query(self) -> None:
        for token in ("animation", "transition", "@keyframes"):
            self.assertNotIn(
                token,
                self.declarations,
                msg=f"斜线纹理是静态渐变，不该出现 {token}",
            )

    def test_style_contains_no_category_roster(self) -> None:
        for category in CATEGORIES:
            self.assertNotIn(
                category,
                self.declarations,
                msg=f"样式里出现分类名 {category}：判定必须留在 format.ts 读 safety",
            )

    def test_new_dependency_was_not_introduced(self) -> None:
        package = (ROOT / "package.json").read_text(encoding="utf-8")
        self.assertNotIn("repeating", package)
        self.assertNotIn("hatch", package)


class ReclaimableHatchDecisionTest(unittest.TestCase):
    """判定与渲染两侧的静态反证。"""

    def setUp(self) -> None:
        self.format_ts = FORMAT_TS.read_text(encoding="utf-8")
        self.cache_view = CACHE_VIEW.read_text(encoding="utf-8")

    def test_decision_reads_safety_not_a_roster(self) -> None:
        self.assertIn("isCategoryReclaimable", self.format_ts)
        # 判定体里逐字比的是 `safety`，出现 `default_select` 就说明语义串了
        body = self.format_ts.split("export function isCategoryReclaimable", 1)[1]
        body = body.split("\n}", 1)[0]
        self.assertIn('safety !== "medium"', body)
        self.assertNotIn("default_select", body)
        for category in CATEGORIES:
            self.assertNotIn(
                f'"{category}"',
                body,
                msg=f"判定里出现分类名 {category}",
            )

    def test_badge_carries_the_class_and_a_testable_hook(self) -> None:
        self.assertIn("isCategoryReclaimable", self.cache_view)
        self.assertIn('"reclaimable-hatch": group.reclaimable', self.cache_view)
        # 渲染测试靠 data-reclaimable 区分「加了类」和「碰巧类名对」
        self.assertIn("data-reclaimable", self.cache_view)


if __name__ == "__main__":
    unittest.main()
