# MacSlim Trusted Operation Broker 设计

**日期：** 2026-09-25  
**状态：** 待用户审阅  
**范围：** 缓存清理、进程终止、应用退出/强制退出、应用卸载、Docker 删除与 prune

## 1. 目标

建立一个位于 Rust 核心层的可信执行边界：

- 扫描结果由后端保存为短期 snapshot。
- 前端和 CLI 只能提交 snapshot 内的 opaque selection key，不能提交 raw path、PID、shell command、Docker 参数或卸载目标。
- 后端根据 snapshot 生成不可变的 operation plan，并返回随机 `operation_id`。
- 执行命令只接受 `operation_id`；后端在副作用前重新校验 snapshot、目标身份、路径、工具状态和 Docker 状态。
- operation 默认 10 分钟有效、单次使用；执行前原子消费，失败后不能重放，必须重新扫描生成计划。
- 进程、应用、缓存和 Docker 的历史记录继续写入现有 SQLite history，并在详情中记录 operation 类型和安全摘要。

## 2. 非目标

- 不把 operation ID 持久化到 SQLite；应用重启后所有 snapshot/operation 自动失效。
- 不允许开发者版通过 feature flag 绕过 Broker。
- 不把“用户点击确认”伪装成后端可信证明；后端保证的是目标不可由前端替换、计划不可重放、状态变化会拒绝执行。
- 不改变现有清理算法、进程安全规则、卸载授权流程和 Docker CLI 行为；本设计只改变输入与调度边界。
- 不把可回滚的白名单、历史记录、扫描和查询命令纳入 operation 流程。

## 3. 根因与当前旁路

当前 Tauri commands 直接接受并执行以下客户端数据：

- `clean_cache(items: Vec<CacheItem>)`：`CacheItem` 含 `path` 与 `command`。
- `kill_processes(pids, names)`：客户端提交 PID/name。
- `quit_application(name)`、`force_quit_application(name, pids)`。
- `uninstall_apps(targets)`、`quit_and_uninstall(app_name, target)`：客户端提交 bundle 与 residue 路径。
- `docker_remove_image/container/volume`、`docker_prune_all`：客户端提交 Docker ID/name。
- CLI 直接调用 `cache_cleaner_clean`、`graceful_kill` 等核心函数。

即使现有路径白名单和进程安全检查存在，客户端仍可替换目标、伪造类型或重放旧数据。Broker 将这些入口收敛为“扫描 → 选择 → 准备 → 执行”四步。

## 4. 核心组件

### 4.1 `OperationStore`

新增 `src-tauri/src/operations.rs`，由 `AppState` 持有 `Arc<Mutex<OperationStore>>`。Store 只负责短期状态和原子消费，不执行具体副作用。

内部状态：

- `SnapshotRegistration { snapshot_id, selection_keys }`：`register_snapshot` 为输入 payload 的每个条目生成独立随机 key，并按输入顺序返回 key 列表；扫描层用它把 key 与展示字段配对。
- `SnapshotPayload` 是后端内部 enum，各 variant 接收条目 Vec；Store 注册时转换为内部 key→entity map，不接受客户端提供的 key。
- `active_snapshots: HashMap<SnapshotKind, SnapshotEntry>`：每个领域保留最新 snapshot；新 scan 替换旧 snapshot 并使其 operation 失效。
- `operations: HashMap<OperationId, OperationEntry>`：不可变 plan、创建时间、过期时间、所属 window label、所属 snapshot ID 列表。
- `SnapshotId` 与 `OperationId`：使用直接依赖 `getrandom = "0.2"` 生成 32 字节随机值并以 hex 编码；ID 不从用户输入派生。
- TTL：snapshot 与 operation 默认 10 分钟；每次 prepare/execute/scan 清理过期条目；最多保留 128 个 snapshot、256 个 operation，避免异常增长。

`OperationStore` 对外只暴露：

- `register_snapshot(kind, snapshot_payload) -> SnapshotRegistration`
- `prepare_cache(snapshot_id, selection_keys, owner) -> PreparedOperation`
- `prepare_process(snapshot_id, selection_keys, mode, owner) -> PreparedOperation`
- `prepare_app_termination(snapshot_id, selection_keys, mode, owner) -> PreparedOperation`
- `prepare_uninstall(app_snapshot_id, residue_snapshot_id, app_keys, residue_keys, quit_running, owner) -> PreparedOperation`
- `prepare_docker(snapshot_id, action, selection_keys, owner) -> PreparedOperation`
- `consume(operation_id, owner) -> OperationPlan`

`consume` 在锁内完成查找、TTL 检查、owner 检查、snapshot 仍有效检查和删除；删除成功后才能返回 plan。并发第二次执行必然得到“操作已使用或已失效”。Tauri command 从 `WebviewWindow` 注入 window label，客户端请求体仍只有 `operation_id`；CLI 使用固定的 `cli` owner。

### 4.2 Snapshot 类型

Snapshot 是后端内部对象，不接受客户端反序列化。注册 snapshot 时为每个可执行条目生成独立随机 selection key（不复用 path、PID、bundle ID 或 Docker ID）；客户端只接收这些 key 和展示字段。每个领域包含 opaque key 到内部实体的映射：

- `CacheSnapshot`：key → 内部 `CacheItem`/typed cache action；保留 label、size、safety 供 UI 展示，但执行只使用 key。
- `ProcessSnapshot`：key → PID、name、exe、start_time、protected/whitelisted 状态和进程树根。
- `ApplicationSnapshot`：key → bundle path、bundle ID、name、扫描到的进程身份。
- `InstalledAppsSnapshot`：key → 安装应用扫描结果，identity 含 bundle path、bundle ID、name、bundle size、is_system。
- `ResidueSnapshot`：key → 应用残留扫描结果；key 不等于路径，identity 记录 app key；一次多应用扫描必须聚合并一次注册为同一 snapshot。
- `DockerSnapshot`：key → image/container/volume/build-cache inventory 记录；key 必须带资源类型作用域，image key 不能用于 container/volume；target identity 含 size/referenced 状态与 inventory fingerprint。

每个 key 只能在其所属 snapshot 与资源类型内使用；`ResidueSnapshot` 的 key 绑定 app key，Docker key 绑定资源类型，Cache/Process key 绑定各自 snapshot。跨 snapshot、跨类型或跨 app 的 key 一律拒绝。

缓存 `command` 字段改为后端 typed action 或内部字段，序列化给 UI 的 view model 不再包含可执行 command。客户端永远不能构造 `CacheItem` 执行对象。

扫描命令与 snapshot 的对应关系固定为：`scan_all` → OverviewProcessSnapshot，`list_all_processes` → ProcessSnapshot，`list_applications` → ApplicationSnapshot，`scan_cache` → CacheSnapshot，`docker_inventory` → DockerSnapshot，`scan_installed_apps` → InstalledAppsSnapshot，`scan_app_residues_batch` → ResidueSnapshot。OverviewProcess 与 Process 使用不同 active slot，允许智能扫描页与进程管理页同时保留各自 snapshot；两者都可被 `prepare_process` 消费。读取结果保留展示字段，但执行只使用 opaque key。

### 4.3 Operation plan

`OperationPlan` 是不可变 enum：

- `Cache { snapshot_id, items }`
- `Process { mode, targets }`
- `AppTerminate { mode, targets }`
- `Uninstall { targets, quit_running }`
- `Docker { action, targets }`

每个 target 只保存后端从 snapshot 解析出的实体和必要身份指纹。执行器不接受 `Vec<CacheItem>`、`Vec<u32>`、`UninstallTarget` 或任意 command string 作为 Tauri 输入。

`ConsumedPlan` 是只能由 `OperationStore::consume` 构造的 crate-private wrapper，保存已核验的 operation ID 与 `OperationPlan`；领域 executor 只接受 `ConsumedPlan`，不接受裸 `OperationPlan`。`process_ops`/`ProcessTarget`/`SystemProcessSignaller` 等可执行进程能力不得对 library 外 crate 公开。

### 4.4 准备与执行 API

保留一个统一的 Tauri 入口：

```text
prepare_operation(request) -> PreparedOperation
execute_operation(operation_id) -> OperationResult
```

`PrepareOperationRequest` 使用 serde tagged enum：

```text
Cache { snapshot_id, item_keys }
Process { snapshot_id, process_keys, mode }
AppTerminate { snapshot_id, app_keys, mode }
AppGracefulQuit { snapshot_id, app_keys }
Uninstall { app_snapshot_id, residue_snapshot_id, app_keys, residue_keys, quit_running }
Docker { snapshot_id, action, target_keys }
```

`PreparedOperation` 至少包含：`operation_id`、`kind`、`expires_at_ms`、项目数量、估算释放空间和用户可读摘要；卸载估算包含 bundle size，Docker 估算包含各 target size，summary 不得暴露内部英文枚举名。

`OperationResult` 使用 tagged enum 返回已有领域结果：缓存 `CleanSummary`、进程 `KillReport`、卸载报告、Docker 执行结果。错误统一为中文人话。

旧 command 不再注册到 `invoke_handler`；保留的读取类 command 不接收破坏性目标。CLI 通过同一 Rust Store 完成 scan → prepare → execute，不接受外部 raw target。

## 5. 各领域校验

### 5.1 缓存

- prepare：验证 snapshot 类型、item key 存在、选择非空；根据 snapshot 内部 typed action 计算摘要。
- execute：重新检查工具是否忙、路径是否存在、路径是否仍在允许清理根内、root ownership 是否变化；扫描时保存的 canonical path/身份指纹必须与执行时一致，祖先 symlink 或目标替换必须拒绝；不允许从客户端读取 command。
- 保留当前 dry-run、白名单、SIGTERM/重试和错误处理语义；失败的 item 进入报告，整个 operation 仍不可重放。

### 5.2 进程与应用终止

- prepare：只接受 snapshot key；受保护/白名单项目仍可显示。白名单目标始终拒绝；protected 目标仅允许显式 Force，Graceful 直接拒绝。扫描结果中 `default_select` 不得与 protected/whitelisted 同时为真。
- execute：刷新系统进程，验证 PID、name、exe、start_time 与 snapshot 一致；PID 被复用、进程消失、变成受保护目标或白名单状态变化时拒绝。
- graceful mode 对进程行继续 SIGTERM → 3 秒 → SIGKILL；应用级“优雅退出”使用独立 `AppGracefulQuit` operation：后端复核 app identity 后调用 AppleScript quit，不复用进程树 SIGTERM；force mode 继续现有强制终止语义。应用级 graceful 不因 protected gate 被禁用，force 仍遵守白名单/protected gate。
- `AppTerminate { mode: Graceful }` **一律在 prepare 阶段拒绝**并返回指向 `app_graceful_quit` 的中文错误：客户端不得把进程树 SIGTERM 伪装成优雅退出。`AppTerminate` 摘要必须写明「进程树终止，不保存数据」，`AppGracefulQuit` 摘要必须写明「应用会收到退出请求，可先保存」。
- `AppGracefulQuit` 的白名单边界与 `AppTerminate` 一致：白名单子进程直接拒绝，protected 子进程不阻止退出。

### 5.3 应用卸载

- prepare：从 `InstalledAppsSnapshot` 与对应 `ResidueSnapshot` 解析 bundle 和 residue；客户端只提交 app/residue key，residue key 必须属于对应 app key。
- execute：重新验证 bundle path、bundle ID、应用名、residue 根和选择集合；residue 必须落在该 bundle 的 Library/dev-tool 允许根内，拒绝 `..`、symlink 逃逸与越界路径；路径变化或 snapshot 过期即拒绝。系统核心应用与同名应用 quit 目标直接拒绝。
- 保留 NSFileManager、rename、管理员批量授权和逐项报告；授权取消不重放 operation。

### 5.4 Docker

- prepare：只接受 inventory snapshot key 与 typed action；prune 计划记录 daemon/inventory fingerprint、分类数量和可回收大小，禁止空泛的“0 个目标”摘要。
- execute：重新获取或检查 Docker inventory，验证 fingerprint、目标仍存在、ID/name 未变、引用状态未改变；任何新增/消失资源导致 fingerprint 不一致时拒绝 prune；daemon 未运行则中文跳过/失败。
- `remove_image/container/volume/prune` 只能由 Broker 调用现有 typed helper；Tauri/CLI 不再接收 Docker CLI 参数数组。

## 6. 前端流程

1. 扫描结果保存 `snapshot_id`；列表显示可用的 selection key，PID/path 仍可显示但不能作为执行输入。
2. 用户选择后，页面保留现有风险提示、>10GB 二次确认和受保护进程确认。
3. 最终确认时调用 `prepare_operation`；若返回过期或 snapshot 失效，提示“请重新扫描”。
4. 准备成功后所有领域必须显示后端 `summary`、`estimated_bytes`、`expires_at_ms`；>10GB 的任何破坏性领域都需明确不可逆确认，然后才调用 `execute_operation(operation_id)`。
5. 执行后清理 selection、刷新对应 snapshot，并按 tagged result 更新页面。
6. 过期、重复点击、窗口关闭、重新扫描和应用重启均使旧 operation 不可执行。

`src/lib/tauri.ts` 删除 raw `cleanCache`、`killProcesses`、`forceQuitApplication`、`uninstallApps`、`quitAndUninstall`、Docker remove/prune 的调用，改为 prepare/execute 类型化封装。各 View 的测试 mock 必须验证调用顺序与参数形状。

## 7. CLI 流程

CLI 仍保留现有用户命令语义，但内部流程改为：

1. 调用领域 scanner。
2. 注册 snapshot 并选择默认安全 key。
3. 通过共享 `OperationStore` prepare，owner 固定为 `cli`。
4. 立即 consume/execute，打印已有领域结果。
5. 不接受命令行传入 path、PID、cache item、卸载 target 或 Docker 参数。

CLI 与 Tauri 共享 Rust domain executor；不允许 CLI 继续直接调用 `cache_cleaner_clean(Vec<CacheItem>)` 或 `graceful_kill` 作为公开旁路。CLI 执行破坏性动作时 history store 打开失败或写入失败必须返回中文错误，不能静默继续并打印成功。

CLI 打开本地存储是**破坏性动作的前置条件**：`Storage::open()` 失败时 `run_default_cache_clean` / `run_default_process_clean` 必须在构造 cleaner / observer / signaller 之前返回中文错误。白名单来源不可读时按 fail-closed 处理（清理路径拒绝一切进程终止）；只有只读扫描路径允许退化为「仅内置白名单」。

## 8. 错误、历史与安全

错误分类统一为：未知 operation、已使用、已过期、snapshot 已更新、窗口不匹配、目标已变化、选择无效、工具忙、权限不足、Docker 不可用、执行失败。错误消息不包含 token、完整内部 plan 或凭据。

现有 history 记录保留；每条破坏性记录增加 operation kind、目标数量和安全摘要。完整 operation ID 不写入用户可见日志或 history；审计只记录不含 token 的 operation kind、目标数量和结果摘要。

history 写入失败不得静默丢弃：成功路径返回「操作已执行，但写入历史记录失败（kind · 目标）：<原因>，请到历史页核对」，复核拒绝路径返回「<复核错误>；操作未执行，但写入历史记录失败（kind · 目标）：<原因>」。两种文案都不包含 operation ID 或 plan 内容。

## 9. 测试与验收

### Rust

- Store：随机 ID、TTL、单次消费、并发消费、owner 绑定、snapshot 替换失效、数量上限。
- Cache：伪造 key/path/command、路径变化、工具忙、root ownership 变化。
- Process：PID 复用、name/exe/start_time 变化、白名单/保护状态变化、进程消失；进程域 public 类型不可被外部 crate 构造。
- 前端：每个领域必须消费 `PreparedOperation.summary/estimated_bytes/expires_at_ms`，并覆盖 >10GB 确认与 protected 默认选择不变量。
- Uninstall：bundle/residue key 交叉引用、路径变化、bundle ID 变化、snapshot 过期。
- Docker：目标消失、ID 复用、引用状态变化、daemon 不可用、prune typed action。
- 所有 executor 使用 mock/fake，不真实杀进程、删文件、卸载或调用 Docker。

### Frontend/静态

- Vitest：每个 View 的 scan → prepare → execute 流程、过期/重复执行错误、结果 tagged 解包。
- 静态测试：旧 raw commands 不得出现在 `invoke_handler` 或 `src/lib/tauri.ts`；前端请求类型不得出现 path/PID/command/UninstallTarget 字段。
- CLI 测试：默认安全项仍能完成 scan → prepare → execute，非法 target 参数不存在。

### 静态门禁

静态门禁由 `scripts/operation_contract.py`（组合入口）、`scripts/operation_surface_checks.py`
（IPC / 前端 / CLI 表面）与 `scripts/source_structure.py`（源码结构解析原语）组成，
由 `scripts/verify-security-config.py` 的 `main()` 调用。

所有判定必须是**结构化**的：先把注释与字符串字面量按等长空格屏蔽，再在屏蔽后的文本上
按括号配平与嵌套深度切分。注释或字符串里出现旧 command / raw 字段名既不算违规，也不算
通过。门禁覆盖：

- `invoke_handler` 恰好一个 `generate_handler!`，command 面精确等于 16 个可信 command，
  其中破坏性入口精确为 `prepare_operation` / `execute_operation`；`#[tauri::command]`
  声明集合与注册集合必须一致。
- `src/lib/tauri.ts` 的每个 `invoke()` 第一个参数必须是静态字符串字面量，封装集合与注册
  集合完全相等；`PrepareOperationRequest` 的 6 个变体字段集逐一钉死（只允许 opaque key、
  `mode`、`action`、`quit_running`）；`CacheItem` 不得有 `command` 字段；任何 View 与
  组件都不得出现 raw wrapper 调用或已删除的 raw 类型。
- `bin/cli.rs` 的 `parse_args` 只接受登记过的 flag，且不出现 `graceful_kill` /
  `cache_cleaner_clean` 等直接领域执行调用或迁移桥 import。
- `cli_operations.rs` 的破坏性路径必须先 `CliSession::open` 再构造领域执行器，且
  `Storage::open()` 不得被 `.ok()` / `.unwrap_or_default()` / `.expect()` 静默降级。
- `PrepareOperationRequest` 的 serde 容器必须 `tag = "type"` + `snake_case` +
  `deny_unknown_fields`；`OperationStore::consume` 必须有 owner 比较并返回错误。
- history 条目不得携带 `operation_id`，`operation` 必须来自 `kind.label()`。
- `prepare_app_termination` 必须在任何目标身份选择之前用 `GRACEFUL_QUIT_ROUTE` 拒绝
  `Graceful`；`AppGracefulQuit` 必须在 `execute_domain_plan` 里路由到 `domains.app_quit`
  并最终走 `uninstaller::quit_app`；应用级路径不得引用 `process_ops::graceful_kill`，
  且它必须保持受限可见性。
- 每个 `OperationKind::label()` 取值都必须在 `HistoryView.opLabel` 有返回 `t(...)` 的分支、
  在 `bin/cli.rs` 的 `history_operation_label` 有字面量分支（否则显示「未知操作」），
  且所用 i18n key 在 `en.ts` 与 `zh-CN.ts` 同时存在、两侧 key 集合一致。
- `getrandom` 必须是 `[dependencies]` 直接依赖且锁定 `0.2`。
- 解析层必须有显式的 unparseable 出口：未闭合的字符串字面量抛
  `SourceParseError`（`ValueError` 子类），每个门禁入口在开头做 parse gate 并**短路**，
  任何文件不可解析都只返回 `错误: <file> 结构解析失败…`，绝不把「解析失败」当成
  「该文件干净」。
- `package.json` 的 `verify` 必须同时跑前端（lint/typecheck/vitest/build）、Python
  （`unittest discover -s scripts/tests`）与 Rust（fmt / `--all-targets` test / clippy
  `-D warnings`）三套；`verify:frontend` 不得包含 Rust 套件。这三条脚本按
  **结构化命令链**校验（`shlex` 词法切分 + `&&` 切段 + 逐段形状），不用 substring 存在性：
  - `verify` / `verify:frontend`：每段必须精确等于 `bun run <script>`，引用的 script 必须
    真实存在；拒绝 `echo` / `printf` / `true` / `false` 这类只提及不执行的段、额外参数、
    非 bun 前缀，以及顶层分隔符 `;` / `|` / `&` / `||`（`&&` 是唯一允许的串联方式，
    否则前面的套件失败不会让整条命令返回非零）。
  - `test:python`：token 列表必须精确等于
    `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/tests -p test_*.py`。
  - `test:rust`：`&&` 连接的 cargo 段，子命令限 `fmt` / `test` / `clippy` 且三段齐全，
    每段按自身要求携带 `fmt: -- --check`、`test: --all-targets`、
    `clippy: --all-targets -D warnings`。

### 验收标准

- 任意 WebView/CLI 客户端无法直接调用 raw 破坏性 command。
- operation 不能重放、跨窗口使用、跨 snapshot 使用或跨 target 使用。
- 进程/路径/Docker 状态变化会被执行前校验拒绝。
- 原有 dry-run、mock、SIGTERM 优先、不可逆操作诚实文案和历史记录能力保持。
- `bun run verify`、Rust fmt/clippy/test、Vitest、Python security checks 全部通过；不执行真实破坏性动作。
