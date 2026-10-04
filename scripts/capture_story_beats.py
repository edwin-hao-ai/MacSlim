#!/usr/bin/env python3
"""驱动 MAS 截屏副本走完「授权 → 扫描 → 确认 → 清理」并抓五拍。

## 为什么要有这个脚本

这五张图讲的是一条**有先后的故事**，所以画面必须来自真实的状态迁移：
授权之前、授权之后、确认弹窗、清理完成、历史记录。而这些状态只能靠
真的点出来 —— NSOpenPanel、确认弹窗、清理动作没有一个能从代码注入。

手工分步点击试过，失败在两处：

1. 固定 AX 路径（`group 2 of UI element 1 of scroll area 1 of ...`）会随
    界面结构变化而失效，报「无效的索引」。改成用 `entire contents` 扁平
    搜索按钮名，不依赖层级。
2. 坐标点击需要按显示尺寸换算，而截图是 2880x1800、预览是缩过的 —— 我
    连续三次把坐标算错。改成**按颜色找控件**：主按钮是品牌青绿、确认按钮
    是红色，全图搜对应色块再取中心，不做任何手工换算。

## 每一步都验证，失败即停

判据一律来自服务端或文件系统，不看界面：

- 授权有没有生效 → 扫描结果里有没有可释放空间
- 清理有没有真的发生 → **fixture 文件是否被删** + 数据库里的历史记录

最后一条是关键：只看界面的话，「报告成功但什么都没删」这种最糟的情况
会被当成通过。这个坑真的踩到过 —— 应用历史上出现过「成功 1 项、释放
10.5 GB」而文件一个没少的记录。

## 用法

```bash
python3 scripts/capture_story_beats.py --locale zh-Hans --out /tmp/story-zh
```
"""
from __future__ import annotations

import argparse
import os
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
# 必须和 _prepare_shot_app.sh 用同一个位置。macOS 上 TMPDIR 指向
# /var/folders/.../T，写死 /tmp 会和构建脚本产出的副本对不上 ——
# 症状是脚本照常往下跑，截出来的却是一屏白。
SHOT_APP = Path(os.environ.get("TMPDIR", "/tmp")) / "MacSlimShot.app"
REGION = "0,0,1440,900"
CANVAS = (2880, 1800)
CONTAINER = Path.home() / "Library/Containers/com.vgoapp.macslim/Data"
DB_PATH = CONTAINER / "Library/Application Support/MacSlim/macslim.db"
GRANTS = CONTAINER / "Library/Application Support/folder-grants.json"
FIXTURE_GLOB = "macslim-shot-fixture-*"
# 注意：**不能**写 `Path.home() / "/Library/Logs"`。pathlib 的 `/` 运算符在
# 右边是绝对路径时会整个丢弃左边，结果是系统的 `/Library/Logs` 而不是用户
# 家目录下的那个 —— 而用户日志目录恰恰在后者。这个坑很安静：路径存在、
# glob 不报错，只是永远匹配不到东西。
LOG_RELATIVE = "Library/Logs"


def user_logs_dir() -> Path:
    return Path.home() / LOG_RELATIVE


def say(message: str) -> None:
    print(f"  {message}", flush=True)


def fail(message: str) -> None:
    print(f"错误: {message}", file=sys.stderr, flush=True)
    raise SystemExit(1)


def run(cmd: list[str], cwd: Path | None = None, check: bool = False) -> None:
    """跑一条命令。

    默认仍然丢掉输出（截图流程里 pkill / screencapture / osascript 的噪音很大），
    但 build / git 这类**失败必须看得见**的调用要 check=True 或自己接输出 ——
    之前所有调用都走同一条静默路径，构建挂了整支脚本也只是若无其事地继续。
    """
    result = subprocess.run(
        cmd,
        cwd=str(cwd) if cwd else None,
        stdout=None if check else subprocess.DEVNULL,
        stderr=None if check else subprocess.DEVNULL,
    )
    if check and result.returncode != 0:
        fail(f"命令失败（{result.returncode}）：{' '.join(cmd)}")


def spawn_app() -> None:
    """把截图副本拉起来。

    用**直接 exec**而不是 `open`。`open` 走 LaunchServices，对放在
    `$TMPDIR`（/var/folders/.../T）里的 Development 签名 app 会直接失败：
    `RBSRequestErrorDomain Code=5 / NSUnderlyingError 162 Launchd job spawn
    failed`。而同样的包直接 exec 起来完全正常 —— 之前一直用 `open`，
    撞上这个之后表现为「脚本跑完，截出一屏白」，很容易误判成签名或沙箱问题。
    """
    binary = SHOT_APP / "Contents/MacOS/macslim"
    if not binary.exists():
        fail(f"截图副本不存在：{binary}（先跑 scripts/_prepare_shot_app.sh）")
    log = open("/tmp/macslim-shot-run.log", "ab")
    subprocess.Popen(
        [str(binary)],
        stdout=log,
        stderr=log,
        stdin=subprocess.DEVNULL,
        start_new_session=True,
    )


def capture(path: Path) -> Image.Image:
    run(["screencapture", "-x", "-R", REGION, "-t", "png", str(path)])
    return Image.open(path).convert("RGB")


def activate() -> None:
    # 用 System Events 设 frontmost，而不是 `tell application "MacSlimShot"
    # to activate`：截图副本是直接 exec 起来的，没经过 LaunchServices 注册，
    # 按 app 名发 AppleScript 有可能找不到它。进程名固定是 macslim。
    run(["osascript", "-e",
         'tell application "System Events" to tell process "macslim" '
         'to set frontmost to true'])
    time.sleep(2)


def click_button(name: str) -> bool:
    """按名字点按钮。用 `entire contents` 扁平搜索，不依赖 AX 层级。

    固定路径（`group N of UI element 1 of scroll area 1 of ...`）试过，
    界面结构一变就报「无效的索引」—— 报的还是中文，排查成本很高。
    """
    script = f'''
on run argv
    set wantName to item 1 of argv
    tell application "System Events" to tell process "macslim"
        try
            set elems to entire contents of window 1
        on error
            return "NOWINDOW"
        end try
        repeat with e in elems
            try
                if (role of e) is "AXButton" then
                    if (name of e as string) is wantName then
                        click e
                        return "CLICKED"
                    end if
                end if
            end try
        end repeat
        return "NOTFOUND"
    end tell
end run
'''
    result = subprocess.run(
        ["osascript", "-s", "o", "-", name],
        input=script, capture_output=True, text=True,
    )
    return result.stdout.strip().endswith("CLICKED")


# 全屏 1440x900 下授权卡片里六个「授权」按钮的实测点坐标（第二行 = 应用日志）。
#
# 为什么需要兜底：`entire contents` 对 WKWebView 里的按钮并不总能列出 ——
# 有时能搜到、有时搜不到（实测「清理」「确认执行」能搜到，「授权」搜不到）。
# 布局是固定的（固定尺寸 + 固定行高），所以按实测坐标点第二行是稳定的；
# 而且点错行的后果只是「授权了另一个目录」，下一步「有没有扫到 fixture」
# 的验证会立刻把它抓出来，不会静默走偏。
GRANT_BUTTON_POINT = (1358, 218)

# 侧栏「历史记录」导航项的实测坐标（第五拍要切过去）。
# 和授权按钮同理：它在 webview 里，AX 搜不到，只能按坐标点。侧栏固定宽度 +
# 固定行高，坐标稳定。
NAV_HISTORY_POINT = (49, 262)


def find_color_block(image: Image.Image, predicate) -> tuple[int, int] | None:
    """全图搜符合颜色判据的像素块，返回其中心的**点坐标**。

    不做手工换算：截图是 2880x1800 像素、而预览给我的图是缩过的，我按预览
    尺寸算坐标连续错了三次。搜色块则与显示比例无关。
    """
    px = image.load()
    width, height = image.size
    best = None
    for y in range(0, height, 3):
        for x in range(0, width, 3):
            r, g, b = px[x, y]
            if predicate(r, g, b) and (best is None or g - r > best[0]):
                best = (g - r, x, y, (r, g, b))
    if best is None:
        return None
    _, bx, by, colour = best
    xs = [x for x in range(max(0, bx - 400), min(width, bx + 400))
          if all(abs(px[x, by][i] - colour[i]) < 30 for i in range(3))]
    ys = [y for y in range(max(0, by - 300), min(height, by + 300))
          if all(abs(px[bx, y][i] - colour[i]) < 30 for i in range(3))]
    return (min(xs) + max(xs)) // 4, (min(ys) + max(ys)) // 4


def is_teal(r: int, g: int, b: int) -> bool:
    return g > 170 and b > 170 and r < 160 and abs(g - b) < 45


def is_confirm_red(r: int, g: int, b: int) -> bool:
    return r > 190 and g < 100 and b < 100


def click_point(point: tuple[int, int]) -> None:
    run(["osascript", "-e",
         f'tell application "System Events" to click at {{{point[0]}, {point[1]}}}'])
    time.sleep(3)


def fixture_files() -> int:
    logs = user_logs_dir()
    return len(list(logs.glob(FIXTURE_GLOB))) if logs.exists() else 0


def db_history() -> list[tuple]:
    if not DB_PATH.exists():
        return []
    con = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
    try:
        return con.execute("select * from history order by id desc").fetchall()
    finally:
        con.close()


# 「0 B」本身也是 teal 大字 —— 实测空态这张图在该区域有 430 个 teal 像素。
# 所以判据不能是「>0」，否则第一拍永远通不过（曾经就是这样：截图拍得完全正确，
# 却被自己的校验判成「出现了可释放空间」）。真实数值（10.50 GB）字形数是
# 「0 B」的三四倍，落在 1200 以上。
MIN_DATA_TEAL = 1200


def reclaimable_bytes(image: Image.Image) -> int:
    """「可释放空间」那一大片 teal 大字的像素面积。

    只判「有没有」，不判具体数值 —— 具体数值由 fixture 体积决定，而那是
    我们自己造的，用不着从像素里反推。返回值会打进日志：阈值万一再偏，
    至少能一眼看出是判据错了还是界面变了。
    """
    px = image.load()
    return sum(1 for y in range(900, 1250, 2) for x in range(2300, CANVAS[0], 2)
               if is_teal(*px[x, y]))


def has_data(image: Image.Image) -> bool:
    count = reclaimable_bytes(image)
    print(f"    （可释放区域 teal 像素 {count}，阈值 {MIN_DATA_TEAL}）", flush=True)
    return count > MIN_DATA_TEAL


def reset_grants() -> None:
    """清掉授权，让第一拍能拍到「一个都没授权」的真实初始态。"""
    if GRANTS.exists():
        GRANTS.unlink()


def verify_grant_point(image: Image.Image) -> None:
    """点之前先确认那个坐标还落在「授权」链接上。

    这道守卫不是形式主义。AX 在 WKWebView 里搜不到按钮（实测 `entire
    contents` 返回 0 个 AXButton），所以只能按坐标点；而坐标一旦因为布局
    变化而失效，点下去可能正好落在主 CTA「清理」上 —— 那一下真的会删东西，
    而且不会有任何提示。判据用颜色区分两者：

    - 「授权」是白底卡片上的深色文字链接，四周以卡片底色（接近白）为主
    - 「清理」是实心青绿色圆角块，坐标周围以青绿为主

    所以看坐标点周围 24x24 像素的网格，青绿占比过高就拒绝点。
    """
    x, y = GRANT_BUTTON_POINT
    # 截图是 2 倍像素，坐标是点，先换算再取样
    px = image.convert("RGB").load()
    teal = 0
    sampled = 0
    for dy in range(-12, 13, 4):
        for dx in range(-12, 13, 4):
            sx, sy = (x + dx) * 2, (y + dy) * 2
            if 0 <= sx < image.width and 0 <= sy < image.height:
                r, g, b = px[sx, sy]
                sampled += 1
                # 与主按钮同族的青绿：g 明显高于 r，且整体偏亮
                if g > 130 and g - r > 45 and b > 110:
                    teal += 1
    if sampled == 0:
        fail(f"取样点 ({x},{y}) 落在截图外，坐标换算有问题")
    if teal / sampled > 0.3:
        fail(
            f"({x},{y}) 周围青绿占比 {teal}/{sampled} —— 那里大概是主按钮"
            f"「清理」而不是「授权」。坐标已失效，拒绝点（点下去会真删东西）。"
        )


def do_grant(image_png: Path) -> None:
    """点「授权」→ 面板已开在目标目录 → 点 Open。

    面板起始目录是 `d0f3075` 修的：现在点哪一行的授权就开在哪一行，所以不
    需要再 ⌘⇧G 手输路径 —— 之前每次都得手打，就是这个 bug 的症状。
    """
    if not click_button("授权"):
        say("AX 搜不到「授权」，改用实测坐标（先过颜色守卫）")
        verify_grant_point(capture(image_png))
        click_point(GRANT_BUTTON_POINT)
    time.sleep(5)
    shot = capture(image_png)
    point = find_color_block(shot, lambda r, g, b: b > 200 and r < 110 and 100 < g < 190)
    if point is None:
        fail("文件选择框的 Open 按钮不可用（面板可能没打开）")
    click_point(point)


def wait_for_reclaimable(image_png: Path, want_data: bool, timeout: int = 40) -> Image.Image:
    deadline = time.time() + timeout
    image = capture(image_png)
    while time.time() < deadline:
        image = capture(image_png)
        has = has_data(image)
        if has == want_data:
            return image
        time.sleep(3)
    return image


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--locale", required=True, choices=["zh-Hans", "en-US"])
    parser.add_argument("--out", required=True)
    return parser.parse_args()


def check_preconditions() -> None:
    logs = user_logs_dir()
    if not logs.exists():
        fail(f"{logs} 不存在 —— 先跑 scripts/screenshot_fixture.sh up")
    if fixture_files() == 0:
        fail("fixture 不在位（没有 macslim-shot-fixture-* 文件）")


def clear_history() -> None:
    """历史清空，让第五拍只包含这次流程的记录。"""
    if not DB_PATH.exists():
        return
    con = sqlite3.connect(DB_PATH)
    con.execute("delete from history")
    con.commit()
    con.close()


def build_shot_app(locale: str) -> None:
    """构建并重签截图副本，拍完把改过的源码还原。

    之前这里假定 `/tmp/MacSlimShot.app` 已经存在，「谁来构建」成了口头约定：
    单独跑这个脚本就会因为副本不存在或签名不对而起不来 / 起成白屏。
    """
    locale_code = "zh-CN" if locale == "zh-Hans" else "en"
    try:
        run([str(ROOT / "scripts/_prepare_shot_app.sh"), "cache", locale_code])
    finally:
        # 截图是改源码（首屏视图 + 语言）编出来的，拍完必须还原。
        # 漏掉这一步的后果很隐蔽：工作区里会留下一个「首屏写死成 cache」的
        # App.tsx，下次谁构建正式包都带着它。
        run(["git", "checkout", "--", "src/App.tsx", "src/i18n/index.tsx",
             "src-tauri/tauri.conf.json"], cwd=ROOT, check=False)


def shoot_ungranted(out: Path) -> None:
    """第一拍：一个目录都没授权的真实初始态。"""
    run(["pkill", "-x", "macslim"])
    time.sleep(2)
    spawn_app()
    time.sleep(16)
    activate()
    shot = capture(out / "01-authorized.png")
    if has_data(shot):
        fail("第一拍应该拍到未授权的空态，却出现了可释放空间")
    say("01-authorized：未授权空态")


def shoot_granted(out: Path, probe: Path) -> None:
    """第二拍：授权之后、真实数据。"""
    wait_for_reclaimable(probe, want_data=False, timeout=12)
    print("  授权中…")
    do_grant(probe)
    image = wait_for_reclaimable(probe, want_data=True, timeout=45)
    if not has_data(image):
        fail("授权之后仍扫不到 fixture —— 授权或扫描有问题")
    say("授权生效，扫到 fixture")
    capture(out / "02-scanned.png")
    say("02-scanned：真实列表")


def shoot_confirm(out: Path, probe: Path) -> None:
    """第三拍：二次确认弹窗。"""
    if not click_button("清理"):
        point = find_color_block(capture(probe), is_teal)
        if point is None:
            fail("找不到清理主按钮")
        click_point(point)
    time.sleep(4)
    image = capture(out / "03-confirm.png")
    if find_color_block(image, is_confirm_red) is None:
        fail("二次确认弹窗没出现")
    say("03-confirm：二次确认弹窗")


def wait_fixture_gone(timeout: int = 40) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline and fixture_files() > 0:
        time.sleep(2)


def shoot_cleaned(out: Path, probe: Path) -> None:
    """第四拍：清理完成 —— 并核对它是真的删掉了，不只是界面变了。"""
    if not click_button("确认执行"):
        point = find_color_block(capture(probe), is_confirm_red)
        if point is None:
            fail("找不到「确认执行」按钮")
        click_point(point)

    wait_fixture_gone()
    remaining = fixture_files()
    history = db_history()
    if remaining != 0:
        fail(f"清理没有真的删掉 fixture（还剩 {remaining} 个文件）。"
             f"历史记录：{history[:1]}")
    if not history or "成功 1 项" not in str(history[0][-1]):
        fail(f"历史记录没有记成功：{history[:1]}")
    say(f"清理真的生效：fixture 已清空，历史记「{history[0][-1]}」")
    time.sleep(3)
    capture(out / "04-cleaned.png")


def shoot_history(out: Path, probe: Path) -> None:
    """第五拍：历史记录。

    侧栏「历史记录」同样在 webview 里，AX 搜不到，只能按坐标点。侧栏是固定
    宽度 + 固定行高，坐标稳定（与授权按钮同理）。
    """
    if not click_button("历史记录"):
        say("AX 搜不到「历史记录」，改用实测坐标")
        click_point(NAV_HISTORY_POINT)
    time.sleep(4)
    capture(out / "05-history.png")
    say("05-history：历史记录")


def report(out: Path) -> None:
    print("完成。")
    for shot in sorted(out.glob("*.png")):
        image = Image.open(shot)
        print(f"  {shot.name} {image.size[0]}x{image.size[1]}")


def main() -> None:
    args = parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    probe = Path("/tmp/_story_probe.png")

    check_preconditions()
    clear_history()
    print(f"[{args.locale}] 五拍 → {out}")
    build_shot_app(args.locale)
    reset_grants()
    activate()

    shoot_ungranted(out)
    shoot_granted(out, probe)
    shoot_confirm(out, probe)
    shoot_cleaned(out, probe)
    shoot_history(out, probe)
    report(out)


if __name__ == "__main__":
    os.environ.setdefault("PYTHONDONTWRITEBYTECODE", "1")
    main()
