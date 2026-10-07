#!/usr/bin/env python3
"""用真正的鼠标事件点击屏幕坐标。

## 为什么不能用 `System Events ... click at {x, y}`

那一条走的是 **Accessibility 通道**：它把「点击」当作一个 AX 动作发给该坐标
下的 AX 元素。对原生控件有效，但对我们这里要点的对象——WKWebView 里的
HTML 按钮（「授权」链接）——**完全无效**：WebKit 不把 DOM 节点暴露成可
执行 AXPress 的元素，于是点击被悄悄丢掉，没有报错、没有日志。

症状具有极强的误导性：前台是对的（截图拍到 app）、坐标是对的（对着截图量
过）、权限也给了（`set frontmost` 生效），但点下去什么都没发生。很容易被
误判成「权限没给」或「坐标失效」。

改成向窗口服务器投递**真正的鼠标按下/抬起事件**（`CGEventPost` 到
`kCGHIDEventTap`），事件按屏幕坐标分发给光标下的窗口，与真实鼠标点击等价。

用法: python3 scripts/_click.py <x> <y> [次数]
坐标是**逻辑点**（与截图的 2880x1800 像素按 2 倍换算）。
"""
from __future__ import annotations

import ctypes
import ctypes.util
import sys
import time


class CGPoint(ctypes.Structure):
    _fields_ = [("x", ctypes.c_double), ("y", ctypes.c_double)]


def _load() -> tuple:
    cg = ctypes.CDLL(ctypes.util.find_library("CoreGraphics"))
    cf = ctypes.CDLL(ctypes.util.find_library("CoreFoundation"))

    cg.CGEventCreateMouseEvent.restype = ctypes.c_void_p
    cg.CGEventCreateMouseEvent.argtypes = [
        ctypes.c_void_p, ctypes.c_uint32, CGPoint, ctypes.c_uint32
    ]
    cg.CGEventPost.restype = None
    cg.CGEventPost.argtypes = [ctypes.c_uint32, ctypes.c_void_p]
    cg.CGEventCreateKeyboardEvent.restype = ctypes.c_void_p
    cg.CGEventCreateKeyboardEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint16, ctypes.c_bool]
    cf.CFRelease.restype = None
    cf.CFRelease.argtypes = [ctypes.c_void_p]
    return cg, cf


KCG_EVENT_LEFT_MOUSE_DOWN = 1
KCG_EVENT_LEFT_MOUSE_UP = 2
KCG_EVENT_KEY_DOWN = 10
KCG_EVENT_KEY_UP = 11
KCG_HID_EVENT_TAP = 0
KCG_MOUSE_BUTTON_LEFT = 0

# 常用键的虚拟键码（US 布局，与键盘语言无关）
KEYCODES = {"return": 36, "enter": 36, "escape": 53, "tab": 48, "space": 49}


def press(key: str) -> None:
    """发一次真实按键。

    文件选择框默认按钮是「打开」，用回车接受它比按坐标点更可靠：面板每次
    出现的位置可能不同，而按钮位置会随之变；回车不依赖任何坐标。
    """
    code = KEYCODES.get(key.lower())
    if code is None:
        raise SystemExit(f"不认识的键：{key}（支持 {sorted(KEYCODES)}）")
    cg, cf = _load()
    for event_type in (KCG_EVENT_KEY_DOWN, KCG_EVENT_KEY_UP):
        event = cg.CGEventCreateKeyboardEvent(None, code, event_type == KCG_EVENT_KEY_DOWN)
        if not event:
            raise SystemExit("CGEventCreateKeyboardEvent 失败")
        cg.CGEventPost(KCG_HID_EVENT_TAP, event)
        cf.CFRelease(event)
        time.sleep(0.05)


def click(x: float, y: float) -> None:
    cg, cf = _load()
    point = CGPoint(float(x), float(y))
    # 先移动光标：某些控件以「光标在不在上面」决定要不要处理 hover/点击。
    move = cg.CGEventCreateMouseEvent(
        None, 5, point, KCG_MOUSE_BUTTON_LEFT  # 5 = kCGEventMouseMoved
    )
    if move:
        cg.CGEventPost(KCG_HID_EVENT_TAP, move)
        cf.CFRelease(move)
        time.sleep(0.05)
    for event_type in (KCG_EVENT_LEFT_MOUSE_DOWN, KCG_EVENT_LEFT_MOUSE_UP):
        event = cg.CGEventCreateMouseEvent(
            None, event_type, point, KCG_MOUSE_BUTTON_LEFT
        )
        if not event:
            raise SystemExit(f"CGEventCreateMouseEvent 失败（type={event_type}）")
        cg.CGEventPost(KCG_HID_EVENT_TAP, event)
        cf.CFRelease(event)
        time.sleep(0.03)


def main() -> None:
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    x, y = float(sys.argv[1]), float(sys.argv[2])
    times = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    for _ in range(times):
        click(x, y)
        time.sleep(0.2)


def press_main() -> None:
    if len(sys.argv) < 3:
        raise SystemExit("用法: _click.py --key <键名>")
    press(sys.argv[2])


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--key":
        press_main()
    else:
        main()
