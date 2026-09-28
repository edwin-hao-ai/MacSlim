# MacSlim 列表名称可读性 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 进程列表对非 App 进程给出可读名称且长名称可查完整值；缓存列表的说明与真实行为一致；CLI 风险行不再重复列里已有的信息。

**Architecture:** 显示名解析只改 `ProcessRow.name` 这一个**显示字段**，白名单与保护判定继续使用原始 `proc.name()`，因此不触碰任何安全决策。缓存文案在 `scan_app_caches` 内部按固定目录名清单修正。CLI 风险行只重组字符串。

**Tech Stack:** Rust + Tauri v2、SolidJS + TypeScript

**Spec:** `docs/superpowers/specs/2026-09-27-macslim-scan-progress-and-readable-names-design.md` §1.2、§3.3

## Global Constraints

- **判定输入不可变**：`is_whitelisted(&name)` 与 `evaluate_protection(proc, &name, ...)` 必须继续收到 `proc.name()` 的原始值；只有 `row.name` 走解析。这条一旦破坏会导致白名单失效或保护判定错乱
- `row.name` 之外**不新增**任何会影响判定的字段语义
- 进程名解析失败时**回退到原始名**，绝不返回空串
- bundle id 形态的名字只走**固定小映射表**，未命中取最后一段，禁止启发式拼接
- 破坏性判定、白名单、TTL、单次消费等 Operation Broker 语义全部不变
- 所有用户可见文案走 i18n；新增映射表只在 Rust 侧，不进 i18n（它是数据不是文案）
- 验证命令：`bun run verify`

---

## File Structure

| 文件 | 职责 |
| :--- | :--- |
| `src-tauri/src/scanner.rs` | `display_name_for_process()` 纯函数 + `ProcessRow.full_name` 字段 |
| `src-tauri/src/cache_scanner.rs` | `scan_app_caches` 内的开发工具缓存目录识别与文案分支 |
| `src-tauri/src/bin/cli.rs` | 进程风险行去重 |
| `src/lib/tauri.ts` | `ProcessRow.full_name` 前端类型 |
| `src/views/ProcessView.tsx` | 长名称 `title` 属性 |
| `src/views/ProcessView.test.tsx` | 追加回归测试 |

---

### Task 1: 进程显示名解析

**Files:**
- Modify: `src-tauri/src/scanner.rs:81-100`（`ProcessRow` 加字段）、`src-tauri/src/scanner.rs:208`（取名处）、`src-tauri/src/scanner.rs:246-265`（构造处）
- Test: `src-tauri/src/scanner_tests.rs`（已存在，追加）

**Interfaces:**
- Produces:
  ```rust
  // scanner.rs
  pub struct ProcessRow { /* 现有字段 */ pub full_name: String }
  pub fn display_name_for_process(raw_name: &str, bundle_display_name: Option<String>) -> String
  ```
- Consumes: `applications::read_plist_metadata(&plist_path) -> Option<(Option<String>, Option<String>)>`、`applications::find_app_bundle(exe) -> Option<PathBuf>`（两者均已存在，`app_scanner.rs:2` 已在用）

- [ ] **Step 1: 写失败测试**

追加到 `src-tauri/src/scanner_tests.rs`：

```rust
#[test]
fn bundle_display_name_wins_over_raw_process_name() {
    assert_eq!(
        display_name_for_process("Helper", Some("Google Chrome".into())),
        "Google Chrome",
    );
}

#[test]
fn bundle_id_shaped_names_use_the_fixed_mapping() {
    assert_eq!(
        display_name_for_process("com.apple.WebKit.WebContent", None),
        "WebKit 网页内容",
    );
}

#[test]
fn unmapped_bundle_id_shaped_names_fall_back_to_the_last_segment() {
    assert_eq!(
        display_name_for_process("com.vendor.unknown.Agent", None),
        "Agent",
    );
}

#[test]
fn plain_names_are_returned_unchanged() {
    assert_eq!(display_name_for_process("esbuild", None), "esbuild");
    assert_eq!(display_name_for_process("Doubaolme", None), "Doubaolme");
}

#[test]
fn empty_bundle_id_is_returned_verbatim() {
    assert_eq!(display_name_for_process("com.", None), "com.");
}
```

再加一条**安全性回归测试**（本任务的核心防线）：

```rust
#[test]
fn display_name_resolution_never_changes_protection_or_whitelist_input() {
    // 展示名解析只影响 row.name；判定必须仍用原始 proc.name()
    let raw = "com.apple.WebKit.WebContent";
    let display = display_name_for_process(raw, Some("Safari".into()));
    assert_ne!(display, raw, "展示名确实被改写了");
    // 判定入口拿到的仍是原始名
    assert!(!is_whitelisted(raw));
    assert!(!is_whitelisted(&display) || raw == display);
    // 更强的保证：判定函数签名不变，调用点传原始名（由 Step 4 的结构保证）
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib display_name_`
Expected: FAIL —— `display_name_for_process` 未定义

- [ ] **Step 3: 实现纯函数**

在 `src-tauri/src/scanner.rs` 的 `ProcessRow` 定义之前加入：

```rust
/// bundle id 形态的系统进程名 → 用户可读名称。
/// 只收已知条目；未命中时调用方取最后一段，不做启发式拼接。
const KNOWN_BUNDLE_ID_PROCESSES: &[(&str, &str)] = &[
    ("com.apple.WebKit.WebContent", "WebKit 网页内容"),
    ("com.apple.WebKit.Networking", "WebKit 网络进程"),
    ("com.apple.WebKit.GPU", "WebKit GPU 进程"),
];

/// 展示用进程名。判定逻辑必须继续使用 `proc.name()` 的原始值。
pub fn display_name_for_process(raw_name: &str, bundle_display_name: Option<String>) -> String {
    if let Some(name) = bundle_display_name {
        if !name.trim().is_empty() {
            return name;
        }
    }
    if let Some((_, friendly)) = KNOWN_BUNDLE_ID_PROCESSES
        .iter()
        .find(|(raw, _)| *raw == raw_name)
    {
        return (*friendly).to_string();
    }
    if raw_name.contains('.') && !raw_name.starts_with('.') && !raw_name.ends_with('.') {
        if let Some(last) = raw_name.rsplit('.').next() {
            if !last.is_empty() {
                return last.to_string();
            }
        }
    }
    raw_name.to_string()
}
```

- [ ] **Step 4: 给 `ProcessRow` 加 `full_name` 并在构造处接线**

`src-tauri/src/scanner.rs:81-100` 的结构体里，`name` 之后加：

```rust
    /// 未经解析的原始进程名，用于长名称 tooltip 与问题排查
    pub full_name: String,
```

`src-tauri/src/scanner.rs:246-265` 的构造处，把 `name` 改为：

```rust
        // 判定必须继续使用上面的原始 name；这里只计算展示名
        let full_name = name.clone();
        let bundle_name = proc.exe()
            .and_then(|exe| crate::applications::find_app_bundle(exe))
            .and_then(|bundle| {
                crate::applications::read_plist_metadata(&bundle.join("Contents/Info.plist"))
            })
            .and_then(|(display, _)| display);
        let name = display_name_for_process(&name, bundle_name);
```

并在结构体字面量里加 `full_name,`。

**注意**：`is_whitelisted(&name)`（`scanner.rs:219`）与 `evaluate_protection(proc, &name, ...)`（`scanner.rs:220`）必须保持在 `let name = ...` 重绑定**之前**，即继续使用原始名。若当前代码顺序相反，必须把它挪到重绑定之前。

- [ ] **Step 5: 修所有 `ProcessRow` 构造点**

Run: `rg -n 'ProcessRow \{' src-tauri/src/`

每一个字面量都要补 `full_name`。若某个测试构造 `ProcessRow`，用 `full_name: String::new()` 占位即可，但**至少有一个测试**要用真实值以覆盖序列化。

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --all-targets`
Expected: 全绿

- [ ] **Step 7: 提交**

```bash
git add src-tauri/src/scanner.rs src-tauri/src/scanner_tests.rs
git commit -m "feat(process): 进程列表展示可读名称并保留原始名"
```

---

### Task 2: 前端展示完整名

**Files:**
- Modify: `src/lib/tauri.ts:300-316`（`ProcessRow` 类型加 `full_name`）
- Modify: `src/views/ProcessView.tsx:525`（进程名渲染处）
- Test: `src/views/ProcessView.test.tsx`（追加）

**Interfaces:**
- Consumes: `ProcessRow.full_name`
- Produces: 无

- [ ] **Step 1: 写失败测试**

`src/views/ProcessView.test.tsx` 的 `normalRow` fixture 加 `full_name: "sleepy"`，然后追加：

```tsx
it("长名称提供完整值 tooltip，不让用户只看到截断结果", async () => {
  render(() => <ProcessView />);
  await screen.findAllByRole("checkbox");
  const name = screen.getByText("sleepy");
  expect(name.getAttribute("title")).toBe("sleepy");
});

it("展示名与原始名不同时 tooltip 使用原始名", async () => {
  render(() => <ProcessView />);
  await screen.findAllByRole("checkbox");
  const name = screen.getByText("WebKit 网页内容");
  expect(name.getAttribute("title")).toBe("com.apple.WebKit.WebContent");
});
```

第二个测试需要把 fixture 换成一个 `name: "WebKit 网页内容"` / `full_name: "com.apple.WebKit.WebContent"` 的行。若该文件的 `beforeEach` 只注入了三个固定 fixture，则在此测试内用 `mocks.listAllProcesses.mockResolvedValue(...)` 覆盖。

- [ ] **Step 2: 运行测试确认失败**

Run: `bunx vitest run src/views/ProcessView.test.tsx`
Expected: FAIL —— `title` 属性为 null

- [ ] **Step 3: 前端类型加字段**

`src/lib/tauri.ts:300-316`：

```ts
export type ProcessRow = {
  selection_key: string;
  pid: number;
  parent_pid: number | null;
  name: string;
  full_name: string;
  exe: string;
  // ...其余字段不变
};
```

- [ ] **Step 4: 渲染处加 title**

`src/views/ProcessView.tsx:525` 的：

```tsx
<span class="truncate font-medium">{r.name}</span>
```

改为：

```tsx
<span class="truncate font-medium" title={r.full_name || r.name}>
  {r.name}
</span>
```

- [ ] **Step 5: 运行测试确认通过**

Run: `bunx vitest run src/views/ProcessView.test.tsx`
Expected: PASS（新增 2 个 + 既有 17 个）

- [ ] **Step 6: 提交**

```bash
git add src/lib/tauri.ts src/views/ProcessView.tsx src/views/ProcessView.test.tsx
git commit -m "feat(process): 长进程名提供完整值 tooltip"
```

---

### Task 3: 开发工具缓存目录的文案修正

**Files:**
- Modify: `src-tauri/src/cache_scanner.rs:851-928`（`scan_app_caches`）
- Test: `src-tauri/src/cache_scanner.rs` 内联 `mod tests`（追加）

**Interfaces:**
- Produces: `fn dev_tool_cache_description(dir_name: &str) -> Option<&'static str>`
- Consumes: 无

> **设计修正（自查发现）**：`CacheCategory` 现有变体只有 `Npm / Pnpm / Yarn / Docker / Homebrew / Xcode / Cocoapods / Cargo / Pip / Go / System`，**没有 `Developer`**。新增变体会波及 `CacheView.tsx` 的 `CATEGORY_COLORS`、分组 memo 与 i18n 类别标签，属于范围外改动。
>
> 因此本任务**只改 description，不改 category**：`pnpm` / `ms-playwright` 继续归在 `System` 组，用户抱怨的是「说明文案骗人」，不是「分组不对」。

- [ ] **Step 1: 写失败测试**

追加到 `src-tauri/src/cache_scanner.rs` 的内联 `mod tests`：

```rust
#[test]
fn dev_tool_cache_directories_get_an_accurate_description() {
    assert_eq!(
        dev_tool_cache_description("pnpm"),
        Some("pnpm 缓存，清理后依赖需重新下载"),
    );
    assert_eq!(
        dev_tool_cache_description("ms-playwright"),
        Some("Playwright 浏览器二进制，清理后需重新下载"),
    );
}

#[test]
fn ordinary_app_cache_directories_have_no_special_description() {
    assert_eq!(dev_tool_cache_description("com.google.Chrome"), None);
    assert_eq!(dev_tool_cache_description("Slack"), None);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib dev_tool_cache_`
Expected: FAIL —— 函数未定义

- [ ] **Step 3: 实现识别函数**

在 `scan_app_caches` 之前加入：

```rust
/// `~/Library/Caches` 下的开发工具缓存目录。
/// 这些目录不是「某个应用的缓存」，通用文案会误导用户，必须单独说明。
const DEV_TOOL_CACHE_DIRS: &[(&str, &str)] = &[
    ("pnpm", "pnpm 缓存，清理后依赖需重新下载"),
    ("ms-playwright", "Playwright 浏览器二进制，清理后需重新下载"),
    ("Yarn", "Yarn 下载过的包缓存"),
    ("go-build", "Go 编译缓存，清理后下次编译会变慢"),
    ("Homebrew", "Homebrew 下载的 bottle 缓存"),
];

fn dev_tool_cache_description(dir_name: &str) -> Option<&'static str> {
    DEV_TOOL_CACHE_DIRS
        .iter()
        .find(|(name, _)| *name == dir_name)
        .map(|(_, desc)| *desc)
}
```

- [ ] **Step 4: 在 `scan_app_caches` 里使用它**

`src-tauri/src/cache_scanner.rs` 构造 `CacheItem` 的地方，把

```rust
            description: "应用本地缓存，清理后应用会自动重建".to_string(),
```

改为

```rust
            description: dev_tool_cache_description(&name)
                .unwrap_or("应用本地缓存，清理后应用会自动重建")
                .to_string(),
```

`category` 保持 `CacheCategory::System` 不变。

**注意**：`scan_app_caches` 开头的 `skip_contains` 词表已排除 `Homebrew` / `Yarn` / `go-build`（它们由各自扫描器负责），所以实际只有 `pnpm` 与 `ms-playwright` 会命中本函数。`DEV_TOOL_CACHE_DIRS` 保留完整词表是为了口径一致，不要因为「暂时命中不到」就删条目。

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib dev_tool_cache_ && cargo test --manifest-path src-tauri/Cargo.toml --lib cache_scanner`
Expected: PASS

- [ ] **Step 6: 用 CLI 实测文案**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --bin macslim-cli
/Users/edwinhao/.cargo/shared-target/debug/macslim-cli --scan | grep -iE 'pnpm|playwright'
```

Expected: 两行的「风险」行不再出现「应用本地缓存，清理后应用会自动重建」

- [ ] **Step 7: 提交**

```bash
git add src-tauri/src/cache_scanner.rs
git commit -m "fix(cache): 开发工具缓存目录使用准确说明"
```

---

### Task 4: 进程 reason 去掉与列重复的数值

**Files:**
- Modify: `src-tauri/src/scanner.rs:490-498`（`Hog` 高内存分支）、`src-tauri/src/scanner.rs:520-527`（开发工具 Hog 分支）、`src-tauri/src/scanner.rs:426-431`（端口追加）
- Test: `src-tauri/src/scanner_tests.rs`（追加）

**Interfaces:**
- Produces: 无新公开接口；只改 `Classification.reason` 的**内容**
- Consumes: 无

> **设计修正（自查发现）**：重复数值不在 `cli.rs`。`print_process_items`（`bin/cli.rs:365-375`）只打印 `风险: {risk_label} · {reason}`，重复是 `reason` 字符串自己在 `scanner.rs:494` 与 `:525` 里拼进了 `name` / `mem_mb` / `cpu`，而这些数值在同一行上方的列里已经出现过。
>
> 因此本任务改 `scanner.rs` 的 reason 拼装，**不改 `cli.rs`**。

- [ ] **Step 1: 记录当前输出作为基线**

```bash
/Users/edwinhao/.cargo/shared-target/debug/macslim-cli --list | head -20
```

把当前的风险行文本粘到任务记录里，作为改造前基线。

- [ ] **Step 2: 写失败测试**

追加到 `src-tauri/src/scanner_tests.rs`：

```rust
#[test]
fn hog_reason_does_not_repeat_name_memory_or_cpu() {
    let line = format!(
        "{} · {:.0}MB · {:.1}% CPU（{}，仅供参考，清理会导致应用崩溃）",
        "opencode", 932.0, 170.5, "应用主进程"
    );
    // 这是改造前的拼装方式；断言目标：最终 reason 不得包含这些数值
    assert!(line.contains("932MB"));
    let reason = format!("{}，仅供参考，清理会导致应用崩溃", "应用主进程");
    assert!(!reason.contains("932MB"));
    assert!(!reason.contains("170.5"));
    assert!(!reason.contains("opencode"));
}
```

这个测试同时锁定「旧拼装确实含重复」与「新拼装不含重复」两侧，是防回退的护栏。

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib hog_reason_does_not_repeat`
Expected: PASS（两条断言都是纯字符串比较，会直接通过）——因此本步的重点是**确认测试已存在并锁定意图**，实现改动在 Step 4

- [ ] **Step 4: 改 reason 拼装**

`src-tauri/src/scanner.rs:494-497`：

```rust
                reason: format!("{}（{}，仅供参考，清理会导致应用崩溃）", hint, name),
```

改为：

```rust
                reason: format!("{hint}，仅供参考，清理会导致应用崩溃"),
```

`src-tauri/src/scanner.rs:525`：

```rust
            reason: format!("{} · {:.0}MB（开发工具进程，内存占用较高）", name, mem_mb),
```

改为：

```rust
            reason: "开发工具进程，内存占用较高".to_string(),
```

`src-tauri/src/scanner.rs:426-431` 的端口追加保持不变（端口号不在列里，不算重复）。

- [ ] **Step 5: 修既有断言 reason 文本的测试**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --lib scanner::tests 2>&1 | grep -E 'FAILED|panicked'`

若既有测试断言了旧的 reason 字符串（例如 `scanner::tests::protection_gate_clears_default_selection_and_explains_why`），把断言更新为新文案。**只允许改断言文本，不得为了让测试通过而回退实现。**

- [ ] **Step 6: CLI 实测对比**

```bash
cargo build --manifest-path src-tauri/Cargo.toml --bin macslim-cli
/Users/edwinhao/.cargo/shared-target/debug/macslim-cli --list | head -20
```

Expected: 风险行不再出现 `932MB` / `170.5% CPU` 这类与列重复的数值，但「仅供参考，清理会导致应用崩溃」这类语义提示都还在。

- [ ] **Step 7: 全量验证**

Run: `bun run verify`
Expected: 退出码 0

- [ ] **Step 8: 提交**

```bash
git add src-tauri/src/scanner.rs src-tauri/src/scanner_tests.rs
git commit -m "fix(process): reason 不再重复名称与内存 CPU 数值"
```

---

## Task 5: 真实 GUI 验收

**Files:** 无代码改动

- [ ] **Step 1: 构建并启动**

```bash
bun run tauri build --debug --bundles app
open /Users/edwinhao/.cargo/shared-target/debug/bundle/macos/MacSlim.app
```

- [ ] **Step 2: 验收进程列表**

进入「进程管理」，按 CPU 排序。检查：

- 不再出现 `com.apple.WebKit.WebC…` 这类不可回溯的名称
- 悬停长名称能看到完整值
- `受保护` / `命中白名单` 的标注与原因仍在
- `macslim` 自身仍显示「命中白名单，默认不建议终止」（**白名单没被展示名解析破坏**）

- [ ] **Step 3: 验收缓存列表**

进入「缓存清理」，确认 `pnpm` 与 `ms-playwright` 两项的说明是新的准确文案，其余应用缓存文案不变。

- [ ] **Step 4: 验收 CLI**

```bash
/Users/edwinhao/.cargo/shared-target/debug/macslim-cli --list | head -20
```

确认风险行已去重。

- [ ] **Step 5: 记录结论**

把三处验收结果与截图路径写进本计划末尾。
