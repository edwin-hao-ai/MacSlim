# MacSlim 发布方案（双版本路线）

日期：2026-09-27
关联：`docs/release-identity-and-asc.md`（身份与 ASC 对接）、`AGENTS.md` §8.5（签名与公证命令）

## 决策记录（2026-09-27，用户已定）

| 决策 | 内容 |
| :--- | :--- |
| **双版本都要发** | 理由：**不��� MAS 推广渠道太难**。Mac App Store 提供自然流量与「上架 App Store」的品牌背书，Developer ID 提供完整功能。两者都发 |
| **首发版本号** | **1.0.0** |
| **MAS 版定位** | 不是「残废版」，而是**「检测 → 报告 → 教用户自己动手」**：照常扫描、给出每项的具体清理命令、一键复制、清理前后对比报告。全是显示文本 + 剪贴板，沙箱完全允许。**不追求执行能力** |
| **MAS 版不做的** | 进程终止、Docker、CLI、自更新、定时清理、代执行任何清理 |
| **Developer ID 版** | 全功能，范围不变 |

### MAS 版 16 条命令的处置（已按实际代码核对）

| 命令 | MAS | 原因 |
| :--- | :--- | :--- |
| `get_system_health` / `scan_all` / `list_all_processes` / `list_applications` / `check_app_running` / `get_history` | ✅ | 只读或在 app 容器内 |
| `get/add/remove_whitelist` | ✅ | 存储在容器内（但进程管理没了则意义有限） |
| `scan_cache` / `scan_installed_apps` / `scan_app_residues_batch` | ✅ | 扫描是只读 |
| `prepare_operation` / `execute_operation` | ➖ | MAS 版**移除**这两个命令（只读报告不需要执行通道）。需按版本裁剪 `invoke_handler` |
| `docker_available` / `docker_inventory` | ❌ | 沙箱禁止执行容器外 `docker` 二进制 |

**即：16 → 13 条**，且 13 条全是只读/扫描/存储类，语义上比「保留执行通道但按 kind 拦」更干净——**MAS 版根本没有执行通道**，从架构上杜绝了审核风险。

### 页面处置

| 页面 | MAS | Developer ID |
| :--- | :--- | :--- |
| 智能扫描 | ✅ 报告 + 清理指导 | ✅ 全功能 |
| 进程管理 | ❌ 移除 | ✅ |
| 应用程序 | ❌ 移除（只保留优雅退出也不做，避免残留执行路径） | ✅ |
| 缓存清理 | 🔄 改为「扫描 + 指导 + 复制命令 + 前后对比」 | ✅ 全功能 |
| 应用卸载 | 🔄 改为「报告残留 + 给出手动删除路径」 | ✅ 全功能 |
| 历史记录 | ✅ | ✅ |
| 设置 | ➖ 去掉自动启动与更新 | ✅ |

**关键设计原则**：MAS 版**不存在 `execute_operation` 这条命令**，所以不存在「用户点了没反应」或「审核时被现场演示杀进程」的场景。这比「保留命令但运行时拒绝」安全得多。

### 由此产生的工程影响（需在实现前处理）

1. `src-tauri/src/lib.rs` 的 `invoke_handler` 要按版本分叉（16 → 13）
2. `scripts/operation_surface_checks.py` 现在钉死 16 条，需改为**按版本分别校验**
3. `src/lib/tauri.ts` 调用不存在的命令要优雅降级，不能白屏
4. `capabilities/default.json` MAS 版去掉 `autostart:*`、`updater:*`、`process:allow-restart`
5. MAS 版 `Cargo.toml` 去掉 `tauri-plugin-updater` / `tauri-plugin-autostart` / `tauri-plugin-process`
6. 新增 `PrivacyInfo.xcprivacy`（`cache_scanner.rs` 的 `metadata.accessed()` 属受限 API）
7. 「清理指导」文案需要一份**命令映射表**（缓存类别 → 建议执行的 shell 命令），这是 MAS 版的核心新增内容

---

## 0. 结论先行

| 渠道 | 现状 | 建议 |
| :--- | :--- | :--- |
| **Developer ID + 公证 + GitHub Releases** | **约 90% 已就绪** | 先发 |
| **Mac App Store（只读报告版）** | 需按版本裁剪 + 新增「清理指导」内容 + 截图元数据 | 后置，与 Developer ID 版并行推进 |

**理由**：Developer ID 路线不需要沙箱改造，链路已经写好且刚加固过。MAS 版定位为获客与信任背书渠道，功能上刻意做「只读报告 + 教学」，以最低审核风险换取上架资格。

---

## 1. 现在缺什么（发布前必须补齐）

### 1.1 两个渠道共同缺

| 缺项 | 状态 | 影响 | 谁来做 |
| :--- | :--- | :--- | :--- |
| **隐私政策页面** | `docs/index.html` 只有一句「100% 隐私」的宣传语，**没有独立的隐私政策页** | **ASC 硬性要求** `PrivacyPolicyUrl`；Developer ID 版也应有 | 我起草，需你确认内容与 hosting 位置 |
| **updater 端点目录** | `docs/updates/` **不存在** | 自更新会失败 | 我建目录 + 首次 `latest.json` |
| **版本号决策** | 当前 `0.2.2` | 首次公开发布建议 `1.0.0`（语义化：0.x 表示未稳定） | 你定 |
| **e2e 门禁** | 21 项清单已设计，未实现 | 没有它就没有 go/no-go 依据 | 我实现 |
| **安装包体积** | `MacSlim.app.tar.gz` = 32.1 MB（debug）；AGENTS.md 目标 <15 MB（release） | 超标需解释或优化 | release 构建后实测 |

### 1.2 只有 Developer ID 版缺

| 缺项 | 说明 |
| :--- | :--- |
| `TAURI_SIGNING_PRIVATE_KEY` | updater 产物签名用。之前 release 时从环境读取，仓库里没有（也不该有） |
| 真实公证执行 | 公证需 2–5 分钟网络，须看到 `Notarizing Finished with status Accepted` |

### 1.3 只有 MAS 版缺（工作量在这）

| 缺项 | 说明 | 量级 |
| :--- | :--- | :--- |
| 沙箱 entitlements | 加 `com.apple.security.app-sandbox`，**删** `allow-jit` | 小 |
| **文件访问层重写** | `~/Library/Caches`、`~/.npm`、`~/.cargo`、`~/.Trash`、`/Applications/*.app` 全在容器外，必须改为 `NSOpenPanel` 逐项授权 | **大** |
| **外部执行层移除** | `cache_scanner.rs` 5 处 `Command::new`（npm/brew/docker 检测），沙箱禁止执行容器外二进制 | **大** |
| 进程管理整块剔除 | 沙箱 + 审核双重不可行 | 中 |
| 自更新整块剔除 | MAS 禁止；`tauri-plugin-updater` 与 `release.sh` 的 updater 段要在 MAS 构建里跳过 | 中 |
| `PrivacyInfo.xcprivacy` | `cache_scanner.rs` 用了 `metadata.accessed()`，属受限 API 类别 `NSPrivacyAccessedAPICategoryFileTimestamp`，必须声明理由 | 小 |
| 编译期裁剪机制 | 用 cargo feature 让 16 条 `invoke` 命令按版本分叉；`operation_surface_checks.py` 需改为按版本分别校验 | **中，且影响现有门禁** |
| App Store 截图 | 需 `APP_MAC_16_10`（16:10）与 `APP_DESKTOP`（1:1）两套尺寸 | 中 |
| ASC 元数据 | 多 locale 文案（描述/关键词/宣传语/支持 URL），`whatsNew` 首版不能填 | 小 |

---

## 2. Developer ID 版：发布清单

### 阶段 A：发布前（我可以直接做）

1. **e2e 门禁**（21 项）—— go/no-go 依据
2. **起草隐私政策页** `docs/privacy.html` + 官网入口（需你确认内容）
3. **建 updater 端点** `docs/updates/` + 确认 `latest.json` 生成链路
4. **release 构建实测体积**，对照 <15 MB 目标
5. **版本号决策**（需你定）
6. 更新 README / 官网版本号与下载链接

### 阶段 B：真实执行（需你参与，涉密凭据）

```bash
APPLE_ID="120298858@qq.com" \
APPLE_PASSWORD="<App 专用密码>" \
APPLE_TEAM_ID="5XNDF727Y6" \
APPLE_SIGNING_IDENTITY="Developer ID Application: Beijing VGO Co;Ltd (5XNDF727Y6)" \
TAURI_SIGNING_PRIVATE_KEY="<updater 私钥>" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="<口令>" \
npm run bundle:arm
```

判定：`Notarizing Finished with status Accepted`。

> ⚠️ **凭据纪律**：`AGENTS.md` 记录的那个 App 专用密码 fallback 已在 P0 阶段从仓库移除，但**历史值仍需你去 Apple 后台撤销/轮换**。上面命令里的密码请用新值，不要用旧的。

随后：

```bash
gh release upload v<版本> "src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/MacSlim_<版本>_aarch64.dmg" --clobber --repo edwin-hao-ai/MacSlim
```

（注意：你的构建 target dir 被配置到 `~/.cargo/shared-target`，路径与 `AGENTS.md` 写的 `src-tauri/target` 不同，实测时以实际输出为准。）

### 阶段 C：发布后

- 官网下载链接更新 → GitHub Pages
- 首次 updater 推送验证
- 观察崩溃/反馈

---

## 3. MAS 版：发布清单（前置依赖见 §1.3）

### 阶段 A：设计（需先走一轮设计与实现计划）

**最关键的未决设计问题**：编译期裁剪的边界。

需要先定：16 条 `invoke` 命令里，哪些在 MAS 构建里**根本不存在**（而不是调用时报错）。这会同时影响：

- `lib.rs` 的 `invoke_handler` 注册
- `capabilities/default.json`（MAS 版不需要 `autostart`、`updater`、`process:allow-restart`）
- `scripts/operation_surface_checks.py`（现在钉死 16 条，需按版本分别校验）
- 前端 `src/lib/tauri.ts`（调用不存在的命令要优雅降级，不能白屏）
- `Operations` 模块的 `Application` 页面（MAS 版的「一键优化」还剩什么？）

### 阶段 B：沙箱化实现

### 阶段 C：ASC 上传

```bash
# 1. 验凭证（复用 iOS 那套 .p8）
python3 /tmp/macslim-asc-check.py

# 2. 上传二进制（不走 API，走 altool）
export API_PRIVATE_KEYS_DIR="$(dirname "$APPLE_API_KEY_PATH")"
xcrun altool --upload-app --type osx --file <.app 或 .pkg 或 .zip> \
  --apiKey "$APPLE_API_KEY" --apiIssuer "$APPLE_API_ISSUER"

# 3. 元数据 + 截图走三段式 API
#    /v1/appStoreVersions (platform=MAC_OS)
#    /v1/appStoreVersionLocalizations
#    /v1/appScreenshotSets → /v1/appScreenshots → PATCH uploaded=true

# 4. 网页确认 build 处理完成（5–20 分钟）→ 选 build → 提交审核
```

**两个坑**（mddock 实测）：
- `/v1/builds` **没有** `filter[bundleId]`，必须先 `/v1/apps?filter[bundleId]=` 拿 app id 再查
- 国内代理会把该域名劫持到 `198.18.x.x` fake-IP 导致 TLS 失败，必须设 DIRECT 或关代理。判活：`curl -I https://api.appstoreconnect.apple.com/v1/apps` 返回 **401** 即正常

---

## 4. 推荐执行顺序

```
现在
 │
 ├─ 1. e2e 门禁（21 项）                    ← 我做，发布的地基
 ├─ 2. 隐私政策页 + updater 端点            ← 我做（内容需你确认）
 ├─ 3. 版本号决策                            ← 你定
 ├─ 4. 真实签名 + 公证 + 上传 GitHub Release ← 你执行（涉密凭据）
 │
 ├─ 5. MAS 裁剪边界设计（走 brainstorming）  ← 需先拿到 4 的真实反馈
 ├─ 6. 沙箱化实现 + App Store 打包
 └─ 7. ASC 上传 + 截图 + 提交审核
```

## 5. 需要你现在决定的 4 件事

1. **版本号**：首发用 `1.0.0` 还是沿用 `0.2.2`？
2. **隐私政策内容**：我起草，你确认。至少要覆盖：本地处理不外传、进程/文件的读取范围、崩溃日志是否收集、联系方式
3. **Apple 专用密码**：旧的必须撤销换新的，发布用哪个？
4. **MAS 版功能边界**：是否接受「只读体检 + 手动选中单个 app 卸载」这个缩水版？
