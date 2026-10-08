# MacSlim 诚实清理报告 + 安全筛选 设计文档

日期：2026-10-08
状态：待评审
关联：AGENTS.md §4.1（不可逆操作诚实表述）、§8.6（沙箱文案不得拼死）、Purge 竞品对齐（清理安全模型）
上游：本文件是「对齐 Purge 清理安全模型」子项目 #1 的 spec。后续子项目 #0（设计 token）、#2（扫描范围）、#3（定时清理）、#4/#5（完全版专有）各自另立 spec。

---

## 1. 背景与问题

在审查竞品 Purge（`purgemac.com`）的清理安全模型后，决定对齐其「安全标签 + 诚实报告」设计语言。踩点后发现 MacSlim 已完成大部分，但存在两处真实缺陷。

### 1.1 已完成、无需重做

- 后端 `cache_scanner.rs:150` 已有三态 `Safety { Safe, Low, Medium }`，每个 `CacheItem` 携带 `safety` + `default_select`。
- 前端 `CacheView.tsx:528-538` 已按 safe/low/medium 渲染徽章；`format.ts:104` 的 `isCategoryReclaimable` 读 `safety` 判定整类可回收。
- 删除前 TOCTOU 复检已存在（`cache_cleaner.rs` 的 `pre_delete_recheck_failed`）。
- 扫描期已丢弃当前构建形态清不掉的项（`cache_scanner.rs` 的 `drop_uncleanable`）。

### 1.2 缺陷一：报告数字不诚实（核心问题）

`CleanReport.freed_bytes` 现在是「成功项的**扫描快照体积**之和」（`cache_cleaner.rs:262`），并未实测，却直接以「释放 X」展示。三类操作各有不同的失真：

| 操作 | 当前 `freed_bytes` 的真实来源 | 是否等于「已释放」 |
| :--- | :--- | :--- |
| 缓存清理 | 成功删除项的扫描体积之和 | ⚠️ 未实测，可能有偏差 |
| **应用卸载** | `UninstallReport.total_freed_bytes`＝**移入废纸篓**的体积之和（`operation_commands.rs:535`） | ❌ **错误**：文件还在废纸篓，空间并未释放 |
| Docker | 恒为 `0`（`operation_commands.rs:558`） | ❌ 明明删了却报 0 |

其中**卸载**这一条最严重：它是本产品「可追溯」的核心页面，却把「移入废纸篓（尚未释放）」当作「已释放」显示，直接违反 AGENTS.md §4.1。

### 1.3 缺陷二：有安全标签、无安全筛选

Purge 提供 `All / Safe to Clean / Check First` 三档筛选 + 计数 + ⌘1–3。MacSlim 只有徽章，用户无法一键「只看安全的」或「只看需要复核的」。

---

## 2. 目标与非目标

### 目标

1. **报告三口径**：把笼统的「释放」拆成 `deleted_bytes` / `trashed_bytes` / `reclaimed_bytes`，只有实测的 `reclaimed_bytes` 能叫「释放」。
2. **卷容量实测**：用 `statfs` 取家目录所在卷的前后可用空间差，带噪声下限。
3. **安全筛选**：缓存页加三档 chip + 计数 + ⌘1–3，映射现有 `Safety`。
4. **历史只加列**：保留旧 `freed_bytes` 列与旧文案兜底，新增结构化字段，旧库平滑升级。
5. **文案 i18n**：所有口径文案走前端词典，后端不拼死中文（§8.6）。
6. **MAS / 完全版同构**：本子项目两边行为一致，无构建分叉。

### 非目标（明确不做）

- ❌ **Trash-by-default**：删除去向保持现状（缓存永久删除；卸载本就是移入废纸篓）。用户已决策。
- ❌ **门禁三态改造**：不把「未授权 / 未白名单」从硬失败降级为 silent-skip。扫描期 `drop_uncleanable` 已等价覆盖；把真实异常降级成静默跳过会掩盖 bug。
- ❌ 大文件扫描、重复文件清理、AI 模型清理（AGENTS.md §6/§9 + 用户决策）。
- ❌ 解析 CLI（npm/docker 等）输出以获取精确释放量——用卷容量实测替代。
- ❌ 逐项 re-measure 每个条目的实时占用。

---

## 3. 设计

### 3.1 报告数据模型

三个口径，语义互不重叠：

| 字段 | 含义 | 来源 |
| :--- | :--- | :--- |
| `deleted_bytes` | 成功**永久删除**的条目体积之和 | 扫描快照的 `size_bytes` |
| `trashed_bytes` | 成功**移入废纸篓**（尚未释放）的体积之和 | 扫描快照的 `size_bytes` |
| `reclaimed_bytes: Option<u64>` | 卷可用空间的**实测**增量 | `statfs` 前后差 − 噪声下限 |

**展示铁律**：

- 只有 `reclaimed_bytes = Some(n)` 才允许用「释放 n」措辞。
- `None` 时必须显式说明「已删除 X（未能测量释放）」或「已移入废纸篓 X（尚未释放）」。
- **绝不**用 `deleted_bytes` / `trashed_bytes` 填空冒充 `reclaimed_bytes`。

各操作的类型映射：

| 操作 | `deleted_bytes` | `trashed_bytes` | `reclaimed_bytes` |
| :--- | :--- | :--- | :--- |
| 缓存清理 | ✓ 成功项之和 | 0 | ✓ 实测 |
| 应用卸载 | 0 | ✓ 已移动项之和 | ✓ 实测（预期 ≈0，因文件仍在废纸篓） |
| Docker | 0（不掌握体积） | 0 | ✓ 实测 |
| 进程终止 / 优雅退出 | — | — | `None`（不涉及空间，仅计数） |

### 3.2 后端：卷容量实测（新增 `volume.rs`）

新增 `src-tauri/src/volume.rs`：

```rust
pub struct VolumeCapacity {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

impl VolumeCapacity {
    /// 家目录所在卷。无权限或调用失败返回 None（与"可用 0"严格区分）。
    pub fn read() -> Option<VolumeCapacity>;
}

/// 低于该增量视为测量噪声，不上报 reclaim。
pub const NOISE_FLOOR_BYTES: u64 = 4 * 1024 * 1024;
```

实现要点：

- 用 `statfs`（`libc` / `nix`），对 `folder_access::scanner_home()` 取卷；`f_bavail × f_bsize`。
- 读失败返回 `None`，**不得**伪装成 0。
- 提供纯函数 `reclaimed(before, after) -> Option<u64>`：仅当两者都可读、差值 ≥ `NOISE_FLOOR_BYTES` 且为正时返回 `Some`，否则 `None`。此函数独立可测。

### 3.3 后端：执行层

- **`cache_cleaner.rs`**：
  - `CleanReport` 的 `freed_bytes` **正名为** `deleted_bytes: u64`（无兼容负担：这是内部执行 DTO）。
  - `CleanSummary` 用 `deleted_bytes: u64` + `reclaimed_bytes: Option<u64>` **替换** `total_freed_bytes`；`success_count` / `fail_count` 不变。
  - `clean_with_runtime` 在循环前后各读一次 `VolumeCapacity`，结果经 `reclaimed()` 落入 `CleanSummary`。
- **`uninstaller.rs`**：`UninstallReport.total_freed_bytes` 正名为 `trashed_bytes`（实现本就把文件移入废纸篓，语义纠正）。
- **`docker.rs`**：`DockerExecutionReport` 增 `reclaimed_bytes: Option<u64>`；执行前后测量。
- 保留 `estimated_bytes`（`PreparedOperation`）——它是操作前的预估，语义未变。

### 3.4 历史 schema 迁移（只加列）

在 `storage.rs` 的 `migrate_history_columns` 追加（沿用现有 ALTER + DEFAULT 模式）：

| 新列 | DDL |
| :--- | :--- |
| `reclaimed_bytes` | `ALTER TABLE history ADD COLUMN reclaimed_bytes INTEGER`（**可空**：NULL＝未实测，区别于 0） |
| `deleted_bytes` | `ALTER TABLE history ADD COLUMN deleted_bytes INTEGER NOT NULL DEFAULT 0` |
| `trashed_bytes` | `ALTER TABLE history ADD COLUMN trashed_bytes INTEGER NOT NULL DEFAULT 0` |

- **旧的 `freed_bytes` 列保留并继续写入**（值＝该操作的主口径：缓存写 `deleted_bytes`，卸载写 `trashed_bytes`），使旧客户端/旧数据仍可渲染。
- `OperationHistoryEntry` **保留** `freed_bytes`（兼容字段，同上语义），并**新增** `deleted_bytes` / `trashed_bytes` / `reclaimed_bytes: Option<u64>`；三个 `*_entry` 构造器同步填充。
- `HistoryEntry` / `log_history` / `recent_history` 同步新字段。
- 旧行 `reclaimed_bytes` 为 NULL → 前端回退到旧的 `freed_bytes` 文案。

### 3.5 前端

**筛选（对齐 Purge）**

- 新增映射（`format.ts` 或新 `safety.ts`）：
  - `Safe → "Safe to Clean"`（i18n key `safety.safe`）
  - `Low | Medium → "Check First"`（i18n key `safety.checkFirst`）
- `CacheView` 顶部加三档 chip：`All / Safe to Clean / Check First`，含实时计数，快捷键 ⌘1–3，遵守 `prefers-reduced-motion`。筛选纯视图层，**不影响**默认勾选（`default_select`）与门禁。

**结果口径（`CacheView` 结果头部）**

- `reclaimed_bytes = Some(n)` → 「实测释放 n」
- `reclaimed_bytes = None` → 「已删除 X（未能测量释放）」
- 卸载结果 → 「已移入废纸篓 X，尚未释放（清空废纸篓后生效）」

**历史（`HistoryView`）**

- 按结构化字段本地化渲染：区分「已删除 / 已移入废纸篓 / 实测释放」。
- `reclaimed_bytes` 为 NULL 的旧行 → 回退到原 `freed_bytes` 呈现，行为与升级前一致。

### 3.6 i18n

新增 key（`src/i18n/zh-CN.ts` / `en.ts`）：

- `safety.all` / `safety.safe` / `safety.checkFirst`（筛选 chip）
- `result.reclaimed`（实测释放）
- `result.deletedUnmeasured`（已删除、未能测量）
- `result.trashedPending`（已移入废纸篓、尚未释放）
- `history.accounting.deleted` / `history.accounting.trashed` / `history.accounting.reclaimed`

后端只下发结构化计数 + `Option`，不拼可读文案（§8.6）。

---

## 4. 数据流

```
clean_with_runtime
  ├─ before = VolumeCapacity::read()          // Option
  ├─ for item in items { clean_item(item) }   // 汇总 deleted_bytes
  ├─ after  = VolumeCapacity::read()          // Option
  └─ reclaimed = reclaimed(before, after)     // Option, 过噪声下限

CleanSummary { deleted_bytes, reclaimed_bytes }
  └─ history_entry() → OperationHistoryEntry { deleted_bytes, trashed_bytes, reclaimed_bytes }
       └─ StorageHistory → storage.log_history（+ 旧 freed_bytes 兼容列）
            └─ recent_history → HistoryEntry → 前端按字段本地化
```

卸载走同一测量外壳，主口径落在 `trashed_bytes`；Docker 只填 `reclaimed_bytes`。

---

## 5. 安全面（MAS / 完全版）

| 能力 | MAS | 完全版 |
| :--- | :--: | :--: |
| `statfs` 读卷容量 | ✅（需真机确认沙箱不拦） | ✅ |
| 安全筛选 / 历史字段 / 口径文案 | ✅ | ✅ |
| 卸载 `trashed_bytes` 口径 | ✅（卸载执行本已 gate） | ✅ |

- 两边**同构**，不引入构建分叉。
- 路径涉及处继续只用 `folder_access::scanner_home()`（§7.5）。
- `statfs` 在 App Sandbox 下是否被拦需**真机验证**（§8.6）；若被拦，MAS 版 `reclaimed_bytes` 退化为 `None`（文案自动走"未能测量"分支），功能不失效。

---

## 6. 测试计划

### Rust 单测

- `volume`: `reclaimed()` 的边界——两读其一为 None、差值为负、差值低于噪声下限、正常增量。
- `cache_cleaner`: `CleanSummary.deleted_bytes` = 成功项体积之和；`reclaimed_bytes` 由注入的假容量读数决定（不依赖真实磁盘）。
- `operation_commands`: 缓存/卸载/Docker 三种 `OperationHistoryEntry` 的口径字段映射正确；卸载**不再**把 `trashed_bytes` 填进会展示为「释放」的字段。
- `storage`: 旧库 ALTER 迁移后旧行 `reclaimed_bytes` 为 NULL；新旧行混读不报错。

### 前端测试

- `format`/筛选：Safe→Safe to Clean、Low/Medium→Check First；chips 过滤 + 计数；⌘1–3。
- `CacheView`：三种口径文案分支（Some / None / 卸载 trashed）。
- `HistoryView`：新行按字段渲染；`reclaimed_bytes` 为 NULL 的旧行回退旧文案。

### 真机验证（§8.6）

- MAS 沙箱构建下：跑一次缓存清理与一次卸载，确认结果页分别显示「实测释放」与「已移入废纸篓，尚未释放」，且卸载数字不再被当作释放。

---

## 7. 影响文件（预估）

**Rust**
- `src-tauri/src/volume.rs`（新增）+ `volume_tests.rs`
- `src-tauri/src/lib.rs`（`mod volume`）
- `src-tauri/src/cache_cleaner.rs`（`CleanReport` / `CleanSummary` / 测量外壳）+ 测试
- `src-tauri/src/operation_commands.rs`（`OperationHistoryEntry` / 三个 `*_entry` / `history`）
- `src-tauri/src/storage.rs`（迁移 + `log_history` + `recent_history` + `HistoryEntry`）
- `src-tauri/src/uninstaller.rs`（`total_freed_bytes` → `trashed_bytes`）
- `src-tauri/src/docker.rs`（`reclaimed_bytes`）

**前端**
- `src/lib/tauri.ts` / `src/lib/ipcTypes.ts`（DTO 字段）
- `src/lib/format.ts`（Safety→筛选映射）或新增 `src/lib/safety.ts`
- `src/views/CacheView.tsx`（chips + 结果口径）
- `src/views/HistoryView.tsx`
- 卸载结果组件（`src/components/*` 或 `UninstallerView.tsx`）
- `src/i18n/zh-CN.ts` / `src/i18n/en.ts`

---

## 8. 落地顺序

1. `volume.rs` + 单测（纯逻辑，先行可测）。
2. `cache_cleaner` 口径改造 + 单测。
3. `uninstaller` / `docker` 口径正名。
4. 历史 schema 迁移 + 单测。
5. 前端 DTO / 筛选 chips / 结果与历史渲染 + i18n。
6. 真机沙箱验证（MAS），按 §8.1 分步提交。
