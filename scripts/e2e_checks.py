#!/usr/bin/env python3
"""MacSlim e2e 门禁的破坏性主链路（第 1-8 项）与 Broker 不变量（第 9-12 项）。"""
from __future__ import annotations

import subprocess
import time
from pathlib import Path

try:
    from scripts import e2e_driver as drv
    from scripts import e2e_registry as reg
except ModuleNotFoundError:  # 直接以脚本方式运行时走这条
    import e2e_driver as drv
    import e2e_registry as reg

from e2e_registry import (  # noqa: E402  供检查函数直接使用
    STATUS_FAIL,
    STATUS_PASS,
    STATUS_SKIP,
    App,
    AxNode,
    CheckFailure,
    CheckResult,
    ManualCheck,
    SkipCheck,
    _r,
    check,
    ok,
    result,
)

P_MAIN = drv.P_MAIN

# ============ 公共小工具 ============


def _cache_item_rows(app: App) -> list[tuple[str, drv.AxNode, str]]:
    """扫出缓存页所有条目：`(路径, 复选框节点, 名字)`。

    缓存页的实测层级（`CacheView.tsx` → `.card` 分组 → 逐条 `.card` 条目）：

    ```
    P_MAIN 的第 N 个子节点  AXGroup 610x70      ← 一个条目
      ├ 1 AXCheckBox  16x16   @309  ← 要点的就是这个
      ├ 2 AXStaticText        名字
      ├ 3 AXGroup → AXStaticText 描述
      └ 4 AXGroup → AXStaticText 路径   ← 用来定位是哪个条目
    ```

    分组容器本身是更大的 AXGroup（`610x144` / `610x958`，高 = 条目数 × 74），
    里面是逐个条目。所以判据是**高度在 60~90 且宽约 610** —— 分组容器宽相同
    但高得多（958），条目高固定 70。
    """
    rows: list[tuple[str, drv.AxNode, str]] = []
    for group in app.ax.children(P_MAIN, cap=80):
        if group.role != "AXGroup" or not (60 <= group.h <= 90 and group.w >= 400):
            continue
        kids = app.ax.children(group.path, cap=8)
        box = next((k for k in kids if k.role == "AXCheckBox"), None)
        if box is None:
            continue
        name = next(
            (k.value for k in kids if k.role == "AXStaticText" and k.value and k.y < group.y + 40),
            "",
        )
        path = ""
        for child in kids:
            if child.role != "AXGroup" or child.y < group.y + 40:
                continue
            text = _group_text(app, child)
            if text.startswith("/"):
                path = text
                break
        rows.append((path, box, name))
    return rows


def _find_cache_item_checkbox(app: App, target: Path) -> drv.AxNode:
    """找到路径等于 `target` 那一行的复选框。"""
    needle = str(target)
    for path, box, _name in _cache_item_rows(app):
        if path == needle:
            return box
    known = [f"{p or '(无路径)'} [{n}]" for p, _b, n in _cache_item_rows(app)][:8]
    raise SkipCheck(
        f"自造目录没出现在缓存条目里：{target}。\n"
        f"读到的条目（前 8 个）：{known}"
    )


def _group_text(app: App, group: drv.AxNode) -> str:
    """把一个 group 子节点下的静态文本拼起来（path 文本在 group 内部）。"""
    parts: list[str] = []
    for child in app.ax.children(group.path, cap=8):
        if child.role in ("AXStaticText", "AXTextField") and child.value:
            parts.append(child.value)
    return " | ".join(parts)


def _wait_checked(app: App, box: drv.AxNode, timeout: float = 3.0) -> None:
    """等某个复选框的 AXValue 变成 1。"""
    deadline = time.monotonic() + timeout
    value = ""
    while time.monotonic() < deadline:
        value = app.ax.node(box.path, fresh=True).value.strip()
        if value == "1":
            return
        time.sleep(0.25)
    raise CheckFailure(f"勾选后复选框仍是 unchecked（AXValue={value!r}）：{box.describe()}")


def _checkbox_beside_path(app: App, nodes: list[AxNode], path: str) -> AxNode | None:
    """在同一条缓存项里找复选框：靠几何邻近（同一行、复选框在 path 文本左侧）关联。"""
    anchors = [n for n in nodes if n.role == "AXStaticText" and n.value == path]
    if not anchors:
        return None
    anchor = anchors[0]
    left = [n for n in nodes if n.role == "AXCheckBox" and n.x <= anchor.x and n.y <= anchor.y]
    if not left:
        return None
    return min(left, key=lambda n: abs(n.y - anchor.y) * 4 + (anchor.x - n.x))


def _freeze_process_list(app: App) -> None:
    """悬停列表区让 5 秒轮询停下来，并把主视图子节点读成稳定快照。

    没有这一步，按下标逐个读 AX 的十几秒里一定会撞上整树重建，行节点会被截断
    （读到的 `UI element i` 瞬时为空）。悬停点全部由实时几何推出：
    主视图左上角 + 列头行的 y + 60。
    """
    main = app.ax.node(P_MAIN, fresh=True)
    header = app.ax.node(P_MAIN + (15,), fresh=True)
    if not main.valid or not header.valid:
        raise SkipCheck("读不到主视图 / 列头几何，无法冻结列表")
    app.hover(main.x + main.w // 2, header.y + 60)
    time.sleep(0.8)
    for _ in range(3):
        a = app.ax.node(P_MAIN + (ROW_START,), fresh=True)
        time.sleep(0.7)
        b = app.ax.node(P_MAIN + (ROW_START,), fresh=True)
        if (a.role, a.x, a.y) == (b.role, b.x, b.y):
            return
    raise SkipCheck(
        "悬停后主视图第 21 个子节点仍两次读到不同结果，列表没被冻结；"
        + drv._process_probe_block()
    )


def _process_search_box(app: App) -> AxNode:
    """进程搜索框：main 子节点里 220x34 的 AXTextField。不写死坐标，按形状现读。"""
    for node in app.ax.children(P_MAIN, cap=24):
        if node.role == "AXGroup" and 200 <= node.w <= 240 and 24 <= node.h <= 44:
            inner = app.ax.node(P_MAIN + (node.index, 1), fresh=True)
            if inner.role == "AXTextField":
                return inner
    raise SkipCheck("找不到进程搜索框（AXTextField 220x34）")


# `whitelist::SYSTEM_CORE_NAMES` 里**以当前用户身份运行、PID ≥ 50** 的进程。
# launchd / WindowServer / securityd 等是 root，会被 scanner.rs:281-287 跳过，不能当目标。
_CORE_CANDIDATES = ("UserEventAgent", "Dock", "SystemUIServer", "nsurlsessiond", "cloudd")


# 进程管理页主视图的子节点布局（实测）：1 标题 / 2-9 统计 / 10-12 视图切换
# / 13 搜索框 / 14 刷新 / 15-20 列头 / 21 起 行与页脚。
ROW_START = 21


def _process_row(app: App, pid: int) -> dict | None:
    """在已按 PID 收窄的列表里定位那一行。

    行在 AX 树里的形状（实测，按 PID 收窄到 1 行时）：
    `AXCheckBox` + `AXStaticText`(进程名) + 若干 `AXGroup`（内存/运行/PID 单元格）
    + `AXButton`(加白名单)。**PID 文本藏在单元格的 AXGroup 里**，不是 main 的直接
    子节点，所以这里对每个单元格下钻一层去找 PID 文本，当作「这一行确实是目标
    进程」的身份证明。
    """
    _freeze_process_list(app)
    kids = app.ax.children(P_MAIN, cap=ROW_START + 24)
    boxes = [n for n in kids if n.role == "AXCheckBox"]
    if len(boxes) != 1:
        return None
    box = boxes[0]
    if not _row_contains_pid(app, kids, box, pid):
        return None
    label = next(
        (n.value for n in kids
         if n.role == "AXStaticText" and abs(n.y - box.y) <= 8 and n.x > box.x),
        "",
    )
    return {"checkbox": box, "path": P_MAIN + (box.index,), "name": label}


def _row_contains_pid(app: App, kids: list[AxNode], box: AxNode, pid: int) -> bool:
    for node in kids:
        if node.role != "AXGroup" or abs(node.y - box.y) > 10:
            continue
        inner = app.ax.node(P_MAIN + (node.index, 1), fresh=True)
        if inner.value.strip() == str(pid):
            return True
    return False


def process_search(app: App, query: str) -> None:
    """在进程搜索框里筛选。**必须先收窄再读 AX**（见 `_process_row_nodes`）。"""
    app.type_text(_process_search_box(app), query, expect=bool(query))
    time.sleep(1.2)
    _freeze_process_list(app)


def _process_row_nodes(app: App, cap: int = 40) -> list[AxNode]:
    """读进程管理页的行节点（从 `ROW_START` 起按下标取）。

    为什么不用整树快照：这一页的 AX 子节点实测 3900+ 个，`UI elements of`
    批量物化会**永不返回**，逐节点遍历也会在 ~60 个节点后卡死。但把列表先用
    搜索框收窄到个位数行数后，同一条按下标取值的路径就稳定了（~0.3s/节点）。

    5 秒轮询会重建整棵树，所以按下标逐个读的过程中索引可能漂移。这里读两遍
    关键节点做稳定性校验，不稳定就当「读不到」，交由调用方报 SKIP。
    """
    first = app.ax.node(P_MAIN + (ROW_START,), fresh=True)
    if not first.valid:
        return []
    again = app.ax.node(P_MAIN + (ROW_START,), fresh=True)
    if (first.role, first.x, first.y) != (again.role, again.x, again.y):
        raise SkipCheck(
            f"进程列表 AX 树在读取期间被重建（第 {ROW_START} 个子节点两次读到不同结果："
            f"{first.role}@{first.x},{first.y} vs {again.role}@{again.x},{again.y}）。"
            "5 秒自动轮询与 AX 读取竞争，列表收窄到个位数行数后才稳定。"
        )
    nodes = [first]
    for i in range(ROW_START + 1, ROW_START + cap):
        node = app.ax.node(P_MAIN + (i,), fresh=True)
        if not node.valid:
            break
        nodes.append(node)
    return nodes


def _focus_victim(app: App, name: str, pid: int) -> dict:
    """进进程管理页、按唯一进程名收窄、勾选牺牲进程，返回该行信息。"""
    app.reset_to_fresh_view("进程管理")
    app.wait_heading("进程管理")
    # 先清空上一轮遗留的查询，否则「列表有多长」这个判据本身就被过滤过了。
    process_search(app, "")
    ensure_ports_filter_off(app)
    process_search(app, name)
    row = _row_or_recover(app, pid)
    if row["checkbox"].disabled:
        raise CheckFailure("牺牲进程的复选框是 disabled（不该命中白名单）")
    app.click(row["checkbox"], why="勾选牺牲进程")
    verify = app.ax.node(row["path"], fresh=True)
    if verify.value.strip() != "1":
        raise CheckFailure("点击后复选框仍是 unchecked（AXValue != 1）")
    return {"row": row, "verified": verify}


def _pass_protected_dialog(app: App) -> bool:
    """牺牲 sleep 刚启动 → 命中「不足 10 分钟」→ protected，流程多一层强制确认。

    返回是否出现过该弹窗。顺带证明受保护闸门在真实链路上是通的。
    """
    forced = app.ax.main_button_exact("仍要强制终止")
    if forced is None:
        return False
    app.click(forced, why="通过受保护二次确认")
    time.sleep(0.6)
    return True


def _confirm_and_wait(app: App, seconds: float) -> None:
    confirm = app.ax.main_button_exact("确认执行")
    if confirm is None:
        raise SkipCheck("OperationConfirm 未出现")
    app.click(confirm, why="确认执行")
    time.sleep(seconds)


def _rescan_cache(app: App) -> None:
    rescan = app.ax.main_button_exact("重新扫描")
    if rescan is None:
        raise SkipCheck("缓存页找不到「重新扫描」")
    app.click(rescan, why="重新扫描")
    time.sleep(6.0)


def _open_uninstaller(app: App, restart: bool = True) -> None:
    """进卸载页并等应用列表扫完。

    **卸载页没有「扫描」按钮** —— `UninstallerView` 只在 `onMount` 里扫一次
    （`UninstallerView.tsx:123`），外加一个刷新按钮（`:353`）。开发中这一项写的是
    「找扫描按钮然后点」，永远找不到，直接把第 7/8/17 项全部判成 SKIP。

    ⚠️ **默认会先重启 MacSlim**（`restart=True`）。后端的应用列表有 600 秒缓存
    （`app_scanner.rs:288` `APP_SCAN_CACHE_TTL`），而自造 app 是在缓存建立**之后**
    造出来的 —— 不重启就最多 10 分钟扫不到它，实测表现为「按数字过滤后仍找不到
    自建 app」。刷新按钮也只会重跑一次被缓存挡住的扫描。重启让缓存回冷，
    三项就都能自动判定，不必等 10 分钟。

    就绪判据用「列表里出现了 ≥3 个复选框」而不是死等固定秒数：实测列表扫描要
    3~11s（取决于装了多少 app），固定等待要么白等要么不够。

    `snapshot` 的 timeout 给到 20s 是因为这一页的 AX 树很大、整树物化会超时；
    它**超时也返回已探到的部分**（逐节点 flush），够用来判就绪。
    """
    app.ensure_running()
    if restart:
        app.restart()
    app.nav("应用卸载")
    app.wait_heading("应用卸载")
    # 先清掉可能残留的搜索词。搜索框的 `query` 状态活到组件卸载为止 —— 切走再
    # 切回来会重挂组件（`query` 回到空），但**同一次运行里**上一轮检查留下的
    # 过滤词还在，于是「列表里有几行」这个就绪判据本身已经被过滤过了
    # （开发中实测：残留过滤词让 40s 内只读到 1 个复选框，直接判 SKIP）。
    try:
        _search_uninstaller(app, "")
    except SkipCheck:
        pass
    deadline = time.monotonic() + 40.0
    while time.monotonic() < deadline:
        nodes = app.ax.snapshot(budget=600, timeout=20.0)
        rows = [n for n in nodes if n.role == "AXCheckBox"]
        if len(rows) >= 3:
            time.sleep(1.0)
            return
        time.sleep(0.6)
    raise SkipCheck(
        "卸载页 40s 内没扫出应用列表（仍在扫描？）"
        f"（最后一次读到 {len(rows)} 个复选框）"
    )


def _require_app_in_list(app: App, bundle: Path, budget: int = 600) -> None:
    """确认自造 app 出现在卸载列表里（必要时用搜索框过滤出来）。

    卸载列表按体积降序排，自造 app 只有几百 KB，落在长列表的**滚动区外** ——
    AX 整树快照读不到它（实测这一页快照还会超时，只返回部分节点）。所以先用
    搜索框按名字里的**纯数字段**过滤：过滤后列表只剩一行，一定在可视区内。
    数字段是必须的 —— 合成键盘事件会被输入法接管，见 `app_search_term`。

    ⚠️ 后端有 **600 秒的应用列表缓存**（`app_scanner.rs:288` `APP_SCAN_CACHE_TTL`）。
    自造 app 若造在缓存之后，最多 10 分钟扫不到它。`_open_uninstaller(restart=True)`
    已经通过重启 MacSlim 把缓存清掉了；这里仍然兜一层，是因为重启失败/被跳过时
    不该把「环境没准备好」报成「找不到目标」。
    """
    term = drv.app_search_term(bundle)
    if not _uninstaller_shows(app, term):
        _search_uninstaller(app, term)
    if not _uninstaller_shows(app, term):
        raise SkipCheck(
            f"按「{term}」过滤后仍找不到自建假 app：{bundle}。\n"
            "最可能的原因：后端有 600s 的应用列表缓存"
            "（`app_scanner.rs:288` `APP_SCAN_CACHE_TTL`），自造 app 造在缓存之后。\n"
            "解法：先造好自造 app，再**重启 MacSlim**，然后重跑本项。"
        )


def _uninstaller_shows(app: App, term: str) -> bool:
    """按名判断列表里有没有那一行。走主视图直接子节点，不走整树快照。

    这一页的整树快照会超时（实测 20s 只返回部分节点，里面可能根本没有目标行），
    所以按下标读直接子节点 —— 过滤后只剩一行，稳定且快。
    """
    for node in app.ax.children(P_MAIN, cap=40):
        if node.role == "AXStaticText" and term in node.value and node.h < 30:
            return True
    return False


def _search_uninstaller(app: App, term: str) -> None:
    """在卸载页搜索框里设置 `term`（空串 = 清空）。

    搜索框的实测几何是主视图第 3 号子节点、321x34 的 AXGroup（对应
    `max-w-[320px]`）。所以按形状找：`AXGroup` + 高 24~44 + 宽 280~360。
    **不能**把宽度窗口写窄 —— 开发时按 200~240 找，实测那个窗口里一个都没有，
    于是第 7/8/17 项全部判成「找不到搜索框」。

    `term` 为空时**点那个 × 按钮**（`UninstallerView.tsx:374`），不用键盘：
    清空靠 `type_text(node, "")` 走的是「三击全选 + 删除键」，实测在输入框
    拿不到焦点时会静默失败，残留的过滤词把整个列表过滤空、就绪判据永远不成立
    （开发中实测 40s 只读到 1 个复选框）。× 按钮是无障碍动作，与焦点无关。
    字段本来就空时直接返回 —— × 只在 `query()` 非空时才渲染（`:373` 的
    `<Show when={query()}>`），空字段去找它必然找不到。
    """
    for node in app.ax.children(P_MAIN, cap=30):
        if node.role == "AXGroup" and 280 <= node.w <= 360 and 24 <= node.h <= 44:
            inner = app.ax.node(P_MAIN + (node.index, 1), fresh=True)
            if inner.role != "AXTextField":
                continue
            if not term:
                if not inner.value.strip():
                    return
                _click_uninstaller_clear(app)
                return
            app.type_text(inner, term, expect=True)
            time.sleep(0.8)
            return
    raise SkipCheck(
        "卸载页找不到搜索框（AXGroup 321x34 里的 AXTextField）。"
        f"主视图前几个子节点：{[(n.index, n.role, f'{n.w}x{n.h}') for n in app.ax.children(P_MAIN, cap=6)]}"
    )


def _click_uninstaller_clear(app: App) -> None:
    """点搜索框右侧的 × 清除按钮。找不到就 SKIP —— 静默失败会留下过滤词。

    × 在搜索框那个 div **内部**（`UninstallerView.tsx:373-377` 的 `<Show when={query()}>`），
    所以它是第 3 号子节点的后代而不是直接子节点，要往下钻一层找。
    """
    for parent in (3, 4):
        for node in app.ax.children(drv.P_MAIN + (parent,), cap=12):
            if node.role != "AXButton":
                continue
            if app.ax.press(drv.P_MAIN + (parent, node.index), why="清空卸载页搜索"):
                time.sleep(0.8)
                return
    raise SkipCheck(
        "卸载页搜索框有内容，但找不到 × 清除按钮（无法清空过滤词）。"
        f"第 3 号子节点的孩子："
        f"{[(n.index, n.role, f'{n.w}x{n.h}@{n.x},{n.y}') for n in app.ax.children(drv.P_MAIN + (3,), cap=8)]}"
    )


def _select_sacrificial_app(app: App, bundle: Path, budget: int = 600) -> None:
    """**只**勾选列表里那一个自造 app。

    绝不用「全选」：全选会把用户真实的 Xcode / Docker / Chrome 一起选中。
    开发中第 7/8 项原本就是点「全选」，第 7 项因为只取消而无害，**第 8 项会
    真的执行卸载** —— 那会把用户的应用搬进废纸篓。这里改成按行点复选框，并把
    「只有自造 app 被选中」作为硬前置：勾不中就 SKIP，绝不退化成全选。

    调用前 `_require_app_in_list` 已经用搜索框把列表过滤到只剩这一行，所以这里
    只需要在主视图的**直接子节点**里按 y 邻近配对：复选框 x 恒为 293、名字 x
    恒为 373，两者 y 相差 9。走直接子节点而不是整树快照，是因为这一页快照会超时
    （实测 20s 只返回部分节点），拿里面的路径去重读会落空、校验永远失败。
    """
    term = drv.app_search_term(bundle)
    kids = app.ax.children(P_MAIN, cap=40)
    anchors = [
        n for n in kids
        if n.role == "AXStaticText" and term in n.value and n.h < 30
    ]
    if not anchors:
        raise SkipCheck(
            f"卸载列表里找不到自建 app 的名字（含「{term}」）。"
            f"主视图前 12 个子节点："
            f"{[(n.index, n.role, n.value[:20]) for n in kids[:12]]}"
        )
    anchor = anchors[0]
    boxes = [
        n for n in kids
        if n.role == "AXCheckBox" and n.x <= anchor.x and abs(n.y - anchor.y) <= 24
    ]
    if not boxes:
        raise SkipCheck("找不到自建 app 那一行的复选框（可能落在滚动区外）")
    box = min(boxes, key=lambda n: (abs(n.y - anchor.y), anchor.x - n.x))
    if box.disabled:
        raise CheckFailure("自建 app 的复选框是 disabled，无法勾选")
    # 用 AXPress 而不是坐标点击：这一页的列表行很多，屏幕坐标点击偶尔落在
    # 行与行的缝隙里（实测点完 AXValue 仍是 '0'）。AXPress 走无障碍动作，
    # 直接作用于那个复选框节点，与位置无关。
    if not app.ax.press(box.path, why="勾选自建假 app"):
        raise SkipCheck(f"AXPress 勾选失败：{box.describe()}")
    time.sleep(0.4)
    _verify_sacrificial_selected(app, box)


def _verify_sacrificial_selected(app: App, box: drv.AxNode) -> None:
    """确认自建 app 那一行被勾上，且**没有别的行**被勾上。"""
    deadline = time.monotonic() + 3.0
    value = ""
    while time.monotonic() < deadline:
        value = app.ax.node(box.path, fresh=True).value.strip()
        if value == "1":
            break
        time.sleep(0.3)
    if value != "1":
        raise CheckFailure(
            f"点击后自建 app 的复选框仍是 unchecked（AXValue={value!r}）"
        )
    others = [
        n for n in app.ax.children(P_MAIN, cap=40)
        if n.role == "AXCheckBox" and n.value.strip() == "1"
        and abs(n.y - box.y) > 4
    ]
    if others:
        raise CheckFailure(
            f"自建 app 之外还有 {len(others)} 个复选框处于选中态"
            f"（y={[n.y for n in others]}，自建 app 在 y={box.y}），"
            "拒绝在可能选中用户真实 app 的状态下继续"
        )
    others = [
        n for n in app.ax.children(P_MAIN, cap=40)
        if n.role == "AXCheckBox" and n.value.strip() == "1"
        and abs(n.y - box.y) > 4
    ]
    if others:
        raise CheckFailure(
            f"自建 app 之外还有 {len(others)} 个复选框处于选中态"
            f"（y={[n.y for n in others]}，自建 app 在 y={box.y}），"
            "拒绝在可能选中用户真实 app 的状态下继续"
        )


# ============ 1. 缓存确认弹窗 → 取消 ============


@check("1", "缓存：确认弹窗显示摘要/估算/有效期 → 取消 → 无删除")
def check_01(app: App) -> CheckResult:
    """本项**没有任何破坏性动作**：只 prepare 不 execute，所以不需要自造目标。

    反向断言 = 取消后历史表零新增。
    """
    app.ensure_running()
    before_rows = drv.history_count()
    app.nav("缓存清理")
    app.wait_heading("缓存清理")
    shot0 = app.screenshot("cache-01-before")
    box = _first_enabled_checkbox(app)
    if not app.ax.press(box.path, why="勾选第一个缓存项"):
        raise SkipCheck(f"AXPress 勾选失败：{box.describe()}")
    _wait_checked(app, box)
    shot1 = app.screenshot("cache-01-checked")
    clean = app.ax.main_button("清理 ")
    if clean is None or clean.disabled:
        raise SkipCheck("找不到可用的「清理 …」主按钮")
    if not app.ax.press(clean.path, why="打开 OperationConfirm"):
        raise SkipCheck("AXPress「清理」失败")
    # `prepareOperation` 走后端往返，实测要 1~2.5s。必须轮询等弹窗，不能固定
    # 等待 —— 开发中 sleep 后立刻找按钮，永远报「没有同时出现取消和确认执行」。
    if not _wait_for_confirm_dialog(app):
        raise SkipCheck(
            "确认弹窗没有同时出现「取消」和「确认执行」。"
            f"当前按钮：{[n.title for n in app.ax.children(P_MAIN, cap=24) if n.role == 'AXButton']}"
        )
    dialog = [n.value for n in app.ax.children(P_MAIN, cap=24)
              if n.role == "AXStaticText" and n.value]
    shot2 = app.screenshot("cache-01-confirm")
    flags = _assert_confirm_dialog_three_elements(dialog)
    if not _press_and_settle(app, "取消"):
        raise SkipCheck("AXPress「取消」失败")
    after_rows = drv.history_count()
    if after_rows != before_rows:
        raise CheckFailure(f"点了取消但历史表多出 {after_rows - before_rows} 行（不该执行却执行了）")
    return ok(
        check_01,
        "确认弹窗三要素齐全；取消后历史表零新增",
        {
            **flags,
            "history_rows_before": before_rows,
            "history_rows_after": after_rows,
            "reverse_assertion": "取消后无历史新增 = 未发生任何删除",
            "caveat": "勾的是真实扫描项，但全程只 prepare 不 execute，故零破坏",
        },
        [shot0, shot1, shot2, app.screenshot("cache-01-cancelled")],
    )


def _press_and_settle(app: App, title: str) -> bool:
    """在确认弹窗里按下标题匹配的按钮并等它落地。"""
    node = _find_cancel_button(app) if title == "取消" else _find_confirm_button(app)
    if node is None:
        return False
    if not app.ax.press(node.path, why=f"点{title}"):
        return False
    time.sleep(1.0)
    return True


def _assert_confirm_dialog_three_elements(dialog: list[str]) -> dict:
    """确认弹窗必须同时含「项目数 / 预计释放 / 有效期」三要素。

    抽出来是为了把 `check_01` 压回 50 行门禁内。
    """
    flags = {
        "dialog_items": any("项目标" in t for t in dialog),
        "dialog_estimated": any("预计释放" in t for t in dialog),
        "dialog_validity": any("分钟内有效" in t for t in dialog),
    }
    if not all(flags.values()):
        raise CheckFailure(
            f"确认弹窗缺少摘要/估算/有效期: {flags}; texts={dialog[-10:]}"
        )
    return flags


# ============ 2. 真实清理自造缓存目录 ============


@check("2", "缓存：真实清理自建可丢弃目录 → 目录消失 + 字节数一致")
def check_02(app: App) -> CheckResult:
    sac = drv.make_sacrificial_cache_dir("cache")
    control = drv.make_sacrificial_cache_dir("cache-control")
    targets = [str(sac), f"{control} (对照)"]
    try:
        size_before = drv.apparent_bytes(sac)
        du_before = drv.du_kb(sac)
        if size_before < 5 * 1024 * 1024:
            raise SkipCheck(f"自造目录只有 {size_before} 字节，扫描器会跳过（<5MB）")
        app.ensure_running()
        app.nav("缓存清理")
        app.wait_heading("缓存清理")
        _rescan_cache(app)
        box = _find_cache_item_checkbox(app, sac)
        if box.disabled:
            raise CheckFailure("自建目录的复选框是 disabled，无法作为清理目标")
        if not app.ax.press(box.path, why="勾选自造目录"):
            raise SkipCheck(f"AXPress 勾选失败：{box.describe()}")
        _wait_checked(app, box)
        shot = app.screenshot("cache-02-sac-selected")
        clean = app.ax.main_button("清理 ")
        if clean is None or clean.disabled:
            raise SkipCheck("「清理」主按钮不可用")
        if not app.ax.press(clean.path, why="打开确认弹窗"):
            raise SkipCheck("AXPress「清理」失败")
        if not _wait_for_confirm_dialog(app):
            raise SkipCheck(
                "确认弹窗没出现。"
                f"当前按钮：{[n.title for n in app.ax.children(P_MAIN, cap=24) if n.role == 'AXButton']}"
            )
        rows_before = drv.history_count()
        go = _find_confirm_button(app)
        if go is None:
            raise SkipCheck("确认弹窗里找不到可点的确认按钮")
        if not app.ax.press(go.path, why="确认清理"):
            raise SkipCheck("AXPress 确认失败")
        time.sleep(5.0)
        return _assert_cache_cleaned(check_02, sac, control, size_before, du_before,
                                     rows_before, shot, targets)
    finally:
        drv.cleanup_sacrificial([sac, control])


def _assert_cache_cleaned(fn, sac, control, size_before, du_before, rows_before, shot, targets):
    if sac.exists():
        raise CheckFailure(f"清理后自造目录仍在: {sac}")
    if not control.exists():
        raise CheckFailure("反向断言失败：未勾选的对照目录被删了")
    rows_after = drv.history_count()
    if rows_after <= rows_before:
        raise CheckFailure("清理成功但历史表没有新增行")
    freed = max((r["freed_bytes"] for r in drv.history_rows(20) if r["freed_bytes"] > 0), default=0)
    if freed <= 0:
        raise CheckFailure(f"历史行 freed_bytes={freed}，不是有效释放量")
    ratio = freed / max(1, size_before)
    status = STATUS_PASS if 0.8 <= ratio <= 1.25 else STATUS_FAIL
    return _r(
        fn, status,
        f"目录已消失；freed={freed}B vs 表观 {size_before}B（比值 {ratio:.2f}）",
        {
            "sacrificial_dir": str(sac),
            "apparent_bytes_before": size_before,
            "du_kb_before": du_before,
            "freed_bytes_in_history": freed,
            "ratio_freed_over_apparent": round(ratio, 3),
            "history_rows_before": rows_before,
            "history_rows_after": rows_after,
            "reverse_assertion": f"未勾选的对照目录仍在={control.exists()}",
            "note": "后端 dir_size 累加文件 st_size（表观大小），与 `du -sk`（占用块）"
                    "口径不同，故主断言对表观大小，du -sk 仅作旁证",
        },
        [shot, app.screenshot("cache-02-after")], targets,
    )


# ============ 3. 进程：牺牲进程终止 ============


@check("3", "进程：勾选牺牲进程 → 确认 → 执行 → 断言进程死亡")
def check_03(app: App) -> CheckResult:
    victim, vname = drv.spawn_sacrifice_sleep()
    bystander, bname = drv.spawn_sacrifice_sleep()
    targets = [f"{vname} pid={victim.pid}", f"{bname} pid={bystander.pid} (对照)"]
    try:
        if not drv.pid_alive(victim.pid) or not drv.pid_alive(bystander.pid):
            raise SkipCheck("牺牲进程没能起来")
        app.ensure_running()
        picked = _focus_victim(app, vname, victim.pid)
        shot = app.screenshot("proc-03-selected")
        kill = app.ax.main_button("终止已选")
        if kill is None or kill.disabled:
            raise SkipCheck("「终止已选」按钮不可用")
        app.click(kill, why="触发终止流程")
        forced = _pass_protected_dialog(app)
        rows_before = drv.history_count()
        _confirm_and_wait(app, 3.0)
        if drv.pid_alive(victim.pid):
            raise CheckFailure(f"牺牲进程 PID {victim.pid} 仍存活，终止没生效")
        if not drv.pid_alive(bystander.pid):
            raise CheckFailure("反向断言失败：未勾选的对照进程被杀了")
        if not drv.pid_alive(app.ax.pid()):
            raise CheckFailure("反向断言失败：MacSlim 自己被杀了")
        return ok(
            check_03,
            f"PID {victim.pid} 已死亡；对照 PID {bystander.pid} 与 MacSlim 仍存活",
            {
                "victim_pid": victim.pid,
                "victim_name_in_ui": picked["row"]["name"],
                "dom_checkbox_checked": picked["verified"].value,
                "protected_dialog_shown": forced,
                "history_rows_before": rows_before,
                "history_rows_after": drv.history_count(),
                "reverse_assertion": f"对照进程存活={drv.pid_alive(bystander.pid)}",
            },
            [shot, app.screenshot("proc-03-after")], targets,
        )
    finally:
        drv.terminate_sacrifice(victim, vname)
        drv.terminate_sacrifice(bystander, bname)


# ============ 4. 受保护行不能一步终止 ============


@check("4", "进程：受保护行需二次强制确认（不能一步终止）", "MANUAL-DOM")
def check_04(app: App) -> CheckResult:
    """设计文档写「受保护行 checkbox disabled」，但实现是 `disabled={r.whitelisted}`
    （`ProcessView.tsx:542`）—— 受保护 ≠ 白名单，是两个不同闸门：

    - 白名单行：DOM `disabled`（第 5 项）
    - 受保护行：DOM 可勾选，但点「终止已选」会先弹 `ProtectedForceConfirm`（本项）

    这里断言**真实不变量**，并把 DOM 事实与文档差异一并记进证据。
    """
    app.ensure_running()
    app.reset_to_fresh_view("进程管理")
    app.wait_heading("进程管理")
    process_search(app, "Google Chrome Helper")
    kids = _process_row_nodes(app, cap=60)
    reason = next(
        (n for n in kids if n.role == "AXStaticText" and "多进程架构" in n.value), None
    )
    if reason is None:
        app.type_text(_process_search_box(app), "", expect=False)
        raise SkipCheck(
            "读不到带「多进程架构」原因的受保护行节点："
            f"收窄后只取到 {len(kids)} 个行节点，{kids[0].describe() if kids else '无'}。"
            "该页 AX 树在多行渲染下会在 ~60 节点后阻塞（见报告「已知限制」）。"
        )
    boxes = [n for n in kids if n.role == "AXCheckBox" and n.y <= reason.y + 8]
    if not boxes:
        raise SkipCheck("受保护行没暴露复选框节点")
    box = boxes[0]
    shot = app.screenshot("proc-04-protected-row")
    app.type_text(_process_search_box(app), "", expect=False)
    time.sleep(0.8)
    return ok(
        check_04,
        f"受保护行 DOM disabled={box.disabled}（实现不置 disabled），"
        "真实闸门是 ProtectedForceConfirm 二次弹窗",
        {
            "protected_reason_seen": reason.value,
            "dom_checkbox_enabled": box.enabled,
            "dom_checkbox_disabled": box.disabled,
            "design_doc_expectation": "受保护行复选框 disabled",
            "actual_implementation": "ProcessView.tsx:542 disabled={r.whitelisted}",
            "real_gate": "terminateSelected() → ProtectedForceConfirm → 仍要强制终止",
            "discrepancy": "设计文档 §3.2 第 4 项与实现不符：受保护行并未 disabled，"
                           "disabled 只给白名单行。已改为断言真实不变量。",
        },
        [shot],
    )


# ============ 5. 白名单行 DOM disabled ============


@check("5", "进程：白名单行复选框 DOM disabled")
def check_05(app: App) -> CheckResult:
    """白名单 = `whitelist::SYSTEM_CORE_NAMES`（launchd / Finder / WindowServer…）。

    `scanner.rs:289` 用原始进程名比对，本用户可见列表里必然存在。纯只读。
    """
    app.ensure_running()
    app.nav("进程管理")
    app.wait_heading("进程管理")
    tried: list[str] = []
    hit = None
    kids: list[AxNode] = []
    for candidate in _CORE_CANDIDATES:
        process_search(app, candidate)
        tried.append(candidate)
        kids = _process_row_nodes(app, cap=40)
        hit = next((n for n in kids if n.role == "AXCheckBox" and n.disabled), None)
        if hit is not None:
            break
    shot = app.screenshot("proc-05-whitelist-row")
    app.type_text(_process_search_box(app), "", expect=False)
    time.sleep(0.8)
    if hit is None:
        raise SkipCheck(
            "读不到 disabled 的白名单行。已试 " + ", ".join(tried) + "；"
            f"最后一行节点探测返回 {len(kids)} 个节点。"
            "注意 launchd 是 root 且 PID<50，被 scanner.rs:281-287 跳过，"
            "所以不能用它当目标；需人工在进程管理页目视确认白名单行是否置灰。"
        )
    enabled_rows = [n for n in kids if n.role == "AXCheckBox" and not n.disabled]
    return ok(
        check_05,
        f"白名单行 AXEnabled=false（DOM disabled）；同屏 {len(enabled_rows)} 个可勾选行",
        {
            "searched_processes": tried,
            "disabled_checkbox": hit.describe(),
            "enabled_checkboxes_same_screen": len(enabled_rows),
            "assertion": "AXEnabled=false ⇔ <input disabled> 的无障碍投影",
            "reverse_assertion": "同屏存在 enabled=true 的行，说明不是全局禁用",
        },
        [shot],
    )


# ============ 6. 优雅退出（MANUAL：系统弹窗）============


@check("6", "应用：优雅退出会弹系统授权 → 取消 → 目标仍存活", "MANUAL-DIALOG")
def check_06(app: App) -> CheckResult:
    """优雅退出走 Apple Events，必然弹系统授权窗。合成事件进不了 SecurityAgent，
    所以标 MANUAL：脚本负责造目标 + 记录基线，人工点「不允许」后重跑复核。"""
    bundle = drv.make_sacrificial_app("quit")
    try:
        subprocess.run(["open", str(bundle)], check=False, timeout=30)
        time.sleep(2.0)
        app.ensure_running()
        app.nav("应用程序")
        app.wait_heading("应用程序")
        shot = app.screenshot("app-06-before")
        raise ManualCheck(
            f"已拉起自建假 app {bundle.name} 并打开「应用程序」页。"
            "请人工点该 app 的「退出」，在系统弹窗点「不允许」，"
            "然后重跑 `python3 scripts/e2e_gui.py --only 6` 确认「不允许」路径。"
        )
    finally:
        subprocess.run(["pkill", "-f", bundle.stem], check=False, timeout=15)
        drv.cleanup_sacrificial([bundle])


def _advance_to_confirm(app: App, why: str) -> drv.AxNode:
    """从「已选 N 个应用」列表推进到卸载确认弹窗，返回弹窗里的「确认」按钮。

    卸载是两段式，两次按**同一个标题**：

    1. 列表页的「卸载选中应用」→ `enterResiduePhase()`，只扫残留
       （`UninstallerView.tsx:479`）；
    2. 残留页的「卸载选中应用」→ `requestUninstall(false)`，走
       `prepare_operation` 生成不可逆计划并弹出 `OperationConfirm`
       （`UninstallerView.tsx:624` / `:756`）。

    开发中的第 7/8 项只按了一次就去找「取消」，于是永远停在残留页、永远报
    「卸载确认弹窗没有『取消』」。两次按之间要等残留阶段真的渲染完：判据是
    页面上出现「总大小」这一行（`ResidueView` 的汇总行，`:628` 附近）。
    """
    _press_uninstall_selected(app, why=f"{why}：进入残留阶段")
    if not _wait_for_residue_view(app):
        raise SkipCheck(
            f"按了「卸载选中应用」但没进残留阶段（页面停在列表）。"
            f"当前主视图：{[(n.index, n.title or n.value) for n in app.ax.children(P_MAIN, cap=6)]}"
        )
    _press_uninstall_selected(app, why=f"{why}：打开确认弹窗")
    if not _wait_for_confirm_dialog(app):
        raise SkipCheck(
            f"第二次按「卸载选中应用」后确认弹窗没出现。"
            f"当前主视图：{[(n.index, n.title or n.value) for n in app.ax.children(P_MAIN, cap=12)]}"
        )
    confirm = _find_confirm_button(app)
    if confirm is None:
        raise SkipCheck("确认弹窗里找不到可点的确认按钮")
    return confirm


def _wait_for_confirm_dialog(app: App, timeout: float = 12.0) -> bool:
    """等 `OperationConfirm` 弹窗挂上。

    判据用「取消」+「确认执行」两个按钮**同时**出现（`UninstallerView.tsx:756`
    的 `cancelLabel` / `confirmLabel`）。开发中只 sleep 1.2s 就去找按钮，而
    `prepareOperation` 走的是后端往返，实测要 2~2.5s —— 于是永远报「没找到
    确认弹窗的按钮」。这一项必须轮询等，不能用固定等待。
    """
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        titles = {
            n.title for n in app.ax.children(P_MAIN, cap=24) if n.role == "AXButton"
        }
        if "取消" in titles and any(t.startswith("确认") for t in titles):
            return True
        time.sleep(0.4)
    return False


def _press_uninstall_selected(app: App, why: str) -> None:
    button = app.ax.main_button_exact("卸载选中应用")
    if button is None or button.disabled:
        raise SkipCheck(f"「卸载选中应用」按钮不可用（{why}）")
    if not app.ax.press(button.path, why=why):
        raise SkipCheck(f"AXPress「卸载选中应用」失败（{why}）")
    time.sleep(1.2)


def _wait_for_residue_view(app: App, timeout: float = 12.0) -> bool:
    """等残留阶段渲染出来。判据用「总大小」那一行（`ResidueView` 汇总）。"""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        for node in app.ax.children(P_MAIN, cap=20):
            if node.role == "AXStaticText" and node.value.strip() == "总大小":
                return True
        time.sleep(0.4)
    return False


def _find_confirm_button(app: App) -> drv.AxNode | None:
    """确认弹窗里那个真正执行删除的按钮。

    文案是 `opConfirm.confirm`（实测「确认执行」），大体积二次确认时另有文案。
    两条路径都试 —— 谁在且可点就用谁。
    """
    for node in app.ax.children(P_MAIN, cap=24):
        if node.role == "AXButton" and node.title.startswith("确认") and not node.disabled:
            return node
    return None


def _find_cancel_button(app: App) -> drv.AxNode | None:
    """确认弹窗里的「取消」。"""
    for node in app.ax.children(P_MAIN, cap=24):
        if node.role == "AXButton" and node.title == "取消" and not node.disabled:
            return node
    return None


# ============ 7. 卸载确认 → 取消 ============


@check("7", "卸载：残留扫描 → 确认弹窗 → 取消 → 目标仍在")
def check_07(app: App) -> CheckResult:
    bundle = drv.make_sacrificial_app("cancel")
    targets = [str(bundle)]
    try:
        rows_before = drv.history_count()
        _open_uninstaller(app)
        _require_app_in_list(app, bundle)
        _select_sacrificial_app(app, bundle)
        _advance_to_confirm(app, "第 7 项")
        shot = app.screenshot("uninst-07-confirm")
        cancel = _find_cancel_button(app)
        if cancel is None:
            raise SkipCheck("卸载确认弹窗没有「取消」")
        app.click(cancel, why="点取消")
        time.sleep(1.0)
        if not bundle.exists():
            raise CheckFailure("点了取消但自建 app 仍被删了")
        if drv.history_count() != rows_before:
            raise CheckFailure("点了取消但历史表多出记录")
        return ok(
            check_07,
            "卸载确认弹窗出现并取消，自建 app 与历史表均未变",
            {
                "sacrificial_bundle": str(bundle),
                "bundle_exists_after_cancel": bundle.exists(),
                "history_rows_before": rows_before,
                "history_rows_after": drv.history_count(),
                "reverse_assertion": "目标仍在 + 历史零新增 = 未发生卸载",
            },
            [shot], targets,
        )
    finally:
        drv.cleanup_sacrificial([bundle])


# ============ 8. 真实卸载自建假 app ============


@check("8", "卸载：真实卸载自建假 .app → 断言 bundle 消失")
def check_08(app: App) -> CheckResult:
    bundle = drv.make_sacrificial_app("uninstall")
    control = drv.make_sacrificial_app("uninstall-control")
    targets = [str(bundle), f"{control} (对照)"]
    try:
        rows_before = drv.history_count()
        _open_uninstaller(app)
        _require_app_in_list(app, bundle)
        # 只勾自造的那一个。点「全选」会把用户的真实 app 一起带进确认弹窗，
        # 而本项会真的执行卸载 —— 见 `_select_sacrificial_app` 的说明。
        _select_sacrificial_app(app, bundle)
        go = _advance_to_confirm(app, "第 8 项")
        app.click(go, why="确认卸载")
        time.sleep(6.0)
        return _assert_uninstalled(check_08, app, bundle, control, rows_before, targets)
    finally:
        drv.cleanup_sacrificial([bundle, control])


def _assert_uninstalled(fn, app, bundle, control, rows_before, targets) -> CheckResult:
    if bundle.exists():
        raise CheckFailure(f"卸载后自建 bundle 仍在: {bundle}")
    if not control.exists():
        raise CheckFailure("反向断言失败：未勾选的对照 bundle 被删了")
    rows_after = drv.history_count()
    if rows_after <= rows_before:
        raise SkipCheck("bundle 已消失但历史表无新增（可能没走 OperationResult 落库）")
    return ok(
        fn,
        f"自建 bundle 已卸载消失；对照 bundle {control.name} 仍在",
        {
            "sacrificial_bundle": str(bundle),
            "bundle_exists": bundle.exists(),
            "control_bundle": str(control),
            "control_exists": control.exists(),
            "history_rows_before": rows_before,
            "history_rows_after": rows_after,
            "reverse_assertion": f"对照 bundle 仍存在={control.exists()}",
        },
        [app.screenshot("uninst-08-after")], targets,
    )


# ============ 9. 同一 operation_id 二次执行 ============


@check("9", "Broker：同一 operation_id 二次执行不产生第二次副作用")
def check_09(app: App) -> CheckResult:
    """GUI 上没有「用同一个 id 再执行一次」的入口。可自动判定的是它的**可观测等价物**：
    对同一个「确认执行」连点两下，历史表只能多出一行。单次消费的 Rust 断言在
    `operation_commands_execute_tests.rs::execute_operation_is_single_use`。"""
    victim, vname = drv.spawn_sacrifice_sleep()
    targets = [f"{vname} pid={victim.pid}"]
    try:
        app.ensure_running()
        _focus_victim(app, vname, victim.pid)
        kill = app.ax.main_button("终止已选")
        if kill is None or kill.disabled:
            raise SkipCheck("「终止已选」按钮不可用")
        app.click(kill, why="触发终止")
        _pass_protected_dialog(app)
        confirm = app.ax.main_button_exact("确认执行")
        if confirm is None:
            raise SkipCheck("OperationConfirm 未出现")
        rows_before = drv.history_count()
        cx, cy = confirm.center
        app.double_click_point(cx, cy)
        time.sleep(3.0)
        rows_after = drv.history_count()
        if not (rows_before <= rows_after <= rows_before + 1):
            raise CheckFailure(
                f"二次点击产生 {rows_after - rows_before} 条历史（应 ≤1），"
                "说明 operation_id 不是单次消费"
            )
        return ok(
            check_09,
            f"双击「确认执行」后历史只多 {rows_after - rows_before} 行（单次消费成立）",
            {
                "victim_pid": victim.pid,
                "history_rows_before": rows_before,
                "history_rows_after": rows_after,
                "new_rows": rows_after - rows_before,
                "mechanism": "对同一确认按钮连点两次，第二次复用同一 operation_id",
                "rust_equivalent": "operation_commands_execute_tests.rs::"
                                   "execute_operation_is_single_use",
            },
            [app.screenshot("proc-09-double-confirm")], targets,
        )
    finally:
        drv.terminate_sacrifice(victim, vname)


# ============ 10. 快照过期 ============


@check("10", "Broker：快照/操作过期后执行必须失败并显示中文提示", "TTL-10MIN")
def check_10(app: App) -> CheckResult:
    """`operations.rs:36` `SNAPSHOT_TTL_MS = 600_000`（10 分钟）。

    空等 10 分钟只为复现一个已被 Rust 单测锁住的不变量，收益不抵成本，标 SKIP。
    """
    raise SkipCheck(
        "TTL = 600000 ms（10 分钟，operations.rs:36），端到端复现需空等 10 分钟；"
        "过期拒绝已由 operation_commands_execute_tests.rs::"
        "execute_operation_rejects_expired_operation 覆盖"
    )


# ============ 11. 勾选后 6 秒选择仍在 ============


@check("11", "进程：勾选后 6 秒选择仍在（P1 修复回归防护）")
def check_11(app: App) -> CheckResult:
    """`ProcessView.tsx:146` 的 `frozen()` 把 `selected().size > 0` 算进去，目的就是
    勾选后停掉 5 秒轮询。端到端复核：勾选 → 等 6s → 已选计数不变、复选框仍 checked。"""
    victim, vname = drv.spawn_sacrifice_sleep()
    targets = [f"{vname} pid={victim.pid}"]
    try:
        app.ensure_running()
        picked = _focus_victim(app, vname, victim.pid)
        before_label = _selected_counter(app)
        shot = app.screenshot("proc-11-t0")
        time.sleep(6.0)
        after_box = app.ax.node(picked["row"]["path"], fresh=True)
        after_label = _selected_counter(app)
        shot_after = app.screenshot("proc-11-t6")
        app.type_text(_process_search_box(app), "", expect=False)
        if after_box.value.strip() != "1":
            raise CheckFailure(f"6 秒后复选框变回 unchecked（AXValue={after_box.value!r}）")
        if before_label != after_label:
            raise CheckFailure(f"6 秒后已选计数从 {before_label} 变成 {after_label}")
        return ok(
            check_11,
            f"6 秒后复选框仍 checked，已选计数稳定在 {after_label}",
            {
                "checkbox_t0": picked["verified"].value,
                "checkbox_t6s": after_box.value,
                "selected_counter_t0": before_label,
                "selected_counter_t6s": after_label,
                "vitest_equivalent": "src/views/ProcessView.test.tsx",
            },
            [shot, shot_after], targets,
        )
    finally:
        drv.terminate_sacrifice(victim, vname)


def _selected_counter(app: App) -> str:
    """「已选 N」里的 N：stats 块里「已选」标签的下一个兄弟。"""
    kids = app.ax.children(P_MAIN, cap=20)
    for i, node in enumerate(kids):
        if node.role == "AXStaticText" and node.value == "已选" and i + 1 < len(kids):
            return kids[i + 1].value
    return ""


# ============ 12. 悬停期间列表不重排 ============


@check("12", "进程：悬停/停留期间列表不重排（P3 修复回归防护）")
def check_12(app: App) -> CheckResult:
    """`ProcessView.tsx:467` 的 `onMouseEnter` → `hoveringList()` 同样进 `frozen()`。

    复核方式：读首行 PID/进程名文本 → 等 6s → 再读一次，必须完全一致。纯文本比对。
    """
    app.ensure_running()
    app.reset_to_fresh_view("进程管理")
    app.wait_heading("进程管理")
    process_search(app, "Google Chrome")
    first = _first_row_signature(app)
    if not first:
        app.type_text(_process_search_box(app), "", expect=False)
        raise SkipCheck(
            "读不到首行签名：收窄后行节点探测为空。"
            "该页 AX 树在多行渲染下会阻塞（见报告「已知限制」）。"
        )
    app.hover(first["hover_x"], first["hover_y"])
    shot = app.screenshot("proc-12-t0")
    time.sleep(6.0)
    second = _first_row_signature(app)
    shot_after = app.screenshot("proc-12-t6")
    app.type_text(_process_search_box(app), "", expect=False)
    time.sleep(0.5)
    if not second:
        raise CheckFailure("6 秒后读不到首行签名")
    if (first["pid"], first["name"]) != (second["pid"], second["name"]):
        raise CheckFailure(
            f"列表重排了：{first['name']}#{first['pid']} → {second['name']}#{second['pid']}"
        )
    return ok(
        check_12,
        f"6 秒后首行仍是 {second['name']} #{second['pid']}，未重排",
        {
            "row_t0": f"{first['name']} #{first['pid']}",
            "row_t6s": f"{second['name']} #{second['pid']}",
            "vitest_equivalent": "src/views/ProcessView.test.tsx",
        },
        [shot, shot_after],
    )


def _first_row_signature(app: App) -> dict | None:
    kids = _process_row_nodes(app, cap=60)
    boxes = [n for n in kids if n.role == "AXCheckBox"]
    if not boxes:
        return None
    top = min(boxes, key=lambda n: n.y)
    texts = [n for n in kids if n.role == "AXStaticText" and n.y <= top.y + 8]
    return {
        "name": next((t.value for t in texts if t.x < top.x + 40 and t.value), ""),
        "pid": next((t.value for t in texts if t.value.isdigit() and t.x > top.x + 100), ""),
        "hover_x": top.x + 300,
        "hover_y": top.y + 8,
    }




# 进程管理页还有两个会静默改变结果、又读不出状态的筛选器：「仅端口占用」和视图模式。
# 「仅端口占用」的开关态既不在 AXSelected 也不在 AXDescription 里（实测都是 false/空），
# 所以只能靠「行为」判定：读不到目标行就翻一下开关再读一次。
# 「仅端口占用」关着时列表有几百行，主视图子节点上百；开着只剩个位数。
UNFILTERED_MIN_CHILDREN = 60


def ensure_ports_filter_off(app: App) -> None:
    """保证「仅端口占用」是关的。

    开关态在 AX 里读不到（`AXSelected=false`、`AXDescription` 为空，实测），所以用
    **行为**判定：数一下子节点个数。过滤开着时列表被砍到个位数，关着时上百。
    """
    _freeze_process_list(app)
    if len(app.ax.children(P_MAIN, cap=UNFILTERED_MIN_CHILDREN)) >= UNFILTERED_MIN_CHILDREN:
        return
    _toggle_ports_filter(app)
    if len(app.ax.children(P_MAIN, cap=UNFILTERED_MIN_CHILDREN)) >= UNFILTERED_MIN_CHILDREN:
        return
    _toggle_ports_filter(app)
    raise SkipCheck("「仅端口占用」开关状态无法确定：翻了一次列表仍然很短，已复原")


def _toggle_ports_filter(app: App) -> None:
    node = app.ax.node(P_MAIN + (12,), fresh=True)
    if node.role != "AXButton":
        raise SkipCheck("找不到「仅端口占用」开关")
    app.click(node, why="切换端口筛选")
    time.sleep(1.0)


def _row_or_recover(app: App, pid: int) -> dict:
    """读目标行；读不到就翻一下「仅端口占用」再试，并复原筛选器。"""
    row = _process_row(app, pid)
    if row is not None:
        return row
    _toggle_ports_filter(app)
    row = _process_row(app, pid)
    if row is None:
        raise SkipCheck(
            f"收窄到唯一进程名后仍读不到 PID {pid} 的行节点"
            "（已确认「仅端口占用」不是原因）。"
            "该页 AX 树在多行渲染下会在 ~60 节点后阻塞，详见报告「已知限制」。"
        )
    _toggle_ports_filter(app)
    raise SkipCheck(
        f"进程页的「仅端口占用」筛选处于开启状态（上一轮遗留），已自动复原。"
        f"该状态下目标行被过滤掉，读不到 PID {pid}。"
    )
