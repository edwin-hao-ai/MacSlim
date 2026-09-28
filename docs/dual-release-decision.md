# 双版本发布路线：Developer ID 完整版 + Mac App Store 缩减版（2026-09-28 决策）

> ## ⚠️ 最高约束：两个版本都必须发
>
> **每次发布都要产出并分发两个产物，任何一个都不能省。**
>
> - **Developer ID 完整版** — 官网 / GitHub Releases，公证后直链下载
> - **Mac App Store 版** — ASC 上架（`com.vgoapp.macslim`，appId `6816609940`）
>
> 两者是**完全分离的构建**，不是同一个二进制的两种配置。MAS 版**不替代**
> 完整版：只发 MAS 版会让开发者丢掉进程管理与 Docker/CLI 驱动的深度清理；
> 只发完整版则 App Store 渠道拿不到任何东西。落地页也据此设计 ——
> 「本页下载的是 Developer ID 全功能版本」与「还会有一个 App Store 版本」
> 两块内容并列摆着，不含糊。
>
> 两条链路的失败**互不阻塞**：MAS 审核被拒或卡住，不影响 Developer ID 版
> 照常发；Developer ID 版发布失败，也不该把 MAS 的发版窗口堵死。

## 1. 决策

**同时保留两个版本，MAS 不替代完整版。**

| | Developer ID 完整版 | Mac App Store 缩减版 |
| --- | --- | --- |
| 用途 | 官网 / GitHub Releases 下载 | ASC 上架（`com.vgoapp.macslim`，appId `6816609940`） |
| 签名 | Developer ID + 公证 | App Store 沙箱签名 |
| 进程管理 | 有 | **无** |
| 端口占用 | 有 | **无** |
| Docker 清理 | 有 | **无** |
| npm / brew CLI 驱动的深度清理 | 有 | **无**（只能按路径删目录） |
| 自更新（updater） | 有 | **无**（走 App Store 更新） |
| 缓存扫描与文件类清理 | 有 | 有（改 Rust 原生遍历删除 + Full Disk Access 向导） |
| 应用列表扫描 / 卸载 | 有 | 有（改 Rust 原生 plist 解析） |

裁决依据：用户 2026-09-28 明确「接受缩水版 MAS」且「完整版也要保留」。

## 2. 为什么必须砍这四类（不是偷懒，是沙箱硬禁止）

App Sandbox 下**没有**任何 entitlement 能放行这三件事：

1. **终止其他进程** —— `src-tauri/src/process_ops.rs:6` 用
   `nix::sys::signal::kill`。沙箱进程不能给别的进程发信号。
2. **调外部 CLI** —— 沙箱只能 exec **自己 bundle 里的**可执行文件。受影响：
   - `cache_scanner.rs:1487+`、`docker.rs:506` → `docker`
   - `ports.rs:13` → `lsof`
   - `cache_cleaner.rs:702` → `osascript`
   - `cache_cleaner.rs:525` → shell
   - `app_scanner.rs:131,161`、`applications.rs:366` → `plutil` / `sips`
3. **自更新** —— MAS 由 App Store 负责更新，updater 插件必须排除，
   capability 里的 `updater:allow-check` / `updater:allow-download-and-install` 也要去掉。

**可改造回来的**：文件类清理。沙箱 + 用户授予的 Full Disk Access 足以覆盖
`~/Library/Caches`、`~/.npm`、`~/.cargo` 的遍历与删除。代价是要写一份
Full Disk Access 授权向导（`PrivacyInfo.xcprivacy` 也要写）。

## 3. 实现约束（已查实，会决定代码怎么写）

**不能加第二个 `invoke_handler`。**
`scripts/tests/test_operation_contract.py:183` 的 `rejects_a_second_invoke_handler`
明确禁止。所以分叉只能写在**同一个** handler 里：

```rust
.invoke_handler(tauri::generate_handler![
    get_system_health,
    scan_cache,
    // ...
    #[cfg(not(feature = "mas"))]
    list_all_processes,
    #[cfg(not(feature = "mas"))]
    docker_available,
    // ...
])
```

配套要改的地方：

- `scripts/operation_surface_checks.py` 的 `EXPECTED_IPC_COMMANDS` 现在是固定 16 条
  （`test_operation_contract.py:55` 断言），要改成「全功能 16 条 / MAS 版 N 条」
  两套期望。
- `src-tauri/capabilities/default.json` 要分叉：MAS 去掉 updater，去掉
  `com.apple.security.automation.apple-events`（`cache_cleaner.rs:702` 依赖它）。
- Cargo 要加 `mas` feature，并且 `tauri.conf.json` 的 `bundle.targets` 从
  `["app","dmg"]` 扩成含 mas。
- 前端要知道当前是哪个 flavor，否则会调到不存在的命令。

## 4. 已知的次生问题

- `docker_available` / `list_all_processes` 去掉后，`ProcessView` / `DockerSection`
  会调不存在的 IPC。前端要么按 flavor 隐藏入口，要么后端保留命令但返回
  「MAS 版不支持」——**后者更省前端改动**，但与「砍掉」的语义有张力，需在实现时定。
- 交接文档里「`invoke_handler` 保持 16 条」这条硬约束在 MAS flavor 下不成立，
  该文档相应章节需要标注适用范围。

## 5. 相关文档

- 可行性与 e2e 门禁设计：`docs/superpowers/mas-feasibility-and-e2e-release-gate.md`
- ASC 身份与对接流程：`docs/release-identity-and-asc.md`
- 商业阶段规划（MAS 原本排在阶段二）：`docs/BUSINESS.md`
