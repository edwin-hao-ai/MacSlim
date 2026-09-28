# MacSlim 产品完整化、双发行与 Liquid Glass 重构设计

- 日期：2026-09-24
- 状态：用户已批准设计
- 适用范围：当前仓库的 UI、安全、功能完整性、商业化与 Mac App Store 提交准备

## 1. 背景

MacSlim 已具备智能扫描、缓存清理、进程管理、应用管理、卸载、历史记录、托盘、自更新、自动启动、CLI 与 Docker 管理等模块，但当前仍有以下结构性问题：

1. Tauri capability 缺少 `core:window:allow-start-dragging`，导致现有 `startDragging()` 路径无法通过权限检查。
2. 前端可向高危 Rust command 提交路径、命令、PID 和卸载对象，后端没有重新建立可信执行边界。
3. 清理结果把“移动到暂存目录或废纸篓”直接记为已释放，可能夸大收益。
4. 若干破坏性操作缺少统一 dry-run、风险分层、>10 GB 二次确认和重新获取指引。
5. 当前二进制使用 private API、root 提权、自更新、Docker/进程管理和容器外访问，无法原样提交 Mac App Store。
6. 页面、托盘、错误提示与部分版本信息仍存在硬编码或不完整状态。
7. UI 已有统一基础，但尚未完整采用 Apple 最新设计语言，窗口拖动与安全交互边界不清晰。
8. 真实活跃用户较少，付费定位尚未经过足够验证，不能以“再加功能”作为产品策略。

本设计先解决正确性、安全性、可理解性和商业闭环，再扩展功能数量。

## 2. 核心产品判断

MacSlim 的产品定位从“大而全的 Mac 清理工具箱”调整为：

> 面向重度 Mac 用户的 Space Rescue / 磁盘救援控制台：解释空间为何增长、预测何时告急、生成可审查的回收计划，并安全执行。

首个核心触发时刻是“磁盘突然告急”。首个核心用户是同时使用 Docker、Xcode、Homebrew、node_modules 等工具的重度 Mac 用户。

以下假设仍需通过发布后的行为数据验证：

- 趋势、归因和救援计划比单次清理能力更值得付费。
- 用户愿意为可信、省时和持续预防付费，但不愿为本地工具长期订阅。
- Free 基础能力不设次数限制，更有利于建立信任。

## 3. 目标与非目标

### 3.1 目标

- 修复安全拖动区域和所有已发现的交互死角。
- 统一前端、IPC、持久化与测试边界。
- 用不可变扫描快照和一次性 operation ID 保护破坏性操作。
- 提供真实、可解释、可追溯的释放结果。
- 建立 A1“克制原生型 Liquid Glass”视觉系统。
- 保留完整 Developer ID 版，并建立可过审的 App Sandbox 版。
- 建立 Free/Pro 清晰、无 dark pattern 的商业化边界。
- 满足 macOS 13 与 macOS 26/27、Intel 与 Apple Silicon 的兼容要求。

### 3.2 非目标

- 不做 Windows/Linux。
- 不做杀毒、反向代理、远程控制或云端设备管理。
- 不做大文件/重复文件扫描。
- 不以“释放内存”或“清理后一定提速”作为付费承诺。
- 不在 App Store 版保留 root、全用户目录扫描、任意进程终止、Docker socket、自更新或通用卸载。
- 不引入 AI/LLM、遥测、远程数据收集或云同步。
- 不使用“无限次清理”之外的次数型限流。

## 4. 已确认的关键决策

1. 采用双版本路线。
2. 采用共享内核、双编译发行构建，不使用运行时能力隐藏。
3. Developer ID 完整版继续保留进程、端口、Docker、CLI、自更新和用户明确要求完整保留的应用卸载。
4. Mac App Store 版只编译沙盒可接受能力。
5. 拖动范围采用安全拖动区，不允许控件、卡片、列表和滚动区域触发拖动。
6. UI 采用 A1“克制原生型 Liquid Glass”。
7. 首个收费闭环是“智能空间救援”。
8. 首发优先一次性买断，建议验证价 US$19.99 / ¥99，覆盖 3 台 Mac。
9. App Store 名称可继续使用 `MacSlim`，副标题建议为 `Space Rescue`。

2026-09-24 的美国 Mac App Store 搜索未发现精确同名应用。`MacSlim` 名称简短、描述性强，满足 30 字符限制，也不属于关键词堆砌。该结论不是商标法律意见；正式商业化前仍应进行专业商标检索。`Mac` 只作为平台描述，应用 About 页面应明确声明与 Apple Inc. 无隶属关系。

## 5. Free 与 Pro 功能边界

### 5.1 Free

- 无限次手动扫描。
- 当前磁盘、系统健康和空间来源概览。
- 每个候选项的路径、大小、来源、最后使用时间、风险、恢复成本和默认选择原因。
- dry-run 与删除预览。
- 明确安全的缓存清理。
- 真实释放空间或移动结果核验。
- 操作日志。
- 重新获取和恢复指引。
- 文件、项目、应用和目录保护规则。
- Developer ID 完整版保留完整应用卸载、残留审计与恢复指引。
- Docker/Xcode 当前空间快照与基础可回收量展示。

MAS 版只展示系统可公开获得的容量信息，以及用户明确授权目录中的 Docker/Xcode 文件占用；不连接 Docker daemon，不宣称完整资产图能力。

### 5.2 Pro

- 7/30/90 天空间历史。
- 预计写满日期与增长趋势。
- 增长来源归因。
- “始终保留至少 N GB”等安全线设置。
- 个性化 Rescue Plan。
- 多卷独立阈值与策略。
- 低空间通知。
- 针对明确可重建缓存的自动守护。
- Docker image/container/volume/BuildKit cache 的项目关系、使用状态和恢复成本。
- Xcode DerivedData、workspace、模拟器和项目资产关系。
- 闲置开发项目发现与批量计划。
- 完整历史和审计导出。

MAS Pro 只对用户授权目录提供历史、趋势、归因和 Rescue Plan；Docker daemon、BuildKit、容器关系和项目级完整资产图属于 Developer ID 完整版，MAS 不做能力降级伪装。

### 5.3 不作为 Pro 卖点

- 应用卸载残留。
- 普通缓存扫描。
- 内存释放或“加速”。
- 杀毒。
- 大文件/重复文件扫描。
- 团队/MDM。
- 云同步。

这些能力如继续存在，应作为 Free 辅助功能或后续独立产品研究，不进入首个付费闭环。

## 6. 双发行技术架构

### 6.1 Cargo feature

建立两个互斥的发行 feature：

- `distribution-full`
- `distribution-mas`

要求：

- 编译时同时启用 `distribution-full` 与 `distribution-mas` 必须直接失败，不能通过 `--all-features` 构建出一个混合发行物。
- MAS 构建不得编译 root 提权、LaunchAgent、Docker CLI/socket、任意进程终止、通用卸载和自更新模块。
- MAS 构建不得包含上述模块依赖和敏感命令字符串。
- Full 构建可包含完整平台适配器。
- 发行差异必须在编译期决定，不通过读取 sandbox 状态在运行时暴露隐藏功能。

### 6.2 Rust 分层

建议职责：

- `core`：不可变模型、风险策略、恢复成本、Rescue Plan、操作状态、错误类型、历史 trait。
- `full-platform`：Docker、CLI、进程、端口、完整卸载、受控提权、自更新和登录项适配。
- `mas-platform`：App Sandbox、用户授权目录、security-scoped bookmark、通知和 `SMAppService.mainApp`。
- Tauri app shell：窗口、IPC 注册、托盘和发行入口。

禁止前端传递可执行 shell command。完整版原 `CacheItem.command` 路径必须移除，命令只能由后端规则生成。

### 6.3 前端构建

- 共享 UI 组件、设计 token、领域类型和视图模型。
- `full` 与 `mas` 使用独立入口和路由清单。
- MAS 入口不 import 高危页面、命令或营销文案，避免危险字符串进入最终 WebView 资源。
- 不使用运行时 feature flag 隐藏不适用页面。

### 6.4 发行身份

- Developer ID 与 Mac App Store 使用独立 Bundle ID、产品标识、签名配置、entitlements 和数据容器。
- 两版共享品牌和核心视觉系统。
- 未经用户明确同意，不在两版之间同步数据。

## 7. Liquid Glass 与 UI 系统

### 7.1 视觉方向

采用 A1“克制原生型 Liquid Glass”：

- 参考 Apple 2026 最新 macOS 27 设计规范。
- Liquid Glass 主要用于 Sidebar、工具栏、浮层、关键状态和主操作。
- 内容列表、表格、数据和正文使用标准材质，确保对比度和阅读效率。
- 禁止所有卡片都透明悬浮。
- 仅关键数据卡片允许受控渐变。

### 7.2 原生实现

- macOS 26/27 使用公开 AppKit `NSGlassEffectView`。
- macOS 13–25 使用公开 vibrancy/API 与 CSS 材质降级。
- 所有正式发行都禁止 `macOSPrivateApi`，MAS 构建额外接受 App Store public API 审核。
- “减少透明度”和“增加对比度”开启时使用实色高对比主题。
- Dark Mode、Retina/HiDPI 和宽色域均需验证。

### 7.3 UI 骨架

- `AppShell`
- `LiquidSidebar`
- `FloatingToolbar`
- `ContentCanvas`
- `RescueDashboard`
- `SpaceTrendChart`
- `RescuePlanCard`
- `RecoveryCostBadge`
- `RiskBadge`
- `DryRunSheet`
- `ConfirmationSheet`
- `PermissionPrimer`
- `OperationLog`

### 7.4 窗口拖动

- capability 增加 `core:window:allow-start-dragging`。
- 使用统一 `SafeDragSurface`。
- 标题栏、侧栏顶部和主视图空白/间距可拖动。
- `button`、`input`、`textarea`、`select`、`a`、`label`、`.card`、`li`、滚动区域和 `[data-no-drag]` 不触发拖动。
- 仅响应主鼠标键。
- 增加 E2E 测试，验证安全区域可拖、交互控件可正常点击、拖动不会破坏文本选择和滚动。

## 8. 核心数据模型

所有扫描结果使用不可变对象。

### 8.1 ScanSnapshot

- `snapshot_id`
- `started_at` / `completed_at`
- `scope`
- `volume_before_bytes`
- `volume_after_bytes`
- `candidates`
- `capability_snapshot`
- `warnings`

### 8.2 Candidate

- `candidate_id`
- `canonical_path`
- `source`
- `size_bytes`
- `last_used_at`
- `risk`
- `recovery_cost`
- `default_selected`
- `default_reason`
- `irreversible`
- `protection_reason`

风险与恢复成本使用固定枚举：

- `Risk`：`Safe`、`Caution`、`Danger`。
- `RecoveryCost`：`Under5Minutes`、`Minutes5To30`、`Over30Minutes`、`Unknown`。

### 8.3 RescuePlan

- `plan_id`
- `snapshot_id`
- `reserve_target_bytes`
- `protected_paths`
- `selected_candidates`
- `expected_reclaim_bytes`
- `expected_recovery_cost`
- `warnings`
- `requires_second_confirmation`

### 8.4 Operation

- `operation_id`
- `plan_id`
- `expires_at`
- `state`
- `started_at` / `finished_at`
- `before_bytes` / `after_bytes`
- `results`
- `failures`
- `recovery_manifest`

## 9. 安全数据流

1. 前端发起扫描请求。
2. 后端根据发行构建能力与用户授权确定扫描范围。
3. 后端扫描并生成不可变 `ScanSnapshot`。
4. 用户设置安全线、查看风险和恢复成本。
5. 后端生成确定性 `RescuePlan`。
6. 前端展示 dry-run 和确认清单。
7. 后端签发有效期 10 分钟、只能消费一次的 `operation_id`。
8. 前端只提交 `operation_id` 和用户确认上下文。
9. 后端重新校验计划有效期、绝对路径、文件身份、权限和当前风险。
10. 后端执行操作并记录事务清单。
11. 后端测量真实卷空间差值与文件状态。
12. 前端显示实际结果、日志和恢复/重新获取指引。

前端安全边界：

- 配置严格 CSP，禁止远程脚本、`unsafe-eval` 和未允许的外部连接。
- 业务 IPC 只接受 schema 校验后的结构化参数。
- 前端不得传递可执行 shell command、任意路径、PID 或卸载对象来直接执行高危操作。

## 10. 破坏性操作策略

### 10.1 默认选择

默认选择仅限：

- 重新获取成本低于 5 分钟。
- 明确属于缓存或临时产物。
- 不在受保护目录。
- 不涉及 Docker volume、活跃项目、应用数据或用户文档。

### 10.2 永不自动执行

- Docker volume 和 `docker system prune --volumes`。
- 活跃项目和用户文档。
- 应用偏好、容器和关键数据，除非逐项确认。
- SIGKILL 进程树。
- 永久删除。
- 系统核心目录。
- 超过 10 GB 的单一计划。

### 10.3 执行方式

- 可恢复项优先移动到受控暂存区或废纸篓。
- 暂存元数据保留 10 分钟。
- 永久删除必须明确标记不可回滚。
- 应用残留必须按 bundle ID、路径和用户确认逐项匹配，禁止仅按名称子串。
- 所有子进程在执行前必须 dry-run 并验证固定命令映射。

### 10.4 释放量

- 不把移动到同一卷内的文件大小视为已释放。
- 分别记录 `moved_to_trash`、`staged_for_removal`、`permanently_deleted` 和 `actual_volume_freed_bytes`。
- 清理完成数字使用真实卷空间差值。

## 11. 错误处理

统一领域错误：

- `PermissionDenied`
- `AuthorizationExpired`
- `FileBusyOrLocked`
- `InsufficientSpace`
- `UnsupportedCapability`
- `PlanExpired`
- `PartialSuccess`
- `Cancelled`
- `Internal`

要求：

- 用户可见文案使用中文人话。
- 错误详情记录在本地操作日志。
- 部分成功必须列出已完成和未完成项目。
- 权限请求前解释扫描范围和必要性。
- 授权过期时重新请求，不静默扩大范围。
- 用户可见日志不展示内部凭据；详细路径和调试信息只写入受保护的本地诊断日志，并进行敏感信息脱敏。

## 12. App Store 合规设计

### 12.1 硬性要求

- App Sandbox。
- 仅使用公开 API。
- Xcode archive 和 App Store Connect 上传。
- 不请求 root/setuid。
- 不使用自更新。
- 不在退出后未经同意运行进程。
- 不包含 CLI symlink 或外部代码安装能力。
- MAS 版不含网络客户端，除非未来功能确实需要并重新审核。

### 12.2 最小 entitlements

MAS 初始只启用 App Sandbox 和确有功能需要的 user-selected read-write。Apple Events、Automation 等能力默认不启用，保留时必须先证明核心需求并提供用途说明。移除无必要的 unsigned executable memory、disable library validation 和 DYLD 类权限。

### 12.3 自启动

使用 `SMAppService.mainApp`，由用户在设置中明确开启。不得用 LaunchAgent 绕过沙盒和审核。

### 12.4 隐私与元数据

- 隐私政策。
- App 内隐私入口。
- App Store Connect App Privacy 问卷。
- Privacy manifest。
- 真实描述、截图和权限文案。
- 2026 年年龄分级问卷。
- 导出合规问卷。
- Review Notes 说明用户授权目录、dry-run、不可逆操作和无需登录。
- 声明 MacSlim 为第三方应用，与 Apple Inc. 无隶属关系。

## 13. 商业化

- 首发优先一次性买断，不做强制订阅。
- 建议验证价 US$19.99 / ¥99，覆盖 3 台 Mac。
- Free 永久保留手动扫描、dry-run、安全清理、日志和恢复指引。
- App Store 使用非消耗型 IAP 解锁 Pro。
- Developer ID 版可使用离线许可证，但不得在 MAS 启动时要求 license key。
- 不按释放 GB 收费。
- 不使用“免费下载”但启动后强制付费的误导文案。
- 最终价格需通过转化、留存和用户访谈验证。

## 14. 测试策略

### 14.1 单元测试

- 路径规范化和边界。
- 符号链接与 TOCTOU 防护。
- 风险和恢复成本分类。
- 默认选择规则。
- Rescue Plan 确定性。
- 一次性 operation token。
- 真实释放量计算。
- 部分失败和恢复清单。

### 14.2 集成测试

- 临时目录中的扫描、dry-run 和执行。
- 权限拒绝与授权过期。
- 锁定文件和部分成功。
- 模拟 Docker 与进程，不调用真实系统。
- 不执行真实 SIGKILL、sudo 或永久删除。
- 完整版/MAS feature 编译和命令可用性矩阵。

### 14.3 UI/E2E

- 800×540 与 900×600。
- 窗口缩放与安全拖动区域。
- 深浅色、减少透明度、减少动态效果。
- 键盘导航、VoiceOver 和对比度。
- 中英文完整性。
- 所有页面的 loading、empty、error、success 和 permission 状态。

### 14.4 性能

- 首次扫描 <3 秒。
- 增量扫描 <1 秒。
- 后台 CPU <2%。
- 后台内存 <80MB。
- 动画仅使用 transform/opacity，不持续触发 layout/paint。
- Liquid Glass 滚动和窗口缩放不得产生持续高频重绘。

### 14.5 覆盖率

- 总覆盖率不低于 80%。
- 安全关键模块 100% 场景覆盖。

## 15. CI 与验收

CI 必须运行：

- 前端 lint。
- TypeScript typecheck。
- 前端测试。
- `cargo fmt --check`。
- `cargo clippy --all-targets --no-default-features --features distribution-full -- -D warnings`。
- `cargo clippy --all-targets --no-default-features --features distribution-mas -- -D warnings`。
- Full 与 MAS 分别执行 Rust 测试。
- MAS/Full 编译矩阵。
- 最终 entitlements 检查。
- 敏感命令和字符串检查。
- Tauri 打包冒烟测试。

最终 MAS archive 必须证明：

- App Sandbox 已启用。
- 不含 root/admin 分支。
- 不含 updater。
- 不含 Docker/任意进程终止/完整卸载命令。
- 不含 private API。
- 签名和 provisioning profile 正确。
- 在干净 macOS 13 与当前 macOS 上运行。
- TestFlight 构建可供完整审核。

## 16. 发布顺序

1. 立即撤销并轮换已暴露的 Apple App 专用密码与 Updater 凭据，迁移到 Keychain/CI Secret。
2. 完成 capability、安全拖动、CSP、路径校验和 operation ID 信任边界。
3. 修正真实释放量、dry-run、确认策略和操作日志。
4. 完成 A1 Liquid Glass、状态页面、i18n、可访问性和响应式。
5. 补齐自动守护、趋势、归因、Rescue Plan 和 Pro 包装。
6. 发布并验证 Developer ID 完整版。
7. 建立独立 MAS target、entitlements、archive 和 TestFlight 流程。
8. 完成外部 TestFlight 与 App Review 预审反馈闭环。
9. 上架 Mac App Store。
10. 根据转化、留存与主动反馈调整价格和功能，不提前扩展大文件、重复文件、团队或云同步。

## 17. 完成定义

本轮完成必须同时满足：

- 安全拖动在所有受支持系统上可预测工作。
- 前端无法直接提交任意 command、path、PID 或 uninstall object 执行高危操作。
- 所有破坏性操作有 dry-run、风险、真实后果和日志。
- 释放空间数字与真实卷空间变化一致。
- A1 Liquid Glass 在 Light/Dark、减少透明度和 800×540 下可用。
- Free/Pro 与 Full/MAS 边界在源码、构建产物和 UI 中一致。
- MAS archive 通过 Sandbox、public API、签名和 App Review 验收矩阵。
- 所有 lint、typecheck、测试、性能和打包验证通过。
