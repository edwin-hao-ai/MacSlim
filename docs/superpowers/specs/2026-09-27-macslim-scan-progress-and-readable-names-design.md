# MacSlim 扫描过程可见化 + 列表名称可读性 设计文档

日期：2026-09-27
状态：待评审
关联：Operation Broker（`2026-09-25-macslim-operation-broker-design.md`）、P0 窗口安全基线

---

## 1. 背景与问题

在真实 GUI e2e 验收（2026-09-26）中发现两类问题，本设计同时处理。

### 1.1 扫描过程完全不可见

所有扫描都是「点下去 → 空白转圈 → 一次性出结果」。用户在等待期间无法判断系统是在工作还是卡死。

实测（2026-09-26，本机 macOS 26.5.1）：

| 扫描 | 实测耗时 | 目标 |
| :--- | ---: | ---: |
| 缓存扫描 `scan_cache` | 4.2s | <3s（AGENTS.md §5 首次扫描） |
| 应用卸载残留扫描 | 与选中应用数线性相关，未单独测量 | <3s |

缓存扫描由 16 个扫描器并行执行（`cache_scanner.rs:157-191`），每个扫描器都要做整树遍历（`dir_size`）。体积最大的几项：

| 扫描项 | 体积 | 单次 `du` 耗时 |
| :--- | ---: | ---: |
| 废纸篓 | 20.0 GB | 2.17s |
| Cargo registry | 5.1 GB | 1.37s |
| Xcode 全目录 | 11.4 GB | 0.97s |

16 个遍历同时抢同一块 SSD 的 I/O，导致总耗时高于最慢的单项。

**已完成的预处理**（本轮之前）：应用缓存目录大小改为 rayon 并行计算；卸载残留扫描改为「9 个 `~/Library` 目录清单只读一次复用 + 按 app 并行」。两者方向正确但都不是主要瓶颈。

### 1.2 列表名称对普通用户不可读

| 列表 | 现状 | 结论 |
| :--- | :--- | :--- |
| 应用程序 | 读 `CFBundleDisplayName`，兜底 `.app` 文件名（`app_scanner.rs:185`） | 可读，不动 |
| 缓存 | 大部分可读（`废纸篓`、`NPM 全局缓存`、`Homebrew 旧包缓存`） | 3 处文案张冠李戴 |
| 进程 | 能匹配到 App 的显示 `Google Chrome Helper (Renderer)`；匹配不到的是**裸二进制名 + CSS 截断** | 需修 |

进程名的根因：`scanner.rs:208` 直接取 `let name = proc.name()`，即 sysinfo 给的原始可执行文件名。因此会出现：

- `com.apple.WebKit.WebC…` —— 这是 bundle id 形态的进程名，不是「这是什么」
- `BackgroundShortcutRun…` —— 系统内部代号，且被 CSS `truncate` 截断
- `Doubaolme` —— 音译名，用户无法对应到任何已知应用

---

## 2. 目标与非目标

### 目标

1. 点击扫描后 **≤200ms 出现第一个阶段名**，用户能判断系统在做什么
2. 等待期间能看到已完成阶段数与已发现体积
3. 进程列表对非 App 进程给出可读名称，且长名称有完整值可查
4. 修正缓存列表中与实际行为不符的说明文案

### 非目标（明确不做）

- **不做结果流式返回**。快照仍然在扫描全部完成后一次性注册。理由：broker 的「注册新快照即作废旧 key」语义与「可持续追加的快照」不兼容，改动会波及全部破坏性操作路径，风险远大于收益。
- **不做假进度条**。前端本地定时器驱动的进度已在 `CacheView.tsx:160-170` 存在（仅用于清理执行阶段），本设计不扩展它，也不把它用于扫描阶段。
- **不拆分 IPC 命令**。不把 16 个扫描器拆成 16 个 `invoke`，以免扩大 broker 的可信命令面。
- 不改 `scan()` 的返回值结构，不改快照/选择/执行链路。
- 不做扫描时长的直接优化（增量缓存、换用系统 `du`、并行度调参），那是独立议题。

---

## 3. 设计

### 3.1 后端：新增一个事件，不新增命令

#### 进度载荷

```rust
#[derive(Clone, Serialize)]
pub struct StageUpdate {
    pub stage: String,          // 阶段名；缓存扫描为编译期常量，卸载扫描为应用显示名
    pub state: &'static str,    // "running" | "done"
    pub item_count: usize,      // done 时有效：该阶段产出的条目数
    pub found_bytes: u64,       // done 时有效：该阶段产出的总体积
}
```

`stage` 取 `String` 而非 `&'static str`：缓存扫描传入的是编译期常量（`.to_string()` 转换，每次扫描 16 次分配，可忽略），卸载扫描要填运行时的应用显示名。用一个类型覆盖两种来源，避免第二套载荷。

事件里**只有阶段名、条目数、体积**。不含文件路径、PID、应用 bundle id——进度通道不承担任何数据下发职责，数据仍然只走快照。

#### 扫描器签名

```rust
pub type ProgressSink = Arc<dyn Fn(StageUpdate) + Send + Sync>;

pub async fn scan(progress: Option<ProgressSink>) -> CacheScanResult
```

`None` 表示不报告（CLI、单元测试），行为与今天完全一致。

16 个匿名闭包改为**命名任务**，每个任务在开始与结束时各回调一次：

```rust
let stages: Vec<(&'static str, Box<dyn FnOnce(&Path) -> Vec<CacheItem> + Send>)> = vec![
    ("npm 缓存", Box::new(|h| scan_npm(h))),
    ("pnpm 缓存", Box::new(|h| scan_pnpm(h))),
    // ...
    ("废纸篓", Box::new(|h| scan_trash(h))),
];
```

每个任务体在 `spawn_blocking` 内部先 `sink(StageUpdate { stage, state: "running", .. })`，拿到结果后 `sink(StageUpdate { stage, state: "done", item_count, found_bytes })`。

阶段名清单（与 `cache_scanner.rs` 现有 16 个扫描器一一对应）：

| 扫描器 | 阶段名 |
| :--- | :--- |
| `scan_npm` | npm 缓存 |
| `scan_pnpm` | pnpm 缓存 |
| `scan_yarn` | Yarn 缓存 |
| `scan_docker` | Docker 镜像与容器 |
| `scan_docker_stale_images` | Docker 无用镜像 |
| `scan_stale_node_modules` | 闲置 node_modules |
| `scan_homebrew` | Homebrew 缓存 |
| `scan_xcode` | Xcode 缓存 |
| `scan_cocoapods` | CocoaPods 缓存 |
| `scan_cargo` | Cargo 缓存 |
| `scan_pip` | pip 缓存 |
| `scan_go` | Go 模块缓存 |
| `scan_app_caches` | 应用缓存 |
| `scan_app_logs` | 应用日志 |
| `scan_crash_reports` | 崩溃报告 |
| `scan_trash` | 废纸篓 |

阶段名描述的是**扫描器**（如「应用缓存」），与结果行里的**条目**（「Google 缓存」）是两个层级，不强行对齐，避免出现「进度说 A、结果说 B」。

#### 命令侧

```rust
#[tauri::command]
async fn scan_cache(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<SnapshotResult<CacheSnapshotView>, String> {
    let sink: ProgressSink = Arc::new(move |u| {
        let _ = app.emit("cache-scan-progress", &u);
    });
    let result = cache_scanner::scan(Some(sink)).await;
    // 快照注册逻辑一字不改
}
```

事件名 `cache-scan-progress` 是硬编码常量，不接受任何用户输入拼接。

#### 卸载残留扫描

`scan_app_residues_batch` 同样发进度，载荷复用 `StageUpdate` 的形状但用独立事件名 `residue-scan-progress`，`stage` 填应用显示名（来自已解析的后端身份，不接受前端传入），`state` 仅用 `done`，由前端按累计 `done` 数除以总数渲染 `12/35`。

`scan_installed_apps` 与 `list_all_processes` 是单步操作，不发阶段事件；前端对它们使用通用「已用 N 秒」计数器。

### 3.2 前端

#### API 层（`src/lib/tauri.ts`）

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
```

`onResidueScanProgress` 同构。两者都返回 `UnlistenFn`，组件 `onCleanup` 必须调用——这是 `core:event:allow-unlisten` 已在 capabilities 中的原因。

#### 渲染

考虑到 900x600 窗口的空间约束，**不做 16 行清单**，采用紧凑三件套：

1. 一行状态文字：`正在扫描：Xcode 缓存 · 已完成 7/16`
2. 一条细进度条：`已完成数 / 总数`
3. 一行累计体积：`已发现 27.4 GB`

已完成的阶段不再逐个列出（避免信息密度失控），但**当前正在扫描的阶段名始终可见**——这是用户判断「系统在动」的主要依据。

扫描完成后三件套整体消失，列表照常出现。

`UninstallerView` 的残留阶段用同样的三件套，`已完成` 数为 `done` 事件累计数。

### 3.3 名称可读性

#### 进程名解析（`scanner.rs`）

在 `scanner.rs:208` 取到 `proc.name()` 之后、`find_app_bundle` 已被用于取图标的同一位置，增加一次显示名解析：

1. 若 `proc.exe()` 落在某个 `.app` bundle 内 → 复用 `app_scanner` 已有的 `read_plist_metadata` 取 `CFBundleDisplayName`，用它作为 `row.name`
2. 否则若 `name` 呈 bundle id 形态（含 `.` 且以段名结尾，如 `com.apple.WebKit.WebContent`）→ 取最后一段并做可读化：命中一张**小而固定**的系统进程映射表则用友好名（首条为 `com.apple.WebKit.WebContent` → `WebKit 网页内容`），未命中则把最后一段原样作为显示名，绝不猜测拼接
3. 否则保留原始名

同时把**解析出的完整名**放进新的 `row.full_name` 字段，前端长名称用 `title` 属性展示完整值，解决 `BackgroundShortcutRun…` 这类截断后无法查看全文的问题。

`row.name` 参与白名单/保护判定（`is_whitelisted`、`evaluate_protection`），因此**解析必须发生在保护判定之后**，或保证解析不改变这些判定的输入。此处选择：判定继续使用原始 `proc.name()`，`row.name` 只用于显示。这是本设计里最容易引入回归的地方，实现时必须保持判定输入不变。

#### 文案修正

真实问题比「三处文案」更窄也更明确：`scan_pnpm` / `scan_yarn` 等**自带正确文案**（如 `cache_scanner.rs:267` 的「pnpm 全局 store，未被项目引用的包可安全剪枝」）。用户看到的错误文案来自 `scan_app_caches`——它对 `~/Library/Caches` 下**每一个**目录统一套用「应用本地缓存，清理后应用会自动重建」（`cache_scanner.rs:921`）。`~/Library/Caches/pnpm` 与 `~/Library/Caches/ms-playwright` 落在该目录下，于是被错误地描述成「应用缓存」。

因此修法是**按目录名识别已知的开发工具缓存目录**，给它们符合真实行为的说明，其余目录保持原文案：

| 目录名 | 现文案 | 改为 |
| :--- | :--- | :--- |
| `pnpm` | 应用本地缓存，清理后应用会自动重建 | pnpm 缓存，清理后依赖需重新下载 |
| `ms-playwright` | 应用本地缓存，清理后应用会自动重建 | Playwright 浏览器二进制，清理后需重新下载 |
| 其余目录 | 应用本地缓存，清理后应用会自动重建 | 不变（对普通应用缓存这是准确的） |

识别用的目录名清单必须小而固定，并**复用** `scan_app_caches` 里已有的 `skip_contains` 词表风格，避免两套互不相干的名单。命中时同时把这些条目标记为开发工具类（`CacheCategory::Developer`），让前端能给出不同的风险提示。

#### 风险行去重

CLI 输出的风险行把列里已有的信息重复了一遍：

```
低风险 · opencode · 932MB · 170.5% CPU（应用主进程，仅供参考，清理会导致应用崩溃） · 受保护（是其他进程的父进程）
```

改为只保留判定结论与原因，删掉与列重复的数值段：

```
低风险 · 应用主进程，清理会导致应用崩溃 · 受保护：是其他进程的父进程
```

---

## 4. 数据流

```
用户点「扫描」
  → invoke("scan_cache")
  → cache_scanner::scan(Some(sink))
  → 16 个 spawn_blocking 任务
       每个任务: sink(running) → 干活 → sink(done)
  → sink 把 StageUpdate emit 到 "cache-scan-progress"
  → 前端 listen 回调更新三件套（不触碰 snapshot/selection）
  → scan() 返回 → snapshot_cache() 注册快照（逻辑不变）
  → 前端拿到快照，渲染列表，卸载进度 UI，unlisten
```

失败路径：任一扫描器 panic 或返回空批，`filter_map(Result::ok)` 的既有行为不变（跳过该项），进度上体现为该阶段 `done` 但 `item_count: 0`。

---

## 5. 安全面

| 项 | 结论 |
| :--- | :--- |
| `core:event:allow-listen` | **已存在**（`capabilities/default.json:7`），无需新增权限 |
| `invoke_handler` 命令数 | 仍为 16，`operation_surface_checks.py` 白名单不变 |
| capability 固定清单 | 不变，`verify-security-config.py` 不变 |
| 事件载荷 | 仅阶段名（编译期常量）、条目数、体积；无路径、无 PID、无用户输入 |
| 事件名 | 硬编码常量，无拼接 |
| 阶段名来源 | 编译期字面量，非用户数据 |

本设计**不扩张任何可信面**。这是它被选中的主要原因之一。

---

## 6. 测试计划

### Rust

1. `scan(Some(fake_sink))`：断言 16 个阶段各上报一次 `running` 与一次 `done`，阶段名与 `cache_scanner` 扫描器一一对应
2. 断言 `done` 事件的 `item_count` / `found_bytes` 与该扫描器实际产出之和一致
3. 断言 `scan(None)` 的返回值与 `scan(Some(_))` 完全相同（进度不影响数据）
4. 断言阶段完成顺序等于真实完成顺序（用一个可控 barrier 让第 3 个阶段最后完成，断言事件顺序）
5. 进程名：构造 bundle 内进程 / bundle id 形态进程名 / 裸名三种输入，断言 `row.name` 可读且 `is_whitelisted`、`evaluate_protection` 的判定输入未被改变

### 前端

1. `CacheView`：收到 `running` 即渲染阶段名（不等待任何 `done`）
2. `CacheView`：完成后列表出现、进度 UI 卸载
3. `CacheView`：组件卸载时调用 `unlisten`
4. `UninstallerView`：残留进度 `12/35` 计数正确
5. 进程列表：长名称渲染 `title` 为完整值

### 验收标准

| 场景 | 判定 |
| :--- | :--- |
| 点击「扫描」 | ≤200ms 出现第一个阶段名，且不是空白 spinner |
| 4.2s 的缓存扫描 | 全程可见当前阶段与已完成计数 |
| 快照与选择 | 行为与今天完全一致，`bun run verify` 全绿 |
| 进程列表 | 无 `xxx…` 形式的不可回溯名称；长名有完整值 tooltip |
| 缓存文案 | `pnpm 缓存` / `ms-playwright 缓存` 说明与实际行为一致 |

---

## 7. 风险

| 风险 | 缓解 |
| :--- | :--- |
| 进度 UI 在 16 个阶段瞬时完成时闪烁 | 阶段最短也有几十毫秒；且进度 UI 只在 `scanning` 为真时存在 |
| 事件量：32 次/次扫描 | 可忽略；卸载场景最多 35 次 |
| 进程名解析误伤保护判定 | 判定输入强制保持 `proc.name()`，只有显示字段走解析；第 5 组测试专门盯这条 |
| `scan()` 签名变更影响其他调用方 | 仅 `lib.rs:118` 与测试调用；`scan(None)` 保持旧行为 |

---

## 8. 与其他工作的关系

- 本设计**不改变** Operation Broker 的任何决策：快照语义、opaque key、TTL、单次消费、10 分钟有效期全部不变
- 扫描时长的直接优化（4.2s → <3s）不在本设计内，另开议题
- 机器可读性修复（`com.apple.WebKit.WebContent` → `WebKit 网页内容`）的映射表保持小而可审计，宁可未命中也不猜
