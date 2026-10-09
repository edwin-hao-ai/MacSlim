# 诚实清理报告 + 安全筛选 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把清理结果从「声称释放 = 扫描体积」改成「只有实测卷容量增量才叫释放」，并给缓存页加安全筛选 chips。

**Architecture:** 新增 `volume.rs` 用 `statfs` 测家目录卷可用空间；缓存/Docker 在执行业务前后各测一次，卸载在移动前后测一次，差值过噪声下限才作为 `reclaimed_bytes`。历史库**只加列**（旧 `freed_bytes` 保留兜底）。前端按结构化字段本地化渲染，并对缓存项加 `All / Safe to Clean / Check First` 三档筛选。

**Tech Stack:** Rust（Tauri v2、`libc`、`rusqlite`）、SolidJS + TypeScript、Tailwind。

**Spec:** `docs/superpowers/specs/2026-10-08-macslim-honest-cleanup-reporting-and-safety-filter-design.md`

## Global Constraints

- 只做缓存 + 卸载 + Docker 三类（进程终止不涉及空间）。
- 删除去向**不变**：缓存/Docker 永久删除；卸载本就移入废纸篓。
- **绝不**拿 `deleted_bytes` / `trashed_bytes` 冒充 `reclaimed_bytes`。
- `reclaimed_bytes` 是可空值：读不到就是 `null`，禁止伪装成 0。
- 历史库**只加列**，不改旧列语义；旧行 `reclaimed_bytes` 为 NULL。
- 文案一律走前端 i18n 词典，后端不拼死中文（AGENTS §8.6）。
- 路径涉及处一律用 `folder_access::scanner_home()`（AGENTS §7.5）。
- 每个 Task 结束前跑该 Task 的验证命令；Rust 改动跑 `cargo test --manifest-path src-tauri/Cargo.toml`，前端改动跑 `bun run test` 与类型检查。提交格式 `<type>: <中文描述>`。

---

## Task 1: 新增 `volume.rs` 卷容量测量

**Files:**
- Create: `src-tauri/src/volume.rs`
- Create: `src-tauri/src/volume_tests.rs`
- Modify: `src-tauri/src/lib.rs`（模块声明）

**Interfaces:**
- Produces:
  - `pub struct VolumeCapacity { pub total_bytes: u64, pub available_bytes: u64 }`
  - `pub const NOISE_FLOOR_BYTES: u64`
  - `VolumeCapacity::read() -> Option<VolumeCapacity>`
  - `VolumeCapacity::read_for(path: &Path) -> Option<VolumeCapacity>`
  - `pub fn reclaimed(before: Option<VolumeCapacity>, after: Option<VolumeCapacity>) -> Option<u64>`

- [ ] **Step 1: 写失败测试** `src-tauri/src/volume_tests.rs`

```rust
use super::{reclaimed, VolumeCapacity, NOISE_FLOOR_BYTES};
use std::path::Path;

fn cap(available: u64) -> VolumeCapacity {
    VolumeCapacity {
        total_bytes: 0,
        available_bytes: available,
    }
}

#[test]
fn none_when_either_reading_is_missing() {
    assert_eq!(reclaimed(None, Some(cap(0))), None);
    assert_eq!(reclaimed(Some(cap(0)), None), None);
    assert_eq!(reclaimed(None, None), None);
}

#[test]
fn none_below_noise_floor() {
    let before = cap(1_000);
    let after = cap(1_000 + NOISE_FLOOR_BYTES - 1);
    assert_eq!(reclaimed(Some(before), Some(after)), None);
}

#[test]
fn some_at_the_noise_floor() {
    let before = cap(1_000);
    let after = cap(1_000 + NOISE_FLOOR_BYTES);
    assert_eq!(
        reclaimed(Some(before), Some(after)),
        Some(NOISE_FLOOR_BYTES)
    );
}

#[test]
fn none_when_available_space_shrinks() {
    assert_eq!(reclaimed(Some(cap(10_000)), Some(cap(9_000))), None);
}

#[test]
fn read_returns_the_home_volume_on_this_machine() {
    let capacity = VolumeCapacity::read().expect("家目录卷应可读");
    assert!(capacity.total_bytes > 0);
    assert!(capacity.available_bytes <= capacity.total_bytes);
}

#[test]
fn read_for_a_missing_path_is_none() {
    assert_eq!(
        VolumeCapacity::read_for(Path::new("/no/such/path/xyzzy")),
        None
    );
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml volume 2>&1 | tail -20`
Expected: 编译失败（`volume` 模块不存在）。

- [ ] **Step 3: 实现** `src-tauri/src/volume.rs`

```rust
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// 卷容量快照。`available_bytes` 是可用字节（对应 `statfs` 的 `f_bavail`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeCapacity {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

/// 低于此增量视为测量噪声，不作为「释放」上报。
///
/// 清理运行期间其它进程仍在写盘，容量差有抖动；太小会把噪声当成果报给用户。
pub const NOISE_FLOOR_BYTES: u64 = 4 * 1024 * 1024;

impl VolumeCapacity {
    /// 读取 `path` 所在卷的容量。失败返回 `None`（**不**伪装成 0）。
    pub fn read_for(path: &Path) -> Option<VolumeCapacity> {
        let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut buf = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: `c_path` 是有效的 NUL 结尾 C 字符串；`buf` 是合法可写指针。
        let rc = unsafe { libc::statfs(c_path.as_ptr(), buf.as_mut_ptr()) };
        if rc != 0 {
            return None;
        }
        let stat = unsafe { buf.assume_init() };
        let block = stat.f_bsize as u64;
        Some(VolumeCapacity {
            total_bytes: stat.f_blocks.saturating_mul(block),
            available_bytes: stat.f_bavail.saturating_mul(block),
        })
    }

    /// 家目录所在卷。与清理路径同源，保证测的是同一个卷。
    pub fn read() -> Option<VolumeCapacity> {
        Self::read_for(&crate::folder_access::scanner_home())
    }
}

/// 两次读数的可用空间增量。任一次缺失、空间反而变小、或增量低于噪声下限，
/// 都返回 `None` —— 表示「测不出可报告的释放量」，与「释放 0」不同。
pub fn reclaimed(before: Option<VolumeCapacity>, after: Option<VolumeCapacity>) -> Option<u64> {
    let before = before?;
    let after = after?;
    let delta = after.available_bytes.checked_sub(before.available_bytes)?;
    (delta >= NOISE_FLOOR_BYTES).then_some(delta)
}

#[cfg(test)]
#[path = "volume_tests.rs"]
mod tests;
```

- [ ] **Step 4: 注册模块** `src-tauri/src/lib.rs`

在 `pub mod user_error;` 之后加一行：

```rust
pub mod volume;
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml volume 2>&1 | tail -20`
Expected: 6 个测试全 PASS。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/volume.rs src-tauri/src/volume_tests.rs src-tauri/src/lib.rs
git commit -m "feat: 新增卷容量实测模块 volume（statfs + 噪声下限）"
```

---

## Task 2: 缓存清理口径改造（`deleted_bytes` + `reclaimed_bytes`）

**Files:**
- Modify: `src-tauri/src/cache_cleaner.rs`（`CleanReport` / `CleanSummary` / `clean_with_runtime`）
- Modify: `src-tauri/src/operation_commands.rs`（`cache_entry`、`cache_history_detail`）
- Test: `src-tauri/src/cache_cleaner_tests.rs`（现有）

**Interfaces:**
- Consumes: `crate::volume::{VolumeCapacity, reclaimed}`
- Produces:
  - `CleanReport { ..., deleted_bytes: u64, ... }`（原 `freed_bytes` 改名）
  - `CleanSummary { reports, deleted_bytes: u64, reclaimed_bytes: Option<u64>, success_count, fail_count }`

- [ ] **Step 1: 改结构体** `src-tauri/src/cache_cleaner.rs`

`CleanReport` 中 `pub freed_bytes: u64,` 改为：

```rust
    pub deleted_bytes: u64,
```

`CleanSummary` 定义改为：

```rust
#[derive(Serialize, Clone, Debug)]
pub struct CleanSummary {
    pub reports: Vec<CleanReport>,
    /// 成功永久删除项的扫描体积之和。**不是**已释放空间。
    pub deleted_bytes: u64,
    /// 卷可用空间的实测增量；读不到或低于噪声下限为 `None`。
    pub reclaimed_bytes: Option<u64>,
    pub success_count: usize,
    pub fail_count: usize,
}
```

- [ ] **Step 2: 改 `clean_with_runtime`**（同文件）

在 `let mut reports = ...` 之前加 `let before = crate::volume::VolumeCapacity::read();`，循环后、构造 `CleanSummary` 前加 `let after = crate::volume::VolumeCapacity::read();`。把 `freed_bytes` 赋值与汇总改成 `deleted_bytes`：

```rust
pub(crate) async fn clean_with_runtime<R: CacheRuntime + ?Sized>(
    items: Vec<CacheItem>,
    runtime: &R,
) -> CleanSummary {
    let before = crate::volume::VolumeCapacity::read();
    let mut reports = Vec::with_capacity(items.len());
    for item in items {
        let start = Instant::now();
        let result = clean_item(&item, runtime).await;
        let (success, error) = match result {
            Ok(()) => (true, None),
            Err(error) => (false, Some(error)),
        };
        reports.push(CleanReport {
            id: item.id.clone(),
            label_key: item.label_key.clone(),
            label_params: item.label_params.clone(),
            success,
            deleted_bytes: if success { item.size_bytes } else { 0 },
            duration_ms: start.elapsed().as_millis() as u64,
            error,
        });
    }
    let after = crate::volume::VolumeCapacity::read();
    let deleted_bytes = reports.iter().map(|report| report.deleted_bytes).sum();
    let success_count = reports.iter().filter(|report| report.success).count();
    CleanSummary {
        fail_count: reports.len() - success_count,
        reports,
        deleted_bytes,
        reclaimed_bytes: crate::volume::reclaimed(before, after),
        success_count,
    }
}
```

- [ ] **Step 3: 改 `operation_commands.rs`**

`cache_history_detail`（约 483-497 行）里 `summary.total_freed_bytes` 改为 `summary.deleted_bytes`，并把量词写清：

```rust
fn cache_history_detail(summary: &CleanSummary) -> String {
    let base = format!(
        "成功 {} 项，失败 {} 项，已删除 {}",
        summary.success_count, summary.fail_count, summary.deleted_bytes
    );
    let Some(reason) = summary
        .reports
        .iter()
        .find(|report| !report.success)
        .and_then(|report| report.error.as_ref())
    else {
        return base;
    };
    format!("{base}；原因：{}", reason.message)
}
```

`cache_entry`（约 511-529 行）中 `freed_bytes: summary.total_freed_bytes` 改为：

```rust
        freed_bytes: summary.deleted_bytes,
```

（历史结构化字段在 Task 3 补齐；本步先保证编译。）

- [ ] **Step 4: 修现有测试**

Run: `cargo test --manifest-path src-tauri/Cargo.toml cache 2>&1 | tail -40`
把 `cache_cleaner_tests.rs` / `operation_commands*` 中引用 `freed_bytes` / `total_freed_bytes` 的断言改成 `deleted_bytes`；`CleanSummary` 构造处补 `reclaimed_bytes: None`。逐个按编译错误修正。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -20`
Expected: 全绿。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/cache_cleaner.rs src-tauri/src/operation_commands.rs src-tauri/src/cache_cleaner_tests.rs
git commit -m "refactor: 缓存清理口径改为 deleted_bytes + reclaimed_bytes，不再用扫描体积冒充释放"
```

---

## Task 3: 历史库加列 + 分发字段（只加列）

**Files:**
- Modify: `src-tauri/src/storage.rs`（schema、`HistoryEntry`、`log_history`、`recent_history`）
- Modify: `src-tauri/src/operation_commands.rs`（`OperationHistoryEntry` 及 6 个 `*_entry`）
- Test: `src-tauri/src/storage_tests.rs`（若无则新建）、`operation_commands` 相关测试

**Interfaces:**
- Produces:
  - `HistoryEntry { ..., deleted_bytes: u64, trashed_bytes: u64, reclaimed_bytes: Option<u64>, ... }`
  - `OperationHistoryEntry` 同名字段
  - `Storage::log_history(...)` 新增 3 个参数

- [ ] **Step 1: 写失败测试**（迁移 + 字段）

在 `src-tauri/src/storage_tests.rs` 追加（若文件不存在，创建并在 `storage.rs` 末尾加 `#[cfg(test)] #[path = "storage_tests.rs"] mod tests;`）：

```rust
use super::*;

#[test]
fn legacy_rows_have_null_reclaimed_after_migration() {
    let storage = Storage::open_in_memory().expect("in-memory storage");
    storage
        .log_history("cache", "2 项缓存", 100, true, "旧数据", 2, 2, 0, "")
        .expect("log");
    let rows = storage.recent_history(10).expect("read");
    assert_eq!(rows.len(), 1);
    // 这条是接口新增字段后的写入：应落到主口径
    assert_eq!(rows[0].deleted_bytes, 100);
    assert_eq!(rows[0].trashed_bytes, 0);
}
```

> 说明：若 `Storage` 目前没有 `open_in_memory()`，本步**先新增一个 `#[cfg(test)] pub(crate) fn open_in_memory() -> Result<Self, UserError>`**（用 `Connection::open_in_memory()` + 同一段 `CREATE TABLE`），让测试不碰真实 DB。

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml storage 2>&1 | tail -20`
Expected: 编译失败（字段/方法不存在）。

- [ ] **Step 3: 迁移与结构体** `src-tauri/src/storage.rs`

`CREATE TABLE history` 的 DDL 里追加三列（放在 `reason_code` 之后）：

```sql
                reclaimed_bytes INTEGER,
                deleted_bytes INTEGER NOT NULL DEFAULT 0,
                trashed_bytes INTEGER NOT NULL DEFAULT 0
```

`migrate_history_columns` 的数组追加：

```rust
            (
                "reclaimed_bytes",
                "ALTER TABLE history ADD COLUMN reclaimed_bytes INTEGER",
            ),
            (
                "deleted_bytes",
                "ALTER TABLE history ADD COLUMN deleted_bytes INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "trashed_bytes",
                "ALTER TABLE history ADD COLUMN trashed_bytes INTEGER NOT NULL DEFAULT 0",
            ),
```

`HistoryEntry` 追加：

```rust
    /// 成功永久删除的体积之和（缓存主口径）。
    pub deleted_bytes: u64,
    /// 移入废纸篓的体积之和（卸载主口径，尚未释放）。
    pub trashed_bytes: u64,
    /// 卷可用空间的实测增量；`None` 表示未能测量（区别于 0）。
    pub reclaimed_bytes: Option<u64>,
```

`log_history` 增加三个参数并在 INSERT 中写入；`recent_history` 的 SELECT 与行解析同步读取三列（`reclaimed_bytes: Option<i64>` → `map(|v| v as u64)`）。

- [ ] **Step 4: `operation_commands.rs` 补齐**

`OperationHistoryEntry` 追加三个字段；`StorageHistory::record` 把新字段传给 `log_history`。六个构造器补齐默认值：

- `cache_entry`：`deleted_bytes: summary.deleted_bytes`，`trashed_bytes: 0`，`reclaimed_bytes: summary.reclaimed_bytes`
- `uninstall_entry`：`deleted_bytes: 0`，`trashed_bytes:`（Task 4 改名后）来源值，`reclaimed_bytes: None`（Task 4 补实测）
- `docker_entry` / `quit_entry` / `process_entry` / `rejection_entry`：`deleted_bytes: 0`，`trashed_bytes: 0`，`reclaimed_bytes: None`

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -20`

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/storage.rs src-tauri/src/storage_tests.rs src-tauri/src/operation_commands.rs
git commit -m "feat: 历史库只加列记录 deleted/trashed/reclaimed，旧 freed_bytes 保留兜底"
```

---

## Task 4: 卸载口径正名 `trashed_bytes` + 实测

**Files:**
- Modify: `src-tauri/src/uninstaller.rs`（`UninstallReport`、`uninstall_app`、`build_report`）
- Modify: `src-tauri/src/operation_commands.rs`（`uninstall_entry`）

**Interfaces:**
- Produces: `UninstallReport { ..., trashed_bytes: u64, reclaimed_bytes: Option<u64>, ... }`（原 `total_freed_bytes` 改名为 `trashed_bytes`）

- [ ] **Step 1: 改 `UninstallReport` 与 `build_report`**

`total_freed_bytes: u64,` → `trashed_bytes: u64,`，并追加 `pub reclaimed_bytes: Option<u64>,`。`build_report` 增加参数 `reclaimed_bytes: Option<u64>`，内部 `let trashed_bytes: u64 = details.iter().map(|d| d.size_bytes).sum();` 并返回新字段。

- [ ] **Step 2: 在 `uninstall_app` 包测量**

```rust
pub(crate) async fn uninstall_app(target: &UninstallTarget) -> UninstallReport {
    let mut all_paths: Vec<String> = Vec::with_capacity(1 + target.residue_paths.len());
    all_paths.push(target.bundle_path.clone());
    all_paths.extend(target.residue_paths.iter().cloned());

    let before = crate::volume::VolumeCapacity::read();
    let (mut details, needs_admin) = trash_with_user_permission(&all_paths).await;
    if !needs_admin.is_empty() {
        details.extend(trash_with_admin(&needs_admin).await);
    }
    let after = crate::volume::VolumeCapacity::read();

    build_report(target, details, crate::volume::reclaimed(before, after))
}
```

- [ ] **Step 3: 改 `uninstall_entry`**（`operation_commands.rs`）

`total_freed_bytes` 引用改为 `trashed_bytes`；`freed_bytes:` 兼容列写 `trashed_bytes`；新增字段：

```rust
        freed_bytes: reports.iter().fold(0_u64, |total, report| {
            total.saturating_add(report.trashed_bytes)
        }),
        deleted_bytes: 0,
        trashed_bytes: reports.iter().fold(0_u64, |total, report| {
            total.saturating_add(report.trashed_bytes)
        }),
        reclaimed_bytes: None,
```

- [ ] **Step 4: 修测试 + 跑**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -30`
按编译错误把 `total_freed_bytes` 改成 `trashed_bytes`。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/uninstaller.rs src-tauri/src/operation_commands.rs
git commit -m "fix: 卸载报告把 total_freed_bytes 正名为 trashed_bytes（文件在废纸篓，尚未释放）"
```

---

## Task 5: Docker 报告补实测 `reclaimed_bytes`

**Files:**
- Modify: `src-tauri/src/docker.rs`（`DockerExecutionReport`）
- Modify: `src-tauri/src/operation_executor.rs`（`run_docker`、`execute_docker_prune`）
- Modify: `src-tauri/src/operation_commands.rs`（`docker_entry`）

**Interfaces:**
- Produces: `DockerExecutionReport { action, succeeded, failed, output, reclaimed_bytes: Option<u64> }`

- [ ] **Step 1: 改结构体与构造器** `docker.rs`

```rust
#[derive(Serialize, Clone, Debug)]
pub struct DockerExecutionReport {
    pub action: String,
    pub succeeded: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub output: String,
    /// 卷可用空间的实测增量；`None` 表示未能测量。
    pub reclaimed_bytes: Option<u64>,
}
```

`new` 里补 `reclaimed_bytes: None`。

- [ ] **Step 2: 包测量** `operation_executor.rs`

`run_docker`：在 `let mut report = DockerExecutionReport::new(action);` 前读 `before`，在 `Ok(report)` 前读 `after` 并 `report.reclaimed_bytes = crate::volume::reclaimed(before, after);`。

`execute_docker_prune`：在 `domain.prune().await?` 前后各读一次，同样赋值。

- [ ] **Step 3: `docker_entry` 落字段**（`operation_commands.rs`）

```rust
        freed_bytes: report.reclaimed_bytes.unwrap_or(0),
        deleted_bytes: 0,
        trashed_bytes: 0,
        reclaimed_bytes: report.reclaimed_bytes,
```

- [ ] **Step 4: 修测试 + 跑**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | tail -30`

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/docker.rs src-tauri/src/operation_executor.rs src-tauri/src/operation_commands.rs
git commit -m "feat: Docker 执行报告补卷容量实测 reclaimed_bytes"
```

---

## Task 6: 前端 DTO 与缓存结果口径

**Files:**
- Modify: `src/lib/tauri.ts`
- Modify: `src/views/CacheView.tsx`
- Modify: `src/views/UninstallerView.tsx`
- Modify: `src/components/DockerSection.tsx`
- Modify: `src/i18n/zh-CN.ts` / `src/i18n/en.ts`
- Test: `src/views/CacheView.test.tsx`、`src/views/UninstallerView.test.tsx`、`src/components/DockerSection.test.tsx`

**Interfaces:**
- Consumes: 后端新字段
- Produces: 前端类型与新文案

- [ ] **Step 1: 改 DTO** `src/lib/tauri.ts`

```ts
export type CleanReport = {
  id: string;
  label_key: string;
  label_params: I18nParams;
  success: boolean;
  deleted_bytes: number;
  duration_ms: number;
  error: TauriError | null;
};

export type CleanSummary = {
  reports: CleanReport[];
  deleted_bytes: number;
  reclaimed_bytes: number | null;
  success_count: number;
  fail_count: number;
};

export type UninstallReport = {
  app_name: string;
  bundle_id: string;
  trashed_bytes: number;
  reclaimed_bytes: number | null;
  moved_count: number;
  failed_count: number;
  details: MoveResult[];
  quit_error: TauriError | null;
};

export type DockerExecutionReport = {
  action: string;
  succeeded: string[];
  failed: [string, string][];
  output: string;
  reclaimed_bytes: number | null;
};
```

`HistoryEntry` 追加：`deleted_bytes: number; trashed_bytes: number; reclaimed_bytes: number | null;`（保留 `freed_bytes`）。

- [ ] **Step 2: 改 `CacheView.tsx` 结果口径**

`confirmClean`（约 278-292 行）：`notifyCleanComplete(outcome.value.total_freed_bytes, ...)` 改为

```ts
      const reclaimed = outcome.value.reclaimed_bytes;
      await notifyCleanComplete(reclaimed, outcome.value.deleted_bytes, outcome.value.success_count);
```

`createEffect`（约 306-315 行）：`const freed = summary()?.total_freed_bytes ?? 0;` 改为

```ts
    const freed = summary()?.reclaimed_bytes ?? 0;
```

结果头部（约 398-412 行）：`releaseLabel` + 大数字改成条件渲染：

```tsx
            <Show when={summary() && !cleaning()}>
              <div class="mt-4">
                <div class="text-[11px] uppercase tracking-[0.16em] text-success-600 dark:text-success-400">
                  {summary()!.reclaimed_bytes != null
                    ? t("cache.releaseLabel")
                    : t("cache.deletedUnmeasuredLabel")}
                </div>
                <div class="mt-1 text-4xl font-bold tabular-nums text-success-600 clean-result-number">
                  {summary()!.reclaimed_bytes != null
                    ? fmtBytes(displayFreedBytes())
                    : fmtBytes(summary()!.deleted_bytes)}
                </div>
                <div class="mt-2 text-sm text-zinc-600 dark:text-zinc-300">
                  {t("cache.cleanSuccessDetail", {
                    count: summary()!.success_count,
                    failed: summary()!.fail_count,
                  })}
                </div>
              </div>
            </Show>
```

清理完成 CTA 文案（约 454 行 `size: fmtBytes(s().total_freed_bytes)`）改为

```tsx
                    size: fmtBytes(s().reclaimed_bytes ?? s().deleted_bytes),
```

- [ ] **Step 3: 改 `UninstallerView.tsx`**

约 327 行 `r.total_freed_bytes` → `r.trashed_bytes`。结果展示处（grep `movedToTrash` / 结果数字）把「释放」措辞改为「已移入废纸篓，尚未释放」。

- [ ] **Step 4: 改 `DockerSection.tsx`**

若展示释放量，改用 `reclaimed_bytes`；为 `null` 时不显示数字。同步 `DockerSection.test.tsx` 的 mock。

- [ ] **Step 5: 加 i18n key**（zh-CN.ts 与 en.ts）

`cache` 命名空间加：

```ts
    deletedUnmeasuredLabel: "已删除（未能测量释放）",
```

`common`（或新 `result` 命名空间）加：

```ts
  result: {
    reclaimed: "实测释放 {size}",
    deletedUnmeasured: "已删除 {count} 项（未能测量释放）",
    trashedPending: "已移入废纸篓 {size}，尚未释放",
  },
```

en.ts 对应英文。`Dict = typeof zhCN`，en 必须结构一致。

- [ ] **Step 6: 改 `notifyCleanComplete` 签名**

找到定义（`grep -rn "notifyCleanComplete" src`），改为 `(reclaimed: number | null, deleted: number, count: number)`：`reclaimed != null` 用 `cache.notifyBody`，否则用新增 `cache.notifyBodyUnmeasured`（"已清理 {count} 项，未能测量释放空间"）。修所有调用点与测试。

- [ ] **Step 7: 跑前端测试与类型检查**

Run: `bun run test 2>&1 | tail -30 && bunx tsc --noEmit`
Expected: 全绿。按报错修正 `.test.tsx` 中的 mock（`total_freed_bytes` → `deleted_bytes` + `reclaimed_bytes`；`UninstallReport` → `trashed_bytes`）。

- [ ] **Step 8: 提交**

```bash
git add src/lib/tauri.ts src/views/CacheView.tsx src/views/UninstallerView.tsx src/components/DockerSection.tsx src/i18n/zh-CN.ts src/i18n/en.ts src/views/*.test.tsx src/components/DockerSection.test.tsx
git commit -m "feat: 前端结果与通知按实测/已删除口径渲染，卸载明确\"移入废纸篓尚未释放\""
```

---

## Task 7: 缓存页安全筛选 chips

**Files:**
- Create: `src/lib/safety.ts`
- Modify: `src/views/CacheView.tsx`
- Modify: `src/i18n/zh-CN.ts` / `src/i18n/en.ts`
- Test: `src/lib/safety.test.ts`

**Interfaces:**
- Produces:
  - `type SafetyFilter = "all" | "safe" | "check"`
  - `function safetyBucket(safety: Safety): "safe" | "check"`
  - `function filterBySafety<T extends { safety: Safety }>(items: T[], filter: SafetyFilter): T[]`

- [ ] **Step 1: 写失败测试** `src/lib/safety.test.ts`

```ts
import { describe, expect, it } from "vitest";
import { filterBySafety, safetyBucket } from "./safety";

const items = [
  { safety: "safe" as const },
  { safety: "low" as const },
  { safety: "medium" as const },
];

describe("safetyBucket", () => {
  it("把 safe 归为 safe，low/medium 归为 check", () => {
    expect(safetyBucket("safe")).toBe("safe");
    expect(safetyBucket("low")).toBe("check");
    expect(safetyBucket("medium")).toBe("check");
  });
});

describe("filterBySafety", () => {
  it("all 返回全部", () => {
    expect(filterBySafety(items, "all")).toHaveLength(3);
  });
  it("safe 只留 safe", () => {
    expect(filterBySafety(items, "safe")).toEqual([{ safety: "safe" }]);
  });
  it("check 只留 low/medium", () => {
    expect(filterBySafety(items, "check")).toEqual([
      { safety: "low" },
      { safety: "medium" },
    ]);
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bunx vitest run src/lib/safety.test.ts 2>&1 | tail -20`
Expected: FAIL（模块不存在）。

- [ ] **Step 3: 实现** `src/lib/safety.ts`

```ts
import type { Safety } from "@/lib/tauri";

export type SafetyFilter = "all" | "safe" | "check";

/** safe → 可直接清理；low/medium → 需要复核。 */
export function safetyBucket(safety: Safety): "safe" | "check" {
  return safety === "safe" ? "safe" : "check";
}

export function filterBySafety<T extends { safety: Safety }>(
  items: T[],
  filter: SafetyFilter,
): T[] {
  if (filter === "all") return items;
  return items.filter((item) => safetyBucket(item.safety) === filter);
}
```

- [ ] **Step 4: 接入 `CacheView.tsx`**

顶部（`grouped` memo 之前）加信号与计数：

```tsx
  const [safetyFilter, setSafetyFilter] = createSignal<SafetyFilter>("all");
  const safeCount = createMemo(() => items().filter((i) => i.safety === "safe").length);
  const checkCount = createMemo(() => items().length - safeCount());
```

`grouped` memo 的循环源改为 `filterBySafety(items(), safetyFilter())`。

列表上方加 chips（`<Show when={snapshot()}>` 内、分组列表前）：

```tsx
      <div class="flex items-center gap-2">
        <For each={[
          { key: "all" as const, label: t("safety.all"), n: items().length },
          { key: "safe" as const, label: t("safety.safe"), n: safeCount() },
          { key: "check" as const, label: t("safety.checkFirst"), n: checkCount() },
        ]}>
          {(chip, index) => (
            <button
              type="button"
              class="px-3 py-1.5 rounded-full text-xs font-medium border transition-colors"
              classList={{
                "bg-brand-500 text-white border-brand-500": safetyFilter() === chip.key,
                "border-black/10 dark:border-white/15 text-zinc-600 dark:text-zinc-300":
                  safetyFilter() !== chip.key,
              }}
              onClick={() => setSafetyFilter(chip.key)}
            >
              {chip.label} · {chip.n}
            </button>
          )}
        </For>
      </div>
```

加 ⌘1–3（组件内一个 `onMount`/`onCleanup` 监听 `keydown`，`metaKey && key in {1,2,3}` 时 `setSafetyFilter` 对应值）。筛选是**纯视图层**：不动 `selected()` 与 `default_select`。

- [ ] **Step 5: 加 i18n key**

zh-CN.ts / en.ts 各加顶层：

```ts
  safety: {
    all: "全部",
    safe: "可安全清理",
    checkFirst: "需先复核",
  },
```

- [ ] **Step 6: 跑测试与类型检查**

Run: `bunx vitest run src/lib/safety.test.ts src/views/CacheView.test.tsx 2>&1 | tail -30 && bunx tsc --noEmit`
Expected: 全绿。

- [ ] **Step 7: 提交**

```bash
git add src/lib/safety.ts src/lib/safety.test.ts src/views/CacheView.tsx src/i18n/zh-CN.ts src/i18n/en.ts
git commit -m "feat: 缓存页加安全筛选 chips（全部/可安全清理/需先复核 + ⌘1-3）"
```

---

## Task 8: 历史页诚实口径渲染

**Files:**
- Modify: `src/views/HistoryView.tsx`
- Modify: `src/i18n/zh-CN.ts` / `src/i18n/en.ts`
- Test: `src/views/HistoryView.test.tsx`

- [ ] **Step 1: 写失败测试**（追加到 `HistoryView.test.tsx`）

新增一条：卸载旧行（`item_count: 0`）仍显示 `freed_bytes`；新行（`item_count > 0`, `operation: "uninstall"`）显示「移入废纸篓」措辞而**不**显示「+」。

```ts
it("卸载新纪录显示移入废纸篓，不显示为释放", async () => {
  (invoke as unknown as Mock).mockResolvedValueOnce([
    {
      id: 1, timestamp: new Date().toISOString(), operation: "uninstall",
      target: "1 个应用", freed_bytes: 1300, success: true, detail: "",
      item_count: 1, ok_count: 1, fail_count: 0, reason_code: "",
      deleted_bytes: 0, trashed_bytes: 1300, reclaimed_bytes: null,
    },
  ]);
  // ...render，断言出现 "废纸篓" 且不出现 "+1.3"
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `bunx vitest run src/views/HistoryView.test.tsx 2>&1 | tail -20`

- [ ] **Step 3: 改 `HistoryView.tsx` 右侧渲染**

把 192-201 行的 `freed_bytes` 块替换为按口径分支：

```tsx
                    <div class="text-right text-xs text-zinc-500 tabular-nums flex-shrink-0">
                      <Show when={e.item_count > 0 && e.operation === "uninstall" && e.trashed_bytes > 0}>
                        <div class="font-medium text-warning-600">
                          {t("history.trashed", { size: fmtBytes(e.trashed_bytes) })}
                        </div>
                      </Show>
                      <Show when={e.item_count > 0 && e.operation !== "uninstall" && e.reclaimed_bytes != null && e.reclaimed_bytes > 0}>
                        <div class="font-medium text-success-600">
                          +{fmtBytes(e.reclaimed_bytes!)}
                        </div>
                      </Show>
                      <Show when={e.item_count > 0 && e.operation !== "uninstall" && e.reclaimed_bytes == null && e.deleted_bytes > 0}>
                        <div class="text-zinc-400">
                          {t("history.deletedUnmeasured", { size: fmtBytes(e.deleted_bytes) })}
                        </div>
                      </Show>
                      <Show when={e.item_count === 0 && e.freed_bytes > 0}>
                        <div class="font-medium text-success-600">
                          +{fmtBytes(e.freed_bytes)}
                        </div>
                      </Show>
                      <div>
                        {fmtRelativeTime(e.timestamp, t, effectiveLocale())}
                      </div>
                    </div>
```

- [ ] **Step 4: 加 i18n key**

```ts
  history: {
    // ...existing
    trashed: "{size} → 废纸篓",
    deletedUnmeasured: "已删除 {size}（未测量）",
  },
```

- [ ] **Step 5: 跑测试与类型检查**

Run: `bunx vitest run src/views/HistoryView.test.tsx 2>&1 | tail -20 && bunx tsc --noEmit`

- [ ] **Step 6: 提交**

```bash
git add src/views/HistoryView.tsx src/i18n/zh-CN.ts src/i18n/en.ts src/views/HistoryView.test.tsx
git commit -m "feat: 历史页区分实测释放/已删除未测量/移入废纸篓，旧数据回退"
```

---

## Task 9: 全量验证 + MAS 真机沙箱核对

**Files:** 无代码改动（验证）

- [ ] **Step 1: Rust 全量**

Run: `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check && cargo test --manifest-path src-tauri/Cargo.toml --all-targets && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
Expected: 退出 0。

- [ ] **Step 2: 前端全量**

Run: `bun run test 2>&1 | tail -30 && bunx tsc --noEmit && bun run build`
Expected: 全绿。

- [ ] **Step 3: 完整 verify**

Run: `bun run verify`
Expected: 退出 0。

- [ ] **Step 4: MAS 真机沙箱验证（AGENTS §8.6）**

按 §8.6 用 Development profile 另签一份 MAS 副本启动，跑一次缓存清理 + 一次卸载：
- 缓存结果页：`statfs` 可读时显示「实测释放 X」；不可读时显示「已删除（未能测量释放）」，**不**谎报数字。
- 卸载结果页：显示「已移入废纸篓，尚未释放」，Historique 对应行显示「→ 废纸篓」而非 `+X`。

- [ ] **Step 5: 记录结论**

在 PR/提交说明里写明：MAS 下 `statfs` 是否可读、卸载口径是否不再谎报。若 `statfs` 被拦，确认降级分支生效且未报错。

---

## Self-Review 记录

**Spec 覆盖**：三口径模型→Task 2/3/4/5；`volume.rs` 实测→Task 1；历史只加列→Task 3；安全筛选→Task 7；i18n→Task 6/7/8；MAS/完全版同构→Task 9。门禁三态与 Trash-by-default 已在 spec 标为不做，无对应任务（符合）。

**类型一致性**：`CleanSummary.deleted_bytes/reclaimed_bytes`、`UninstallReport.trashed_bytes/reclaimed_bytes`、`DockerExecutionReport.reclaimed_bytes`、`HistoryEntry.deleted_bytes/trashed_bytes/reclaimed_bytes`、`VolumeCapacity::{read,read_for}/reclaimed/NOISE_FLOOR_BYTES` 在前后端命名一致。

**已知顺序约束**：Task 2 会暂时让前端类型与新 Rust DTO 不一致（前端 `bun` 构建此时失败属预期），Task 6 补齐；Task 2-5 的验证命令只跑 Rust。全量 `bun run verify` 只在 Task 9 要求通过。
