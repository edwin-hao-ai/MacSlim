#!/usr/bin/env python3
"""给截屏叠上营销文案层。

## 为什么需要自己叠

App Store 页面自己会渲染标题和说明，但那是**一张营销图**的效果，不是 macOS
截屏那种「原样展示应用窗口」的效果。而 Apple 不允许把营销文字烧进 macOS
截图再当作展示图 —— 于是要一条故事线，就只能自己在 PNG 上叠。

## 为什么叠在顶部而不是底部

底部是这一产品的动作区：「清理 10.5 GB (1)」主按钮 + 「重新扫描」+ 不可撤销
提示。盖住它等于盖住故事的高潮。顶部相对安静，被盖掉的只是页面自己的标题
那一行 —— 而标题栏下面紧跟着我们要讲的话，信息上是冗余的。

侧栏的品牌字样（MacSlim + 图标）落在 x<560px，文案从 x=640px 起，所以品牌
仍然可见 —— 叠字不该把「这是谁做的」也盖掉。

## 判据必须来自文案本身

每张图的文案都写死在一份 JSON 里（`scripts/screenshot_story.zh-Hans.json` /
`.en-US.json`），不在脚本里硬编码。这样：

- 改文案不需要碰代码
- 可以用门禁断言「每张图都有文案」「中英两套的 key 完全一致」
- 漏了某张图会**直接失败**，而不是悄悄传一张没故事的图上 App Store

## 用法

```bash
python3 scripts/annotate_screenshots.py --dir /tmp/ms-shot/final --locale zh-Hans
python3 scripts/annotate_screenshots.py --dir /tmp/ms-shot/en --locale en-US
```

输出写到 `<dir>/../<name>-shot/<文件名>`，原图不动 —— 原始截屏是无价的事实，
叠坏了要能重做。
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
STORY_FILES = {
    "zh-Hans": ROOT / "scripts/screenshot_story.zh-Hans.json",
    "en-US": ROOT / "scripts/screenshot_story.en-US.json",
}

# 截图是 1440x900 点 @2x = 2880x1800 像素。文案尺寸按像素给。
CANVAS = (2880, 1800)
# 文案栏只覆盖**内容区**，侧栏完整保留。
#
# 试过整幅盖满：文字是清楚了，但侧栏的 MacSlim 品牌和「智能扫描 / 进程管理」
# 两项被盖掉，导航列表从「缓存清理」才开始 —— 一眼看着像应用坏了。这比多盖
# 一点像素的代价大得多。
GUTTER = 560
TEXT_PAD = 72
MARGIN_TOP = 72
HEADLINE_MAX = CANVAS[0] - GUTTER - TEXT_PAD - 120
FADE = 56  # 实心栏下方的柔化高度

# 配色跟着应用主题走。应用是浅色的（截屏实测），所以文案栏也用浅色 ——
# 深色条贴在浅色界面 上像贴歪了的贴纸，而浅色栏读起来像是应用自己的标题栏。
# 截屏脚本会强制浅色外观，两边因此不会脱节。
BAR_TOP = (247, 248, 250)
INK = (11, 18, 32)
INK_SOFT = (71, 82, 100)

FONTS = {
    # Hiragino Sans GB 有 W3/W6 两档，索引 2 才是 W6（粗）
    "zh-Hans": "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "en-US": "/System/Library/Fonts/SFNS.ttf",
}
FONT_FALLBACK = "/System/Library/Fonts/HelveticaNeue.ttc"


def fail(message: str) -> None:
    print(f"错误: {message}")
    raise SystemExit(1)


def load_font(locale: str, size: int, bold: bool) -> ImageFont.FreeTypeFont:
    path = FONTS.get(locale, FONT_FALLBACK)
    index = 2 if (locale == "zh-Hans" and bold) else 0
    try:
        return ImageFont.truetype(path, size, index=index)
    except OSError:
        return ImageFont.truetype(FONT_FALLBACK, size, index=0)


def fit_headline(text: str, locale: str, start: int, floor: int = 52) -> ImageFont.FreeTypeFont:
    """标题太长就逐级缩小，直到单行放得下。

    不缩字的后果很具体：中文标题一长就压到画布右沿之外被裁掉（这个坑在
    `whitespace-nowrap` 那次已经踩过一次），而英文标题更容易长。
    """
    size = start
    while size > floor:
        font = load_font(locale, size, bold=True)
        if font.getbbox(text)[2] <= HEADLINE_MAX:
            return font
        size -= 4
    return load_font(locale, floor, bold=True)


def scrim(band: int) -> Image.Image:
    """顶部**实心**文案栏 + 下方的柔化过渡。

    第一版做的是「整段渐变压暗」，看起来更轻，但有两个实打实的问题：

    1. 渐变是半透明的，下面的应用文字（卡片标题、说明）照样透出来，和我们的
       白字**叠在一起**，两边都不可读。
    2. 副标题正好落在应用卡片标题那一行上。

    所以必须是实心的：文案栏就是用来盖住那一行页面标题的（它与我们要讲的
    话说的是同一件事，信息冗余）。但实心与实心之间不能有硬边，否则像贴纸 ——
    所以只在**下沿**留一段渐变淡出，让界面从栏下「长出来」。
    """
    width = CANVAS[0]
    total = band + FADE
    layer = Image.new("RGBA", (width, total), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    draw.rectangle([(GUTTER, 0), (width, band)], fill=(*BAR_TOP, 255))
    for y in range(band, total):
        ratio = (y - band) / FADE
        alpha = int(255 * (1 - ratio))
        draw.line([(GUTTER, y), (width, y)], fill=(*BAR_TOP, alpha))
    return layer


def annotate(source: Path, target: Path, copy: dict, locale: str) -> None:
    image = Image.open(source).convert("RGBA")
    if image.size != CANVAS:
        fail(f"{source.name} 是 {image.size[0]}x{image.size[1]}，期望 {CANVAS[0]}x{CANVAS[1]}")

    headline_font = fit_headline(copy["headline"], locale, start=84)
    sub_font = load_font(locale, 42, bold=False)

    gap = 22
    head_box = headline_font.getbbox(copy["headline"])
    head_h = head_box[3] - head_box[1]
    sub_text = copy.get("subline")
    sub_h = 0
    if sub_text:
        sub_h = sub_font.getbbox(sub_text)[3] - sub_font.getbbox(sub_text)[1]

    band = MARGIN_TOP + head_h + (gap + sub_h if sub_text else 0) + 40

    composed = image.copy()
    composed.alpha_composite(scrim(band), (0, 0))

    draw = ImageDraw.Draw(composed)
    x = GUTTER + TEXT_PAD
    y = MARGIN_TOP - head_box[1]
    draw.text((x, y), copy["headline"], font=headline_font, fill=(*INK, 255))
    if sub_text:
        y += head_h + gap - sub_font.getbbox(sub_text)[1]
        draw.text((x, y), sub_text, font=sub_font, fill=(*INK_SOFT, 255))

    target.parent.mkdir(parents=True, exist_ok=True)
    composed.convert("RGB").save(target, format="PNG", optimize=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dir", required=True, help="原始截屏目录")
    parser.add_argument("--locale", required=True, choices=sorted(STORY_FILES))
    parser.add_argument("--out", help="输出目录（默认 <dir>/../<name>-shot）")
    args = parser.parse_args()

    story_path = STORY_FILES[args.locale]
    if not story_path.exists():
        fail(f"找不到文案文件：{story_path}")
    story = json.loads(story_path.read_text(encoding="utf-8"))

    directory = Path(args.dir)
    shots = sorted(directory.glob("*.png"))
    if not shots:
        fail(f"{directory} 里没有 PNG")

    out_dir = Path(args.out) if args.out else directory.parent / f"{directory.name}-shot"

    missing = [s.name for s in shots if s.name not in story]
    if missing:
        fail(
            "这些图在文案文件里没有对应条目：" + "、".join(missing) +
            "。宁可不叠，也不要传一张没有故事的图上去。"
        )

    print(f"叠字 {len(shots)} 张（{args.locale}）→ {out_dir}")
    for shot in shots:
        target = out_dir / shot.name
        annotate(shot, target, story[shot.name], args.locale)
        print(f"  {shot.name}：{story[shot.name]['headline']}")
    print("完成。")


if __name__ == "__main__":
    main()