# MacSlim P0 窗口与安全配置基线 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 MacSlim 安全拖动区域，建立前端 lint/typecheck/test 基线，并用可执行检查收紧 CSP、capability、entitlements 与发布脚本。

**Architecture:** 前端使用独立 `AppShell` 与 `SafeDragSurface`，只保留 `getCurrentWindow().startDragging()` 一条拖动路径，并通过最小 Tauri capability 授权。发布配置由 Python 标准库校验器统一检查，CI 和本地 `verify` 脚本共同执行；本阶段不迁移缓存清理协议，也不实现 Liquid Glass 或 MAS target。

**Tech Stack:** SolidJS 1.9、TypeScript 5.7、Vite 6、Tauri 2、Vitest 3.2、Solid Testing Library、Oxlint 1.85、Python 3 标准库、Rust 1.80+

**Spec:** `docs/superpowers/specs/2026-09-24-macslim-product-hardening-app-store-design.md`

## Global Constraints

- 所有用户可见文案、错误提示与文档使用中文；技术专有名词保留英文。
- 不新增代码注释；修改文件时保留既有注释，不创建新的注释。
- 不打印、复制、提交或自动轮换任何凭据。
- 不执行真实 `sudo`、进程终止、Docker prune、永久删除或用户目录清理测试。
- 保留现有工作区改动；`.gitignore` 已被用户修改，只能做最小追加。
- 未经用户明确要求，不执行 `git commit`、`git add`、push 或其他远程操作。
- 发布与公证以本机已确认的 `Developer ID Application: Beijing VGO Co;Ltd (5XNDF727Y6)` 和可用 `macslim-notary` Keychain profile 为准；不得读取或输出私钥、密码。
- 发布实现参考 `/Users/edwinhao/MDDock/docs/build-release.md`、`docs/internal/release-desktop.md`、`docs/manuals/release/RELEASE-SOP.md` 与 `scripts/sign-dmg.sh` 的已验证通用原则：保留 secure timestamp，单独 notarize/staple DMG，并用 codesign、stapler、spctl 验收；不得复制 MDDock 专属域名、路径或凭据。
- 拖动仅允许标题栏、Sidebar 品牌区和显式 `SafeDragSurface`；按钮、输入、链接、卡片、列表和内容滚动区不得触发。
- 正式配置不得出现 CSP 通配符、`unsafe-eval`、远程脚本或未列入 allowlist 的 capability。
- 本计划不实现 `NSGlassEffectView`、A1 视觉重构、Space Rescue、Pro 收费、缓存 operation ID 或 Mac App Store target。

---

## File Map

### 前端拖动与验证

- Modify: `package.json` — lint、typecheck、Vitest 与 verify 脚本。
- Modify: `bun.lock` — 锁定新增开发依赖，由 Bun 生成。
- Modify: `vite.config.ts` — 复用现有配置加入 jsdom Vitest。
- Modify: `src/lib/window-drag.ts` — 纯目标策略与唯一 Tauri 拖动边界。
- Create: `src/lib/window-drag.test.ts` — 拖动目标与按钮键测试。
- Create: `src/components/shell/SafeDragSurface.tsx` — 显式安全拖动容器。
- Create: `src/components/shell/SafeDragSurface.test.tsx` — 子控件点击与拖动隔离测试。
- Create: `src/components/shell/AppShell.tsx` — 无业务依赖的窗口布局插槽。
- Create: `src/components/shell/AppShell.test.tsx` — 工具栏可拖、内容不可拖测试。
- Modify: `src/App.tsx:1-61` — 保留状态、托盘监听和 TabPanel，只替换布局壳。
- Modify: `src/components/Sidebar.tsx:1-76` — 导航恢复纯交互，拖动只放在品牌区。
- Modify: `src/styles.css:152-164` — 删除失效的 WebKit drag region 规则。
- Modify: `src-tauri/capabilities/default.json:6-20` — 增加 start dragging 并收敛 allowlist。

### 安全配置与发布

- Modify: `src-tauri/tauri.conf.json:29-31` — 生产 CSP、开发 CSP。
- Modify: `src-tauri/entitlements.plist:4-26` — 只保留 JIT、网络 client、Apple Events。
- Create: `scripts/verify-security-config.py` — JSON、plist、CSP、capability、entitlements 与脚本契约校验。
- Create: `scripts/tests/test_verify_security_config.py` — 校验器单元测试。
- Modify: `scripts/publish-update.sh:32-116` — 删除密码 fallback，安全生成 manifest。
- Modify: `scripts/release.sh:44-81` — 公证失败必须非零退出并验证票据。
- Modify: `scripts/sign.sh:10-111` — 统一 Keychain profile，移除无时间戳成功路径。
- Modify: `scripts/README.md:25-43` — 只记录变量名和 Keychain 流程。
- Modify: `.gitignore:29-52` — 最小追加 `.superpowers/` 与 App Store Connect 私钥模式。
- Modify: `.github/workflows/ci.yml:1-90` — 最小权限、固定 Bun、完整验证。

---

### Task 1: 建立前端验证基线并修复安全拖动

**Files:**
- Modify: `package.json:6-35`
- Modify: `bun.lock`
- Modify: `vite.config.ts:1-27`
- Modify: `src/lib/window-drag.ts:1-21`
- Create: `src/lib/window-drag.test.ts`
- Create: `src/components/shell/SafeDragSurface.tsx`
- Create: `src/components/shell/SafeDragSurface.test.tsx`
- Create: `src/components/shell/AppShell.tsx`
- Create: `src/components/shell/AppShell.test.tsx`
- Modify: `src/App.tsx:1-61`
- Modify: `src/components/Sidebar.tsx:1-76`
- Modify: `src/styles.css:152-164`
- Modify: `src-tauri/capabilities/default.json:6-20`

**Interfaces:**
- Consumes: Tauri `getCurrentWindow().startDragging(): Promise<void>`、现有 `ViewId`、七个业务 View。
- Produces: `isSafeWindowDragTarget(target: EventTarget | null): boolean`、`handleWindowDrag(event: MouseEvent): void`、`SafeDragSurface(props: ParentProps<HTMLDivElement>): JSX.Element`、`AppShell(props: AppShellProps): JSX.Element`。

- [ ] **Step 1: 记录工作区基线**

Run:

```bash
git status --short
git diff -- .gitignore
```

Expected: 记录用户已有改动；不得还原或覆盖 `.gitignore`。

- [ ] **Step 2: 安装固定版本的前端验证依赖**

Run:

```bash
bun add -d oxlint@1.85.0 vitest@3.2.4 @solidjs/testing-library@0.8.10 jsdom@26.1.0
```

Expected: `package.json` 与 `bun.lock` 更新，命令退出 0。

- [ ] **Step 3: 增加脚本和 Vitest 配置**

在 `package.json` 中设置：

```json
{
  "scripts": {
    "dev": "vite",
    "build": "bun run typecheck && vite build",
    "typecheck": "tsc --noEmit",
    "lint": "oxlint src",
    "test": "vitest run",
    "test:watch": "vitest",
    "tauri": "tauri",
    "bundle:arm": "tauri build --target aarch64-apple-darwin",
    "bundle:intel": "tauri build --target x86_64-apple-darwin",
    "bundle:universal": "tauri build --target universal-apple-darwin"
  }
}
```

将 `vite.config.ts` 的配置入口改为：

```ts
import { defineConfig } from "vitest/config";
```

并在现有对象中加入：

```ts
test: {
  environment: "jsdom",
  include: ["src/**/*.test.{ts,tsx}"],
  clearMocks: true,
  restoreMocks: true,
},
```

- [ ] **Step 4: 写拖动策略失败测试**

创建 `src/lib/window-drag.test.ts`：

```ts
import { beforeEach, describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  startDragging: vi.fn(async () => undefined),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startDragging: tauriMocks.startDragging }),
}));

import {
  handleWindowDrag,
  isSafeWindowDragTarget,
} from "@/lib/window-drag";

const dispatchMouseDown = (target: Element, button = 0) => {
  const event = new MouseEvent("mousedown", {
    bubbles: true,
    cancelable: true,
    button,
  });
  target.dispatchEvent(event);
  return event;
};

describe("window drag", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    tauriMocks.startDragging.mockClear();
  });

  it("starts dragging for a safe left-button target", () => {
    const surface = document.createElement("div");
    document.body.append(surface);

    const event = dispatchMouseDown(surface);

    expect(event.defaultPrevented).toBe(true);
    expect(tauriMocks.startDragging).toHaveBeenCalledTimes(1);
  });

  it("ignores non-primary buttons", () => {
    const surface = document.createElement("div");
    document.body.append(surface);

    const event = dispatchMouseDown(surface, 1);

    expect(event.defaultPrevented).toBe(false);
    expect(tauriMocks.startDragging).not.toHaveBeenCalled();
  });

  it.each([
    "button",
    "input",
    "textarea",
    "select",
    "a",
    "label",
    "summary",
    "li",
  ])("rejects interactive selector %s", (tagName) => {
    const element = document.createElement(tagName);
    document.body.append(element);
    expect(isSafeWindowDragTarget(element)).toBe(false);
  });

  it("rejects cards and explicit no-drag elements", () => {
    const card = document.createElement("div");
    card.className = "card";
    const noDrag = document.createElement("div");
    noDrag.dataset.noDrag = "true";
    document.body.append(card, noDrag);

    expect(isSafeWindowDragTarget(card)).toBe(false);
    expect(isSafeWindowDragTarget(noDrag)).toBe(false);
  });

  it("rejects contenteditable elements", () => {
    const editable = document.createElement("div");
    editable.contentEditable = "true";
    document.body.append(editable);

    expect(isSafeWindowDragTarget(editable)).toBe(false);
  });
});
```

- [ ] **Step 5: 运行测试并确认 RED**

Run:

```bash
bun run test -- src/lib/window-drag.test.ts
```

Expected: FAIL，原因是 `isSafeWindowDragTarget` 尚不存在，且当前实现未排除 `summary` 与 `contenteditable`。

- [ ] **Step 6: 实现唯一拖动边界**

将 `src/lib/window-drag.ts` 替换为：

```ts
import { getCurrentWindow } from "@tauri-apps/api/window";

const NON_DRAG_SELECTOR = [
  "button",
  "input",
  "textarea",
  "select",
  "a",
  "label",
  "summary",
  ".card",
  "li",
  "[data-no-drag]",
  "[contenteditable]:not([contenteditable='false'])",
].join(",");

export const isSafeWindowDragTarget = (target: EventTarget | null) => {
  if (!(target instanceof Element)) return false;
  return target.closest(NON_DRAG_SELECTOR) === null;
};

export const handleWindowDrag = (event: MouseEvent) => {
  if (event.button !== 0 || !isSafeWindowDragTarget(event.target)) return;
  event.preventDefault();
  void getCurrentWindow()
    .startDragging()
    .catch((error: unknown) => {
      console.error("拖动窗口失败", error);
    });
};
```

- [ ] **Step 7: 写 SafeDragSurface 和 AppShell 失败测试**

创建 `src/components/shell/SafeDragSurface.test.tsx`：

```tsx
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  startDragging: vi.fn(async () => undefined),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startDragging: tauriMocks.startDragging }),
}));

import SafeDragSurface from "@/components/shell/SafeDragSurface";

describe("SafeDragSurface", () => {
  beforeEach(() => tauriMocks.startDragging.mockClear());
  afterEach(cleanup);

  it("drags from the surface but preserves nested controls", () => {
    let clicks = 0;
    render(() => (
      <SafeDragSurface data-testid="surface">
        <button onClick={() => clicks += 1}>操作</button>
      </SafeDragSurface>
    ));

    const surface = screen.getByTestId("surface");
    const button = screen.getByRole("button", { name: "操作" });

    fireEvent.mouseDown(surface, { button: 0 });
    fireEvent.mouseDown(button, { button: 0 });
    fireEvent.click(button);

    expect(tauriMocks.startDragging).toHaveBeenCalledTimes(1);
    expect(clicks).toBe(1);
  });
});
```

创建 `src/components/shell/AppShell.test.tsx`：

```tsx
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  startDragging: vi.fn(async () => undefined),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startDragging: tauriMocks.startDragging }),
}));

import AppShell from "@/components/shell/AppShell";

describe("AppShell", () => {
  beforeEach(() => tauriMocks.startDragging.mockClear());
  afterEach(cleanup);

  it("only exposes the toolbar as a drag surface", () => {
    render(() => (
      <AppShell
        sidebar={<aside data-testid="sidebar">Sidebar</aside>}
        toolbar={<h1>智能扫描</h1>}
      >
        <div data-testid="content">内容</div>
      </AppShell>
    ));

    fireEvent.mouseDown(screen.getByRole("heading", { name: "智能扫描" }), {
      button: 0,
    });
    fireEvent.mouseDown(screen.getByTestId("content"), { button: 0 });

    expect(tauriMocks.startDragging).toHaveBeenCalledTimes(1);
  });
});
```

- [ ] **Step 8: 运行组件测试并确认 RED**

Run:

```bash
bun run test -- src/components/shell/SafeDragSurface.test.tsx src/components/shell/AppShell.test.tsx
```

Expected: FAIL，原因是两个组件尚不存在。

- [ ] **Step 9: 实现 SafeDragSurface 与 AppShell**

创建 `src/components/shell/SafeDragSurface.tsx`：

```tsx
import { type Component, type ParentProps } from "solid-js";
import { handleWindowDrag } from "@/lib/window-drag";

const SafeDragSurface: Component<ParentProps<HTMLDivElement>> = (props) => (
  <div {...props} onMouseDown={handleWindowDrag} />
);

export default SafeDragSurface;
```

创建 `src/components/shell/AppShell.tsx`：

```tsx
import { type Component, type JSX } from "solid-js";
import SafeDragSurface from "@/components/shell/SafeDragSurface";

type AppShellProps = {
  sidebar: JSX.Element;
  toolbar: JSX.Element;
  children: JSX.Element;
};

const AppShell: Component<AppShellProps> = (props) => (
  <div class="flex h-full bg-[rgb(var(--bg-app))/var(--bg-app-alpha)]">
    {props.sidebar}
    <main class="flex min-w-0 flex-1 flex-col">
      <SafeDragSurface class="flex h-12 items-center border-b border-black/5 px-6 dark:border-white/5">
        {props.toolbar}
      </SafeDragSurface>
      <div class="min-h-0 flex-1 overflow-hidden">{props.children}</div>
    </main>
  </div>
);

export default AppShell;
```

- [ ] **Step 10: 接入 App 与 Sidebar，移除重复拖动路径**

在 `src/App.tsx` 中删除 `handleWindowDrag` import，保留 `ViewId`、信号、托盘监听和 `TabPanel`，把根布局改为：

```tsx
return (
  <AppShell
    sidebar={<Sidebar current={view()} onChange={setView} />}
    toolbar={
      <h1 class="pointer-events-none text-sm font-medium text-zinc-500">
        {t(`nav.${view()}`)}
      </h1>
    }
  >
    <TabPanel id="scan" active={view()}><ScanView /></TabPanel>
    <TabPanel id="process" active={view()}><ProcessView /></TabPanel>
    <TabPanel id="applications" active={view()}><ApplicationsView /></TabPanel>
    <TabPanel id="cache" active={view()}><CacheView /></TabPanel>
    <TabPanel id="uninstaller" active={view()}><UninstallerView /></TabPanel>
    <TabPanel id="history" active={view()}><HistoryView /></TabPanel>
    <TabPanel id="settings" active={view()}><SettingsView /></TabPanel>
  </AppShell>
);
```

在 `src/components/Sidebar.tsx` 中删除 `handleWindowDrag` import、`aside` 上的 `data-tauri-drag-region` 和 `onMouseDown`，把品牌区域改为：

```tsx
<SafeDragSurface class="flex h-13 items-end px-5 pb-2">
  <div class="flex items-center gap-2">
    <div class="flex h-6 w-6 items-center justify-center rounded-lg bg-gradient-to-br from-brand-400 to-brand-600 shadow-sm">
      <Activity size={13} class="text-white" />
    </div>
    <span class="text-[15px] font-semibold tracking-tight">
      {t("common.appName")}
    </span>
  </div>
</SafeDragSurface>
```

并增加：

```tsx
import SafeDragSurface from "@/components/shell/SafeDragSurface";
```

从 `src/styles.css` 删除 `.drag-region`、`.drag-region *` 和 `.no-drag` 三个规则块。

- [ ] **Step 11: 增加运行时 capability**

将 `src-tauri/capabilities/default.json` 的 permissions 改为：

```json
[
  "core:app:allow-version",
  "core:event:allow-listen",
  "core:window:allow-start-dragging",
  "autostart:allow-enable",
  "autostart:allow-disable",
  "autostart:allow-is-enabled",
  "notification:allow-is-permission-granted",
  "notification:allow-request-permission",
  "notification:allow-notify",
  "process:allow-restart",
  "updater:allow-check",
  "updater:allow-download-and-install"
]
```

- [ ] **Step 12: 运行前端验证**

Run:

```bash
bun run test -- src/lib/window-drag.test.ts src/components/shell/SafeDragSurface.test.tsx src/components/shell/AppShell.test.tsx
bun run test
bun run lint
bun run typecheck
bun run build
```

Expected: 全部退出 0；无未处理 Promise、TypeScript 错误、lint error 或 Vite build error。

- [ ] **Step 13: 原生窗口冒烟验证**

Run:

```bash
bun run tauri dev
```

Expected:

1. 顶部标题栏空白处可拖动窗口。
2. Sidebar 品牌区可拖动窗口。
3. Sidebar 导航按钮正常点击。
4. 主内容、卡片、列表和滚动区不触发拖动。
5. Web Inspector 不再出现 `start_dragging not allowed by ACL`。
6. 在 900×600 与 800×540 两种尺寸各验证一次。

---

### Task 2: 用自动化校验收紧 CSP、capability 与 entitlements

**Files:**
- Create: `scripts/verify-security-config.py`
- Create: `scripts/tests/test_verify_security_config.py`
- Modify: `src-tauri/tauri.conf.json:29-31`
- Modify: `src-tauri/capabilities/default.json:6-20`
- Modify: `src-tauri/entitlements.plist:4-26`

**Interfaces:**
- Consumes: 仓库根目录、Task 1 的最终 capability allowlist。
- Produces: `validate_csp(value: object) -> list[str]`、`validate_capabilities(value: object) -> list[str]`、`validate_entitlements(value: dict) -> list[str]`、`validate_secret_fallbacks(files: dict[str, str]) -> list[str]`、CLI `scripts/verify-security-config.py`。

- [ ] **Step 1: 写校验器失败测试**

创建 `scripts/tests/test_verify_security_config.py`：

```python
import importlib.util
import pathlib
import unittest

SCRIPT_PATH = pathlib.Path(__file__).resolve().parents[1] / "verify-security-config.py"
SPEC = importlib.util.spec_from_file_location("verify_security_config", SCRIPT_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)

VALID_CSP = (
    "default-src 'self' ipc: http://ipc.localhost; "
    "connect-src ipc: http://ipc.localhost; "
    "img-src 'self' asset: http://asset.localhost blob: data:; "
    "style-src 'self' 'unsafe-inline'; script-src 'self'; "
    "object-src 'none'; base-uri 'self'; frame-src 'none'; frame-ancestors 'none'"
)


class SecurityConfigTests(unittest.TestCase):
    def test_accepts_production_csp(self):
        self.assertEqual(MODULE.validate_csp(VALID_CSP), [])

    def test_rejects_unsafe_eval(self):
        errors = MODULE.validate_csp("default-src 'self'; script-src 'self' 'unsafe-eval'")
        self.assertTrue(any("unsafe-eval" in error for error in errors))

    def test_rejects_capability_drift(self):
        errors = MODULE.validate_capabilities({"permissions": ["core:default"]})
        self.assertTrue(any("capability" in error for error in errors))

    def test_rejects_extra_entitlement(self):
        errors = MODULE.validate_entitlements(
            {"com.apple.security.cs.disable-library-validation": True}
        )
        self.assertTrue(any("entitlement" in error for error in errors))

    def test_rejects_inline_secret_fallback(self):
        errors = MODULE.validate_secret_fallbacks(
            {
                "scripts/publish-update.sh": (
                    'export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-fallback}"\n'
                )
            }
        )
        self.assertTrue(any("publish-update.sh:1" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: 运行测试并确认 RED**

Run:

```bash
python3 -m unittest discover -s scripts/tests -p "test_*.py"
```

Expected: ERROR，因为 `scripts/verify-security-config.py` 尚不存在。

- [ ] **Step 3: 实现安全配置校验器**

创建 `scripts/verify-security-config.py`：

```python
#!/usr/bin/env python3

import json
import pathlib
import plistlib
import sys

EXPECTED_CAPABILITIES = {
    "core:app:allow-version",
    "core:event:allow-listen",
    "core:window:allow-start-dragging",
    "autostart:allow-enable",
    "autostart:allow-disable",
    "autostart:allow-is-enabled",
    "notification:allow-is-permission-granted",
    "notification:allow-request-permission",
    "notification:allow-notify",
    "process:allow-restart",
    "updater:allow-check",
    "updater:allow-download-and-install",
}

EXPECTED_ENTITLEMENTS = {
    "com.apple.security.cs.allow-jit",
    "com.apple.security.network.client",
    "com.apple.security.automation.apple-events",
}


def validate_csp(value: object) -> list[str]:
    if not isinstance(value, str) or not value.strip():
        return ["生产 CSP 不能为空"]
    errors = []
    lowered = value.lower()
    if "*" in value:
        errors.append("生产 CSP 不能包含通配符")
    if "unsafe-eval" in lowered:
        errors.append("生产 CSP 不能包含 unsafe-eval")
    if "http://" in lowered or "https://" in lowered:
        remote_sources = [part for part in value.split(";") if "http://" in part.lower() or "https://" in part.lower()]
        allowed_hosts = ("ipc.localhost", "asset.localhost")
        allowed = all(any(host in part.lower() for host in allowed_hosts) for part in remote_sources)
        if not allowed:
            errors.append("生产 CSP 不能加载远程脚本或资源")
    script_source = next(
        (part.strip() for part in value.split(";") if part.strip().lower().startswith("script-src")),
        "",
    )
    if "'unsafe-inline'" in script_source.lower():
        errors.append("script-src 不能包含 unsafe-inline")
    return errors


def validate_capabilities(value: object) -> list[str]:
    if not isinstance(value, dict) or not isinstance(value.get("permissions"), list):
        return ["capability permissions 必须是数组"]
    actual = set(value["permissions"])
    missing = sorted(EXPECTED_CAPABILITIES - actual)
    extra = sorted(actual - EXPECTED_CAPABILITIES)
    errors = []
    if missing:
        errors.append(f"capability 缺少: {', '.join(missing)}")
    if extra:
        errors.append(f"capability 多余: {', '.join(extra)}")
    return errors


def validate_entitlements(value: dict) -> list[str]:
    actual = {key for key, enabled in value.items() if enabled is True}
    missing = sorted(EXPECTED_ENTITLEMENTS - actual)
    extra = sorted(actual - EXPECTED_ENTITLEMENTS)
    errors = []
    if missing:
        errors.append(f"entitlement 缺少: {', '.join(missing)}")
    if extra:
        errors.append(f"entitlement 多余: {', '.join(extra)}")
    return errors


def validate_secret_fallbacks(files: dict[str, str]) -> list[str]:
    errors = []
    for name, content in files.items():
        for number, line in enumerate(content.splitlines(), start=1):
            if "TAURI_SIGNING_PRIVATE_KEY_PASSWORD" in line and ":-" in line:
                errors.append(f"{name}:{number} 存在凭据 fallback")
    return errors


def main() -> int:
    root = pathlib.Path(__file__).resolve().parents[1]
    tauri_path = root / "src-tauri/tauri.conf.json"
    capability_path = root / "src-tauri/capabilities/default.json"
    entitlements_path = root / "src-tauri/entitlements.plist"
    publish_path = root / "scripts/publish-update.sh"

    tauri = json.loads(tauri_path.read_text(encoding="utf-8"))
    capabilities = json.loads(capability_path.read_text(encoding="utf-8"))
    entitlements = plistlib.loads(entitlements_path.read_bytes())

    errors = []
    errors.extend(validate_csp(tauri.get("app", {}).get("security", {}).get("csp")))
    errors.extend(validate_capabilities(capabilities))
    errors.extend(validate_entitlements(entitlements))
    errors.extend(validate_secret_fallbacks({"scripts/publish-update.sh": publish_path.read_text(encoding="utf-8")}))

    if errors:
        for error in errors:
            print(f"错误: {error}", file=sys.stderr)
        return 1

    print("安全配置校验通过")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 4: 运行校验器测试并确认 GREEN**

Run:

```bash
python3 -m unittest discover -s scripts/tests -p "test_*.py"
```

Expected: 5 tests PASS。

- [ ] **Step 5: 配置生产 CSP 与开发 CSP**

将 `src-tauri/tauri.conf.json` 的 `app.security` 替换为：

```json
"security": {
  "csp": "default-src 'self' ipc: http://ipc.localhost; connect-src ipc: http://ipc.localhost; img-src 'self' asset: http://asset.localhost blob: data:; style-src 'self' 'unsafe-inline'; script-src 'self'; object-src 'none'; base-uri 'self'; frame-src 'none'; frame-ancestors 'none'",
  "devCsp": "default-src 'self' ipc: http://ipc.localhost http://localhost:1420; connect-src ipc: http://ipc.localhost http://localhost:1420 ws://localhost:1421; img-src 'self' asset: http://asset.localhost blob: data:; style-src 'self' 'unsafe-inline'; script-src 'self' http://localhost:1420; object-src 'none'; base-uri 'self'; frame-src 'none'; frame-ancestors 'none'"
}
```

- [ ] **Step 6: 收敛 Developer ID entitlements**

将 `src-tauri/entitlements.plist` 的 `dict` 内容替换为：

```xml
<dict>
    <key>com.apple.security.cs.allow-jit</key>
    <true/>
    <key>com.apple.security.network.client</key>
    <true/>
    <key>com.apple.security.automation.apple-events</key>
    <true/>
</dict>
```

- [ ] **Step 7: 运行静态与 Tauri 语义验证**

Run:

```bash
python3 scripts/verify-security-config.py
python3 -m json.tool src-tauri/tauri.conf.json >/dev/null
python3 -m json.tool src-tauri/capabilities/default.json >/dev/null
plutil -lint src-tauri/entitlements.plist
bun run tauri build --debug --no-bundle
```

Expected: 全部退出 0；无 capability identifier、CSP 或 plist 错误。

---

### Task 3: 让签名、公证与 Updater 发布脚本 fail closed

**Files:**
- Modify: `scripts/verify-security-config.py`
- Modify: `scripts/tests/test_verify_security_config.py`
- Modify: `scripts/publish-update.sh:32-116`
- Modify: `scripts/release.sh:44-81`
- Modify: `scripts/sign.sh:10-111`
- Modify: `scripts/README.md:25-43`
- Modify: `.gitignore:29-52`

**Interfaces:**
- Consumes: Task 2 的校验器。
- Produces: `validate_release_contracts(files: dict[str, str]) -> list[str]`；三个发布脚本在缺少签名、公证、timestamp 或 updater 凭据时必须返回非零。

- [ ] **Step 1: 写发布脚本契约失败测试**

在 `scripts/tests/test_verify_security_config.py` 增加：

```python
    def test_rejects_timestamp_fallback(self):
        errors = MODULE.validate_release_contracts(
            {"scripts/sign.sh": 'TIMESTAMP_FLAG="--timestamp=none"\nexit 0\n'}
        )
        self.assertTrue(any("timestamp" in error for error in errors))

    def test_rejects_ignored_gatekeeper_failure(self):
        errors = MODULE.validate_release_contracts(
            {"scripts/release.sh": 'spctl -a -vv -t install "$DMG_PATH" || true\n'}
        )
        self.assertTrue(any("Gatekeeper" in error for error in errors))

    def test_requires_consistent_notary_profile(self):
        errors = MODULE.validate_release_contracts(
            {
                "scripts/release.sh": 'PROFILE_NAME="macslim-notary"\n',
                "scripts/sign.sh": 'PROFILE_NAME="macflow-notary"\n',
            }
        )
        self.assertTrue(any("notary" in error for error in errors))

    def test_requires_local_certificate_preflight(self):
        errors = MODULE.validate_release_contracts(
            {
                "scripts/release.sh": 'PROFILE_NAME="macslim-notary"\n',
                "scripts/sign.sh": 'PROFILE_NAME="macslim-notary"\n',
            }
        )
        self.assertTrue(any("签名证书" in error for error in errors))

    def test_requires_app_and_dmg_notarization_checks(self):
        errors = MODULE.validate_release_contracts(
            {
                "scripts/release.sh": 'PROFILE_NAME="macslim-notary"\n',
                "scripts/sign.sh": 'PROFILE_NAME="macslim-notary"\n',
            }
        )
        self.assertTrue(any("stapler" in error for error in errors))
        self.assertTrue(any("Gatekeeper" in error for error in errors))
```

- [ ] **Step 2: 运行测试并确认 RED**

Run:

```bash
python3 -m unittest discover -s scripts/tests -p "test_*.py"
```

Expected: ERROR，因为 `validate_release_contracts` 尚不存在。

- [ ] **Step 3: 实现发布脚本契约校验**

在 `scripts/verify-security-config.py` 增加：

```python
def validate_release_contracts(files: dict[str, str]) -> list[str]:
    errors = []
    sign = files.get("scripts/sign.sh", "")
    release = files.get("scripts/release.sh", "")

    if "--timestamp=none" in sign or "使用无时间戳签名" in sign:
        errors.append("签名脚本不能接受无 timestamp 成功路径")
    if "spctl" in release and "|| true" in release:
        errors.append("release.sh 不能忽略 Gatekeeper 失败")
    if "exit 0" in release and "notarytool 凭证未配置" in release:
        errors.append("缺少 notary 凭据时必须非零退出")

    release_profile = ""
    sign_profile = ""
    for line in release.splitlines():
        if line.startswith("PROFILE_NAME="):
            release_profile = line.split("=", 1)[1].strip().strip('"')
    for line in sign.splitlines():
        if line.startswith("PROFILE_NAME="):
            sign_profile = line.split("=", 1)[1].strip().strip('"')
    if release_profile and sign_profile and release_profile != sign_profile:
        errors.append("release.sh 与 sign.sh 的 notary profile 不一致")

    for name, content in (("release.sh", release), ("sign.sh", sign)):
        if "security find-identity -v -p codesigning" not in content:
            errors.append(f"{name} 缺少本机签名证书预检")
        if "xcrun stapler validate \"$APP_PATH\"" not in content:
            errors.append(f"{name} 缺少 .app stapler 验证")
        if "xcrun stapler validate \"$DMG_PATH\"" not in content:
            errors.append(f"{name} 缺少 DMG stapler 验证")
        if "spctl -a -t exec -vv" not in content:
            errors.append(f"{name} 缺少 .app Gatekeeper 验证")
        if "spctl -a -t install -vv" not in content:
            errors.append(f"{name} 缺少 DMG Gatekeeper 验证")
    return errors
```

在 `main()` 中读取并校验：

```python
release_paths = {
    "scripts/publish-update.sh": root / "scripts/publish-update.sh",
    "scripts/release.sh": root / "scripts/release.sh",
    "scripts/sign.sh": root / "scripts/sign.sh",
}
release_files = {name: path.read_text(encoding="utf-8") for name, path in release_paths.items()}
errors.extend(validate_release_contracts(release_files))
```

- [ ] **Step 4: 更新 updater 凭据与 manifest 生成**

在 `scripts/publish-update.sh` 中删除默认密码赋值；默认密钥文件存在时仍要求显式密码：

```bash
if [ -f "$HOME/.tauri/macslim-updater.key" ]; then
  export TAURI_SIGNING_PRIVATE_KEY_PATH="$HOME/.tauri/macslim-updater.key"
  if [ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]; then
    echo "错误: 使用默认 updater 密钥时必须设置 TAURI_SIGNING_PRIVATE_KEY_PASSWORD" >&2
    exit 1
  fi
  echo "==> 使用 Keychain/环境变量提供的 updater 签名密码"
else
  echo "错误: 未找到 updater 私钥" >&2
  exit 1
fi
```

将 manifest 生成替换为不会把 release notes 插入 Python 源码的形式：

```bash
MANIFEST_PATH="${MANIFEST_DIR}/latest.json"
VERSION="$VERSION" \
NOTES="$NOTES" \
PUB_DATE="$PUB_DATE" \
SIGNATURE="$SIGNATURE" \
DMG_BASENAME="$DMG_BASENAME" \
MANIFEST_PATH="$MANIFEST_PATH" \
python3 - <<'PY'
import json
import os
import pathlib

manifest = {
    "version": os.environ["VERSION"],
    "notes": os.environ["NOTES"],
    "pub_date": os.environ["PUB_DATE"],
    "platforms": {
        "darwin-aarch64": {
            "signature": os.environ["SIGNATURE"],
            "url": f"https://github.com/edwin-hao-ai/MacSlim/releases/latest/download/{os.environ['DMG_BASENAME']}",
        }
    },
}
pathlib.Path(os.environ["MANIFEST_PATH"]).write_text(
    json.dumps(manifest, indent=2, ensure_ascii=False),
    encoding="utf-8",
)
PY
```

- [ ] **Step 5: 让 release.sh 在本机证书、notary profile 或 Gatekeeper 失败时退出非零**

在 `scripts/release.sh` 解析 target 后、调用 `bun run tauri build` 前加入预检：

```bash
SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-Developer ID Application: Beijing VGO Co;Ltd (${TEAM_ID})}"
PROFILE_NAME="${MACSlim_NOTARY_PROFILE:-macslim-notary}"

if ! security find-identity -v -p codesigning | grep -Fq "$SIGNING_IDENTITY"; then
  echo "错误: Keychain 中未找到签名证书 $SIGNING_IDENTITY" >&2
  exit 1
fi

if ! xcrun notarytool history --keychain-profile "$PROFILE_NAME" >/dev/null 2>&1; then
  echo "错误: notarytool Keychain profile 不可用: $PROFILE_NAME" >&2
  exit 1
fi

echo "==> 签名证书: $SIGNING_IDENTITY"
echo "==> Notary profile: $PROFILE_NAME"
```

保留 `set -euo pipefail`，将 `scripts/release.sh:44-59` 的缺失凭据分支结尾改为：

```bash
  echo "配置完后重跑此脚本即可自动 notarize。"
  exit 1
fi
```

在 staple 后加入：

```bash
xcrun stapler validate "$APP_PATH"
xcrun stapler validate "$DMG_PATH"
```

将最终验证改为：

```bash
echo "==> 签名与 Gatekeeper 验证..."
codesign --verify --deep --strict --verbose=2 "$APP_PATH"
codesign -dvvv "$APP_PATH" 2>&1 | grep -q "Timestamp="
codesign --verify --verbose=4 "$DMG_PATH"
spctl -a -t exec -vv "$APP_PATH"
spctl -a -t install -vv "$DMG_PATH"
```

`spctl` 或 secure timestamp 检查失败必须直接结束，不允许 `|| true`。

- [ ] **Step 6: 统一 sign.sh 的证书、secure timestamp 与 notary profile**

将签名身份和 profile 改为可覆盖但有安全默认值：

```bash
SIGNING_ID="${APPLE_SIGNING_IDENTITY:-Developer ID Application: Beijing VGO Co;Ltd (${TEAM_ID})}"
PROFILE_NAME="${MACSlim_NOTARY_PROFILE:-macslim-notary}"
```

在签名前执行：

```bash
if ! security find-identity -v -p codesigning | grep -Fq "$SIGNING_ID"; then
  echo "错误: Keychain 中未找到签名证书 $SIGNING_ID" >&2
  exit 1
fi

if ! xcrun notarytool history --keychain-profile "$PROFILE_NAME" >/dev/null 2>&1; then
  echo "错误: notarytool Keychain profile 不可用: $PROFILE_NAME" >&2
  exit 1
fi
```

删除 timestamp 失败后改用 `--timestamp=none` 的分支，所有签名保持：

```bash
TIMESTAMP_FLAG="--timestamp"
```

所有 `codesign` 失败必须直接终止，不使用 `|| true` 吞掉错误。notary profile 不可用时改为 `exit 1`。成功公证后加入：

```bash
codesign --verify --deep --strict --verbose=2 "$APP_PATH"
codesign -dvvv "$APP_PATH" 2>&1 | grep -q "Timestamp="
xcrun stapler validate "$APP_PATH"
xcrun stapler validate "$DMG_PATH"
spctl -a -t exec -vv "$APP_PATH"
spctl -a -t install -vv "$DMG_PATH"
```

- [ ] **Step 7: 更新发布说明和敏感文件忽略规则**

在 `scripts/README.md` 中只记录：

```bash
security find-identity -v -p codesigning
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD
xcrun notarytool store-credentials macslim-notary
```

同时注明当前证书 Common Name、Team ID、默认 profile 名，以及正式发布必须依次通过 `codesign --verify`、`xcrun stapler validate`、`spctl -a -t exec -vv` 和 `spctl -a -t install -vv`。不得放置真实密码、默认密码或可登录值。

在 `.gitignore` 末尾最小追加：

```gitignore
.superpowers/
*.p8
AuthKey_*.p8
private_keys/
```

- [ ] **Step 8: 运行发布脚本验证**

Run:

```bash
python3 -m unittest discover -s scripts/tests -p "test_*.py"
python3 scripts/verify-security-config.py
bash -n scripts/publish-update.sh scripts/release.sh scripts/sign.sh
```

Expected: 全部退出 0；缺少凭据的脚本路径不得返回成功。

---

### Task 4: 接入统一 verify 与 CI 门禁

**Files:**
- Modify: `package.json:6-35`
- Modify: `.github/workflows/ci.yml:1-90`
- Modify: `scripts/README.md:9-23`
- Modify: `src-tauri/src/scanner.rs:332-455,535-613`
- Modify mechanically: `src-tauri/src/bin/cli.rs`、`app_scanner.rs`、`applications.rs`、`cache_cleaner.rs`、`cache_scanner.rs`、`dev_tool_rules.rs`、`docker.rs`、`lib.rs`、`ports.rs`、`process_ops.rs`、`process_safety.rs`、`residue_scanner.rs`、`storage.rs`、`tray.rs`、`uninstaller.rs`、`whitelist.rs`

**Interfaces:**
- Consumes: Tasks 1–3 的全部验证入口。
- Produces: `bun run security:check`、`bun run test:python`、`bun run test:rust`、`bun run verify`，CI PR 门禁；`idle_memory_threshold(total_mem_mb: f64) -> f64`。

- [ ] **Step 0: 让 Rust clippy gate 验证动态闲置内存阈值**

在 `src-tauri/src/scanner.rs` 提取纯函数：

```rust
fn idle_memory_threshold(total_mem_mb: f64) -> f64 {
    (total_mem_mb * 0.01).max(80.0)
}
```

`classify_one` 使用该函数替代硬编码 100 MB，并增加测试：

```rust
#[test]
fn idle_memory_threshold_scales_with_total_memory() {
    assert_eq!(idle_memory_threshold(512.0), 80.0);
    assert_eq!(idle_memory_threshold(8_000.0), 80.0);
    assert_eq!(idle_memory_threshold(32_000.0), 320.0);
}
```

运行：

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib scanner::tests::idle_memory_threshold_scales_with_total_memory
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Expected: 测试与 clippy 退出 0，不改变默认选择策略。

- [ ] **Step 0b: 清理既有 Rust fmt/clippy 基线**

先运行 `cargo fmt --manifest-path src-tauri/Cargo.toml --all`，接受纯格式化 diff。然后只修复当前 `cargo clippy --all-targets -- -D warnings` 的 12 个错误：

- `manual_contains`：`SYSTEM_CORE_APPS` 与 `dangerous_literal` 改用 `contains`。
- `for_kv_map`：遍历 `processes().values()`。
- `unnecessary_sort_by`：三处倒序 size 排序改用 `sort_by_key(Reverse(...))`。
- `needless_borrow`：移除 `&parent_pids` 多余借用。
- `useless_format`：固定文案改 `.to_string()`。
- `too_many_arguments`：将递归 DFS 的只读上下文收敛为私有 `DfsContext`，不改变遍历顺序或输出。
- `if_same_then_else`：tray 当前三个分支输出同一 `●`，删除无效分支和对应未使用参数，保持现有显示不变。

不得用全局 `clippy::allow` 掩盖问题。运行：

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Expected: 三条命令退出 0，Rust 业务测试保持通过。

- [ ] **Step 1: 增加统一脚本**

在 `package.json` 的 scripts 中加入：

```json
"security:check": "python3 scripts/verify-security-config.py",
"test:python": "python3 -m unittest discover -s scripts/tests -p \"test_*.py\"",
"test:rust": "cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check && cargo test --manifest-path src-tauri/Cargo.toml --all-targets && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings",
"verify": "bun run lint && bun run typecheck && bun run test && bun run test:python && bun run security:check && bun run test:rust && bun run build"
```

- [ ] **Step 2: 固定 CI 权限和 Bun 版本**

在 `.github/workflows/ci.yml` 顶层增加：

```yaml
permissions:
  contents: read
```

将所有 `bun-version: latest` 或未指定版本的 setup 步骤统一为：

```yaml
- name: 安装 Bun
  uses: oven-sh/setup-bun@v2
  with:
    bun-version: 1.3.11
```

- [ ] **Step 3: 扩展 Rust CI**

将：

```yaml
- name: cargo test (lib)
  working-directory: src-tauri
  run: cargo test --lib --verbose
```

替换为：

```yaml
- name: cargo test
  working-directory: src-tauri
  run: cargo test --all-targets --verbose
```

- [ ] **Step 4: 扩展前端 CI**

将 `tsc --noEmit + vite build` 步骤替换为：

```yaml
- name: 前端与安全配置验证
  run: bun run verify
```

- [ ] **Step 5: 增加 shell 与 plist 语法验证**

在 Rust job 中加入：

```yaml
- name: 发布脚本语法
  run: bash -n scripts/publish-update.sh scripts/release.sh scripts/sign.sh

- name: Entitlements 语法
  run: plutil -lint src-tauri/entitlements.plist
```

- [ ] **Step 6: 更新脚本文档的验证入口**

在 `scripts/README.md` 的发版工作流中加入：

```bash
bun run verify
bun run tauri build --debug --no-bundle
./scripts/release.sh arm
```

- [ ] **Step 7: 运行完整本地验证**

Run:

```bash
bun run verify
git diff --check
bun run tauri build --debug --no-bundle
```

Expected:

- Vitest 全部通过。
- Python 校验器测试全部通过。
- Rust all-targets 测试全部通过。
- Oxlint、TypeScript、Vite build、capability、CSP、plist 与 shell 语法全部通过。
- `git diff --check` 无空白错误。

- [ ] **Step 8: 最终原生回归**

Run:

```bash
bun run tauri dev
```

Expected:

1. 拖动、导航、滚动、设置、通知、开机启动和更新入口正常。
2. Web Inspector 无 CSP violation 和 capability ACL 拒绝。
3. 控制台无未处理 Promise 或敏感凭据输出。
4. `git status --short` 仅包含本计划预期文件和用户原有改动。

---

## Plan Self-Review Checklist

- [x] 规范中的拖动 capability、SafeDragSurface 与内容区隔离已映射到 Task 1。
- [x] 严格 CSP、最小 capability 和最小 Developer ID entitlements 已映射到 Task 2。
- [x] 凭据 fallback、公证失败和 updater manifest 注入已映射到 Task 3。
- [x] lint、typecheck、前端测试、Rust 全 target 测试与 CI 已映射到 Task 4。
- [x] 计划没有 TBD、TODO、占位实现或未定义的接口。
- [x] `operation_id`、Liquid Glass、Space Rescue、Pro 和 MAS 被明确拆到后续计划，避免本阶段伪完成。
