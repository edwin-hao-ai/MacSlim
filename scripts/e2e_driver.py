#!/usr/bin/env python3
"""MacSlim e2e 门禁的观察通道与窗口驱动（`scripts/e2e_gui.py` 的底层）。

三件事：

1. **AX 探针** —— 把 WKWebView 的 DOM 状态读出来。
   `<input disabled>` → `AXCheckBox` + `AXEnabled=false`；`checked` → `AXValue=0/1`。
   这是 DOM 派生属性，不是像素采样（docs/…-release-gate.md §3.1）。
2. **窗口驱动** —— `open` 启动、置顶、坐标归一化、`cliclick` 点击 / 打字、
   `screencapture` 抓图（截图只作人工复核产物，不参与断言）。
   **所有点击坐标都来自实时 AX 几何**，全文件没有一个魔法坐标常量。
3. **牺牲目标工厂** —— 破坏性目标一律脚本自造（§3.4）。

## 为什么要拆文件 + 一次 osascript 只做一个 AX 操作

进程管理页的 AX 子节点实测 3900+ 个（`count of UI elements of main` = 3924）。
在同一 AppleScript 进程里连做多个 AX 取值会互相拖死，而 `UI elements of main`
这种批量物化更会永不返回。所以：

- 轻量视图用**整树快照**（`walk_snapshot`），逐节点 flush 到文件，超时也留部分结果；
- 进程管理页用**路径节点**（`node` / `children`），一次 `osascript` 取一个节点，
  按下标逐个下钻（`UI element i of X` 是 O(1)，不会物化整棵树）。

只读「角色真正拥有」的 attribute 也是硬要求：读一个该角色不存在的 attribute 会让
AX 调用阻塞到 messaging timeout（实测 6s/次），整树遍历会从 0.5s 变成几十秒。
"""
from __future__ import annotations

import os
import shutil
import signal
import random
import sqlite3
import subprocess
import tempfile
import time
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

REPO = Path(__file__).resolve().parents[1]
ARTIFACTS = REPO / "artifacts" / "e2e"
OSA_DIR = ARTIFACTS / ".osa"

APP_PROCESS = "macslim"
DEBUG_APP = Path.home() / ".cargo/shared-target/debug/bundle/macos/MacSlim.app"
DB_PATH = Path.home() / "Library/Application Support/MacSlim/macslim.db"

SAC_CACHE_PREFIX = "macslim-e2e-sac-"
SAC_APP_PREFIX = "MacSlimE2E-"
SAC_APPS_DIR = Path.home() / "Applications"

# 窗口归一化目标坐标。每次交互前都重设 —— 开发中真实踩过「窗口实际在 (281,138)
# 却按 (270,135) 算点击点，11pt 偏移导致全部落空」。
NORMAL_POS = (60, 60)
NORMAL_SIZE = (900, 600)

# AX 路径前缀（实测 window → group → group → scrollArea → webArea）
P_WEB = (1, 1, 1, 1)
P_SIDEBAR = P_WEB + (1,)
P_MAIN = P_WEB + (2,)

NAV_LABELS = (
    "智能扫描",
    "进程管理",
    "应用程序",
    "缓存清理",
    "应用卸载",
    "历史记录",
    "设置",
)

# 承载系统模态对话框的进程。它们在前台时必然盖住 MacSlim 窗口，且会静默吃掉
# 所有合成点击 —— 见 `App.assert_no_overlay`。
OVERLAY_PROCESSES = frozenset({"UserNotificationCenter", "SecurityAgent", "CoreServicesUIAgent"})

LIGHT_VIEW_BUDGET = 260

# ============ 异常 ============


class SkipCheck(Exception):
    """环境不满足，无法自动判定。必须带原因。"""


class ManualCheck(Exception):
    """需要人工介入（系统弹窗 / 原生拖动 / 合成事件驱不动的区域）。"""


class CheckFailure(Exception):
    """断言失败。"""


# ============ 子进程 ============


def run(
    argv: list[str], timeout: float = 30.0, cwd: Path | None = None
) -> subprocess.CompletedProcess:
    return subprocess.run(
        argv, capture_output=True, text=True, timeout=timeout, cwd=str(cwd) if cwd else None
    )


def osa(script: str, *args: str, timeout: float = 12.0) -> str:
    """跑一段 AppleScript，返回 stdout。失败或超时返回 ""，永不抛异常。"""
    path = _asa_file(script)
    try:
        proc = subprocess.run(
            ["osascript", str(path), *[str(a) for a in args]],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        return ""
    finally:
        path.unlink(missing_ok=True)
    return proc.stdout.strip() if proc.returncode == 0 else ""


def _asa_file(script: str) -> Path:
    """把 AppleScript 文本落成临时文件并返回路径。调用方负责删除。"""
    OSA_DIR.mkdir(parents=True, exist_ok=True)
    path = OSA_DIR / f"run-{uuid.uuid4().hex[:8]}.applescript"
    path.write_text(script, encoding="utf-8")
    return path


def _parse_poll_text(payload: str) -> list[tuple[float, str]]:
    """解析常驻采样通道返回的 payload。

    每条记录一行：`<秒>|<ROLE=value;ROLE=value;…>`。节点之间用 `;` 分隔而不是
    换行 —— 换行会让一条记录跨多行，按行解析就会把秒数与 payload 错位（实测
    采样数从 60 掉到 9，最后直接归零）。
    """
    samples: list[tuple[float, str]] = []
    for line in payload.splitlines():
        head, sep, body = line.partition("|")
        if not sep:
            continue
        try:
            samples.append((float(head), body))
        except ValueError:
            continue
    return samples


# ============ AppleScript ============

_ASA_NODE = r"""
on run argv
	set procName to (item 1 of argv)
	set out to ""
	tell application "System Events"
		set nodeRef to window 1 of process procName
		repeat with i from 2 to (count of argv)
			set nodeRef to UI element ((item i of argv) as integer) of nodeRef
		end repeat
		set out to (my axAttr(nodeRef, "AXRole")) & tab & (my axPair(nodeRef, "AXPosition")) & tab & (my axPair(nodeRef, "AXSize")) & tab & (my axAttr(nodeRef, "AXTitle")) & tab & (my axAttr(nodeRef, "AXValue")) & tab & (my axAttr(nodeRef, "AXEnabled")) & tab & (my axAttr(nodeRef, "AXSelected")) & tab & (my axAttr(nodeRef, "AXFocused")) & tab & (my axAttr(nodeRef, "AXDescription"))
	end tell
	return out
end run

on axAttr(nodeRef, attrName)
	set outVal to "?"
	tell application "System Events"
		try
			set raw to value of attribute attrName of nodeRef
			if raw is missing value then
				set outVal to ""
			else
				set outVal to raw as text
			end if
		on error
			set outVal to "?"
		end try
	end tell
	return outVal
end axAttr

on axPair(nodeRef, attrName)
	set outPair to ","
	tell application "System Events"
		try
			set raw to value of attribute attrName of nodeRef
			if raw is not missing value then set outPair to (item 1 of raw as text) & "," & (item 2 of raw as text)
		on error
			set outPair to "?"
		end try
	end tell
	return outPair
end axPair
"""

_ASA_WALK = r"""
on run argv
	set maxDepth to (item 1 of argv) as integer
	set budget to (item 2 of argv) as integer
	set procName to (item 3 of argv)
	set outPath to (item 4 of argv)
	set buf to ""
	do shell script ": > " & quoted form of outPath
	tell application "System Events"
		set buf to my walkAX(window 1 of process procName, 0, maxDepth, budget, 0, buf, outPath)
	end tell
	if buf is not "" then do shell script "printf '%s' " & quoted form of buf & " >> " & quoted form of outPath
	return "n=" & (length of buf)
end run

on axGet(nodeRef, attrName)
	set outVal to ""
	tell application "System Events"
		try
			set raw to value of attribute attrName of nodeRef
			if raw is not missing value then set outVal to raw as text
		end try
	end tell
	return outVal
end axGet

on axPair(nodeRef, attrName)
	set outPair to ","
	tell application "System Events"
		try
			set raw to value of attribute attrName of nodeRef
			if raw is not missing value then set outPair to (item 1 of raw as text) & "," & (item 2 of raw as text)
		end try
	end tell
	return outPair
end axPair

on walkAX(nodeRef, lvl, maxDepth, budget, n, buf, outPath)
	if n >= budget then return buf
	set roleTxt to my axGet(nodeRef, "AXRole")
	if roleTxt is "" then return buf
	set n to n + 1
	set posTxt to my axPair(nodeRef, "AXPosition")
	set sizeTxt to my axPair(nodeRef, "AXSize")
	set nameTxt to ""
	set valTxt to ""
	set enaTxt to ""
	if roleTxt is "AXButton" or roleTxt is "AXHeading" or roleTxt is "AXLink" then
		set nameTxt to my axGet(nodeRef, "AXTitle")
	else if roleTxt is "AXCheckBox" or roleTxt is "AXRadioButton" or roleTxt is "AXTextField" or roleTxt is "AXStaticText" or roleTxt is "AXRow" then
		set valTxt to my axGet(nodeRef, "AXValue")
	end if
	if roleTxt is "AXCheckBox" or roleTxt is "AXRadioButton" or roleTxt is "AXButton" or roleTxt is "AXTextField" then
		set enaTxt to my axGet(nodeRef, "AXEnabled")
	end if
	set buf to buf & lvl & tab & roleTxt & tab & posTxt & tab & sizeTxt & tab & nameTxt & tab & valTxt & tab & enaTxt & linefeed
	if (length of buf) > 1 then
		do shell script "printf '%s' " & quoted form of buf & " >> " & quoted form of outPath
		set buf to ""
	end if
	if lvl >= maxDepth then return buf
	set kids to {}
	tell application "System Events"
		try
			set kids to UI elements of nodeRef
		end try
	end tell
	repeat with k in kids
		if n >= budget then exit repeat
		set buf to my walkAX(k, lvl + 1, maxDepth, budget, n, buf, outPath)
	end repeat
	return buf
end walkAX
"""

_ASA_WINDOW = r"""
on run argv
	set px to (item 2 of argv) as integer
	set py to (item 3 of argv) as integer
	tell application "System Events"
		tell process (item 1 of argv)
			set frontmost to true
			set position of window 1 to {px, py}
			delay 0.15
			set p to position of window 1
			set s to size of window 1
			return ((item 1 of p) as text) & "," & ((item 2 of p) as text) & " " & ((item 1 of s) as text) & "x" & ((item 2 of s) as text)
		end tell
	end tell
end run
"""

_ASA_PID = r"""
on run argv
	tell application "System Events"
		if not (exists process (item 1 of argv)) then return ""
		return (unix id of process (item 1 of argv)) as text
	end tell
end run
"""

_ASA_FRONTMOST = r"""
on run argv
	tell application "System Events"
		return name of first application process whose frontmost is true
	end tell
end run
"""

# 按下 AX 树里某个路径指向的节点。`argv` = [进程名, 索引1, 索引2, …]，
# 索引从 `window 1 of process` 之后开始逐层下钻。
_ASA_PRESS = r"""
on run argv
	set procName to (item 1 of argv)
	tell application "System Events"
		set nodeRef to window 1 of process procName
		repeat with i from 2 to (count of argv)
			set nodeRef to UI element ((item i of argv) as integer) of nodeRef
		end repeat
		perform action "AXPress" of nodeRef
	end tell
	return "pressed"
end run
"""

# 「按标题 AXPress + 密集采样」的单进程通道。
#
# 为什么必须常驻：缓存扫描实测 0.95–3.5s，而一次 `osascript` 进程启动 + AX 取值
# 约 0.65s —— 逐次起进程的话，一轮采样周期就吃掉整个扫描窗口，只能采到 1~2 个
# 阶段名。改成一次 osascript 里循环，就只剩 AX 调用本身的开销。
#
# 三个坑，都踩过：
#   1. AppleScript 的 `UI element i of X` 必须在 `tell application "System Events"`
#      块内；写在 handler 外面会报「预期是行尾却找到标识符」。
#   2. `seconds` 是保留字，`set seconds to ...` 直接编译失败（-10003）。
#   3. handler 参数名不能叫 `main_`（与 tell 块里的变量撞名）。
_ASA_PRESS_AND_POLL = r"""
on axGet(n, a)
	set o to ""
	tell application "System Events"
		try
			set raw to value of attribute a of n
			if raw is not missing value then set o to raw as text
		end try
	end tell
	return o
end axGet

on run argv
	set procName to (item 1 of argv)
	set btnIdx to (item 2 of argv) as integer
	set outPath to (item 3 of argv)
	set dur to (item 4 of argv) as integer
	set loIdx to (item 5 of argv) as integer
	set hiIdx to (item 6 of argv) as integer
	set startT to current date
	-- 采样结果先攒在内存里，循环结束后一次性返回。
	-- 每轮都 `do shell script` 追加写文件，实测把采样周期拖到 ~1.5s（14s 只采到
	-- 9 次），而缓存扫描只有 0.95–3.5s —— 攒完再一次性 `return` 反而采到 60+ 次。
	-- 内存占用有界：每次只追加一个短字符串，扫描期间最多几百 KB。
	set all to ""
	tell application "System Events"
		set base to window 1 of process procName
		repeat with k from 1 to 4
			set base to UI element 1 of base
		end repeat
		set rootRef to UI element 2 of base
		-- 按钮下标由 Python 侧先读出来传进来。这里**不能**在 AppleScript 里
		-- 按标题遍历：主视图有 40+ 直接子节点，逐个取值的往返实测把
		-- 「按下 → 第一次采样」拉长到秒级，而缓存扫描实测 0.95–3.5s ——
		-- 遍历还没走完，扫描已经结束，一个阶段都采不到（开发中踩过）。
		-- btnIdx = 0 表示只采样不按键。
		if btnIdx > 0 then
			perform action "AXPress" of (UI element btnIdx of rootRef)
			set pressed to "yes"
		end if
		set startT to current date
		repeat
			set el to (current date) - startT
			if el > dur then exit repeat
			-- 采样循环必须内联在 run 里，且**每个节点必须先读 AXRole 再读 AXValue**。
			-- 三个坑都验证过（每个都曾让「按下了扫描却一个阶段都读不到」）：
			-- 1. 抽成 `on collect(rootRef, …)` handler 会让传进去的 AX 引用失效。
			-- 2. 在已处于 `tell application "System Events"` 的循环里再嵌一层
			--    `tell` + `try` 同样失效。
			-- 3. 只读 AXValue 而不先读 AXRole：AX 客户端在首次解析某个节点前，
			--    直接取 AXValue 会静默返回空串 —— 实测整轮只吐出第 2 号节点的
			--    "2"（恰好是唯一已被解析过的节点）。先读一次 AXRole 强制解析，
			--    同一个节点的 AXValue 随后就能取到。这是 AppleScript AX 最反直觉
			--    的一处，注释留在这里以免下一个人「优化」掉那次 Role 读取。
			--
			-- 也只读节点自身、不下钻：扫描中进度是第 4/5 号直接子节点上的两个
			-- AXStaticText（「正在扫描：X」+「已完成 n/16」），扫描前后那两格是
			-- 摘要卡的 AXGroup。只读孩子会把扫描中的变化整个吞掉。
			--
			-- 节点之间用 `;` 而不是换行：payload 带换行会让一条记录跨多行，
			-- 按行解析就把秒数与 payload 错位（实测采样数从 60 掉到 9 再归零）。
			set s to ""
			repeat with i from loIdx to hiIdx
				-- 越界索引直接跳过：勾选/展开会让主视图重排，下标随时会失效。
				-- 用 `UI element i of rootRef` 无保护地取会抛 -1719，整个采样
				-- 通道跟着挂掉（实测 last=14 而当时只有 11 个子节点）。
				try
					set s to s & (my axGet(UI element i of rootRef, "AXRole")) & "=" & (my axGet(UI element i of rootRef, "AXValue")) & ";"
				end try
			end repeat
			if s is not "" then set all to all & ((el as text) & "|" & s & linefeed)
		end repeat
	end tell
	return "pressed=" & pressed & linefeed & all
end run
"""


# ============ AX 节点 ============


@dataclass
class AxNode:
    """一个 AX 节点。`value` / `enabled` 是 DOM 派生属性（checked / disabled）。"""

    role: str
    x: int = -1
    y: int = -1
    w: int = -1
    h: int = -1
    title: str = ""
    value: str = ""
    enabled: str = ""
    selected: str = ""
    focused: str = ""
    description: str = ""
    index: int = 0
    depth: int = 0
    path: tuple = ()

    @property
    def valid(self) -> bool:
        return self.role not in ("", "?")

    @property
    def checked(self) -> bool:
        return self.value.strip() == "1"

    @property
    def disabled(self) -> bool:
        return self.enabled.strip() == "false"

    @property
    def center(self) -> tuple[int, int]:
        return (self.x + self.w // 2, self.y + self.h // 2)

    def text(self) -> str:
        return self.title or self.value

    def describe(self) -> str:
        bits = [self.role, f"@({self.x},{self.y})", f"{self.w}x{self.h}"]
        if self.title:
            bits.append(f"name={self.title!r}")
        if self.value:
            bits.append(f"value={self.value!r}")
        if self.enabled:
            bits.append(f"enabled={self.enabled}")
        return " ".join(bits)


def _pair(raw: str) -> tuple[int, int]:
    parts = raw.replace("x", ",").split(",")
    if len(parts) != 2:
        return (-1, -1)
    try:
        return (int(parts[0]), int(parts[1]))
    except ValueError:
        return (-1, -1)


class Ax:
    """AX 读侧。一次调用 = 一个节点；带极短 TTL 缓存，避免同屏反复付进程成本。"""

    def __init__(self, process: str = APP_PROCESS, timeout: float = 12.0) -> None:
        self.process = process
        self.timeout = timeout
        self.ttl = 0.4
        self._cache: dict[tuple, tuple[float, AxNode]] = {}

    def invalidate(self) -> None:
        self._cache.clear()

    def frontmost(self) -> str:
        return osa(_ASA_FRONTMOST, timeout=self.timeout)

    def alive(self) -> bool:
        return bool(osa(_ASA_PID, self.process, timeout=self.timeout))

    def pid(self) -> int:
        raw = osa(_ASA_PID, self.process, timeout=self.timeout)
        return int(raw) if raw.isdigit() else -1

    def node(self, path: tuple[int, ...], fresh: bool = False) -> AxNode:
        now = time.monotonic()
        if not fresh and path in self._cache:
            stamp, cached = self._cache[path]
            if now - stamp < self.ttl:
                return cached
        out = osa(_ASA_NODE, self.process, *[str(i) for i in path], timeout=self.timeout)
        node = AxNode(
            role="", index=path[-1] if path else 0, depth=len(path), path=tuple(path)
        )
        if out:
            fields = (out.split("\t") + [""] * 9)[:9]
            node.role = fields[0]
            node.x, node.y = _pair(fields[1])
            node.w, node.h = _pair(fields[2])
            node.title = fields[3]
            node.value = fields[4]
            node.enabled = fields[5]
            node.selected = fields[6]
            node.focused = fields[7]
            node.description = fields[8]
        self._cache[path] = (now, node)
        return node

    def children(self, path: tuple[int, ...], cap: int = 40) -> list[AxNode]:
        out: list[AxNode] = []
        for i in range(1, cap + 1):
            node = self.node(path + (i,), fresh=True)
            if not node.valid:
                break
            out.append(node)
        return out

    def find_child(
        self,
        path: tuple[int, ...],
        predicate: Callable[[AxNode], bool],
        cap: int = 40,
    ) -> AxNode | None:
        for node in self.children(path, cap):
            if predicate(node):
                return node
        return None

    def main_button(self, prefix: str, cap: int = 40) -> AxNode | None:
        return self.find_child(
            P_MAIN, lambda n: n.role == "AXButton" and n.title.startswith(prefix), cap
        )

    def main_button_exact(self, title: str, cap: int = 40) -> AxNode | None:
        return self.find_child(
            P_MAIN, lambda n: n.role == "AXButton" and n.title == title, cap
        )

    def press(self, path: tuple[int, ...], why: str = "") -> bool:
        """对 `path` 指向的节点做 `AXPress`。返回是否成功。

        为什么需要它：屏幕坐标点击会经过鼠标命中测试，页面忙时被 WKWebView
        直接吞掉（实测整机 CPU 84% 时连点两次侧栏都切不过去视图）；`AXPress`
        走无障碍动作，与位置、与命中测试都无关。它也是唯一能点到**滚动区外**
        节点的手段（缓存页「重新扫描」实测在 y=2269，屏幕只有 1080 高）。
        """
        out = osa(
            _ASA_PRESS, self.process, *[str(i) for i in path], timeout=self.timeout
        )
        return out == "pressed"

    def snapshot(
        self, max_depth: int = 30, budget: int = LIGHT_VIEW_BUDGET, timeout: float = 6.0
    ) -> list[AxNode]:
        """整树快照。超时也返回已探到的部分（逐节点 flush）。"""
        OSA_DIR.mkdir(parents=True, exist_ok=True)
        out_path = OSA_DIR / f"snap-{uuid.uuid4().hex[:8]}.tsv"
        out_path.touch()
        osa(
            _ASA_WALK,
            str(max_depth),
            str(budget),
            self.process,
            str(out_path),
            timeout=timeout,
        )
        try:
            return self._parse_snapshot(out_path)
        finally:
            out_path.unlink(missing_ok=True)

    @staticmethod
    def _parse_snapshot(out_path: Path) -> list[AxNode]:
        nodes: list[AxNode] = []
        text = out_path.read_text(encoding="utf-8", errors="replace")
        for line in text.splitlines():
            fields = line.split("\t")
            if len(fields) < 7:
                continue
            node = AxNode(role=fields[1], depth=int(fields[0] or 0))
            node.x, node.y = _pair(fields[2])
            node.w, node.h = _pair(fields[3])
            node.title = fields[4]
            node.value = fields[5]
            node.enabled = fields[6]
            if node.valid:
                nodes.append(node)
        return nodes


# ============ 窗口驱动 ============


class App:
    """MacSlim 窗口驱动。"""

    def __init__(self, ax: Ax, shot_prefix: str = "shot") -> None:
        self.ax = ax
        self.shot_prefix = shot_prefix
        self.shot_index = 0

    def ensure_running(self, launch: bool = True) -> None:
        if self.ax.alive():
            self.normalize_window()
            self.wait_ax_ready()
            return
        if not launch:
            raise SkipCheck("MacSlim 未运行（--no-launch 时不自动拉起）")
        if not DEBUG_APP.exists():
            raise SkipCheck(f"找不到 debug bundle: {DEBUG_APP}")
        subprocess.run(["open", str(DEBUG_APP)], check=False, timeout=30)
        for _ in range(40):
            time.sleep(0.5)
            if self.ax.alive():
                break
        else:
            raise SkipCheck(f"`open {DEBUG_APP.name}` 后 20s 内没出现窗口")
        self.normalize_window()
        self.wait_ax_ready()

    def restart(self) -> None:
        """重启 MacSlim，并等 WebView 的 AX 树就绪。

        为什么需要：后端的应用列表有 **600 秒缓存**（`app_scanner.rs:288`
        `APP_SCAN_CACHE_TTL`）。自造 app 是在缓存建立之后造出来的，进程不重启
        就最多 10 分钟扫不到它（第 7/8/17 项都会因此找不到目标）。重启让缓存
        回到冷状态，自造 app 立刻可见 —— 这比「等 10 分钟」或「判 SKIP」都
        对：这三项本该能自动判定。

        必须**先 quit 再 open**：`kill` 之后 app 不会自启（实测 20s 内没回来），
        所以要显式 `open` 拉起来。quit 用 AppleScript 而不是 `kill`，让 macOS
        走正常退出路径 —— `kill` 会留下未落盘的偏好设置。
        """
        pid = self.ax.pid()
        osa('tell application "MacSlim" to quit', timeout=20)
        for _ in range(40):
            time.sleep(0.5)
            current = self.ax.pid()
            if current in (-1, 0) or current != pid:
                break
        else:
            raise SkipCheck(f"quit 之后 20s 内 MacSlim 仍在（pid={pid}）")
        if self.ax.alive():
            raise SkipCheck(
                f"MacSlim 仍有 AX 记录（pid={self.ax.pid()}），"
                "可能还有别的实例在跑，重启会打断别的检查项"
            )
        subprocess.run(["open", str(DEBUG_APP)], check=False, timeout=30)
        for _ in range(60):
            time.sleep(0.5)
            if self.ax.pid() not in (-1, pid):
                break
        else:
            raise SkipCheck("quit + open 之后 30s 内 MacSlim 没回来")
        self.normalize_window()
        self.wait_ax_ready()

    def wait_ax_ready(self, timeout: float = 25.0) -> None:
        """等 WebView 的 AX 树真的挂上。

        进程起来 ≠ 可交互：实测 `open` 之后 `window 1` 立刻可读，但它的第一个
        子节点有 10 秒以上是 `AXButton 68x22`（启动画面），`UI element 2 of
        window` 要再等十几秒才变成 `AXGroup 900x600`（真正的 WebView）。这期间
        侧栏读不到 → 报「侧栏找不到 XX 按钮」，看起来像产品缺陷，其实是驱动
        抢跑。这里死磕 `P_WEB` 下钻到底，直到 `AXWebArea` 出现。
        """
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            node = self.ax.node(P_WEB, fresh=True)
            if node.role == "AXWebArea" and node.w > 0:
                return
            time.sleep(0.5)
        raise SkipCheck(
            f"MacSlim 进程在，但 WebView 的 AX 树 {timeout:.0f}s 内没就绪"
            f"（当前 P_WEB = {self.ax.node(P_WEB, fresh=True).describe()}）。"
            "通常是启动画面还没切走，或被系统弹窗/权限对话框挡住。"
        )

    def assert_no_overlay(self) -> None:
        """确认没有系统弹窗压在 MacSlim 窗口上。

        实测踩过：系统弹出「某 app 想访问你的提醒事项」权限对话框时，它浮在
        MacSlim 之上，`cliclick` 与 `AXPress` 的坐标全部落到对话框上，**点击
        静默失效** —— 脚本会一路跑到超时，然后报「点了两次侧栏都没切过去」，
        把环境问题伪装成产品缺陷。这里显式检测并立刻停下。

        判据是 `UserNotificationCenter` / `SecurityAgent` 这两个真正承载系统
        对话框的进程是否在前台 —— 它们出现时必然盖住我们的窗口。
        """
        front = self.ax.frontmost()
        if front in OVERLAY_PROCESSES:
            raise SkipCheck(
                f"系统弹窗进程「{front}」在前台，正盖住 MacSlim 窗口："
                "合成点击会被它吃掉。请先手动处理该弹窗再重跑本项。"
            )

    def normalize_window(self) -> tuple[int, int, int, int]:
        """置顶并把窗口挪到固定坐标，之后所有坐标都以这个原点推算。"""
        out = osa(
            _ASA_WINDOW,
            self.ax.process,
            str(NORMAL_POS[0]),
            str(NORMAL_POS[1]),
            timeout=12.0,
        )
        if not out:
            raise SkipCheck("无法置顶 / 归一化 MacSlim 窗口（辅助功能权限？）")
        head, _, size = out.partition(" ")
        x, y = _pair(head)
        w, h = _pair(size)
        return (x, y, w, h)

    def window_rect(self) -> tuple[int, int, int, int]:
        return self.normalize_window()

    def click(self, node: AxNode, why: str = "") -> None:
        if not node.valid or node.w <= 0:
            raise CheckFailure(f"点击目标无效: {node.describe()} ({why})")
        cx, cy = node.center
        self.click_point(cx, cy)

    def click_point(self, x: int, y: int) -> None:
        run(["cliclick", f"c:{x},{y}"], timeout=15)
        self.ax.invalidate()
        time.sleep(0.35)

    def hover(self, x: int, y: int) -> None:
        run(["cliclick", f"m:{x},{y}"], timeout=15)
        self.ax.invalidate()

    def frontmost(self) -> str:
        return osa(_ASA_FRONTMOST, timeout=self.ax.timeout)

    def focus_field(self, node: AxNode) -> AxNode:
        """把焦点真正落到输入框上。

        合成点击有时只把窗口激活、没把焦点交给输入框，随后 `t:` 就会把字符
        打进**别的 app**（实测踩过：焦点落到 Terminal，字符被当成快捷键）。
        所以这里死磕 `AXFocused=true`，并且每轮之前都确认前台就是 MacSlim。
        """
        for _ in range(4):
            self.normalize_window()
            if self.ax.frontmost() != self.ax.process:
                continue
            self.click_point(*node.center)
            probe = self.ax.node(node.path, fresh=True)
            if probe.focused.strip() in ("1", "true"):
                return probe
            time.sleep(0.4)
        raise CheckFailure(
            f"输入框拿不到键盘焦点（AXFocused 一直不为真，前台="
            f"{self.ax.frontmost()!r}）。合成事件没能进入 WKWebView。"
        )

    def type_text(self, node: AxNode, text: str, expect: bool = True) -> None:
        """往输入框里写入 `text`，并**回读校验**。

        曾踩的坑：`cliclick kp:cmd,a` 不是合法语法（`kp:` 只认固定功能键），于是
        「全选」静默失败，`t:` 一直往旧内容后面追加，搜索框攒出一长串垃圾 ——
        表现是「找不到目标」，根因却在别处。现在用 `tc:` 三击全选 + 回读比对。
        """
        if not node.valid or node.w <= 0:
            raise CheckFailure(f"输入框目标无效: {node.describe()}")
        self.focus_field(node)
        cx, cy = node.center
        run(["cliclick", f"tc:{cx},{cy}"], timeout=15)
        time.sleep(0.25)
        if text:
            run(["cliclick", f"t:{text}"], timeout=20)
        else:
            # 三击只是选中，不删字。空串必须显式敲删除键，否则旧查询会一直留着。
            run(["cliclick", "kp:delete"], timeout=15)
        time.sleep(0.5)
        self.ax.invalidate()
        if not expect:
            return
        actual = self.ax.node(node.path, fresh=True).value
        if actual.strip() != text.strip():
            raise CheckFailure(
                f"输入框回读不一致：期望 {text!r}，实际 {actual!r}。"
                "合成键盘事件没落到输入框。"
            )

    def double_click_point(self, x: int, y: int) -> None:
        run(
            ["cliclick", f"dd:{x},{y}", f"du:{x},{y}", f"dd:{x},{y}", f"du:{x},{y}"],
            timeout=20,
        )
        time.sleep(0.2)
        self.ax.invalidate()

    def screenshot(self, tag: str) -> str:
        """人工复核产物。**不参与任何断言**（§3.1）。"""
        ARTIFACTS.mkdir(parents=True, exist_ok=True)
        self.shot_index += 1
        x, y, w, h = self.normalize_window()
        name = f"{self.shot_prefix}-{self.shot_index:02d}-{tag}.png"
        dest = ARTIFACTS / name
        run(["screencapture", "-o", "-x", f"-R{x},{y},{w},{h}", str(dest)], timeout=30)
        return str(dest.relative_to(REPO)) if dest.exists() else ""

    def nav(self, label: str) -> AxNode:
        """点侧栏。按钮位置从 AX 现读，不写死 y 偏移。

        合成点击会丢，原因有三类，逐个处理：
        1. 窗口刚被归一化、上一次点击的余波 → 重试。
        2. 系统弹窗盖住窗口 → 事件被对话框吃掉。`assert_no_overlay` 直接判掉，
           否则会被误报成产品缺陷（开发中真的踩到过：系统弹出「某 app 想访问你的
           提醒事项」，点侧栏全部落空，报「点了两次都没切过去」）。
        3. **页面正忙时点击丢失**。智能扫描页有 5 秒轮询 + 1.5s 一次的指标刷新，
           CPU 高时 WKWebView 会直接吞掉合成点击（实测整机 CPU 84% 时连点两次
           侧栏都切不过去）。这时 `AXPress` 比屏幕坐标点击可靠 —— 它走无障碍
           动作，不经过鼠标命中测试。所以第二次尝试改用 AXPress。
        """
        self.assert_no_overlay()
        self.normalize_window()
        target: AxNode | None = None
        for node in self.ax.children(P_SIDEBAR + (2,), cap=10):
            if node.title == label:
                target = node
                break
        if target is None:
            raise SkipCheck(f"侧栏找不到「{label}」按钮")
        self.click(target, why=f"侧栏 {label}")
        if self.heading() == label:
            return target
        time.sleep(0.5)
        self.normalize_window()
        self.assert_no_overlay()
        self.ax.press(target.path, why=f"AXPress 侧栏 {label}")
        if self.heading() == label:
            return target
        time.sleep(0.6)
        raise CheckFailure(
            f"「{label}」点了（坐标点击 + AXPress）主视图都没切过去"
            f"（当前「{self.heading()}」）。若页面此刻很忙，可稍后重跑本项。"
        )

    def reset_to_fresh_view(self, label: str, away: str = "智能扫描") -> None:
        """强制重挂目标视图，让它重新 `load()` 一遍。

        `ProcessView` 的 `frozen()`（`ProcessView.tsx:142`）在 `hoveringList()` 为真
        时停掉 5 秒轮询，而 `onMouseLeave` 只有指针真的移出列表才触发。上一轮把
        指针停在列表里做「悬停不重排」检查时，指针一直没走，frozen 就一直挂着，
        `rows()` 停在旧快照上 —— 新起的牺牲进程永远进不了列表。所以每轮进程相关
        检查前先把指针挪到侧栏（触发 mouseleave），再「切走 → 切回」强制重挂。
        """
        rect = self.normalize_window()
        self.hover(rect[0] + 100, rect[1] + 400)
        time.sleep(0.4)
        if self.heading() == label:
            self.nav(away)
            time.sleep(0.6)
        self.nav(label)

    def heading(self) -> str:
        node = self.ax.node(P_MAIN + (1,), fresh=True)
        return node.title if node.valid else ""

    def wait_heading(self, expected: str, timeout: float = 15.0) -> float:
        """等主视图标题变成期望值。标题文本是 DOM 文本，不是像素。"""
        start = time.monotonic()
        while time.monotonic() - start < timeout:
            if self.heading() == expected:
                return time.monotonic() - start
            time.sleep(0.4)
        current = self.heading()
        raise CheckFailure(f"等不到主视图标题「{expected}」，当前是「{current or '读不到'}」")

    def texts(self, budget: int = LIGHT_VIEW_BUDGET) -> list[str]:
        return [n.value for n in self.ax.snapshot(budget=budget) if n.role == "AXStaticText"]

    def press_and_poll(
        self,
        button_title: str,
        seconds: float,
        first: int = 2,
        last: int = 5,
    ) -> tuple[str, list[tuple[float, str]]]:
        """按 `button_title` 并持续采样主视图的静态文本，返回 `(verdict, 采样序列)`。

        verdict 形如 `pressed=yes`；按钮不在 AX 树里时先抛 SkipCheck，所以调用方
        拿到的 verdict 一定是「按下了」。

        采样全在**一个** `osascript` 进程内完成、结果一次性返回（不落盘、不每轮
        起进程）：缓存扫描实测 0.95–3.5s，而一次 `osascript` 启动 + AX 取值约
        0.65s，逐次起进程会整个吃掉扫描窗口（开发中实测只采到 1 个阶段名，
        误判成「产品没有进度」）。

        按钮下标由**这里**先读出来再传进 AppleScript，不让 AppleScript 按标题
        遍历：主视图有 40+ 直接子节点，逐个取值会把「按下 → 首次采样」拖到秒级。

        为什么不用 `click()`：按钮常在滚动区外（实测 y=2269 > 屏幕高 1080），
        `cliclick` 的屏幕坐标点不到；`AXPress` 与位置无关。
        """
        self.assert_no_overlay()
        self.normalize_window()
        button = self.ax.main_button_exact(button_title)
        if button is None:
            raise SkipCheck(
                f"主视图 AX 树里没有标题为「{button_title}」的按钮"
                f"（当前视图标题 = {self.heading()!r}），扫描没被触发"
            )
        script_path = _asa_file(_ASA_PRESS_AND_POLL)
        try:
            proc = subprocess.run(
                [
                    "osascript", str(script_path), self.ax.process,
                    str(button.index), "-", str(seconds), str(first), str(last),
                ],
                capture_output=True, text=True, timeout=seconds + 60,
            )
            if proc.returncode != 0:
                raise SkipCheck(
                    "常驻轮询通道执行失败："
                    f"rc={proc.returncode} {proc.stderr.strip()[-200:]}"
                )
        finally:
            script_path.unlink(missing_ok=True)
        verdict, _, payload = proc.stdout.partition("\n")
        return (verdict.strip(), _parse_poll_text(payload))


# ============ 可观测量 ============


def pid_alive(pid: int) -> bool:
    """PID 存活探针，等价于 `kill -0`，不发送任何信号。"""
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def spawn_sacrifice_sleep(seconds: int = 1800) -> tuple[subprocess.Popen, str]:
    """造一个牺牲进程，返回 `(Popen, 唯一进程名)`。

    两个坑，都踩过：

    1. **不用 `/bin/sleep` 直接跑。** `ProcessView` 的搜索是
       `String(r.pid).includes(q)`（`ProcessView.tsx:200`），而 macOS 的 PID 会
       回绕（实测已回绕到三位数），短 PID 会同时命中一堆进程，列表收窄不下去，
       AX 树随即在多行渲染下阻塞。改用唯一可执行文件名搜索，命中必然只有 1 行。
    2. **名字只用数字。** `cliclick t:` 是合成键盘事件，会被当前输入法接管：
       实测输入 "macslim-…" 落进输入框的是「嘛超市里面-😑额-…」这种拼音候选。
       数字则原样送达。10 位随机数还天然不可能是 PID（≤5 位）或端口的子串。
    """
    bin_dir = Path(tempfile.gettempdir()) / f"macslim-e2e-bin-{uuid.uuid4().hex[:8]}"
    bin_dir.mkdir(parents=True, exist_ok=True)
    name = f"{random.SystemRandom().randrange(10**9, 10**10)}"
    link = bin_dir / name
    link.symlink_to("/bin/sleep")
    proc = subprocess.Popen(
        [str(link), str(seconds)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    return (proc, name)


def cleanup_sacrifice_bin(name: str) -> None:
    for parent in Path(tempfile.gettempdir()).glob("macslim-e2e-bin-*"):
        if (parent / name).exists():
            shutil.rmtree(parent, ignore_errors=True)


def du_kb(path: Path) -> int:
    proc = run(["du", "-sk", str(path)], timeout=60)
    if proc.returncode != 0:
        return -1
    try:
        return int(proc.stdout.split()[0])
    except (ValueError, IndexError):
        return -1


def apparent_bytes(path: Path) -> int:
    """与后端 `cache_scanner::dir_size` 同口径：只累加普通文件的 `metadata().len()`。"""
    total = 0
    for root, _dirs, files in os.walk(path):
        for name in files:
            try:
                total += (Path(root) / name).lstat().st_size
            except OSError:
                continue
    return total


def history_rows(limit: int = 50) -> list[dict]:
    if not DB_PATH.exists():
        return []
    conn = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True, timeout=5)
    try:
        cur = conn.execute(
            "SELECT id, operation, target, freed_bytes, success, detail "
            "FROM history ORDER BY id DESC LIMIT ?",
            (limit,),
        )
        return [
            {
                "id": r[0],
                "operation": r[1],
                "target": r[2],
                "freed_bytes": r[3],
                "success": bool(r[4]),
                "detail": r[5],
            }
            for r in cur.fetchall()
        ]
    except sqlite3.Error:
        return []
    finally:
        conn.close()


def history_count() -> int:
    if not DB_PATH.exists():
        return -1
    conn = sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True, timeout=5)
    try:
        return int(conn.execute("SELECT COUNT(*) FROM history").fetchone()[0])
    except sqlite3.Error:
        return -1
    finally:
        conn.close()


# ============ 牺牲目标工厂（§3.4 安全边界）============


def make_sacrificial_cache_dir(tag: str, mib: int = 8) -> Path:
    """在缓存扫描器真实覆盖的根下造一个可丢弃目录。

    `cache_scanner::scan_app_caches` 逐个遍历 `~/Library/Caches/*`，所以放在这里才会
    被扫到（`/tmp` 是独立卷，扫不到）。`< 5MB` 的目录会被扫描器跳过，所以写 8 MiB。
    名字带随机后缀，绝不会命中任何既有目录。
    """
    path = Path.home() / "Library/Caches" / f"{SAC_CACHE_PREFIX}{tag}-{uuid.uuid4().hex[:10]}"
    path.mkdir(parents=True, exist_ok=False)
    with (path / "e2e-sacrifice.bin").open("wb") as fh:
        chunk = b"\0" * (1024 * 1024)
        for _ in range(mib):
            fh.write(chunk)
    return path


def make_sacrificial_app(tag: str) -> Path:
    """在 `~/Applications` 拼一个最小可运行 `.app` bundle。

    `app_scanner::scan_installed_apps` 扫 `/Applications` 与 `~/Applications`；选后者
    是为了不污染系统级目录。bundle id 带随机后缀，绝不会撞真实 app。

    **随机后缀是纯数字**（10 位），不是 hex。原因：卸载列表按体积降序排，自造 app
    只有几百 KB、一定落在滚动区外，要靠搜索框过滤才看得见；而 `cliclick t:` 是
    合成键盘事件，会被当前输入法接管 —— 实测把 `MacSlimE2E-residue-efcb8846` 打
    进输入框的是「马超里SMEE-residue-而非拆吧8'8」这种拼音候选。数字原样送达。
    与 `spawn_sacrifice_sleep` 用数字后缀是同一个坑，两处都留了注释。
    """
    SAC_APPS_DIR.mkdir(parents=True, exist_ok=True)
    name = f"{SAC_APP_PREFIX}{tag}-{random.SystemRandom().randrange(10**9, 10**10)}"
    bundle = SAC_APPS_DIR / f"{name}.app"
    macos = bundle / "Contents" / "MacOS"
    macos.mkdir(parents=True)
    (bundle / "Contents" / "Resources").mkdir(parents=True)
    ident = f"com.macslim.e2e.{tag}.{uuid.uuid4().hex[:8]}"
    (bundle / "Contents" / "Info.plist").write_text(_info_plist(name, ident), encoding="utf-8")
    exe = macos / name
    exe.write_text(
        "#!/bin/sh\n"
        "# MacSlim e2e 门禁自造的牺牲 app：只 sleep，绝不碰用户数据。\n"
        "while true; do sleep 3600; done\n",
        encoding="utf-8",
    )
    exe.chmod(0o755)
    return bundle


def app_search_term(bundle: Path) -> str:
    """从自造 app 的 bundle 名里取出**末段纯数字**，作为卸载页的搜索词。

    名字形如 `MacSlimE2E-residue-7943722234.app`，末段是 `7943722234`。

    两个坑：

    1. **只能取末段，不能把所有数字拼起来。** 名字里有 `E2E`，把全部数字拼起来
       会得到 `27943722234` —— 而它在原名里**不是连续子串**（`E2E` 与
       `7943722234` 之间隔着 `-residue-`）。卸载页的过滤是
       `name.toLowerCase().includes(q)`（`UninstallerView.tsx:130`），拼起来的
       串永远匹配不上（开发中实测：过滤后「未找到可卸载的应用」）。
    2. **必须纯数字。** `cliclick t:` 是合成键盘事件，会被当前输入法接管 ——
       实测把 `MacSlimE2E-residue-efcb8846` 打进输入框的是「马超里SMEE-residue-
       而非拆吧8'8」这种拼音候选。数字原样送达。与 `spawn_sacrifice_sleep`
       用数字后缀是同一个坑，两处都留了注释。
    """
    for segment in reversed(bundle.stem.split("-")):
        if segment.isdigit():
            return segment
    raise SkipCheck(f"自造 app 名里没有末段数字，无法安全地作为搜索词：{bundle.name}")


def _info_plist(name: str, ident: str) -> str:
    entries = [
        ("CFBundleIdentifier", ident),
        ("CFBundleName", name),
        ("CFBundleDisplayName", name),
        ("CFBundleExecutable", name),
        ("CFBundlePackageType", "APPL"),
        ("CFBundleVersion", "1.0"),
        ("CFBundleShortVersionString", "1.0"),
    ]
    body = "".join(f"\t<key>{k}</key><string>{v}</string>\n" for k, v in entries)
    return (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" '
        '"http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n'
        '<plist version="1.0">\n<dict>\n' + body + "\t<key>LSUIElement</key><true/>\n"
        "</dict>\n</plist>\n"
    )


def cleanup_sacrificial(paths: list[Path]) -> None:
    for path in paths:
        shutil.rmtree(path, ignore_errors=True)


def terminate_sacrifice(proc: subprocess.Popen, name: str = "") -> None:
    if proc.poll() is None:
        proc.kill()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        pass
    if name:
        cleanup_sacrifice_bin(name)


# ============ 资源采样 ============


def _proc_cpu_seconds(pid: int) -> float:
    out = run(["ps", "-o", "time=", "-p", str(pid)], timeout=10).stdout.strip()
    if not out:
        return 0.0
    try:
        seconds = 0.0
        for part in out.split(":"):
            seconds = seconds * 60 + float(part)
    except ValueError:
        return 0.0
    return seconds


def _rss_kb(pid: int) -> int:
    out = run(["ps", "-o", "rss=", "-p", str(pid)], timeout=10).stdout.strip()
    return int(out) if out.isdigit() else 0


def sample_idle(pid: int, seconds: float) -> tuple[float, int]:
    """空闲资源占用。CPU 用 ps time 的多段差值，内存取 RSS 峰值。"""
    start = _proc_cpu_seconds(pid)
    peak_rss = 0
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        peak_rss = max(peak_rss, _rss_kb(pid))
        time.sleep(1.0)
    cpu = 100.0 * (_proc_cpu_seconds(pid) - start) / seconds if seconds > 0 else 0.0
    return (cpu, peak_rss)


def _process_probe_block() -> str:
    return (
        "进程管理页每 5 秒自动轮询并整树重建（ProcessView.tsx:183），"
        "按下标逐个读 AX 的 ~15s 窗口里必然撞上一次重建，索引会漂、"
        "`UI element i` 瞬时返回空，于是读到的行节点被截断。"
        "解法是先让页面进入 frozen()：把指针悬停在列表区触发 onMouseEnter → "
        "hoveringList()，轮询立刻停。悬停点由实时几何推出，不含魔法坐标。"
    )
