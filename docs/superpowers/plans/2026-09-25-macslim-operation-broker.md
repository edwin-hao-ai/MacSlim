# MacSlim Trusted Operation Broker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将缓存、进程/应用终止、应用卸载和 Docker 删除/prune 全部收敛到后端 snapshot/plan 与单次 `operation_id`，阻断客户端提交 raw target 的旁路。

**Architecture:** Rust `OperationStore` 保存短期 snapshot 和不可变 operation plan；Tauri/CLI 只提交 snapshot 内 opaque selection key，执行前由领域 executor 重新校验目标身份并原子消费 operation。前端改为 scan → prepare → execute，CLI 复用同一 Store；所有破坏性旧 command 从 IPC 移除。

**Tech Stack:** Rust 2021、Tauri 2、SolidJS/TypeScript、Vitest、Python unittest、现有 sysinfo/nix/bollard/Tokio/rusqlite。

**Spec:** `docs/superpowers/specs/2026-09-25-macslim-operation-broker-design.md`

## Global Constraints

- operation 与 snapshot TTL 固定 10 分钟；最多保留 128 个 snapshot、256 个 operation。
- `SnapshotId`、`OperationId` 与 selection key 使用 `getrandom = "0.2"` 生成 32 字节随机值的 hex 编码；不得从 path、PID、bundle ID 或 Docker ID 派生。
- `execute_operation` 的客户端请求体只能包含 `operation_id`；Tauri command 从 `WebviewWindow` 注入 owner label，CLI owner 固定为 `cli`。
- 新 scan 替换同领域 active snapshot；旧 snapshot 下的 operation 必须失效。
- operation 在副作用前原子消费；执行失败后不可重放，必须重新扫描。
- 前端/CLI 不得提交 raw path、PID、shell command、Docker 参数、`UninstallTarget` 或 cache command。
- 不执行真实进程终止、文件删除、卸载、Docker 命令、签名、公证或发布；测试使用 mock/fake。
- 不新增代码注释；用户可见错误使用中文；函数 <50 行，文件 <800 行。
- 不 commit/add/push；保留用户已有工作区改动。
- 读取类 command 保持可直连；白名单、历史和设置的现有行为保持。

## File Map

- Create `src-tauri/src/operations.rs`：ID、SnapshotPayload、OperationPlan、OperationStore、TTL/owner/selection 校验。
- Create `src-tauri/src/operation_executor.rs`：按已消费 plan 调用缓存、进程、卸载、Docker 领域实现。
- Modify `src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`：加入直接 `getrandom = "0.2"`。
- Modify `src-tauri/src/cache_scanner.rs`、`cache_cleaner.rs`：typed cache action、内部 item、snapshot view。
- Modify `src-tauri/src/scanner.rs`、`process_ops.rs`、`process_safety.rs`：process identity/start_time/key。
- Modify `src-tauri/src/applications.rs`、`app_scanner.rs`、`residue_scanner.rs`：app/residue opaque key 与 identity。
- Create `src-tauri/src/residue_policy.rs`：残留路径允许根与路径漂移校验。
- Modify `src-tauri/src/uninstaller.rs`、`docker.rs`：只供 operation executor 调用的 typed target 与状态复核。
- Modify `src-tauri/src/lib.rs`：AppState、snapshot command wrappers、prepare/execute command、移除 raw destructive handler。
- Modify `src/lib/tauri.ts`、相关 Solid views/tests：scan → prepare → execute。
- Create `src-tauri/src/cli_operations.rs`：CLI 窄 public Broker 入口，内部使用 owner=`cli`。
- Modify `src-tauri/src/bin/cli.rs`：CLI 改用 cli_operations，移除 raw executor 旁路。
- Modify `src/i18n/en.ts`、`src/i18n/zh-CN.ts`：过期、snapshot 变化、执行失败等用户提示。
- Modify `scripts/verify-security-config.py` 与 Python tests：禁止 raw IPC/CLI 入口。
- Create/modify Rust unit/integration tests 与 frontend Vitest tests：Broker、重放、TTL、identity revalidation、UI 流程。

---

### Task 1: 实现 OperationStore、ID 与不可变 plan

**Files:**
- Create: `src-tauri/src/operations.rs`
- Create: `src-tauri/src/operations_tests.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces the following concrete internal types in `operations.rs`:

```rust
pub enum CacheAction { Npm, Pnpm, Yarn, Docker, Homebrew, Xcode, Cocoapods, Cargo, Pip, Go, System, StaleNodeModules }
pub struct ProcessIdentity { pub pid: u32, pub name: String, pub exe: String, pub start_time: u64 }
pub struct AppIdentity { pub bundle_path: String, pub bundle_id: String, pub app_name: String, pub processes: Vec<ProcessIdentity> }
pub struct InstalledAppIdentity { pub bundle_path: String, pub app_name: String, pub bundle_id: String }
pub struct ResidueIdentity { pub app_key: String, pub path: String, pub category: String, pub size_bytes: u64 }
pub struct DockerTarget { pub resource_type: String, pub id: String, pub name: String }
pub struct SnapshotRegistration { pub snapshot_id: String, pub selection_keys: Vec<String> }
pub struct ConsumedPlan { pub(crate) operation_id: String, pub(crate) plan: OperationPlan }
pub struct PreparedOperation { pub operation_id: String, pub kind: String, pub expires_at_ms: u64, pub item_count: usize, pub estimated_bytes: u64, pub summary: String }
```

- `SnapshotPayload` has `Cache(Vec<CacheItem>)`, `Process(Vec<ProcessIdentity>)`, `Application(Vec<AppIdentity>)`, `InstalledApps(Vec<InstalledAppIdentity>)`, `Residue(Vec<ResidueIdentity>)`, and `Docker(Vec<DockerTarget>)` variants.
- `register_snapshot` returns `SnapshotRegistration`; it generates one random selection key per input item and returns keys in input order.
- `OperationPlan` has `Cache`, `Process`, `AppTerminate`, `Uninstall`, and `Docker` variants; it is not serializable and contains only the typed backend targets resolved from a snapshot.
- `OperationStore` exposes the methods listed below.
- `OperationStore::register_snapshot(kind, payload) -> Result<SnapshotRegistration, String>`。
- `OperationStore::prepare_cache(snapshot_id, keys, owner) -> Result<PreparedOperation, String>`。
- `OperationStore::prepare_process(snapshot_id, keys, mode, owner) -> Result<PreparedOperation, String>`。
- `OperationStore::prepare_app_termination(snapshot_id, keys, mode, owner) -> Result<PreparedOperation, String>`。
- `OperationStore::prepare_uninstall(app_snapshot_id, residue_snapshot_id, app_keys, residue_keys, quit_running, owner) -> Result<PreparedOperation, String>`。
- `OperationStore::prepare_docker(snapshot_id, action, keys, owner) -> Result<PreparedOperation, String>`。
- `OperationStore::consume(operation_id, owner) -> Result<ConsumedPlan, String>`。

- [ ] **Step 1: 写 Store 失败测试**

在 `operations.rs` 通过 `#[cfg(test)] #[path = "operations_tests.rs"] mod tests;` 引入独立测试子模块；测试先定义以下行为：

```rust
#[test]
fn operation_is_single_use_and_owner_bound() {
    let mut store = OperationStore::new();
    let registration = store.register_snapshot(SnapshotKind::Cache, cache_payload("item")).unwrap();
    let key = registration.selection_keys[0].clone();
    let prepared = store.prepare_cache(&registration.snapshot_id, vec![key], "main").unwrap();
    assert!(store.consume(&prepared.operation_id, "other").is_err());
    assert!(store.consume(&prepared.operation_id, "main").is_ok());
    assert!(store.consume(&prepared.operation_id, "main").is_err());
}

#[test]
fn new_snapshot_invalidates_old_operation() {
    let mut store = OperationStore::new();
    let old = store.register_snapshot(SnapshotKind::Cache, cache_payload("item")).unwrap();
    let key = old.selection_keys[0].clone();
    let operation = store.prepare_cache(&old.snapshot_id, vec![key], "main").unwrap();
    store.register_snapshot(SnapshotKind::Cache, cache_payload("new-item")).unwrap();
    assert!(store.consume(&operation.operation_id, "main").is_err());
}

#[test]
fn unknown_selection_key_is_rejected() {
    let mut store = OperationStore::new();
    let registration = store.register_snapshot(SnapshotKind::Process, process_payload("p1")).unwrap();
    assert!(store.prepare_process(&registration.snapshot_id, vec!["forged"], ProcessMode::Graceful, "main").is_err());
}
```

在同一测试模块定义两个明确的内存 helper：`fn cache_payload(label: &str) -> SnapshotPayload` 用 `CacheItem { id: label.to_owned(), category: CacheCategory::System, label: label.to_owned(), description: String::new(), path: None, size_bytes: 1, safety: Safety::Safe, default_select: true, command: None, recover_hint: String::new() }` 构造单元素 Vec；`fn process_payload(label: &str) -> SnapshotPayload` 用 `ProcessIdentity { pid: 1, name: label.to_owned(), exe: String::new(), start_time: 1 }` 构造单元素 Vec。Task 2 修改 CacheItem 字段时同步更新这个 fixture。再覆盖 TTL、并发 consume、最大条目数、residue key 跨 app、Docker key 跨类型、空选择和随机 ID 长度。所有测试只使用内存 fake payload。

- [ ] **Step 2: 运行 RED**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml operations --lib
```

Expected: FAIL，因为 `operations` module、Store 和 domain identity 类型尚不存在。

- [ ] **Step 3: 加入直接依赖与类型定义**

在 `[dependencies]` 加：

```toml
getrandom = "0.2"
```

`operations.rs` 定义固定 10 分钟 TTL、128 snapshot 上限、256 operation 上限；ID 生成失败返回中文错误。`SnapshotPayload` 为后端内部 enum，至少包含 `Cache`、`Process`、`Application`、`InstalledApps`、`Residue`、`Docker` 六个变体。`OperationPlan` 为不可变 enum，内部目标使用后端 identity struct，不实现 `Deserialize`。

- [ ] **Step 4: 实现注册、准备与消费**

`register_snapshot` 为每个条目生成独立 selection key，保存 active snapshot；同领域新 snapshot 替换旧 snapshot 并清理旧 operation。`consume` 在锁内检查 ID、TTL、owner、所属 snapshot 是否 active，然后删除记录并返回 plan；任何失败都不能返回部分 plan。

- [ ] **Step 5: 接入 AppState 但不注册破坏性 command**

在 `AppState` 增加：

```rust
pub operations: Arc<Mutex<OperationStore>>
```

`run()` 初始化 `OperationStore::new()`；本 Task 暂不移除旧 command，先让 Store 单元测试通过。

- [ ] **Step 6: 运行 GREEN 与格式检查**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml operations --lib
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Expected: Store 测试、fmt、clippy 通过；不执行真实副作用。

---

### Task 2: 迁移缓存为 typed action 与可信 snapshot

**Files:**
- Modify: `src-tauri/src/cache_scanner.rs`
- Modify: `src-tauri/src/cache_cleaner.rs`
- Modify: `src-tauri/src/operations.rs`
- Create: `src-tauri/src/operation_executor.rs`
- Create: `src-tauri/src/cache_cleaner_tests.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/cache_scanner.rs` tests and `src-tauri/src/cache_cleaner_tests.rs`

**Interfaces:**
- `CacheAction` 为固定 enum，覆盖 npm/pnpm/yarn/docker/homebrew/Xcode/CocoaPods/Cargo/Pip/Go/System/StaleNodeModules。
- `CacheItem` 的 action/path 只供后端 snapshot；序列化 view 不包含 command。
- `OperationStore::register_cache_snapshot(items) -> Result<SnapshotRegistration, String>`。
- `operation_executor::execute_cache_plan(consumed: ConsumedPlan) -> Result<CleanSummary, String>` 仅接受 Store consume 返回的 wrapper。

- [ ] **Step 1: 写缓存 RED 测试**

覆盖：伪造 selection key、伪造 command、路径不在允许根、工具忙、root ownership 改变、typed action dispatch、空选择、snapshot 过期。测试断言 `CacheItem` 的 serialized JSON 不含 `command`，且客户端传入 command 不能影响执行。

- [ ] **Step 2: 运行 RED**

```bash
cargo test --manifest-path src-tauri/Cargo.toml cache --lib
```

Expected: 新 typed action/serialization 测试失败。

- [ ] **Step 3: 将 CacheItem command 改为内部 typed action**

保留现有 category、label、size、safety 与展示路径；删除可序列化 command 字段，所有 scanner constructor 改为 `CacheAction` 变体。`cache_cleaner` 只对 typed action dispatch，路径删除仍经过 `is_cleanup_path_allowed`、busy check 和 root ownership check。

- [ ] **Step 4: 实现缓存 snapshot/prepare/execute**

`scan_cache` 注册 `SnapshotKind::Cache`；每个 item 获得独立 key。prepare 只按 key 从 snapshot 取内部 item。执行前重新 canonicalize、检查 busy/ownership，再调用现有清理实现；`CleanSummary` 记录失败 item，operation 仍已消费。

- [ ] **Step 5: 收紧 raw helper 可见性**

任意 command 执行函数改为 `pub(crate)`；`cache_cleaner::clean` 与 `lib.rs` 的 `cache_cleaner_clean` 暂时保留为 CLI 迁移桥，直到 Task 6 由 CLI 改为 Store 后删除。该桥不出现在 Tauri invoke handler，Task 7 静态门禁会拒绝它残留。

- [ ] **Step 6: 运行 GREEN**

```bash
cargo test --manifest-path src-tauri/Cargo.toml cache --lib
cargo test --manifest-path src-tauri/Cargo.toml operations --lib
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
```

---

### Task 3: 迁移进程与应用终止 identity

**Files:**
- Modify: `src-tauri/src/scanner.rs`
- Modify: `src-tauri/src/process_ops.rs`
- Modify: `src-tauri/src/process_safety.rs`
- Modify: `src-tauri/src/applications.rs`
- Modify: `src-tauri/src/operations.rs`
- Modify: `src-tauri/src/operation_executor.rs`
- Test: process/applications unit tests and new executor tests

**Interfaces:**
- `ProcessIdentity { pid, name, exe, start_time }`。
- `ProcessMode::{Graceful, Force}`；whitelisted target 始终拒绝，protected target 只允许 Force。
- `operation_executor::execute_process_plan(consumed, observer, signaller)` 与 `execute_app_termination_plan(consumed, observer, signaller)` 只接受已消费 plan 和显式注入的 observer/signaller。
- `scan_all`、`list_all_processes`、`list_applications` 返回带 selection key 的 snapshot view。

- [ ] **Step 1: 写进程 RED 测试**

覆盖 PID 复用、name/exe/start_time 变化、进程消失、白名单/保护状态变化、应用 child key 跨应用、force/graceful mode 分派、空选择。所有 signal 调用使用 mock recorder，不调用真实 `kill`。

- [ ] **Step 2: 运行 RED**

```bash
cargo test --manifest-path src-tauri/Cargo.toml process --lib
```

Expected: identity/revalidation 测试失败。

- [ ] **Step 3: 添加 identity 与 opaque key**

`ProcessInfo`、`ProcessRow`、`AppInfo`、`AppChildProcess` 增加 `selection_key` 与后端 start_time；UI 可以显示 PID/name，但 operation request 只接受 key。`ProcessIdentity` 从同一 sysinfo sample 生成，不能使用前端回传 name/exe。

- [ ] **Step 4: 实现执行前复核与 executor**

刷新系统后逐目标比较 PID/name/exe/start_time、protected、whitelisted；任何不一致返回中文错误。graceful 保持 SIGTERM → 3 秒 → SIGKILL，force 走既有强制终止函数；应用级目标由后端 snapshot 的 app key 展开 PID 集合。

- [ ] **Step 5: 移除 raw Tauri 依赖**

`kill_processes`、force/graceful raw helper 改为 `pub(crate)`；本 Task 可暂留旧函数供编译迁移，但 Task 5 前不得出现在 invoke handler。

- [ ] **Step 6: 运行 GREEN**

```bash
cargo test --manifest-path src-tauri/Cargo.toml process --lib
cargo test --manifest-path src-tauri/Cargo.toml operations --lib
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

---

### Task 4: 迁移卸载与 Docker typed plan

**Files:**
- Modify: `src-tauri/src/app_scanner.rs`
- Modify: `src-tauri/src/residue_scanner.rs`
- Modify: `src-tauri/src/uninstaller.rs`
- Modify: `src-tauri/src/docker.rs`
- Modify: `src-tauri/src/operations.rs`
- Create: `src-tauri/src/residue_policy.rs`
- Modify: `src-tauri/src/operation_executor.rs`
- Test: uninstaller/docker unit tests and executor tests

**Interfaces:**
- `InstalledAppIdentity` 增加 `bundle_size_bytes` 与 `is_system`；`ResidueIdentity` 的 app key 必须绑定同一 InstalledApps snapshot；`DockerTarget` 增加 `size_bytes`、`referenced` 与 `reclaimable`，并由 Store 生成 inventory fingerprint。
- `OperationStore::register_residues_for_apps(Vec<(app_key, Vec<ResidueIdentity>)>)` 一次聚合并注册多个 app 的残留，避免同类单槽覆盖；**空 batch 被忽略，全部为空时注册空 snapshot**，使「所选应用没有残留」仍可 `prepare_uninstall`。`residue_scanner::register_app_residue_batch` 是 Task 5 的正式接线入口（单应用 `register_app_residues` 只是它的薄封装）。
- `residue_policy::{allowed_roots, ensure_within_roots, resolve_canonical}`：根 = `~/Library` 的 9 个固定子目录（container 根，严格在其下）+ 当前 bundle 的 `dev_tool_rules` 声明路径（exact 根，允许等于根本身）；**每个根与候选走同一套 canonical 解析**，解析失败的根跳过。
- `DockerAction::{RemoveImage, RemoveContainer, RemoveVolume, Prune}`；prune plan 保存 fingerprint/分类数量/可回收大小摘要，`item_count` 与中文摘要**只统计 `reclaimable` 资源**。
- `operation_executor::execute_uninstall_plan` 与 `execute_docker_plan` 只接受已消费 plan。

- [ ] **Step 1: 写卸载/Docker RED 测试**

覆盖 app/residue key 跨 snapshot、跨 app、bundle ID/path 变化、Docker key 跨类型、目标消失/ID 复用/引用状态变化、daemon 不可用、prune typed action。测试只检查 typed helper 调用和报告，不执行 `mv`、osascript 或 Docker CLI。

- [ ] **Step 2: 运行 RED**

```bash
cargo test --manifest-path src-tauri/Cargo.toml uninstaller --lib
cargo test --manifest-path src-tauri/Cargo.toml docker --lib
```

Expected: identity/typed action 测试失败。

- [ ] **Step 3: 添加 installed/residue/Docker opaque key**

扫描结果为每个 app、residue、Docker resource 生成 selection key；序列化结果可保留 path/name 展示，但 executor target 只从 snapshot 读取。Residue key 必须记录 app key；Docker key 必须记录 resource type。

- [ ] **Step 4: 实现卸载复核与执行**

prepare 使用两个 active snapshot ID；执行重新验证 bundle path、bundle ID、app name、residue 允许根和选择集合。allowed roots 由 `~/Library` 的固定子目录与当前 bundle 的 dev-tool 规则组成；拒绝 `..`、symlink 逃逸、Library 外路径。系统核心应用和同名 quit 目标拒绝。保留现有管理员授权与逐项报告，但 `uninstall_app`/`quit_and_uninstall` 改为 `pub(crate)`，只由 executor 调用。多应用残留必须经 `register_residues_for_apps` 一次注册。

- [ ] **Step 5: 实现 Docker 复核与执行**

执行前重新获取 inventory/目标状态；daemon 未运行、ID/name 变化、引用状态变化或 inventory fingerprint 不一致时拒绝。prune 记录并展示分类数量与可回收大小，不能显示 0 个目标。`run_docker` 与 remove/prune helper 改为 `pub(crate)`，不接受客户端 args。

- [ ] **Step 6: 运行 GREEN**

```bash
cargo test --manifest-path src-tauri/Cargo.toml uninstaller --lib
cargo test --manifest-path src-tauri/Cargo.toml docker --lib
cargo test --manifest-path src-tauri/Cargo.toml operations --lib
```

补充验证：`quit_app` 的 AppleScript 字符串转义 `\\` 与 `"`；同名应用 quit 目标拒绝；系统核心应用 key 不能 prepare uninstall；bundle/Docker 估算空间与中文 summary；fake domain 调用计数在类型不匹配时确实为零。

---

### Task 5: 接入 Tauri snapshot/prepare/execute API

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Create: `src-tauri/src/operation_commands.rs`（command 边界；原计划的 `operation_executor.rs` 承载 dispatch）
- Modify: `src-tauri/src/operations.rs`、`src-tauri/src/operation_types.rs`、`src-tauri/src/operation_registry.rs`
- Modify: `src-tauri/src/operation_executor.rs`
- Modify（删 raw helper）: `src-tauri/src/applications.rs`、`src-tauri/src/uninstaller.rs`
- Test: `src-tauri/src/operation_commands_tests.rs` + `operation_commands_{scan,prepare,execute,contract}_tests.rs`、`src-tauri/src/operation_executor_dispatch_tests.rs`

**Interfaces:**
- `SnapshotResult<T> { snapshot_id, expires_at_ms, value }`；`expires_at_ms` 来自
  `OperationStore::snapshot_expiry_ms(kind)`。
- `SnapshotKind` 包含 `OverviewProcess` 与 `Process` 两个独立 active slot；`scan_all` 使用前者、`list_all_processes` 使用后者，`prepare_process` 接受两者。
- 缓存视图用 `Keyed<T> { selection_key, #[serde(flatten)] item }`（`CacheItem` 不加字段，
  避免继续触碰 1363 行的 `cache_scanner.rs`）；其余领域沿用各自结构体上的 `selection_key`。
- `scan_app_residues_batch(app_snapshot_id, app_keys)`：短锁解析 identity → `spawn_blocking` 逐个扫描
  → 短锁 **一次** `register_app_residue_batch`，返回同一个 residue `snapshot_id` + 每组 `app_key`。
- `PrepareOperationRequest` tagged enum（`tag = "type"` + `deny_unknown_fields`）：`Cache`、`Process`、
  `AppTerminate`、`Uninstall`、`Docker`；只含 snapshot ID、opaque key、typed mode/action/`quit_running`。
- `OperationResult` tagged enum（`tag = "kind"`）：`CleanSummary` / `ProcessKillReport`（进程与强制应用终止）/
  `AppGracefulQuitReport`（AppleScript 优雅退出）/`Vec<UninstallReport>` / `DockerExecutionReport`。
- `OperationSource` trait：`&Mutex<OperationStore>` 在锁内完成一次 `consume` 即释放，
  使 `execute_operation_with` 既能被生产 command 直接使用，又能在测试中用 fake domain 走同一条路径。
- 进程白名单 policy 由 `process_whitelist_policy(Arc<Storage>)` 生成，scan 回填与 execute 复核共用同一闭包。
- Tauri commands：`prepare_operation(request)`, `execute_operation(operation_id)`。
  终止类 plan 走 `spawn_blocking`，缓存/卸载/Docker 走 async 路径。
- 历史：`OperationHistoryEntry { operation, target, freed_bytes, success, detail }`，只含 kind/数量/中文摘要，
  **不含 operation ID**；复核拒绝额外写一条 `success=false` 审计。
- 历史写入失败（成功路径与复核拒绝路径 alike）**必须显式返回中文错误**，文案区分「操作已执行」与
  「操作未执行」并提示到历史页核对；`HistorySink::record` 的 `Result` 不得被 `let _ =` 丢弃
  （operation 已消费后不得假装成功）。

- [x] **Step 1: 写 command 边界 RED 测试**

覆盖每个 scan command 返回 snapshot ID；旧 raw command 不在 handler；execute 缺 operation_id/错误 owner/重复消费/过期/snapshot 替换均失败；prepare 不执行副作用。

- [x] **Step 2: 运行 RED**

```bash
cargo test --manifest-path src-tauri/Cargo.toml tauri_commands --lib
```

Expected: 新 commands 不存在或仍能调用 raw target。

- [x] **Step 3: 添加 snapshot wrapper 与统一 commands**

`scan_all` → OverviewProcessSnapshot，`list_all_processes` → ProcessSnapshot；两者使用不同 active slot，均可被 prepare_process 消费。`list_applications`/`scan_cache`/`docker_inventory`/`scan_installed_apps` 在返回前注册对应 snapshot；`scan_app_residues_batch` 接收多个 app key，逐个扫描后经 `register_app_residue_batch` **一次**注册 Residue snapshot 并返回各应用的 key 分组。`prepare_operation` 注入 `WebviewWindow::label()`；`execute_operation` 注入同一 label，先 consume，再调用 executor。

- [x] **Step 4: 移除 raw destructive handler**

从 `invoke_handler` 移除 `kill_processes`、`clean_cache`、`quit_application`、`force_quit_application`、`uninstall_apps`、`quit_and_uninstall`、Docker remove/prune commands；读取类 command 保持。现有 `default.json` capability 不扩大，security validator 继续验证精确 allowlist。

- [x] **Step 5: 运行 GREEN 与 Rust 全测**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
```

---

### Task 6: 迁移 SolidJS 前端与 CLI

**Files:**
- Modify: `src/lib/tauri.ts`
- Modify: `src/views/CacheView.tsx`
- Modify: `src/views/ProcessView.tsx`
- Modify: `src/views/ApplicationsView.tsx`
- Modify: `src/views/UninstallerView.tsx`
- Modify: `src/components/DockerSection.tsx`
- Modify: `src/i18n/en.ts`, `src/i18n/zh-CN.ts`
- Create: `src-tauri/src/cli_operations.rs`
- Modify: `src-tauri/src/bin/cli.rs`
- Modify（Task 6 fix wave 必需）: `src-tauri/src/operations.rs`、`operation_types.rs`、`operation_registry.rs`、`operation_executor.rs`、`uninstaller.rs`、`applications.rs`
- Test: corresponding `*.test.ts`, `*.test.tsx`, CLI tests

**Interfaces:**
- `scan*` 返回带 `snapshot_id` 与 opaque selection keys 的 view model；缓存使用 `Keyed<CacheItem>` flatten 形状。
- `prepareOperation(request)` 与 `executeOperation(operationId)` 是唯一破坏性 Tauri API。
- `CacheItem`/process/app/residue/Docker request 类型不包含 path/PID/command/UninstallTarget 字段。
- 进程、应用、Docker 页面在打开确认/已 prepare 期间暂停轮询，避免同页 snapshot 自我作废；执行后刷新 snapshot。
- CLI 不直接调用 `graceful_kill`/`cache_cleaner_clean`/Docker helper；通过 `macslim_lib::cli_operations` 的窄 public 入口在内部使用 owner=`cli` 的 Store 与 typed executor，不接受 raw target 参数。

- [x] **Step 1: 写前端/CLI RED 测试**

覆盖每个 View 的 scan → prepare → execute 顺序、operation 过期错误、重复点击错误、tagged result 解包、刷新 snapshot；断言 invoke payload 不含 raw path/PID/command。CLI 测试断言默认安全项走 Store，非法 target 参数不存在。

- [x] **Step 2: 运行 RED**

```bash
bun run test -- src/lib/tauri.test.ts src/views/CacheView.test.tsx src/views/ProcessView.test.tsx src/views/ApplicationsView.test.tsx src/views/UninstallerView.test.tsx src/components/DockerSection.test.tsx
cargo test --manifest-path src-tauri/Cargo.toml --bin macslim-cli
```

Expected: 新 API/流程测试失败。

- [x] **Step 3: 迁移 TypeScript API 与视图**

将 selection state 改为 opaque keys；最终确认时先 prepare，再 execute。删除 `CacheItem.command` 字段与任何死分支/测试 fixture。过期、snapshot 更新、执行失败显示中文 i18n 文案；执行后刷新对应 scan。ApplicationsView 的 graceful 按钮必须调用独立 `app_graceful_quit` operation（AppleScript），不能把 Process SIGTERM 树杀标成优雅退出；force 仍走 protected gate。

- [x] **Step 4: 迁移 CLI**

`run_cache_scan` 与 `run_process` 注册 snapshot、选择默认 key、prepare/consume/execute；不再调用 raw `cache_cleaner_clean` 或 `graceful_kill`。CLI 仍不新增破坏性命令行参数。破坏性 CLI 动作打开 SQLite history 失败必须返回中文错误，不得用 `Option<Storage>` 静默放行。

- [x] **Step 5: 运行 GREEN**

```bash
bun run typecheck
bun run test
bun run verify:frontend
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
```

### Task 6 fix round 1（review I-1 / I-2 / I-3 关闭）

**Files:** `src/lib/tauri.ts`、`src/views/CacheView.tsx`、`src/views/ApplicationsView.tsx`、
`src/i18n/{en,zh-CN}.ts`、`src-tauri/src/{operation_types,operations,operations_prepare,operation_registry,operation_executor,operation_commands,applications,uninstaller,cli_operations,lib}.rs`、
`src-tauri/src/bin/cli.rs`、`src-tauri/src/{operations_app_quit_tests,operation_executor_app_quit_tests,cli_operations_tests}.rs`、
以及既有测试文件的契约更新（`operations_process_tests` / `operation_executor_process_tests` /
`operation_executor_dispatch_tests` / `operation_commands_{tests,fakes,execute_tests,prepare_tests,scan_tests}.rs`）。

**Interfaces（fix round 1 追加）**
- `OperationKind::AppGracefulQuit` / label `app_graceful_quit`；`OperationPlan::AppGracefulQuit { targets }`；
  `OperationResult::AppGracefulQuit(Vec<AppGracefulQuitReport>)`。
- `PrepareOperationRequest::AppGracefulQuit { snapshot_id, app_keys }`（`deny_unknown_fields` 生效，
  无 `mode` 字段）。
- `OperationStore::prepare_app_graceful_quit`：只接受 whitelist-gate，**允许 protected**（AppleScript
  退出不杀进程），拒绝无进程目标与白名单子进程。
- `prepare_app_termination(mode = Graceful)` **一律拒绝**并指向 `app_graceful_quit`，杜绝客户端把
  进程树 SIGTERM 标成优雅退出。
- `AppQuitter` domain trait（`observe_apps` + `quit`）、`applications::SystemAppQuitter`
  （复用 `uninstaller::read_installed_identity` 复核 + `uninstaller::quit_app` AppleScript）、
  `run_app_graceful_quit`（逐应用 `revalidate_installed_app`）。
- `DomainServices` 增加第 4 个 domain 槽 `app_quit`。
- `cli_operations::CliSession::open(Result<Storage, String>)`：`Storage::open()` 失败即返回中文错误；
  `clean_cache_in_session` / `clean_processes_in_session` 在打开 session 失败时不构造 cleaner /
  observer / signaller；`CliHistory` 无 storage 时 `record` 返回错误；`CliWhitelist` 在
  `UnreadableStorage::Deny` 下拒绝一切进程（清理路径），`UnreadableStorage::Degrade` 下退化为内置
  白名单（仅扫描路径）。

**Step 1-2 RED**：`bunx tsc --noEmit` 报 `Type 'true' is not assignable to type 'never'`（`CacheItem.command`）
与 `Property 'command' is missing ... but required in type 'CacheItem'`；Vitest
`tauri.test.ts` 3 failed / `ApplicationsView.test.tsx` 7 failed / `CacheView.test.tsx` 1 failed；
`cargo test --lib app_quit` 12 errors（`AppQuitter` / `AppGracefulQuitReport` / `DomainServices` 4 泛型
/ `run_default_*_with` 不存在）。

**Step 3 GREEN**：见 `task-6-report.md` Fix round 1。

---

### Task 7: 静态安全门禁、集成测试与完整验证

**Files:**
- Modify: `scripts/verify-security-config.py`
- Create: `scripts/source_structure.py`、`scripts/operation_surface_checks.py`、`scripts/operation_contract.py`
- Modify: `scripts/tests/test_verify_security_config.py`
- Create: `scripts/tests/test_operation_contract.py`
- Create: `src-tauri/src/operation_commands_integration_tests.rs`、`src/views/HistoryView.test.tsx`
- Modify: `src/views/HistoryView.tsx`、`src/i18n/en.ts`、`src/i18n/zh-CN.ts`、`src-tauri/src/bin/cli.rs`
- Test: all Rust/TypeScript/Python suites

**Interfaces:**
- Security validator must reject raw destructive IPC names and raw request fields.
- Validator must confirm `prepare_operation`/`execute_operation` are registered and `getrandom` is direct dependency.
- Validator must confirm CLI has no direct `graceful_kill`/`cache_cleaner_clean` target call.
- `source_structure.mask_code` 屏蔽注释与字符串（等长），所有判定在屏蔽后的文本上按括号配平与嵌套深度切分，禁止 substring 判定。
- `operation_contract.validate_operation_contract_files(root)` 是唯一入口，返回中文错误列表；由 `verify-security-config.py` 的 `main()` 调用。

- [x] **Step 1: 写静态契约 RED 测试**

测试扫描 `src-tauri/src/lib.rs` invoke handler、`src/lib/tauri.ts`、Views、CLI：旧 command/function 名称、raw request 字段或 direct executor 调用均失败；当前有效源码通过。

- [x] **Step 2: 运行 RED**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.tests.test_operation_contract -v
```

Expected: 当前旧入口导致失败。

- [x] **Step 3: 实现 validator 与集成测试**

校验 operation commands、capability、getrandom、无 raw IPC/CLI 旁路；不要用 substring 替代结构检查。集成测试覆盖 store → executor → history 的边界，失败路径不调用真实领域 executor。

**Step 3 GREEN**：见 `task-7-report.md`。

**Fix round 1（review CHANGES REQUESTED）**：C-1（解析层静默吞并未闭合字符串）
与 I-1（verify 链只做 token 存在性）已关闭，见 `task-7-report.md` 的 Fix round 1 段。

- [x] **Step 4: 运行完整验证**

```bash
bun run verify
bun run verify:frontend
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/tests -p "test_*.py" -v
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
git diff --check
```

Expected: 所有本地检查通过；没有真实破坏性动作、签名、公证、上传或 Git 写操作。

- [ ] **Step 5: 手工验收**

在 macOS 原生窗口中分别验证：缓存/进程/应用/Docker/卸载的 scan → prepare → execute 流程；重复点击、过期、重新扫描、窗口切换和权限取消均显示中文错误且不重放。记录任何 native 差异到最终报告。

**Step 5 状态：未执行。** 本轮不启动 GUI（沙箱内无法拉起真实 Tauri 窗口），因此这是唯一未完成的验收项。`task-7-report.md` 的 Concerns 记录了需要人工在原生窗口确认的清单，以及每条对应的自动化覆盖。

## Final Fix Wave（whole-plan review 后）

- [ ] 进程域类型边界：`process_ops`/`SystemProcessSignaller`/`ProcessTarget` 收为 crate-private，静态门禁验证外部 crate 无法构造/调用。
- [ ] CLI session 顺序门禁改为 `mask_code` 后的结构化索引，注释/字符串不能伪造顺序。
- [ ] `default_select` 与 protected/whitelisted 不重叠；主扫描列表显示保护状态并禁止不安全默认选择。
- [ ] 所有领域确认 UI 消费 `PreparedOperation.summary`、`estimated_bytes`、`expires_at_ms`；>10GB 的 cache/Docker/uninstall/process 都有不可逆确认。
- [ ] Cargo test 门禁拒绝 `--no-run`、`--list`、`-q/--quiet`；补结构化 mutation tests。
- [ ] 补 `ConsumedPlan.operation_id` provenance、清理新增 dead_code 过渡标记，并更新 ledger 记账/评审路径。
- [ ] 手工 GUI 验收仍需用户在 macOS 原生窗口执行，未由自动化替代。

