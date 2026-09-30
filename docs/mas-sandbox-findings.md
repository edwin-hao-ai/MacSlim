# MAS 沙箱实测记录（2026-09-29）

这份文档记录**真机实测**结果，不是推断。MAS 的所有能力判断都应以此为准。

## 背景

Tauri v2 **移除了 Mac App Store 支持**（`BundleType` 只有
`deb/rpm/appimage/msi/nsis/app/dmg`，没有 `mas`），所以 MAS 产物由
`scripts/release-mas.sh` 手工串标准 macOS 工具产出。

## 凭据

| 项 | 值 |
|---|---|
| 证书 | `Mac App Distribution: Beijing VGO Co;Ltd`（ASC API 创建，id `XV9LBSSZBJ`） |
| 钥匙串显示名 | `3rd Party Mac Developer Application: Beijing VGO Co;Ltd (5XNDF727Y6)` |
| 有效期至 | 2027-09-29 |
| profile | `MacSlim MAS 1.0` / `MAC_APP_STORE`（id `d20fee78-…`） |
| bundleId | `com.vgoapp.macslim`（ASC `6BJU3FZ2NU`） |
| appId | `6816609940`，版本 `1.0` / `PREPARE_FOR_SUBMISSION` / `MAC_OS` |

全部由 App Store Connect API 自动创建，无需开发者后台网页操作。

## 实测结论

### ✅ 能跑

- **桌面端 App 能启动**，UI 完整渲染（侧栏 7 项、欢迎页、卡片）
- **系统健康扫描正常**：`get_system_health` 返回真实值
  （CPU 42.3% / 内存 5.2GB of 8.0GB / 磁盘 213GB of 228GB）
  —— 说明 `sysinfo` 读 CPU/内存/磁盘在沙箱下**不被拦截**

### ❌ 不能跑

**`macslim-cli` 一起签名后启动即 SIGTRAP（exit 133）**

崩溃栈全在 dyld 初始化阶段，与业务代码无关：

```
_libsecinit_appsandbox
  → _os_activity_initicate_impl
    → libSystemInitializer
      → dyld::MachOAnalyzer::forEachInitializer
        → dyld::RuntimeState::findAndRunAllInitializers
```

`--version`（只做 `println!`）也打不出来，证明根本没进 `main`。
**沙箱 profile 校验在进程启动极早期失败后主动 trap。**

处理：`release-mas.sh` 的 `strip_cli` 在签名前删除它。
理由是留着一个必定崩溃的可执行文件会被审核拒，且它对 App Store 用户
毫无价值 —— 沙箱里 exec 不了外部工具，而 CLI 的存在意义正是驱动它们。

### ❓ 未验证

以下页面在 MAS 下**没有逐页验证**，不能假定可用：

- 进程管理（观察到的现象是列表为空，但未查清是沙箱限制还是同一类崩溃）
- 应用程序列表（依赖读 `/Applications`）
- 缓存清理（依赖读 `~/Library/Caches` 等）
- 真实清理操作
- Full Disk Access 授权流程（**尚未实现**）

`files.all` entitlement 已在包里，但**用户未在系统设置里授予 FDA 时**，
沙箱仍会拒绝这些路径。所以「缓存清理能不能用」必须实测，不能从 entitlement 推断。

## 事故记录：MAS 构建覆盖了完整版产物

**现象**：MAS 构建与完整版共用 `src-tauri/target`，MAS 构建把 Developer ID
签名的 `MacSlim.app` 整个顶掉（变成 3rd Party 签名），已构建好的 `.dmg` 也没了。

**影响**：GitHub 上已上传的 v1.0.0 不受影响（本地产物可重建），但本地完整版
调试环境被破坏。

**修复**：
- `release-mas.sh` 固定 `CARGO_TARGET_DIR=~/.cargo/shared-target-mas`
- `.gitignore` 加 `target-mas/`
- 门禁 `MasTargetIsolationTests` 断言脚本里有独立 target、路径不来自
  共享 target、目录被忽略 —— 不靠人记着

## 已知会被审核挑的点（已处理）

- `LSRequiresCarbon`：Tauri 生成的 Info.plist 带着，Carbon 早已移除 → 已删
- `NSAppleEventsUsageDescription`：MAS 不发 AppleEvent，与实际能力不符 → 已删
- `temporary-exception.*`：MAS 不支持该键 → entitlements 里没有

## 尚未确认的审核风险

- **PrivacyInfo 的 required-reason 是从代码用途推断的**：
  `FileTimestamp / C617.1`（判扫描快照过期）、`DiskSpace / E174.1`（磁盘图）。
  ASC 会逐条核对**实际使用**，猜错直接拒审。建议沙箱跑通后用 `fs_usage`
  确认真的调用了那些 API。
- **MAS 版砍掉了进程管理、Docker、端口占用、pnpm/yarn/go 清理**。
  技术上能过审，但用户下载后会发现功能少一大截，属产品级风险。
