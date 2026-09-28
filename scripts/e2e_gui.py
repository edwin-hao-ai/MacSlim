#!/usr/bin/env python3
"""MacSlim 发布 go/no-go 门禁 —— 21 项 e2e 回归。

需求来源：`docs/superpowers/mas-feasibility-and-e2e-release-gate.md` §3。

## 断言原则（§3.1）

**不靠像素。** `screencapture` 只产出人工复核产物，自动断言只打可观测副作用：

- 牺牲进程 PID 的存活状态（`os.kill(pid, 0)`）
- 目标路径是否消失 / 是否仍在
- 清理前后体积差值 与 SQLite 历史 `freed_bytes` 的一致性
- 阶段进度渲染出的「已完成 n/N」与阶段名
- 同一 `operation_id` 二次执行是否产生第二次副作用
- 窗口 `position` / `size`
- SQLite 历史记录行数与字段

DOM 属性（`disabled` / `checked`）走**无障碍树投影**读取：WKWebView 把
`<input disabled>` 映射成 `AXCheckBox` + `AXEnabled=false`，把 `checked` 映射成
`AXValue=0/1`。这是 DOM 派生属性，不是像素采样。

## 安全边界（§3.4）

**所有破坏性目标都由脚本自己造**，绝不选中用户真实进程 / 真实 app / 真实缓存：

| 破坏性检查 | 自造目标 | 造法 |
| :--- | :--- | :--- |
| 2 缓存清理 | `~/Library/Caches/macslim-e2e-sac-<uuid>` | 脚本 `mkdir` + 写 8 MiB |
| 3 进程终止 | `/bin/sleep` 牺牲进程 | 脚本 `Popen` |
| 7 / 8 应用卸载 | `~/Applications/MacSlimE2E-<uuid>.app` | 脚本拼 bundle + `chmod +x` |
| 6 优雅退出 | 同上（MANUAL，系统弹窗无法自动取消） | 同上 |

每一项破坏性检查都带**反向断言**（「不该发生的没发生」）：一个未被选中的对照牺牲
目标必须仍然存活 / 仍然存在。完整自查见 `e2e_registry.SAFETY_SELF_AUDIT`，并写进
产出的 JSON 报告。

## 落点

| 文件 | 职责 |
| :--- | :--- |
| `scripts/e2e_gui.py` | 本文件：CLI、驱动、汇总表、退出码 |
| `scripts/e2e_driver.py` | AX 探针、窗口驱动、可观测量、牺牲目标工厂 |
| `scripts/e2e_registry.py` | 结果模型、注册表、安全自查、报告写入 |
| `scripts/e2e_checks.py` | 第 1-8 项破坏性主链路 + 第 9-12 项 Broker 不变量 |
| `scripts/e2e_checks_readonly.py` | 第 13-17 项只读功能 + 第 18-21 项发布指标 |

入口 `bun run verify:e2e`，**不并入 `verify`**（`verify` 必须保持只读、无破坏性）。
产物 `artifacts/e2e/<timestamp>.json` + 截图（截图仅供人工复核）。

## 用法

```
python3 scripts/e2e_gui.py --list       # 只列检查项，不执行任何动作
python3 scripts/e2e_gui.py --only 13    # 单跑一项（只读）
python3 scripts/e2e_gui.py              # 全跑
```

退出码：任一 FAIL → 非 0；SKIP / MANUAL 不影响退出码（但会进报告等人工裁决）。
"""
from __future__ import annotations

import argparse
import subprocess
import sys
import time

try:
    from scripts import e2e_driver as drv
    from scripts import e2e_registry as reg
    import scripts.e2e_checks  # noqa: F401  触发 @check 注册
    import scripts.e2e_checks_readonly  # noqa: F401
except ModuleNotFoundError:  # `python3 scripts/e2e_gui.py` 时走这条
    import e2e_driver as drv
    import e2e_registry as reg
    import e2e_checks  # noqa: F401
    import e2e_checks_readonly  # noqa: F401

CheckFailure = reg.CheckFailure
REPO = drv.REPO


def execute(entry: dict, launch: bool) -> reg.CheckResult:
    """跑一项检查。任何异常都转成结果 —— 门禁不能因一项崩掉而中止。"""
    fn = entry["fn"]
    started = time.monotonic()
    app = reg.App(reg.Ax(), shot_prefix=f"chk{entry['num']}")
    try:
        outcome = fn(app)
    except reg.ManualCheck as exc:
        outcome = reg.result(fn, reg.STATUS_MANUAL, str(exc), {"needs": "human"})
    except reg.SkipCheck as exc:
        outcome = reg.result(fn, reg.STATUS_SKIP, str(exc), {"needs": "environment"})
    except CheckFailure as exc:
        outcome = reg.result(fn, reg.STATUS_FAIL, str(exc))
    except subprocess.TimeoutExpired as exc:
        outcome = reg.result(fn, reg.STATUS_FAIL, f"子进程超时: {exc}")
    except Exception as exc:  # noqa: BLE001
        outcome = reg.result(fn, reg.STATUS_FAIL, f"{type(exc).__name__}: {exc}")
    outcome.seconds = round(time.monotonic() - started, 2)
    return outcome


def print_list() -> None:
    print(f"MacSlim e2e 发布门禁 —— 共 {len(reg.CHECKS)} 项（--list 不执行任何动作）\n")
    width = max(len(c["title"]) for c in reg.CHECKS) + 2
    for entry in reg.CHECKS:
        note = f"  [{entry['note']}]" if entry["note"] else ""
        print(f"  {entry['num']:>2}. {entry['title']:<{width}}{note}")
    print(
        "\n破坏性检查（2 / 3 / 8）会真的删文件、真的杀进程，但目标全部由脚本自己造：\n"
        "  · /bin/sleep 牺牲进程\n"
        "  · ~/Library/Caches/macslim-e2e-sac-*（脚本 mkdir + 写 8MiB）\n"
        "  · ~/Applications/MacSlimE2E-*.app（脚本拼 bundle）\n"
        "每项都带反向断言；`verify:e2e` 不并入 `verify`。"
    )


def select(only: list[str]) -> list[dict]:
    if not only:
        return list(reg.CHECKS)
    wanted = {n.strip() for n in only}
    known = {c["num"] for c in reg.CHECKS}
    unknown = wanted - known
    if unknown:
        print(f"未知检查项: {sorted(unknown)}", file=sys.stderr)
        raise SystemExit(2)
    return [c for c in reg.CHECKS if c["num"] in wanted]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="MacSlim 发布 go/no-go 门禁（21 项 e2e 回归）")
    parser.add_argument("--list", action="store_true", help="只列检查项，不执行")
    parser.add_argument("--only", action="append", default=[], help="只跑指定编号，可重复")
    parser.add_argument(
        "--no-launch", action="store_true", help="MacSlim 未运行时不自动拉起"
    )
    args = parser.parse_args(argv)

    if args.list:
        print_list()
        return 0

    results = []
    for entry in select(args.only):
        print(f"→ {entry['num']}. {entry['title']} …", flush=True)
        outcome = execute(entry, launch=not args.no_launch)
        results.append(outcome)
        print(f"  {outcome.line()}  ({outcome.seconds}s)", flush=True)

    counts = reg.summarize(results)
    window_rect = list(drv.NORMAL_POS) + list(drv.NORMAL_SIZE)
    path = reg.write_report(results, counts, window_rect)
    reg.print_summary(results, counts, path)
    return 1 if counts[reg.STATUS_FAIL] else 0


if __name__ == "__main__":
    raise SystemExit(main())
