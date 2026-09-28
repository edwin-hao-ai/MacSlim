# MacSlim 扫描过程可见化 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用户点击扫描后 ≤200ms 看到第一个阶段名，等待期间能看到当前阶段、已完成计数与已发现体积。

**Architecture:** 后端在既有 16 个并行扫描器内部通过回调上报阶段进度，`scan_cache` 命令把回调桥接成 Tauri 事件 `cache-scan-progress`；前端监听事件渲染紧凑三件套（当前阶段名 / 完成计数 / 累计体积）。快照注册、选择、执行链路一律不动。

**Tech Stack:** Rust + Tauri v2（`tauri::Emitter`）、SolidJS + TypeScript、`@tauri-apps/api/event`

**Spec:** `docs/superpowers/specs/2026-09-27-macslim-scan-progress-and-readable-names-design.md` §3.1–3.2、§4–6

## Global Constraints

- 阶段名是**编译期字面量**，不接受任何用户输入拼接；事件名 `cache-scan-progress` / `residue-scan-progress` 是硬编码常量
- 事件载荷只允许 `{ stage, state, item_count, found_bytes }` 四个字段；**禁止**携带文件路径、PID、bundle id
- `invoke_handler` 命令数必须保持 **16**；不新增任何 `invoke` 命令
- `src-tauri/capabilities/default.json` **不改动**（`core:event:allow-listen` 与 `core:event:allow-unlisten` 已存在）
- `scan(None)` 的行为与今天完全一致（CLI 与单元测试走这条路径）
- `cache_scanner::scan()` 的返回值 `CacheScanResult` 结构不变；快照注册逻辑一字不改
- 所有用户可见文案走 i18n（`src/i18n/zh-CN.ts` + `src/i18n/en.ts` 同步添加），禁止硬编码中文
- 错误提示必须是中文人话
- 新增函数 <50 行；`cache_scanner.rs` 已超 800 行，**本计划不再往它里面加新逻辑**，只做签名与任务表改造
- 验证命令：`bun run verify`（含 lint / typecheck / vitest / python / security / cargo fmt+test+clippy / build）

---

## File Structure

| 文件 | 职责 |
| :--- | :--- |
| `src-tauri/src/scan_progress.rs`（新建） | `StageUpdate` 类型 + `ProgressSink` 类型别名。独立文件，避免继续膨胀 `cache_scanner.rs` |
| `src-tauri/src/cache_scanner.rs` | `scan()` 改为接受 `Option<ProgressSink>`，16 个匿名闭包改为命名任务表 |
| `src-tauri/src/lib.rs` | `scan_cache` / `scan_app_residues_batch` 把 sink 桥接成事件 |
| `src/lib/tauri.ts` | `StageUpdate` 前端类型 + `onCacheScanProgress` / `onResidueScanProgress` 监听函数 |
| `src/components/ScanStageProgress.tsx`（新建） | 三件套进度 UI，无业务逻辑，纯展示 |
| `src/views/CacheView.tsx` | 用 `ScanStageProgress` 替换空白 spinner |
| `src/views/UninstallerView.tsx` | 残留扫描阶段接入 `ScanStageProgress` |
| `src/i18n/zh-CN.ts` / `en.ts` | 进度文案 |

---

### Task 1: StageUpdate 类型与缓存扫描改造

**Files:**
- Create: `src-tauri/src/scan_progress.rs`
- Modify: `src-tauri/src/cache_scanner.rs:152-217`（`scan()` 整体）、`src-tauri/src/cache_scanner.rs:1135`（内联 `mod tests`，追加测试）
- Test: `src-tauri/src/cache_scanner.rs` 内联 `mod tests`（该文件**没有**独立的 `cache_scanner_tests.rs`，不要新建）

**Interfaces:**
- Produces:
  ```rust
  // src-tauri/src/scan_progress.rs
  #[derive(Clone, Serialize)]
  pub struct StageUpdate {
      pub stage: String,
      pub state: &'static str,   // "running" | "done"
      pub item_count: usize,
      pub found_bytes: u64,
  }
  pub type ProgressSink = std::sync::Arc<dyn Fn(StageUpdate) + Send + Sync>;
  ```
  ```rust
  // src-tauri/src/cache_scanner.rs
  pub async fn scan(progress: Option<crate::scan_progress::ProgressSink>) -> CacheScanResult
  ```
- Consumes: 无（首个任务）

- [ ] **Step 1: 新建 `scan_progress.rs`**

```rust
use serde::Serialize;

/// 单个扫描阶段的进度更新。
/// 只允许携带阶段名、条目数与体积，不得加入路径、PID 等用户数据。
#[derive(Clone, Serialize)]
pub struct StageUpdate {
    pub stage: String,
    /// "running" | "done"
    pub state: &'static str,
    pub item_count: usize,
    pub found_bytes: u64,
}

impl StageUpdate {
    pub fn running(stage: &str) -> Self {
        Self {
            stage: stage.to_string(),
            state: "running",
            item_count: 0,
            found_bytes: 0,
        }
    }

    pub fn done(stage: &str, items: &[CacheItem]) -> Self {
        Self {
            stage: stage.to_string(),
            state: "done",
            item_count: items.len(),
            found_bytes: items.iter().map(|i| i.size_bytes).sum(),
        }
    }
}

pub type ProgressSink = std::sync::Arc<dyn Fn(StageUpdate) + Send + Sync>;
```

在 `src-tauri/src/lib.rs` 的模块声明区加入 `mod scan_progress;`（与 `mod cache_scanner;` 同级）。

- [ ] **Step 2: 写失败测试**

> ⚠️ **实现时更正（2026-09-27）**：本 Step 给出的 `stage_completion_is_reported_in_real_completion_order` 测试代码有真实缺陷，**照抄会死锁整个测试套件**：
> - `spawn_blocking` 闭包返回 `run_stages` 的 future 但从不 poll → `Barrier::new(2)` 只剩 1 个到达者 → 死锁
> - `unused_must_use` 触发，`clippy -D warnings` 必挂
> - 阶段体与 sink 写进同一个 vec，导致 `["快阶段","快阶段","慢阶段"]` 双重计数；`slow_seen` 声明后从未使用
>
> 正确写法：用 `tokio::spawn` 真正驱动 future，并把「执行顺序」与「上报顺序」拆成 `seen` / `executed` 双通道。实现者采用此写法，已先观察到 RED 后转绿。
> 另外 Task 2 Step 2 预测的 RED 不成立（其唯一测试只覆盖 Task 1 已完成的 `StageUpdate` serde 形状，对 Task 2 新增的两处 `emit` 零覆盖），需另补 2 个覆盖 Task 2 产出的测试。

追加到 `src-tauri/src/cache_scanner.rs:1135` 的内联 `mod tests` 内部（该模块已有 `use super::*;`，因此直接写 `scan(...)` 而非 `cache_scanner::scan(...)`）：

```rust
#[test]
fn cache_scan_reports_every_stage_when_a_sink_is_attached() {
    let seen = std::sync::Mutex::new(Vec::new());
    let sink: crate::scan_progress::ProgressSink = std::sync::Arc::new(move |u| {
        seen.lock().unwrap().push(u);
    });

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(scan(Some(sink)));

    let seen = seen.into_inner().unwrap();
    let running: Vec<&str> = seen
        .iter()
        .filter(|u| u.state == "running")
        .map(|u| u.stage.as_str())
        .collect();
    let done: Vec<&str> = seen
        .iter()
        .filter(|u| u.state == "done")
        .map(|u| u.stage.as_str())
        .collect();

    assert_eq!(running.len(), 16, "16 个扫描器各上报一次开始");
    assert_eq!(done.len(), 16, "16 个扫描器各上报一次完成");
    assert!(running.contains(&"废纸篓"));
    assert!(running.contains(&"Cargo 缓存"));
    assert!(running.contains(&"应用缓存"));
    assert_eq!(
        seen.iter()
            .filter(|u| u.state == "done")
            .map(|u| u.found_bytes)
            .sum::<u64>(),
        result.total_bytes,
        "各阶段体积之和必须等于最终总量",
    );
}

#[test]
fn stage_completion_is_reported_in_real_completion_order() {
    use std::sync::{Arc, Barrier};
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let gate = Arc::new(Barrier::new(2));

    let slow_gate = gate.clone();
    let slow_seen = seen.clone();
    let fast_seen = seen.clone();
    let stages: Vec<(&'static str, CacheScanTestFn)> = vec![
        ("慢阶段", Box::new(move |_h| {
            slow_gate.wait();
            vec![]
        })),
        ("快阶段", Box::new(move |_h| {
            fast_seen.lock().unwrap().push("快阶段".into());
            vec![]
        })),
    ];

    let sink: crate::scan_progress::ProgressSink = Arc::new(move |u| {
        if u.state == "done" {
            seen.lock().unwrap().push(u.stage);
        }
    });
    let sink_for_task = sink.clone();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let task = tokio::task::spawn_blocking(move || {
            run_stages(stages, Some(sink_for_task));
        });
        // 快阶段必须在慢阶段放行之前就完成并上报
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        gate.wait();
        task.await.unwrap();
    });

    let order = seen.lock().unwrap().clone();
    assert_eq!(
        order,
        vec!["快阶段".to_string(), "慢阶段".to_string()],
        "完成顺序必须是真实完成顺序，而不是注册顺序",
    );
}

#[test]
fn cache_scan_without_a_sink_behaves_identically() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let with = rt.block_on(scan(None));
    assert!(!with.items.is_empty() || with.total_bytes == 0);
}
```

同时在 `mod tests` 顶部加类型别名，供上面使用：

```rust
type CacheScanTestFn = Box<dyn FnOnce(Arc<PathBuf>) -> Vec<CacheItem> + Send>;
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib cache_scan`
Expected: FAIL —— `scan` 接受 0 个参数；`run_stages` / `crate::scan_progress` 未定义

- [ ] **Step 4: 抽出 `run_stages` 并改造 `scan()`**

先把任务执行循环抽成可测试的 `run_stages`，再让 `scan()` 只负责组装阶段表。这样 `scan()` 保持在 50 行以内，也让完成顺序可被确定性地断言。

```rust
type ScanFn = Box<dyn FnOnce(Arc<PathBuf>) -> Vec<CacheItem> + Send>;

/// 运行所有阶段并汇总结果，同时按真实完成顺序上报进度。
async fn run_stages(
    stages: Vec<(&'static str, ScanFn)>,
    progress: Option<crate::scan_progress::ProgressSink>,
) -> Vec<CacheItem> {
    let mut tasks = Vec::new();
    for (stage, f) in stages {
        let home = Arc::new(dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        let sink = progress.clone();
        tasks.push(tokio::task::spawn_blocking(move || {
            if let Some(sink) = &sink {
                sink(crate::scan_progress::StageUpdate::running(stage));
            }
            let batch = f(home);
            if let Some(sink) = &sink {
                sink(crate::scan_progress::StageUpdate::done(stage, &batch));
            }
            batch
        }));
    }
    let mut items: Vec<CacheItem> = Vec::new();
    for t in tasks {
        if let Ok(batch) = t.await {
            items.extend(batch);
        }
    }
    items
}

pub async fn scan(progress: Option<crate::scan_progress::ProgressSink>) -> CacheScanResult {
    let h = Arc::new(dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
    let stages: Vec<(&'static str, ScanFn)> = vec![
        ("npm 缓存", Box::new(move |home| scan_npm(&home))),
        ("pnpm 缓存", Box::new(move |home| scan_pnpm(&home))),
        ("Yarn 缓存", Box::new(move |home| scan_yarn(&home))),
        ("Docker 镜像与容器", Box::new(|_home| scan_docker())),
        ("Docker 无用镜像", Box::new(|_home| scan_docker_stale_images())),
        ("闲置 node_modules", Box::new(move |home| scan_stale_node_modules(&home))),
        ("Homebrew 缓存", Box::new(|_home| scan_homebrew())),
        ("Xcode 缓存", Box::new(move |home| scan_xcode(&home))),
        ("CocoaPods 缓存", Box::new(move |home| scan_cocoapods(&home))),
        ("Cargo 缓存", Box::new(move |home| scan_cargo(&home))),
        ("pip 缓存", Box::new(move |home| scan_pip(&home))),
        ("Go 模块缓存", Box::new(move |home| scan_go(&home))),
        ("应用缓存", Box::new(move |home| scan_app_caches(&home))),
        ("应用日志", Box::new(move |home| scan_app_logs(&home))),
        ("崩溃报告", Box::new(move |home| scan_crash_reports(&home))),
        ("废纸篓", Box::new(move |home| scan_trash(&home))),
    ];
    let _ = h;
    let mut items = run_stages(stages, progress).await;
    items.retain(|i| i.size_bytes > 0);
    items.sort_by_key(|item| std::cmp::Reverse(item.size_bytes));
    let total_bytes = items.iter().map(|i| i.size_bytes).sum();
    let scanned_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    CacheScanResult { items, total_bytes, scanned_at_ms }
}
```

把 `scan()` 里那句多余的 `let _ = h;` 一并删掉，`h` 只在 `run_stages` 内部按阶段构造即可（`Arc<PathBuf>` 每次构造开销可忽略，且避免 `scan()` 持有一个用不到的变量）。

- [ ] **Step 5: 修所有调用方**

`src-tauri/src/lib.rs:118` 改为 `cache_scanner::scan(None).await`（Task 2 才换成带 sink 的版本）。

`rg -n 'cache_scanner::scan\(' src-tauri/src` 确认没有其他调用点；若有测试直接调用，一并补 `None`。

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib cache_scan`
Expected: PASS（2 个新测试 + 既有 cache_scanner 测试全绿）

- [ ] **Step 7: 提交**

```bash
git add src-tauri/src/scan_progress.rs src-tauri/src/cache_scanner.rs src-tauri/src/lib.rs
git commit -m "feat(scan): 缓存扫描按阶段上报进度"
```

---

### Task 2: 命令侧桥接成 Tauri 事件

**Files:**
- Modify: `src-tauri/src/lib.rs:114-121`（`scan_cache`）、`src-tauri/src/lib.rs:136-160`（`scan_app_residues_batch`）
- Test: `src-tauri/src/operation_commands_scan_tests.rs` 或新建 `src-tauri/src/lib_progress_tests.rs`

**Interfaces:**
- Consumes: `ProgressSink`、`StageUpdate`（Task 1）
- Produces: 事件名 `cache-scan-progress`、`residue-scan-progress`，载荷 `StageUpdate`

- [ ] **Step 1: 写失败测试**

新建 `src-tauri/src/lib_progress_tests.rs`：

```rust
#[test]
fn stage_update_serialises_only_the_four_permitted_fields() {
    let json = serde_json::to_value(scan_progress::StageUpdate::done(
        "废纸篓",
        &[],
    ))
    .unwrap();
    let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
    assert_eq!(keys, vec!["found_bytes", "item_count", "stage", "state"]);
    assert!(!json.to_string().contains("path"));
}
```

在 `lib.rs` 的 `#[cfg(test)]` 模块区加入：
```rust
#[cfg(test)]
#[path = "lib_progress_tests.rs"]
mod lib_progress_tests;
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib stage_update_serialises`
Expected: FAIL —— `scan_progress` 未在 `lib.rs` 中以该路径可见

- [ ] **Step 3: 改造 `scan_cache`**

`src-tauri/src/lib.rs:114-121` 替换为：

```rust
#[tauri::command]
async fn scan_cache(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<SnapshotResult<CacheSnapshotView>, String> {
    use tauri::Emitter;
    let sink: crate::scan_progress::ProgressSink = std::sync::Arc::new(move |update| {
        let _ = app.emit("cache-scan-progress", &update);
    });
    let result = cache_scanner::scan(Some(sink)).await;
    let mut operations = state.operations.lock().map_err(|error| error.to_string())?;
    operation_commands::snapshot_cache(&mut operations, result)
}
```

`ProgressSink` 一律用全路径 `crate::scan_progress::ProgressSink` 引用，不做 `pub use` 重导出，全仓库保持一致。

- [ ] **Step 4: 改造 `scan_app_residues_batch`**

`src-tauri/src/lib.rs:146-157` 的 `spawn_blocking` 块替换为：

```rust
    let groups = tauri::async_runtime::spawn_blocking(move || {
        use rayon::prelude::*;
        use tauri::Emitter;
        let index = residue_scanner::LibraryIndex::build();
        selections
            .par_iter()
            .map(|(app_key, identity)| {
                let residue = residue_scanner::scan_residues_with_index(
                    index.as_ref(),
                    &identity.bundle_id,
                    &identity.app_name,
                );
                let _ = app.emit(
                    "residue-scan-progress",
                    scan_progress::StageUpdate {
                        stage: residue.app_name.clone(),
                        state: "done",
                        item_count: residue.items.len(),
                        found_bytes: residue.total_bytes,
                    },
                );
                (app_key.clone(), residue)
            })
            .collect::<Vec<_>>()
    })
```

需要把 `app: tauri::AppHandle` 加进 `scan_app_residues_batch` 的参数列表（`lib.rs:137-141`）。

- [ ] **Step 5: 确认命令数没变**

Run: `python3 scripts/verify-security-config.py && python3 scripts/operation_surface_checks.py`
Expected: 两个脚本都通过；`operation_surface_checks.py` 报告的命令数仍是 16

- [ ] **Step 6: 运行 Rust 测试**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --all-targets`
Expected: 全绿

- [ ] **Step 7: 提交**

```bash
git add src-tauri/src/lib.rs src-tauri/src/lib_progress_tests.rs src-tauri/src/residue_scanner.rs
git commit -m "feat(scan): 扫描进度桥接为 Tauri 事件"
```

---

### Task 3: 前端监听 API 与三件套进度组件

**Files:**
- Modify: `src/lib/tauri.ts`
- Create: `src/components/ScanStageProgress.tsx`
- Test: `src/components/ScanStageProgress.test.tsx`（新建）

**Interfaces:**
- Consumes: 事件名 `cache-scan-progress` / `residue-scan-progress`，载荷 `StageUpdate`
- Produces:
  ```ts
  export type StageUpdate = {
    stage: string;
    state: "running" | "done";
    item_count: number;
    found_bytes: number;
  };
  export function onCacheScanProgress(cb: (u: StageUpdate) => void): Promise<() => void>;
  export function onResidueScanProgress(cb: (u: StageUpdate) => void): Promise<() => void>;
  // ScanStageProgress props:
  // { current: string | null; doneCount: number; total: number | null; foundBytes: number }
  ```

- [ ] **Step 1: 写失败测试**

新建 `src/components/ScanStageProgress.test.tsx`：

```tsx
import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/i18n", () => ({
  useI18n: () => ({ t: (k: string, p?: Record<string, unknown>) =>
    p ? `${k}:${JSON.stringify(p)}` : k }),
}));

import ScanStageProgress from "@/components/ScanStageProgress";

describe("ScanStageProgress", () => {
  afterEach(cleanup);

  it("第一个 running 事件到达后立刻显示阶段名，不等待任何 done", () => {
    render(() => (
      <ScanStageProgress current="Xcode 缓存" doneCount={0} total={16} foundBytes={0} />
    ));
    expect(screen.getByTestId("scan-stage-name").textContent).toBe("Xcode 缓存");
    expect(screen.getByTestId("scan-stage-count").textContent).toContain("0");
    expect(screen.getByTestId("scan-stage-count").textContent).toContain("16");
  });

  it("已完成计数与累计体积都可见", () => {
    render(() => (
      <ScanStageProgress current={null} doneCount={7} total={16} foundBytes={27_400_000_000} />
    ));
    expect(screen.getByTestId("scan-stage-count").textContent).toContain("7");
    expect(screen.getByTestId("scan-stage-bytes").textContent).toBeTruthy();
  });

  it("总阶段数未知时不渲染进度条", () => {
    const { container } = render(() => (
      <ScanStageProgress current="Chrome" doneCount={3} total={null} foundBytes={0} />
    ));
    expect(container.querySelector('[data-testid="scan-stage-bar"]')).toBeNull();
  });
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `bunx vitest run src/components/ScanStageProgress.test.tsx`
Expected: FAIL —— 模块不存在

- [ ] **Step 3: 在 `src/lib/tauri.ts` 加类型与监听函数**

在文件顶部 import 区加入 `import { listen, type UnlistenFn } from "@tauri-apps/api/event";`，并在 `ProcessRow` 类型附近加入：

```ts
export type StageUpdate = {
  stage: string;
  state: "running" | "done";
  item_count: number;
  found_bytes: number;
};

export async function onCacheScanProgress(
  cb: (u: StageUpdate) => void,
): Promise<UnlistenFn> {
  return listen<StageUpdate>("cache-scan-progress", (e) => cb(e.payload));
}

export async function onResidueScanProgress(
  cb: (u: StageUpdate) => void,
): Promise<UnlistenFn> {
  return listen<StageUpdate>("residue-scan-progress", (e) => cb(e.payload));
}
```

- [ ] **Step 4: 新建 `ScanStageProgress.tsx`**

```tsx
import { type Component, Show } from "solid-js";
import { useI18n } from "@/i18n";

type Props = {
  current: string | null;
  doneCount: number;
  total: number | null;
  foundBytes: number;
};

const formatBytes = (bytes: number): string => {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(0)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${bytes} B`;
};

const ScanStageProgress: Component<Props> = (props) => {
  const { t } = useI18n();
  const percent = () =>
    props.total && props.total > 0
      ? Math.min(100, Math.round((props.doneCount / props.total) * 100))
      : 0;

  return (
    <div class="space-y-2" data-testid="scan-stage-progress">
      <div class="flex items-center gap-2 text-xs text-zinc-500">
        <span data-testid="scan-stage-name">
          <Show when={props.current} fallback={t("scanProgress.working")}>
            {t("scanProgress.current", { stage: props.current ?? "" })}
          </Show>
        </span>
        <span class="ml-auto" data-testid="scan-stage-count">
          <Show when={props.total !== null} fallback={props.doneCount}>
            {t("scanProgress.count", {
              done: props.doneCount,
              total: props.total ?? 0,
            })}
          </Show>
        </span>
      </div>
      <Show when={props.total !== null}>
        <div
          data-testid="scan-stage-bar"
          class="h-1 rounded-full bg-black/5 dark:bg-white/5 overflow-hidden"
        >
          <div
            class="h-full bg-brand-500 transition-[width] duration-300"
            style={{ width: `${percent()}%` }}
          />
        </div>
      </Show>
      <div class="text-xs text-zinc-400 tabular-nums" data-testid="scan-stage-bytes">
        {t("scanProgress.found", { size: formatBytes(props.foundBytes) })}
      </div>
    </div>
  );
};

export default ScanStageProgress;
```

- [ ] **Step 5: 加 i18n 文案**

`src/i18n/zh-CN.ts` 顶层加：

```ts
  scanProgress: {
    current: "正在扫描：{stage}",
    working: "正在扫描…",
    count: "已完成 {done}/{total}",
    found: "已发现 {size}",
  },
```

`src/i18n/en.ts` 同步加：

```ts
  scanProgress: {
    current: "Scanning: {stage}",
    working: "Scanning…",
    count: "{done}/{total} done",
    found: "{size} found",
  },
```

- [ ] **Step 6: 运行测试确认通过**

Run: `bunx vitest run src/components/ScanStageProgress.test.tsx`
Expected: PASS（3 个测试）

- [ ] **Step 7: 提交**

```bash
git add src/lib/tauri.ts src/components/ScanStageProgress.tsx src/components/ScanStageProgress.test.tsx src/i18n/zh-CN.ts src/i18n/en.ts
git commit -m "feat(scan): 前端进度监听 API 与三件套进度组件"
```

---

### Task 4: CacheView 接入真实进度

**Files:**
- Modify: `src/views/CacheView.tsx:92-112`（`runScan`）、`src/views/CacheView.tsx:298-305`（spinner 处）
- Test: `src/views/CacheView.test.tsx`（追加）

**Interfaces:**
- Consumes: `onCacheScanProgress`、`ScanStageProgress`
- Produces: 无

- [ ] **Step 1: 写失败测试**

追加到 `src/views/CacheView.test.tsx`。在该文件已有的 `vi.mock("@/lib/tauri", ...)` 里补上 `onCacheScanProgress`，让它返回一个**可手动触发**的注册函数：

```ts
const stageHandlers: Array<(u: { stage: string; state: string; item_count: number; found_bytes: number }) => void> = [];
const unlistenMock = vi.fn();

vi.mock("@/lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/tauri")>();
  return {
    ...actual,
    // ...既有 mock 保持不变
    onCacheScanProgress: vi.fn(async (cb: (u: never) => void) => {
      stageHandlers.push(cb as never);
      return unlistenMock;
    }),
  };
});
```

测试体：

```tsx
it("扫描期间显示真实阶段名与完成计数", async () => {
  render(() => <CacheView />);
  await waitFor(() => expect(stageHandlers.length).toBeGreaterThan(0));

  stageHandlers[0]({
    stage: "废纸篓",
    state: "running",
    item_count: 0,
    found_bytes: 0,
  });

  expect(screen.getByTestId("scan-stage-name").textContent).toContain("废纸篓");
  expect(screen.queryByText("cache.scanning")).toBeNull();
});

it("扫描完成后卸载进度并渲染列表", async () => {
  render(() => <CacheView />);
  await waitFor(() => expect(stageHandlers.length).toBeGreaterThan(0));
  stageHandlers[0]({ stage: "废纸篓", state: "done", item_count: 1, found_bytes: 100 });
  await waitFor(() => expect(screen.queryByTestId("scan-stage-progress")).toBeNull());
});

it("组件卸载时调用 unlisten", async () => {
  const { unmount } = render(() => <CacheView />);
  await waitFor(() => expect(stageHandlers.length).toBeGreaterThan(0));
  unmount();
  expect(unlistenMock).toHaveBeenCalled();
});
```

- [ ] **Step 2: 运行测试确认失败**

Run: `bunx vitest run src/views/CacheView.test.tsx`
Expected: FAIL —— 找不到 `scan-stage-name`

- [ ] **Step 3: 改造 `runScan`**

`src/views/CacheView.tsx` 的 import 区加入 `onCacheScanProgress` 与 `ScanStageProgress`，新增三个 signal：

```ts
  const [stageCurrent, setStageCurrent] = createSignal<string | null>(null);
  const [stageDone, setStageDone] = createSignal(0);
  const [stageFound, setStageFound] = createSignal(0);
  const CACHE_STAGE_TOTAL = 16;
```

`runScan` 改为：

```ts
  const runScan = async () => {
    setScanning(true);
    setSummary(null);
    setStageCurrent(null);
    setStageDone(0);
    setStageFound(0);
    const unlisten = await onCacheScanProgress((u) => {
      if (u.state === "running") setStageCurrent(u.stage);
      else {
        setStageDone((n) => n + 1);
        setStageFound((n) => n + u.found_bytes);
      }
    });
    try {
      const snapshotResult = await scanCache();
      setSnapshot(snapshotResult);
      setSelected(
        new Set(
          snapshotResult.value.items
            .filter((item) => item.default_select)
            .map((item) => item.selection_key),
        ),
      );
    } catch (e) {
      console.error(e);
    } finally {
      unlisten();
      setScanning(false);
    }
  };
```

- [ ] **Step 4: 替换 spinner**

`src/views/CacheView.tsx:298-305` 的 `fallback` 内容替换为：

```tsx
              <ScanStageProgress
                current={stageCurrent()}
                doneCount={stageDone()}
                total={CACHE_STAGE_TOTAL}
                foundBytes={stageFound()}
              />
```

- [ ] **Step 5: 运行测试确认通过**

Run: `bunx vitest run src/views/CacheView.test.tsx`
Expected: PASS（新增 3 个 + 既有全部）

- [ ] **Step 6: 提交**

```bash
git add src/views/CacheView.tsx src/views/CacheView.test.tsx
git commit -m "feat(cache): 扫描期展示真实阶段进度"
```

---

### Task 5: UninstallerView 残留扫描进度

**Files:**
- Modify: `src/views/UninstallerView.tsx:121-145`（`enterResiduePhase`）
- Test: `src/views/UninstallerView.test.tsx`（追加）

**Interfaces:**
- Consumes: `onResidueScanProgress`、`ScanStageProgress`
- Produces: 无

- [ ] **Step 1: 写失败测试**

在该测试文件已有的 `@/lib/tauri` mock 中补 `onResidueScanProgress`（结构同 Task 4 的 `onCacheScanProgress`，用独立 handler 数组 `residueHandlers`），并追加：

```tsx
it("残留扫描期间显示已完成计数", async () => {
  render(() => <UninstallerView />);
  await enterResiduePhase();
  await waitFor(() => expect(residueHandlers.length).toBeGreaterThan(0));

  residueHandlers[0]({
    stage: "Google Chrome",
    state: "done",
    item_count: 3,
    found_bytes: 1024,
  });

  expect(screen.getByTestId("scan-stage-count").textContent).toContain("1");
});
```

复用该文件里**已经存在**的 `enterResiduePhase()` helper（`src/views/UninstallerView.test.tsx:156`，它会勾选第一个 app、点击 `uninstaller.uninstallSelected` 并等待 `Notes` 出现），不要另写进入残留阶段的辅助函数。

- [ ] **Step 2: 运行测试确认失败**

Run: `bunx vitest run src/views/UninstallerView.test.tsx`
Expected: FAIL —— 找不到 `scan-stage-count`

- [ ] **Step 3: 改造 `enterResiduePhase`**

```ts
  const enterResiduePhase = async () => {
    ...
    setResidueLoading(true);
    setResidueGroups([]);
    setStageCurrent(null);
    setResidueDone(0);
    setResidueFound(0);
    const unlisten = await onResidueScanProgress((u) => {
      setStageCurrent(u.stage);
      setResidueDone((n) => n + 1);
      setResidueFound((n) => n + u.found_bytes);
    });
    try {
      const snapshot = await scanAppResiduesBatch(currentAppSnapshot, keys);
      ...
    } finally {
      unlisten();
      setResidueLoading(false);
    }
  };
```

新增 signal：`stageCurrent: string | null`、`residueDone: number`、`residueFound: number`。

`total` 传 `selectedAppList().length`（选中 app 数就是总阶段数）。

- [ ] **Step 4: 在残留 loading 处渲染 `ScanStageProgress`**

`residueLoading` 为真的分支里，把纯 spinner 换成：

```tsx
              <ScanStageProgress
                current={stageCurrent()}
                doneCount={residueDone()}
                total={selectedAppList().length}
                foundBytes={residueFound()}
              />
```

- [ ] **Step 5: 运行测试确认通过**

Run: `bunx vitest run src/views/UninstallerView.test.tsx`
Expected: PASS

- [ ] **Step 6: 全量验证**

Run: `bun run verify`
Expected: 退出码 0；Rust 测试数 = 370 + 本计划新增；Vitest = 124 + 本计划新增

- [ ] **Step 7: 提交**

```bash
git add src/views/UninstallerView.tsx src/views/UninstallerView.test.tsx
git commit -m "feat(uninstall): 残留扫描展示逐应用进度"
```

---

## Task 6: 真实 GUI 验收

**Files:** 无代码改动；产出验收记录

**Interfaces:**
- Consumes: Task 1–5 的全部产出
- Produces: 验收结论

- [ ] **Step 1: 构建并启动真实 bundle**

```bash
bun run tauri build --debug --bundles app
open /Users/edwinhao/.cargo/shared-target/debug/bundle/macos/MacSlim.app
```

Expected: 窗口出现（900x600）。若报错缺 `TAURI_SIGNING_PRIVATE_KEY`，那是 updater 产物的预期拒绝，`.app` 本身可用。

- [ ] **Step 2: 测量首字节延迟**

进入「缓存清理」，点击扫描，用 `screencapture` 抓连续帧，测量**第一个阶段名出现**的时刻。

判定：≤200ms 出现阶段名，且不再是空白 spinner。

- [ ] **Step 3: 观察完整扫描过程**

观察三件套是否全程更新：当前阶段名变化、已完成计数递增到 16、累计体积增长。

- [ ] **Step 4: 验证安全面未扩张**

```bash
python3 scripts/verify-security-config.py
python3 scripts/operation_surface_checks.py
git diff --stat src-tauri/capabilities/default.json
```

Expected: 两个脚本通过；capabilities 无 diff；命令数仍 16。

- [ ] **Step 5: 记录验收结论**

把「首字节延迟」「总扫描耗时」「是否出现旧 spinner」写进本计划末尾的验收记录小节。
