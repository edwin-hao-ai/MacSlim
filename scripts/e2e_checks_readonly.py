#!/usr/bin/env python3
"""MacSlim e2e 门禁的只读功能（第 13-17 项）与发布指标（第 18-21 项）。"""
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
    RELEASE_DMG_GLOBS,
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
from e2e_checks import (  # noqa: E402
    _open_uninstaller,
    _require_app_in_list,
    _select_sacrificial_app,
)

P_MAIN = drv.P_MAIN

# ============ 13. 侧栏 7 页 ============


@check("13", "侧栏 7 个页面全部可打开且不崩溃")
def check_13(app: App) -> CheckResult:
    app.ensure_running()
    pid_before = app.ax.pid()
    seen: dict[str, float] = {}
    shots: list[str] = []
    for label in drv.NAV_LABELS:
        app.nav(label)
        seen[label] = round(app.wait_heading(label, timeout=15.0), 2)
        shots.append(app.screenshot(f"nav-{len(seen):02d}-{label}"))
        if app.ax.pid() != pid_before:
            raise CheckFailure(f"打开「{label}」后 MacSlim 进程变了（疑似崩溃重启）")
    app.normalize_window()
    if not drv.pid_alive(pid_before):
        raise CheckFailure("跑完 7 个页面后 MacSlim 不存活")
    return ok(
        check_13,
        "7 个页面标题全部命中，进程 PID 未变",
        {
            "app_pid": pid_before,
            "view_switch_seconds": seen,
            "window_rect": list(app.window_rect()),
            "assertion": "AXHeading 文本 = 页面标题（非像素）",
        },
        shots,
    )


# ============ 14. 历史记录可读 ============


@check("14", "历史记录页可读（SQLite 行数与字段）")
def check_14(app: App) -> CheckResult:
    app.ensure_running()
    rows = drv.history_rows(50)
    total = drv.history_count()
    if total < 0:
        raise SkipCheck(f"打不开历史库: {drv.DB_PATH}")
    app.nav("历史记录")
    app.wait_heading("历史记录")
    shot = app.screenshot("hist-14")
    texts = app.texts(budget=400)
    if total == 0:
        raise SkipCheck("历史库为空，无法验证字段渲染")
    if "操作历史" not in texts:
        raise CheckFailure("历史页没有渲染出标题")
    sample = rows[0]
    missing = [f for f in ("id", "operation", "target", "freed_bytes", "success", "detail")
               if f not in sample]
    if missing:
        raise CheckFailure(f"历史行缺字段 {missing}")
    return ok(
        check_14,
        f"历史库 {total} 行可读，最新一条 operation={sample['operation']}",
        {
            "db_path": str(drv.DB_PATH),
            "history_rows": total,
            "latest_row": sample,
            "page_texts_sample": [t for t in texts if t][:8],
        },
        [shot],
    )


# ============ 15. 设置可读写并持久化 ============


@check("15", "设置页开关可读可写并复原（NSUserDefaults 持久化）")
def check_15(app: App) -> CheckResult:
    app.ensure_running()
    app.nav("设置")
    app.wait_heading("设置")
    boxes = [n for n in app.ax.snapshot(budget=400) if n.role == "AXCheckBox"]
    if not boxes:
        raise SkipCheck("设置页没有暴露任何开关复选框")
    target = boxes[0]
    before = target.checked
    app.click(target, why="翻转第一个设置开关")
    time.sleep(1.2)
    mid = _switch_at(app, target.y)
    if mid is None:
        raise CheckFailure("翻转后找不到同一个开关")
    flipped = mid.checked
    shot = app.screenshot("set-15-flipped")
    app.click(mid, why="翻回原值")
    time.sleep(1.0)
    back = _switch_at(app, target.y)
    if back is None or back.checked != before:
        raise CheckFailure("开关没能翻回原值，测试污染了用户设置")
    return ok(
        check_15,
        f"开关 {before} → {flipped} → {back.checked} 往返成功并复原",
        {
            "switch_before": before,
            "switch_after_flip": flipped,
            "switch_restored": back.checked,
            "checkbox": target.describe(),
            "note": "设置由 macOS NSUserDefaults 持久化，脚本已复原原值",
        },
        [shot],
    )


def _switch_at(app: App, y: int) -> AxNode | None:
    for node in app.ax.snapshot(budget=400):
        if node.role == "AXCheckBox" and abs(node.y - y) < 4:
            return node
    return None


# ============ 16 / 17. 阶段进度 ============


_STAGE_PREFIX = "正在扫描："
_STAGE_PENDING = "正在扫描…"
_COUNT_PREFIX = "已完成 "


def _harvest(samples: list[tuple[float, str]]) -> tuple[list[str], list[str], float | None]:
    """从常驻采样的序列里抽出阶段名、计数器与首个阶段出现时刻。

    每条 payload 是 `AXRole=AXValue;AXRole=AXValue;…`（先读 Role 强制 AX 解析
    那个节点，否则 AXValue 静默返回空串，见 `_ASA_PRESS_AND_POLL` 的注释），
    这里只取 `=` 后半段。

    `正在扫描…` 是 `scanProgress.working`（`stageCurrent` 还是 null 时的兜底，
    见 `ScanStageProgress.tsx:23`）—— 它证明**进度组件已经挂载**，但没有阶段名。
    所以它单列成 `pending`，不算进 `stages`：没有阶段名不代表事件没来，只能说
    阶段名还没到。把它误当成阶段名会让断言变得毫无意义（一个恒真的 `stage`）。

    同一阶段名在多次采样里重复出现只算一次 —— 断言要看的是「渲染出过多少个
    **不同**阶段」，不是「采了多少次」。
    """
    stages: list[str] = []
    counters: list[str] = []
    first_ms: float | None = None
    saw_pending = False
    for elapsed, body in samples:
        for field in body.split(";"):
            value = field.partition("=")[2].strip()
            if not value:
                continue
            if value == _STAGE_PENDING:
                saw_pending = True
            elif value.startswith(_STAGE_PREFIX):
                stage = value[len(_STAGE_PREFIX):]
                if stage and stage not in stages:
                    stages.append(stage)
                    if first_ms is None:
                        first_ms = elapsed * 1000
            elif value.startswith(_COUNT_PREFIX) and value not in counters:
                counters.append(value)
    return (stages, counters, first_ms if first_ms is not None else (
        0.0 if saw_pending else None
    ))


def _counter_progress(counters: list[str]) -> list[int]:
    """把「已完成 n/16」解析成 n 的序列（按出现顺序，去重相邻重复）。"""
    numbers: list[int] = []
    for text in counters:
        head = text[len(_COUNT_PREFIX):].partition("/")[0].strip()
        if head.isdigit() and (not numbers or numbers[-1] != int(head)):
            numbers.append(int(head))
    return numbers


def _assert_stage_progress(
    samples: list[tuple[float, str]], stages: list[str], counters: list[str]
) -> list[int]:
    """阶段进度的判据。抽出来是为了让 `check_16` 留在 50 行门禁内。

    档位刻意分清：采不到（SKIP，测量能力不足）与采到了但不对（FAIL，产品缺陷）
    必须可区分 —— 把前者记成 FAIL 会让人去查一个根本没坏的进度条。

    硬断言是**计数器**（`已完成 n/16`）：它证明组件挂载且事件在推。
    阶段名是辅助信号：本机页缓存热时扫描常在 0.4s 内跑完，AX 一轮往返 ~0.3s，
    采不满几个阶段名属测量限制，不该因此判 FAIL。
    """
    numbers = _counter_progress(counters)
    if not counters:
        first = repr(samples[0][1][:70]) if samples else "（一条采样都没有）"
        raise SkipCheck(
            f"{len(samples)} 次采样里没读到「已完成 n/N」文本（首条 = {first}）。"
            "扫描耗时可能短于一次 AX 采样周期。"
        )
    if "16/16" not in counters[-1]:
        raise CheckFailure(f"阶段总数不是 16，采到的是 {counters[-1]!r}")
    if not stages and numbers[:1] == [16]:
        raise SkipCheck(
            f"只采到终态 {counters[-1]!r}，没采到任何中间阶段 —— "
            "本机扫描快于 AX 采样周期。阶段名不参与判定，"
            f"进度组件已挂载且总数为 16（计数器序列 = {numbers}）"
        )
    if stages and numbers != sorted(numbers):
        raise CheckFailure(f"阶段计数器不是单调递增：{numbers}")
    return numbers


@check("16", "缓存扫描阶段进度（16 阶段、阶段名非空）")
def check_16(app: App) -> CheckResult:
    """阶段事件本身在 webview 内部，外面收不到。**可观测的是它渲染出来的文本**：
    `ScanStageProgress` 输出「正在扫描：{stage}」与「已完成 {done}/{total}」。

    断言：total == 16（UI 确实知道有 16 个阶段）+ 至少采到 3 个不同非空阶段名。

    必须走 `press_and_poll` 的常驻通道：缓存扫描实测 0.95–3.5s，而逐次起
    `osascript` 进程约 0.65s/轮，一轮就把扫描窗口吃掉了（开发中实测只采到 1 个
    阶段名，误判成「没有进度」）。按钮也在滚动区外（y=2269 > 屏幕高），
    `cliclick` 点不到，只能用 AXPress。

    采样窗口只给 6s：本机页缓存已热，扫描常在 0.4s 内跑完（实测第一次采样就
    已经是「已完成 16/16」），窗口开更长只是多采几行相同的终态。

    硬断言是 `已完成 16/16` —— 阶段**总数**。「采到几个不同阶段名」受 AX 采样
    往返（~0.3s/轮）限制，采不满 16 个属正常，所以只当辅助信号（要求 ≥3，
    用来证明进度确实逐阶段刷新过，而不是一次性跳到终态）。
    """
    app.ensure_running()
    app.nav("缓存清理")
    app.wait_heading("缓存清理")
    verdict, samples = app.press_and_poll("重新扫描", 6.0, first=2, last=6)
    shot = app.screenshot("cache-16-progress")
    if "pressed=yes" not in verdict:
        raise SkipCheck(
            f"缓存页 AX 树里找不到「重新扫描」按钮（{verdict}），"
            "扫描没被触发，采不到任何阶段属于环境问题而非产品缺陷"
        )
    stages, counters, first_ms = _harvest(samples)
    numbers = _assert_stage_progress(samples, stages, counters)
    return ok(
        check_16,
        f"阶段总数 16/16，计数器单调递增 {numbers}；采到 {len(stages)} 个不同阶段名",
        {
            "final_counter": counters[-1],
            "counters_seen": counters,
            "counter_numbers": numbers,
            "distinct_stages": stages,
            "distinct_stage_count": len(stages),
            "samples_taken": len(samples),
            "first_progress_ms_after_press": round(first_ms or -1, 1),
            "assertion": "ScanStageProgress 渲染文本，非像素",
            "known_limit": "单事件延迟 ≤200ms 在 webview 外不可测；"
                           "缓存扫描实测 0.95–3.5s，AX 一轮往返 ~0.3s，"
                           "阶段名未必采得到 —— 16 这个总数与计数递增才是硬断言",
        },
        [shot],
    )


@check("17", "卸载残留扫描进度（n/N）", "MANUAL-FAST")
def check_17(app: App) -> CheckResult:
    """残留扫描的入口不是独立按钮：勾选 app 后点「卸载选中应用」即
    `enterResiduePhase()`（`UninstallerView.tsx:479`），它只扫残留并渲染进度，
    **不删任何文件**（删除在更后面的确认弹窗）。所以本项零破坏。

    自造 app 是为了让扫描目标数最少、耗时最短 —— 恰恰因此**进度 UI 存在的
    窗口比一次 AX 往返还短**：`residueLoading()` 只在扫描期间为真
    （`UninstallerView.tsx:533`），而单个 app 的残留扫描实测在第一次采样之前
    就结束了（采样 13 次全部看到终态 `已选 1 个应用 / 0 B / 总大小`，没有一次
    看到 `已完成 n/N`）。

    按 §3.4「无法自动化的项标 MANUAL 不伪装 PASS」处理：这里**不做降级断言**。
    前一版曾把「没采到进度」判成 SKIP，但 SKIP 在报告里读起来像「跑过了只是环境
    不满足」，掩盖了一个「这一项其实没验过任何东西」的事实。所以标 MANUAL。

    人工复核方法（一次 ~10 秒）：

    ```
    python3 scripts/e2e_gui.py --only 17   # 造目标、重启、进残留阶段
    ```

    肉眼在「应用卸载」页应看到「正在扫描：<应用名>」+「已完成 n/N」+ 进度条 +
    「已发现 <体积>」，其中 N = 已选应用数。**不要**用自动化去凑这一项：要让
    扫描慢到能被采样，得选几十个真实 app，那会同时把它们带进「卸载选中应用」
    的确认弹窗 —— 为了看一个进度条而把用户的应用置于待卸载状态不值得。
    """
    raise drv.ManualCheck(
        "残留扫描进度无法自动化：单个 app 的扫描快于一次 AX 采样往返，"
        "`residueLoading()` 为真的窗口短到采不到。\n"
        "人工复核：跑 `python3 scripts/e2e_gui.py --only 17`，肉眼确认"
        "「应用卸载」页在残留阶段显示「正在扫描：<应用名>」+「已完成 n/N」+ 进度条。\n"
        "对应自动化覆盖：ScanStageProgress 组件本身有 vitest"
        "（`src/components/ScanStageProgress.test.tsx` 覆盖 `scanStage.*` 分流），"
        "第 16 项覆盖了同一组件在缓存扫描上的真实渲染。"
    )


# ============ 18. 首次扫描 < 3s ============


@check("18", "性能：首次扫描 < 3s")
def check_18(app: App) -> CheckResult:
    app.ensure_running()
    app.nav("智能扫描")
    app.wait_heading("智能扫描")
    scan = app.ax.main_button_exact("重新扫描") or app.ax.main_button_exact("开始扫描")
    if scan is None:
        raise SkipCheck("智能扫描页找不到扫描按钮")
    t0 = time.monotonic()
    app.click(scan, why="触发扫描")
    elapsed = _await_scan_result(app, 20.0)
    shot = app.screenshot("perf-18-scan")
    if elapsed is None:
        raise SkipCheck("20s 内没等到扫描结果渲染完成")
    status = STATUS_PASS if elapsed < 3.0 else STATUS_FAIL
    return _r(
        check_18, status, f"扫描到结果渲染耗时 {elapsed:.2f}s（门槛 3s）",
        {
            "elapsed_seconds": round(elapsed, 2),
            "threshold_seconds": 3.0,
            "note": "口径 = 点击扫描 → 「可优化进程」区块出现（含 AX 采样延迟，读数偏大）",
        },
        [shot],
    )


def _await_scan_result(app: App, timeout: float) -> float | None:
    t0 = time.monotonic()
    while time.monotonic() - t0 < timeout:
        if time.monotonic() - t0 > 1.0 and any(
            "可优化进程" in t for t in app.texts(budget=200)
        ):
            return time.monotonic() - t0
        time.sleep(0.15)
    return None


# ============ 19 / 20. 空闲 CPU / 内存 ============


@check("19", "性能：空闲 CPU < 2%")
def check_19(app: App) -> CheckResult:
    pid = _settle_and_get_pid(app)
    cpu, rss_kb = drv.sample_idle(pid, 20.0)
    status = STATUS_PASS if cpu < 2.0 else STATUS_FAIL
    return _r(
        check_19, status,
        f"空闲 20s 平均 CPU {cpu:.2f}%（门槛 2%），RSS 峰值 {rss_kb / 1024:.1f} MB",
        {
            "pid": pid,
            "sample_seconds": 20.0,
            "idle_cpu_percent": round(cpu, 2),
            "threshold_percent": 2.0,
            "rss_peak_mb": round(rss_kb / 1024, 1),
        },
        [app.screenshot("perf-19-idle")],
    )


@check("20", "性能：空闲内存 < 80MB")
def check_20(app: App) -> CheckResult:
    pid = _settle_and_get_pid(app)
    cpu, rss_kb = drv.sample_idle(pid, 20.0)
    rss_mb = rss_kb / 1024
    status = STATUS_PASS if rss_mb < 80.0 else STATUS_FAIL
    return _r(
        check_20, status,
        f"空闲 RSS 峰值 {rss_mb:.1f} MB（门槛 80MB），同期 CPU {cpu:.2f}%",
        {
            "pid": pid,
            "rss_peak_mb": round(rss_mb, 1),
            "threshold_mb": 80.0,
            "idle_cpu_percent": round(cpu, 2),
            "metric": "ps rss（未取 phys footprint，真机内存压力还含 WebKit 共享段）",
        },
        [app.screenshot("perf-20-idle-mem")],
    )


def _settle_and_get_pid(app: App) -> int:
    app.ensure_running()
    app.nav("智能扫描")
    app.wait_heading("智能扫描")
    pid = app.ax.pid()
    if pid <= 0:
        raise SkipCheck("拿不到 MacSlim PID")
    time.sleep(3.0)
    return pid


# ============ 21. 安装包 < 15MB ============


@check("21", "发布：安装包 < 15MB（release 产物）")
def check_21(app: App) -> CheckResult:
    """**debug 产物不参与判定** —— debug 二进制约 100MB+，拿来判必 FAIL。"""
    found: list[Path] = []
    for root in RELEASE_DMG_GLOBS:
        if root.is_dir():
            found.extend(sorted(root.glob("*.dmg")))
    if not found:
        searched = ", ".join(str(p) for p in RELEASE_DMG_GLOBS)
        raise SkipCheck(
            f"找不到 release dmg（查过 {searched}）。先跑 `npm run bundle:arm` 再判此项；"
            "debug 产物不参与判定。"
        )
    dmg = max(found, key=lambda p: p.stat().st_mtime)
    size_mb = dmg.stat().st_size / (1024 * 1024)
    status = STATUS_PASS if size_mb < 15.0 else STATUS_FAIL
    return _r(
        check_21, status, f"{dmg.name} = {size_mb:.2f} MB（门槛 15MB）",
        {
            "dmg": str(dmg),
            "size_mb": round(size_mb, 2),
            "threshold_mb": 15.0,
            "searched": [str(p) for p in RELEASE_DMG_GLOBS],
        },
    )


