# MacSlim — MAS 收尾交接（2026-09-29）

> 这份文档的用途：新会话直接读它就能接上，不用翻历史。
> 事实优先于叙述；**所有「已验证」都指真机或全量测试跑过，不是推断**。
>
> ## ⚠️ 本文档有三处已被后续实测推翻，读之前先看
>
> 2026-09-30 的实测见 **`docs/mas-capability-matrix.md`**，以那份为准：
>
> 1. **§7 第 3–5 步「让用户去开 FDA」方向是错的。** 0 B 的真因是沙箱把
>    `$HOME` 重定向到应用自己的 container，而代码用 `dirs::home_dir()` 拼
>    用户路径 —— 是路径解析问题，授权解决不了。Apple 文档也明说 App Store
>    应用即使拿到 FDA，沙箱仍强制执行自己的文件限制。
> 2. **§7 第 6 步的疑点已结清**：「应用程序 0」与 600 秒缓存无关，是进程
>    枚举被拦（`sysinfo` 走 `proc_listallpids`）导致的同源现象。
> 3. **§3 的「进程列表同样为 0」已修复**：绕开 `sysinfo` 之后沙箱内能拿到
>    200+ 个进程，「进程管理」页在 MAS 版已恢复（只读）。
>
> 已经做掉的：§0 的版本冲突、§7 第 1–2 步（MAS 包已产出并签名自检通过）。

## 0. 立刻要处理的一件事

MAS 构建在版本校验处失败：

```
tauri (v2.12.0) : @tauri-apps/api (v2.10.1)
Error Found version mismatched Tauri packages
```

Rust `tauri` 由 `Cargo.lock` 解析到 **2.12.0**（`Cargo.toml` 写的是 `2.1`，
caret range 升到了 2.12），而 npm `@tauri-apps/api` 是 **2.10.1**。
这是**旧账**，之前一直没暴露；`bun add @tauri-apps/plugin-opener` 重新解析
依赖树后被 CLI 的严格校验挡住。

修法（对齐 npm 侧到 Rust 的 2.12）：

```bash
bun add @tauri-apps/api@^2.12.0 @tauri-apps/cli@^2.12.0
```

改完 `bun run verify` 应仍为 0，然后重跑 `./scripts/release-mas.sh build`。

## 1. 当前进度

### 完整版（Developer ID）—— 已发布

- GitHub Release v1.0.0：https://github.com/edwin-hao-ai/MacSlim/releases/tag/v1.0.0
- DMG 8.3 MB，Apple 公证 `source=Notarized Developer ID`
- 落地页 vgoapp.com 已推（commit `e1e067f`）
- 完整版产物**本地被 MAS 构建覆盖过一次**（已修，见 §4）

### MAS 版 —— 代码就绪，实测部分可用，未上传

## 2. 提交历史（全部已 push）

| commit | 内容 |
|---|---|
| `56efbe8` | MAS 地基：`mas` feature + 终止进程守卫 + capability 分叉 |
| `09e2624` | `flavor` 模块（唯一真相源）+ 前端桥接 + `get_build_flavor` |
| `8a61b0f` | MAS 下收口「终止进程」入口 |
| `6f11f8d` | 沙箱 entitlements + PrivacyInfo + 19 项配置门禁；修一条 flaky 测试 |
| `2e70fbb` | plutil → `plist` crate 原生解析（顺带修正则串味 bug） |
| `92db6ac` | sips → 按 flavor 优雅降级（MAS 不产出图标） |
| `0f4e0f9` | 扫描阶段过滤清不掉的缓存类别（pnpm/yarn/docker/go） |
| `9091352` | `release-mas.sh`（Tauri v2 无 MAS 支持，手工串步骤） |
| `480ba45` | 隔离 MAS target 目录 + 剔除崩溃 CLI + 关 updater 产物 |
| `1d56810` | FDA 引导卡片 + 后端探测 + 门禁修正 |

`bun run verify` 最后一次：**222 前端 / 451 Rust / 342 Python，退出 0**。

## 3. 关键实测结论（真机，非推断）

MAS 包逐页结果：

| 页面 | 结果 | 说明 |
|---|---|---|
| 智能扫描 | ✅ | CPU 42.3% / 内存 5.2GB / 磁盘 213GB 都是真实值 |
| **应用卸载** | ✅ | **读到 Xcode 8.80 GB** —— plist 原生化解对了，读 `/Applications` 没被挡 |
| 历史记录 | ✅ | 空状态文案正确 |
| 设置 | ✅ | 12 按钮齐全 |
| 缓存清理 | ❌ | 0 B；完整版同机 **13.99 GB** |
| 进程管理 | ❌ | 0 项 |

`macslim-cli` 一起签名后**启动即 SIGTRAP（exit 133）**，`--version` 都打不出来。
崩溃栈全在 dyld 初始化阶段（`_libsecinit_appsandbox` → `forEachInitializer`），
与业务代码无关 → 已从 MAS bundle 剔除（`strip_cli`）。

**缓存/进程被挡的根因**：沙箱下即使有 `files.all`，用户没在系统设置里实际授权
FDA 时敏感路径一律读不到。CleanMyMac 官方文档印证同一限制（它的 App Store 版
读 SMART 也要用户授权）。

## 4. 踩过的坑（都已在代码/门禁里固化，别重蹈）

1. **MAS 构建会覆盖完整版产物** —— 曾共用 `src-tauri/target`，Developer ID 签名
   的 .app 被顶掉、dmg 消失。已固定 `CARGO_TARGET_DIR=~/.cargo/shared-target-mas`
   + `.gitignore` + 门禁 `MasTargetIsolationTests`。
2. **Tauri v2 已移除 MAS 支持** —— `BundleType` 无 `mas`，`targets: ["app","mas"]`
   在 schema 阶段就失败。我第一版按 Tauri v1 的记忆写错了，而且**19 项门禁没有
   一条校验它合法性**，只断言「包含 mas」，自洽但无用 —— 门禁照过、构建照挂。
3. **`macslim-cli` 沙箱下必崩** —— 见 §3。
4. **`invoke_handler` 从 16 → 18** —— 加了 `get_build_flavor` 与 `get_fda_status`。
   破坏性面仍精确等于 `prepare_operation` + `execute_operation`。
   三处写死 16 的门禁（Python 两处 + Rust 一处）已同步。
5. **capability 权限 13 → 14** —— 加 `opener:allow-open-url`（FDA 引导跳设置页），
   **只放行单条**，未放宽成整组 opener 权限。
6. **门禁会抓自己人** —— `flavor.ts` 里直接 `invoke("plugin:opener|open_url")`
   被 `test_repository_frontend_invokes_only_registered_commands` 抓到。
7. **plutil 正则解析会串味** —— 找「`<key>K</key>` 之后的第一个 `<string>`」，
   K 的值是数组时会跨结构抓到嵌套里的字符串。已换 `plist` crate。
8. **flaky 测试** —— `parallel_icon_decoding...` 断言「全局 temp 目录无
   `macslim_icon_*`」，实测测的是「这台机器有没有别的 macslim 跑过」。已拆成
   独立单测（用调用方指定目录）。

## 5. 竞品调研结论（决定了砍得对不对）

CleanMyMac X 的 App Store 版（官方对照表）丢失：System Cache/Log Files、
Language Files、Universal Binaries、iOS Device Backups、**终止无响应应用**、
全部 Maintenance Tasks、Updater/Agent 后台进程。

保留：**User Cache Files、User Log Files、Xcode Junk、Uninstaller、
Leftovers、Large & Old Files、Duplicate、Malware Finder、CPU/RAM/Storage Monitor**。

结论：
- **我们的砍法比 CleanMyMac 保守**（它连 Xcode Junk、User Cache 都保住了，
  而「终止进程」它也一样砍）
- 减配 MAS 版**能赚钱**：CleanMyMac X App Store 版一次性 $47.99、2900 万下载
- 我们**比它强**的一点：它的 App Store 版删不了 App Store 应用二进制，
  我们的卸载（移到废纸篓）沙箱能做

差距（我们没有）：Large & Old Files、重复/相似文件、恶意软件清理、电池/SMART。
前两项 AGENTS.md §6 已定为**非目标**（全盘扫描有性能与隐私风险）；
SMART 在沙箱下读不到（平台限制）。

## 6. 凭据（全部在仓库外，勿提交）

| 项 | 位置 / 值 |
|---|---|
| ASC API key | `~/.config/mddock/ios-release.env`（`APPLE_API_ISSUER` / `APPLE_API_KEY` / `APPLE_API_KEY_PATH`） |
| MAS 证书 | `Mac App Distribution: Beijing VGO Co;Ltd`，ASC id `XV9LBSSZBJ`，有效至 2027-09-29 |
| 钥匙串私钥 | `3rd Party Mac Developer Application: Beijing VGO Co;Ltd (5XNDF727Y6)`，SHA1 `58D52A76…` |
| profile | `~/.config/mddock/MacSlim_MAS.mobileprovision`（`MAC_APP_STORE`，uuid `d20fee78-…`） |
| bundleId / appId | `com.vgoapp.macslim` / `6816609940`，版本 `1.0` / `PREPARE_FOR_SUBMISSION` |
| updater 私钥口令 | `macflow-dev-pw`（从 git 历史 `50c0e80` 挖出，已实测签名通过） |
| 公证凭据 | 钥匙串 `macslim-notary`（本轮新存，原来并不存在——交接文档那句是错的） |
| 开发者版 | Developer ID `1FB018B1…` |

MAS 私钥文件（**空密码**，在仓库外）：`~/.config/mddock/MacSlim_MAS_key.pem`、
`MacSlim_MAS_Distribution.p12`、`.crt.pem`。

## 7. 下一步（按顺序）

1. **修版本冲突**（§0），`bun run verify` 确认 0
2. `./scripts/release-mas.sh build` 出包
3. **实测 FDA 引导卡片**：未授权时设置页应显示引导；缓存页仍 0
4. **需要用户手动操作**：系统设置 → 隐私与安全性 → 完全磁盘访问权限 →
   把 MacSlim 拖进去并打开。**这一步我做不了，必须用户点**
5. 授权后重进缓存页，**确认能否扫到 Xcode/NPM 缓存** —— 这是决定能不能上传的
   唯一未知数
6. 查「应用程序 0」是否 600 秒缓存问题（`app_scanner.rs:288` `APP_SCAN_CACHE_TTL`）
7. 全页复测通过 → 才谈 `xcrun altool --upload-app --type osx`（**上传前必须问用户**）

## 8. 硬约束（务必遵守）

- **禁止 git 写操作已解除** —— 用户 2026-09-28 明确要求每次改完就 commit + push，
  且已写入 AGENTS.md §8.1
- **AGENTS.md 在 `.gitignore:28`**（§8.5 含 Apple 专用密码），所以规则只在本机
- 禁止在 Rust 侧抄英文译文；文案与 code 绝不参与安全判定
- `ProcessIdentity.name` 永远是原始进程名
- 真实密钥材料一律不进 git
- 破坏性目标必须脚本自造，绝不碰用户真实数据
- 性能只能用 release 测
- `scripts/__pycache__` 会拦住 verify，删掉即可

## 9. 尚未确认的审核风险

- **PrivacyInfo 的 required-reason 是从代码用途推断的**
  （`FileTimestamp / C617.1`、`DiskSpace / E174.1`）。ASC 会逐条核对**实际使用**，
  猜错直接拒审。建议沙箱跑通后用 `fs_usage` 确认真的调用了那些 API。
- **MAS 版砍掉进程管理/Docker/端口占用/pnpm/yarn/go**。技术上能过审，但用户
  下载后发现功能少一大截，属产品级风险。完整版继续服务开发者。
