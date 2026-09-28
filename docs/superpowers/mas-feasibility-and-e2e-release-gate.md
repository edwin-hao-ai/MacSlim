# MacSlim 发布前 e2e 回归与 Mac App Store 可行性 结论

日期：2026-09-27

---

## 1. 结论先行

1. **MacSlim 不适合上 Mac App Store。** 不是「裁剪几个高危功能就能上」，而是核心价值与 App Sandbox 存在**结构性冲突**。建议走 Developer ID + 公证 + GitHub Releases 分发。
2. **e2e 回归门禁应该建，但不是靠截图断言。** 用可观测副作用断言（进程存活、窗口几何、SQLite 历史记录、扫描阶段事件），截图只作为人工复核产物。

---

## 2. Mac App Store 兼容性逐项核对

App Sandbox 的硬约束是：进程被关进沙箱容器，**默认不能读写容器外任何文件，也不能执行容器外可执行文件**。对照 MacSlim 现有实现：

| 功能 | 依赖的系统能力 | App Sandbox 下 | 结论 |
| :--- | :--- | :--- | :--- |
| 缓存清理 | 写 `~/Library/Caches`、`~/.npm`、`~/.cargo`、`~/.Trash`、`~/Library/Developer/Xcode/DerivedData` | 全部在容器外，沙箱拒绝 | ❌ 核心功能失效 |
| 开发缓存（NPM/pnpm/Yarn/Cargo/Go/pip/brew） | 调用 5 处 `Command::new`（`cache_scanner.rs`）执行外部二进制 | 沙箱禁止执行容器外可执行文件 | ❌ 无法执行 |
| 应用卸载 | 删除 `/Applications/*.app` + `~/Library` 残留 | 容器外，需用户逐项授权（powerbox） | ⚠️ 只能做「用户手动选中的 app」，无法批量 |
| 进程管理 | 对任意 PID 发 SIGTERM/SIGKILL | 信号本身不被沙箱直接拦，但杀其他 App 进程在审核上等同于「干扰系统与其他 App」 | ❌ 必被拒 |
| 优雅退出 | Apple Events（`com.apple.security.automation.apple-events` 已在 entitlements） | 沙箱可用，但需逐次授权 | ⚠️ 可保留 |
| Docker | 调用 `docker` CLI + socket | 容器外可执行文件 + socket 访问 | ❌ 移除 |
| 自更新 | Tauri updater → GitHub Pages | MAS 禁止自更新（更新必须走 App Store） | ❌ 必须移除 |
| CLI 共享 `core/` crate | 与桌面端同 crate | 独立进程，不在沙箱内 | ❌ 沙盒版不含 CLI |
| 隐私 | 不采集任何数据 | — | ✅ 这是加分项 |

### 2.1 如果坚持要做 MAS 版，产品会变成什么

剩下的只有：**系统健康只读面板 + 用户手动选中的单个 app 卸载 + 优雅退出**。

也就是说 MAS 版不再是「一键式开发者 + 普通用户系统运维工具」，而是一个「体检 + 轻量卸载」小工具。AGENTS.md 里写的「MAS 编译期裁剪高危能力」需要修正为：**裁剪掉的是全部差异化和全部自动化能力**。

### 2.2 审核风险（即使技术上能过）

- **2.1 App Completeness / 4.x**：一个能杀进程、能删系统目录的 app 很难通过人工审核
- **2.5.2 不得使用未公开 API**：`sysinfo` 读取他人进程信息在沙箱外可用，沙箱内受限
- **4.2 最低功能量**：只剩只读面板时极易被判「功能不足」
- 同赛道已有明确先例：**CleanMyMac、iStat Menus、DaisyDisk、AppCleaner 均不在 MAS**（AppCleaner 只做「用户主动选中后删除」这一件事）

### 2.3 若仍要做，技术准备清单

| 项 | 内容 |
| :--- | :--- |
| 沙箱 entitlements | 加 `com.apple.security.app-sandbox=true`；**删除** `allow-jit`（MAS 不允许 JIT） |
| 外部执行 | 全部移除或改为容器内实现 |
| 文件访问 | 改为 `NSOpenPanel` 逐项授权（powerbox），`com.apple.security.files.user-selected.read-write` |
| 自更新 | 整块移除 `updater` 插件、`TAURI_SIGNING_PRIVATE_KEY`、`release.sh` 的 updater 产物 |
| 隐私清单 | 新增 `PrivacyInfo.xcprivacy`。**注意**：`cache_scanner.rs` 用了 `metadata.accessed()`，属于 `NSPrivacyAccessedAPICategoryFileTimestamp` 受限 API，必须声明理由 |
| Info.plist | 补 `NSPrivacyAccessedAPITypes`、权限用途说明 |
| 签名 | App Store 发行证书 + provisioning profile，Tauri 配 `signingIdentity` + App Store Connect API Key |
| 上架材料 | App Store Connect 记录、截图、隐私政策 URL、App Privacy 问卷（无采集，最快）、审核联系人 |
| 验证 | `sandbox-exec` 拒绝路径自查、Xcode 静态分析、`spctl -a -vvv` |

**工作量估计**：不是小改。沙箱化会强制重写文件访问层与外部执行层，且产品价值大幅缩水。

---

## 3. e2e 回归门禁设计

### 3.1 原则：不靠像素

`screencapture` 截图可以做**人工复核产物**，但不能做自动断言依据（像素比对在 Retina 缩放、字体渲染、主题变化下极脆）。自动断言只打可观测副作用：

| 断言对象 | 手段 |
| :--- | :--- |
| 进程终止真的发生/没发生 | 牺牲进程 PID 的存活状态（`kill -0`） |
| 应用卸载真的发生 | `/Applications` 目标是否消失 + 残留目录是否消失 |
| 缓存清理体积 | 清理前后 `du -sk` 差值 与 历史记录 `freed_bytes` 是否一致 |
| 扫描进度可见 | 阶段事件数量 = 16、阶段名非空、首个事件延迟 |
| 快照/TTL/单次消费 | 同一 operation_id 二次执行必须失败 |
| 窗口拖动 | 窗口 `position` 变化（拖动由真机人工确认，合成事件无法驱动原生拖动） |
| 历史记录 | 直接读 SQLite 行数与字段 |

### 3.2 门禁任务清单（老功能 + 新功能）

**破坏性主链路（每条都自带牺牲目标，零风险）**
1. 缓存：扫描 → 勾选 → 确认弹窗显示摘要/估算/有效期 → 取消（断言无删除）
2. 缓存：真实清理一个自建的可丢弃目录 → 断言目录消失 + 释放字节数与 `du` 差值一致
3. 进程：勾选牺牲进程 → 确认 → 执行 → 断言进程死亡
4. 进程：受保护行不可勾选（断言 disabled）
5. 进程：白名单行不可勾选
6. 应用：优雅退出现在弹系统授权 → 取消（断言目标仍存活）
7. 卸载：残留扫描 → 确认弹窗 → 取消（断言目标仍在）
8. 卸载：真实卸载一个自建假 `.app` → 断言 bundle 消失

**Broker 不变量（回归重点）**
9. 同一 operation_id 二次执行 → 必须失败
10. 快照过期后执行 → 必须失败并显示中文提示
11. 勾选后不操作，6 秒后选择仍在（P1 修复的回归防护）
12. 悬停列表时列表不重排（P3 修复的回归防护）

**只读功能**
13. 7 个侧栏页面全部可打开且不崩溃
14. 历史记录可读
15. 设置可读写并持久化
16. **新功能**：缓存扫描阶段进度（16 阶段、首个事件 ≤200ms）
17. **新功能**：卸载残留扫描进度（n/N）

**发布指标（AGENTS.md §5）**
18. 首次扫描 <3s
19. 空闲 CPU <2%
20. 空闲内存 <80MB
21. 安装包 <15MB

### 3.3 落点

- 脚本：`scripts/e2e_gui.py`（Python，沿用仓库既有 `scripts/tests` 风格，产出 JSON 报告 + 截图到 `artifacts/e2e/`）
- 入口：`bun run verify:e2e`（**默认不进 `verify`**，因为它会真的删文件、真的杀进程）
- 报告：`.superpowers/sdd/…/e2e-report.md`，作为发布 go/no-go 依据

### 3.4 门禁的安全边界

- 所有破坏性目标都由脚本**自己造**（`/tmp` 下的假 `.app`、牺牲 `sleep` 进程、自建缓存目录）
- 绝不选中用户真实进程、真实 app、真实缓存
- 每一项都断言「不该发生的没发生」作为反向安全检查

---

## 4. 建议的推进顺序

1. 先写完 Plan B（名称可读性）并执行
2. 再实现扫描进度（Plan A）
3. 再建 e2e 门禁，覆盖上面 21 项
4. 门禁全绿 → 按 Developer ID 路线出 release（签名 + 公证 + 更新渠道）
5. MAS 结论归档为「不做」，除非产品定位变更
