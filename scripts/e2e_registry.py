#!/usr/bin/env python3
"""MacSlim e2e 门禁的结果模型与注册表（`scripts/e2e_gui.py` / `e2e_checks.py` 共用）。

拆成独立模块是为了让「模型」和「检查实现」都能待在 AGENTS.md §7.1 的文件行数上限内，
同时不引入 `scripts/e2e_gate/` 这类子目录 —— 子目录会躲过
`scripts/tests/test_function_limits.py` 对 `scripts/*.py` 的 50 行函数门禁。
"""
from __future__ import annotations

import json
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Callable

try:
    from scripts import e2e_driver as drv
except ModuleNotFoundError:  # 直接 `python3 scripts/e2e_checks.py` 时走这条
    import e2e_driver as drv

REPO = drv.REPO
ARTIFACTS = drv.ARTIFACTS

STATUS_PASS = "PASS"
STATUS_FAIL = "FAIL"
STATUS_SKIP = "SKIP"
STATUS_MANUAL = "MANUAL"
ALL_STATUSES = (STATUS_PASS, STATUS_FAIL, STATUS_SKIP, STATUS_MANUAL)

SkipCheck = drv.SkipCheck
ManualCheck = drv.ManualCheck
CheckFailure = drv.CheckFailure
AxNode = drv.AxNode
App = drv.App
Ax = drv.Ax

# 第 21 项判「安装包 < 15MB」用。**release 产物才参与判定** —— debug 二进制约
# 100MB+，拿它判必 FAIL，会把一个真问题伪装成门禁自身故障。
RELEASE_DMG_GLOBS = (
    Path.home() / ".cargo/shared-target/aarch64-apple-darwin/release/bundle/dmg",
    REPO / "src-tauri/target/aarch64-apple-darwin/release/bundle/dmg",
)

SAFETY_SELF_AUDIT = {
    "principle": "所有破坏性目标都由脚本自己造，绝不选中用户真实数据",
    "destructive_targets": {
        "2": "~/Library/Caches/macslim-e2e-sac-<tag>-<uuid10>（脚本 mkdir + 写 8MiB，finally rmtree）",
        "3": "/bin/sleep 牺牲进程（脚本 Popen，finally kill）",
        "7": "~/Applications/MacSlimE2E-cancel-<uuid8>.app（只取消不卸载，finally rmtree）",
        "8": "~/Applications/MacSlimE2E-uninstall-<uuid8>.app（finally rmtree）+ 对照 bundle",
        "17": "~/Applications/MacSlimE2E-residue-<uuid8>.app（finally rmtree）",
        "6": "~/Applications/MacSlimE2E-quit-<uuid8>.app（MANUAL，finally pkill + rmtree）",
    },
    "never_targeted": [
        "用户真实进程（第 3/9/11 项只按脚本 spawn 的 PID 精确搜索，绝不做全选）",
        "/Applications 与 ~/Applications 里的既有 app（只操作随机后缀的自建 bundle）",
        "~/Library/Caches 与 ~/Library/Logs 里的既有目录（第 1 项只 prepare 不 execute）",
        "任何 ~/.npm / ~/.cargo / Docker / Xcode 缓存（第 1 项只点取消）",
    ],
    "reverse_assertions": [
        "3: 未勾选的对照 sleep 进程必须仍存活，且 MacSlim 自身 PID 不变",
        "2: 未勾选的对照缓存目录必须仍存在",
        "8: 未勾选的对照 bundle 必须仍存在",
        "1/7: 取消后 SQLite 历史表行数必须零新增",
    ],
    "conclusion": (
        "脚本内没有任何指向用户既有进程 / app / 缓存的选择逻辑；"
        "全部破坏性目标自造，且每项带反向断言"
    ),
}


@dataclass
class CheckResult:
    """单项检查结果。`evidence` 是给人看的硬证据，不是像素。"""

    name: str
    status: str
    title: str
    detail: str
    evidence: dict = field(default_factory=dict)
    screenshots: list = field(default_factory=list)
    destructive_targets: list = field(default_factory=list)
    seconds: float = 0.0

    @property
    def passed(self) -> bool:
        return self.status == STATUS_PASS

    def line(self) -> str:
        return f"{self.status:<6} {self.name:<4} {self.title} — {self.detail}"


CHECKS: list[dict] = []


def check(num: str, title: str, note: str = "") -> Callable:
    """注册一个检查项。`note` 写明这一项在真实窗口上能不能自动判定。"""

    def deco(fn: Callable[[App], CheckResult]) -> Callable[[App], CheckResult]:
        fn.check_num = num  # type: ignore[attr-defined]
        fn.check_title = title  # type: ignore[attr-defined]
        CHECKS.append({"num": num, "title": title, "fn": fn, "note": note})
        return fn

    return deco


def result(
    fn: Callable,
    status: str,
    detail: str,
    evidence: dict | None = None,
    shots: list | None = None,
    targets: list | None = None,
) -> CheckResult:
    return CheckResult(
        name=getattr(fn, "check_num"),
        status=status,
        title=getattr(fn, "check_title"),
        detail=detail,
        evidence=evidence or {},
        screenshots=list(shots or []),
        destructive_targets=list(targets or []),
    )


def ok(
    fn: Callable, detail: str, evidence: dict, shots=None, targets=None
) -> CheckResult:
    return result(fn, STATUS_PASS, detail, evidence, shots, targets)


def _r(
    fn: Callable, status: str, detail: str, evidence: dict, shots=None, targets=None
) -> CheckResult:
    """`ok` 的阈值判定版：调用方自己算出 PASS/FAIL（例如实测 2.9s vs 门槛 3s）。

    与 `ok` 分开而不是加参数，是为了让「无条件通过」和「算出来才通过」在
    调用点上一眼可辨 —— 门禁报告里 PASS 的来源需要可追。
    """
    return result(fn, status, detail, evidence, shots, targets)


def summarize(results: list[CheckResult]) -> dict:
    counts = {s: 0 for s in ALL_STATUSES}
    for res in results:
        counts[res.status] = counts.get(res.status, 0) + 1
    return counts


def write_report(results: list[CheckResult], counts: dict, window_rect: list) -> Path:
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    payload = {
        "generated_at": stamp,
        "app_pid": Ax().pid(),
        "window_rect": window_rect,
        "counts": counts,
        "checks": [asdict(r) for r in results],
        "safety_self_audit": SAFETY_SELF_AUDIT,
    }
    path = ARTIFACTS / f"{stamp}.json"
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
    return path


def print_summary(results: list[CheckResult], counts: dict, path: Path) -> None:
    print("\n" + "=" * 96)
    print(f"{'状态':<7}{'#':<4}{'检查项':<46}证据")
    print("-" * 96)
    for res in results:
        title = res.title if len(res.title) <= 44 else res.title[:43] + "…"
        print(f"{res.status:<7}{res.name:<4}{title:<46}{res.detail}")
    print("=" * 96)
    print(
        f"PASS {counts[STATUS_PASS]} · FAIL {counts[STATUS_FAIL]} · "
        f"SKIP {counts[STATUS_SKIP]} · MANUAL {counts[STATUS_MANUAL]}"
    )
    print(f"报告: {path.relative_to(REPO)}")
    print(f"安全自查: {SAFETY_SELF_AUDIT['conclusion']}")
