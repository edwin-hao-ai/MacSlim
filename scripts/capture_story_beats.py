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
# 和授权按钮同理：它在 webview 里，AX 搜不到（实测 entire contents 返回
# 0 个 AXButton），只能按坐标点。侧栏固定宽度 + 固定行高，实测各行中心
# （pt）：智能扫描 78、进程管理 116、缓存清理 154、应用卸载 192、
# 历史记录 230、设置 268，行距 38。
NAV_HISTORY_POINT = (49, 230)

# 主 CTA「清理」所在区域（像素，左上-右下）。
#
# 必须给搜索限定区域：全图搜青绿会先命中左上角侧栏的选中底色，点下去会
# 导航到别的页面。而 03 拍的校验又会被「智能扫描」页那个**红色磁盘环**骗过
# —— 于是脚本报「二次确认弹窗没出现」以外的错，真正的问题（点错了地方）
# 完全看不出来。区域按实测：按钮在 px x 620~740、y 1500~1600 附近，
# 留足余量并排除侧栏（侧栏宽约 320 px）。
CLEAN_BUTTON_REGION = (350, 1400, 1500, 1790)

# 二次确认弹窗所在区域（像素）。弹窗是居中的模态框，实测约占
# px x 990~1890、y 590~1230。缩到这块有两个作用：校验「弹窗有没有出现」
# 不会被别处的红色骗过（「智能扫描」页那个 87% 的红色磁盘环就骗过一次），
# 找「确认执行」也不会误点到别处。
CONFIRM_DIALOG_REGION = (900, 550, 2000, 1300)


def find_color_block(
    image: Image.Image,
    predicate,
    region: tuple[int, int, int, int] | None = None,
) -> tuple[int, int] | None:
    """搜符合颜色判据的像素块，返回其中心的**点坐标**。

    不做手工换算：截图是 2880x1800 像素、而预览给我的图是缩过的，我按预览
    尺寸算坐标连续错了三次。搜色块则与显示比例无关。

    **必须能限定 region**：不限定就是在整屏里找，而它挑的是 `g - r` 最大的
    那一个点 —— 于是找「清理」主按钮时会先命中左上角侧栏的选中底色，点下去
    导航到别的页面。这个 bug 很隐蔽：截图里确实有按钮、搜索也确实搜到了
    「青绿色」，只是搜到的是另一个青绿色。region 是像素坐标 (x0, y0, x1, y1)。
    """
    x0, y0, x1, y1 = region if region else (0, 0, *image.size)
    x0, y0 = max(0, x0), max(0, y0)
    x1, y1 = min(image.width, x1), min(image.height, y1)
    px = image.load()
    best = None
    for y in range(y0, y1, 3):
        for x in range(x0, x1, 3):
            r, g, b = px[x, y]
            if predicate(r, g, b) and (best is None or g - r > best[0]):
                best = (g - r, x, y, (r, g, b))
    if best is None:
        return None
    _, bx, by, colour = best
    xs = [x for x in range(max(x0, bx - 400), min(x1, bx + 400))
          if all(abs(px[x, by][i] - colour[i]) < 30 for i in range(3))]
    ys = [y for y in range(max(y0, by - 300), min(y1, by + 300))
          if all(abs(px[bx, y][i] - colour[i]) < 30 for i in range(3))]
    return (min(xs) + max(xs)) // 4, (min(ys) + max(ys)) // 4


def is_teal(r: int, g: int, b: int) -> bool:
    return g > 170 and b > 170 and r < 160 and abs(g - b) < 45


def is_confirm_red(r: int, g: int, b: int) -> bool:
    return r > 190 and g < 100 and b < 100


def click_point(point: tuple[int, int]) -> None:
    """点一个屏幕坐标。

    **必须**投递真正的鼠标事件（`_click.py` 走 CGEventPost），不能用
    `System Events ... click at`：后者是 Accessibility 通道，对 WKWebView 里
    的 HTML 按钮完全无效 —— WebKit 不把 DOM 节点暴露成可执行 AXPress 的
    元素，点击被悄悄丢掉，既不报错也没有日志。

    这个坑的误导性在于所有表面证据都是对的：前台对（截图拍到 app）、坐标对
    （对着截图量的）、权限对（`set frontmost` 生效）。实测把侧栏点击换成
    CGEvent 后立刻生效，而 `click at` 同一坐标毫无反应。
    """
    run([sys.executable, str(ROOT / "scripts/_click.py"),
         str(point[0]), str(point[1])])
    time.sleep(3)


def fixture_files() -> int:
    logs = user_logs_dir()
    return len(list(logs.glob(FIXTURE_GLOB))) if logs.exists() else 0


def db_history() -> list[dict]:
    """最近的历史记录，**按列名**取。

    曾经用 `select *` 再取 `row[-1]` —— 而 `history` 表后来加了
    item_count / ok_count / fail_count / reason_code 四列，`[-1]` 从 detail
    变成了 reason_code（空串）。断言于是永远失败，看起来像「清理没成功」，
    实际清理是成功的。位置索引对会变的表结构太脆，改成按名字取。
    """
    if not DB_PATH.exists():
        return []
    con = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True)
    con.row_factory = sqlite3.Row
    try:
        rows = con.execute("select * from history order by id desc").fetchall()
        return [dict(row) for row in rows]
    finally:
        con.close()


def is_reclaim_value(r: int, g: int, b: int) -> bool:
    """「可释放空间」那个大数字的颜色。

    实测色是 (65, 143, 174) —— 注意它**不是**主按钮那种 teal：模块里已有的
    `is_teal` 判据是 `g > 170 and b > 170`，数字的 g 只有 143，一个都匹配不上
    （第一次写错就栽在这：数字明明在屏幕上，计数却是 70）。
    """
    return g > 120 and b > 150 and g - r > 40 and b - r > 60


# 数字是右对齐的，所以「有没有数据」看**横向跨度**比看像素总数稳得多。
# 实测：空态「0 B」跨度 86 px，「10.50 GB」跨度 259 px —— 3 倍差距；
# 而按像素总数只有 2.75 倍（430 vs 1183），阈值定在哪边都心虚。
MIN_VALUE_SPAN = 150


def reclaim_value_span(image: Image.Image) -> int:
    """那个大数字的横向像素跨度；没有数字时返回 0。

    只判「有没有」，不反推具体数值 —— 数值由 fixture 体积决定，是我们自己
    造的，没必要从像素里读出来。
    """
    px = image.load()
    xs = [x for y in range(980, 1180, 2) for x in range(2300, CANVAS[0], 2)
          if is_reclaim_value(*px[x, y])]
    return (max(xs) - min(xs)) if xs else 0


def has_data(image: Image.Image) -> bool:
    return reclaim_value_span(image) > MIN_VALUE_SPAN


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
    # 截图是 2 倍像素，坐标是点，先换算再取样。
    # 判据用 is_teal —— 它就是对主按钮标定的（实测按钮色 (82,179,208)）。
    image = image.convert("RGB")
    px = image.load()
    teal = 0
    sampled = 0
    for dy in range(-12, 13, 4):
        for dx in range(-12, 13, 4):
            sx, sy = (x + dx) * 2, (y + dy) * 2
            if 0 <= sx < image.width and 0 <= sy < image.height:
                sampled += 1
                if is_teal(*px[sx, sy]):
                    teal += 1
    if sampled == 0:
        fail(f"取样点 ({x},{y}) 落在截图外，坐标换算有问题")
    if teal / sampled > 0.3:
        fail(
            f"({x},{y}) 周围青绿占比 {teal}/{sampled} —— 那里大概是主按钮"
            f"「清理」而不是「授权」。坐标已失效，拒绝点（点下去会真删东西）。"
        )


def window_names() -> list[str]:
    result = subprocess.run(
        ["osascript", "-e",
         'tell application "System Events" to tell process "macslim" '
         'to get name of every window'],
        capture_output=True, text=True,
    )
    return [n.strip() for n in result.stdout.split(",") if n.strip()]


def press_key(key: str) -> None:
    run([sys.executable, str(ROOT / "scripts/_click.py"), "--key", key])
    time.sleep(1)


def do_grant(image_png: Path) -> None:
    """点「授权」→ 在选择框里回车确认。

    两步都必须用**真**事件：
    - 点「授权」：`System Events click at` 走 AX 通道，对 WKWebView 里的
      HTML 链接无效（详见 click_point 的注释）
    - 确认选择框：用回车接受默认按钮，而不是按坐标找那个蓝色「打开」。
      面板每次出现的位置会变，按坐标找既脆又容易误点到 app 自己的蓝色元素
      —— 之前就是这么把面板点没了，然后报「授权之后仍扫不到 fixture」。

    面板起始目录是 `d0f3075` 修的：点哪一行的授权就开在哪一行（实测点
    「应用日志」那一行，面板开在 ~/Library/Logs），所以不需要再手工输路径。
    """
    if not click_button("授权"):
        say("AX 搜不到「授权」，改用实测坐标（先过颜色守卫）")
        verify_grant_point(capture(image_png))
        click_point(GRANT_BUTTON_POINT)

    # 等面板真的出现再回车。直接 sleep 后回车会在面板还没起来时把回车发给
    # 主窗口 —— 那就成了「什么都没发生」。
    for _ in range(10):
        time.sleep(1)
        if len(window_names()) >= 2:
            break
    else:
        fail(f"点了「授权」但选择框没出现（当前窗口：{window_names()}）")

    press_key("return")

    # 再等面板消失，确认回车被面板吃掉了而不是落到别处
    for _ in range(10):
        time.sleep(1)
        if len(window_names()) < 2:
            break


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
        point = find_color_block(capture(probe), is_teal, CLEAN_BUTTON_REGION)
        if point is None:
            fail("找不到清理主按钮")
        click_point(point)
    time.sleep(4)
    image = capture(out / "03-confirm.png")
    if find_color_block(image, is_confirm_red, CONFIRM_DIALOG_REGION) is None:
        fail("二次确认弹窗没出现")
    say("03-confirm：二次确认弹窗")


def wait_fixture_gone(timeout: int = 40) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline and fixture_files() > 0:
        time.sleep(2)


def shoot_cleaned(out: Path, probe: Path) -> None:
    """第四拍：清理完成 —— 并核对它是真的删掉了，不只是界面变了。"""
    if not click_button("确认执行"):
        point = find_color_block(capture(probe), is_confirm_red, CONFIRM_DIALOG_REGION)
        if point is None:
            fail("找不到「确认执行」按钮")
        click_point(point)

    wait_fixture_gone()
    remaining = fixture_files()
    history = db_history()
    if remaining != 0:
        fail(f"清理没有真的删掉 fixture（还剩 {remaining} 个文件）。"
             f"历史记录：{history[:1]}")
    # 用结构化字段判定，而不是去匹配 detail 里的中文措辞 ——
    # 措辞会随本地化/文案调整变化，ok_count 不会。
    if not history or history[0]["ok_count"] < 1 or history[0]["fail_count"] != 0:
        fail(f"历史记录没有记成功：{history[:1]}")
    say(
        f"清理真的生效：fixture 已清空，"
        f"历史记「{history[0]['detail']}」（ok={history[0]['ok_count']} "
        f"fail={history[0]['fail_count']}）"
    )
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
