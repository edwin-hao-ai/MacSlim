# MacSlim 发布身份与 App Store Connect 对接

日期：2026-09-27
状态：已记录，待发布阶段执行

---

## 1. 已确认的发布身份

| 项 | 值 | 来源 |
| :--- | :--- | :--- |
| App ID Prefix（Team ID） | `5XNDF727Y6` | 用户提供；`APPLE_TEAM_ID` 实测一致 |
| Bundle ID | `com.vgoapp.macslim` | 用户提供；**已写入 `src-tauri/tauri.conf.json`** |
| ASC App 名称 | MacSlim | ASC 实测 |
| ASC App ID | `6816609940` | ASC 实测（`GET /v1/apps?filter[bundleId]=com.vgoapp.macslim`） |
| ASC 平台 | **`MAC_OS`** | ASC 实测（`appStoreVersions.platform`），确认是 Mac 应用而非 iOS |
| 已建版本 | `1.0`，状态 `PREPARE_FOR_SUBMISSION` | ASC 实测 |
| 签名证书（Developer ID 路线） | `Developer ID Application: Beijing VGO Co;Ltd (5XNDF727Y6)` | `AGENTS.md` §8.5 |

### 1.1 ASC 连通性实测（2026-09-27，只读）

复用 mddock 项目的 iOS 凭证（`~/.config/mddock/ios-release.env`）验证通过：

- ES256 JWT 认证成功，该 Key 可见 **19 个 App**
- `filter[bundleId]=com.vgoapp.macslim` 精确命中 1 条
- 私钥文件存在；Key ID / Issuer ID **未打印、未落盘到本仓库**

### 1.2 Bundle ID 变更记录

`com.macslim.desktop` → `com.vgoapp.macslim`，共 9 处：

- `src-tauri/tauri.conf.json:5`
- `scripts/tests/test_updater_artifact.py` 8 处（updater 的 identifier 校验夹具，必须与真实 ID 一致，否则「标识符不匹配」用例语义会失效）

**无需数据迁移**：`~/Library/Application Support/com.macslim.desktop` 不存在（该 app 从未落过盘），所以没有历史记录 / 白名单 / 设置需要搬迁。

验证：`bun run verify` 退出 0。

---

## 2. 路线决定（用户已定）

**双版本路线**：同一份代码，编译期分出两套构建。

| | MAS 版 | Developer ID 版 |
| :--- | :--- | :--- |
| 分发渠道 | Mac App Store（走 ASC 上传，`appId=6816609940`） | 官网 / GitHub Releases 下载 |
| 签名 | `com.apple.security.app-sandbox` + App Store 证书 | `Developer ID Application: Beijing VGO Co;Ltd (5XNDF727Y6)` + hardened runtime + 公证 |
| 功能 | 沙箱允许范围内（体检、用户手动选中单个 app 卸载、优雅退出） | 全功能（进程管理、开发缓存、Docker、完整卸载、CLI） |
| 自更新 | **禁止**，必须剔除 `tauri-plugin-updater` | 保留现有 updater 链路 |
| 裁剪方式 | 编译期（cargo feature / tauri 配置），不是运行时开关 | 默认全开 |

**技术约束（仍然有效，实现时必须带上）**：

1. 两套签名配置互斥，需要分开配置
2. 沙箱外不可为：`~/Library/Caches`、`~/.npm`、`~/.cargo`、`~/.Trash`、`/Applications/*.app` 的删除，任意 PID 的信号，执行 npm/brew/docker CLI
3. MAS 禁止自更新 → `tauri-plugin-updater` 与 `release.sh` 的 updater 产物在 MAS 构建里整块剔除
4. **公证 ≠ ASC 上架**：MAS 走 ASC 上传，不需要 `notarytool` / `stapler`
5. `cache_scanner.rs` 使用 `metadata.accessed()`，属 Apple 受限 API 类别 `NSPrivacyAccessedAPICategoryFileTimestamp`，MAS 需 `PrivacyInfo.xcprivacy` 声明理由
6. **ASC 只接受沙箱构建**——不存在「全功能版走 ASC 上架」的路径，这是选双版本的直接原因

**待设计**（Plan A 完成后单独走一轮设计与实现）：编译期裁剪的具体边界——哪些 `invoke` 命令在 MAS 构建里不存在、capability 如何分叉、`invoke_handler` 的 16 条命令如何按版本裁剪、静态门禁（`operation_surface_checks.py` 钉死 16 条）如何改为按版本分别校验。**这是本路线最大的未决问题。**

---

## 3. ASC API 连接方式（源自 mddock 项目，实测可用）

### 3.1 认证

| 项 | 值 |
| :--- | :--- |
| Base URL | `https://api.appstoreconnect.apple.com` |
| 认证 | JWT，**ES256（ECDSA P-256）**，用 `.p8` 私钥签名 |
| JWT header | `{"alg":"ES256","kid":"<KEY_ID>","typ":"JWT"}` |
| JWT payload | `{"iss":"<ISSUER_ID>","iat":now-60,"exp":iat+900,"aud":"appstoreconnect-v1"}` |
| Header | `Authorization: Bearer <jwt>` / `Accept: application/json` / `Content-Type: application/json` |
| 签名实现 | `openssl dgst -sha256 -sign <p8>` → DER → 手工解析成 R‖S 各 32 字节 raw → base64url 去 padding |

凭据字段名（`~/.config/mddock/ios-release.env`，mode 0600）：

```
APPLE_API_KEY=<Key ID>
APPLE_API_ISSUER=<Issuer ID, UUID>
APPLE_API_KEY_PATH=$HOME/.config/mddock/AuthKey_<KEY_ID>.p8
APPLE_TEAM_ID=<Team ID>
```

### 3.2 两个必踩的坑（mddock 实测）

1. **网络**：该域名在国内常被代理劫持到 `198.18.x.x` fake-IP 导致 TLS 失败，脚本内的 `no_proxy` 绕不过 DNS 级劫持，必须设为 DIRECT 或关代理。判活：`curl -I https://api.appstoreconnect.apple.com/v1/apps` 返回 **HTTP/2 401** 即正常。
2. **`/v1/builds` 没有 `filter[bundleId]`**。必须两步：`/v1/apps?filter[bundleId]=X` → app id → `/v1/builds?filter[app]=<id>&sort=-version&limit=1`。mddock 的 `docs/PRD-VERIFICATION.md` 与部分 memories 里的 `filter[bundleId]` 写法是**错的**，不要抄。

### 3.3 可复用资产（mddock 项目）

| 路径 | 用途 |
| :--- | :--- |
| `scripts/ios-test-credentials.py` | 110 行，**最干净的 JWT 模板**，改 URL 即可当 macOS 凭证自检 |
| `scripts/ios-app-store-metadata.py` | `ASCClient` 类（JWT+重试）、版本/本地化 CRUD、**截图三段式上传** |
| `scripts/release-ios.sh` | 33KB 端到端参考实现，JWT heredoc 与 `altool` 调用段可直接摘抄 |
| `scripts/ios-metadata.json` | 配置模板结构（appId / bundleId / platform / locales / screenshots） |
| `scripts/release-desktop-mac.sh` | mddock 的 macOS 签名+公证+DMG 流程（Developer ID 路线参考） |

### 3.4 上传一个 macOS 构建的端点序列

| 步骤 | 端点 | 关键参数 |
| :--- | :--- | :--- |
| 0 验凭证 | `GET /v1/apps` | 401 = token 无效；200 = 通 |
| 1 取 app id | `GET /v1/apps` | `filter[bundleId]=com.vgoapp.macslim`；**不能取 `data[0]`**（该 key 可见多个 app） |
| 2 取当前 build | `GET /v1/builds` | `filter[app]=<id>&sort=-version&limit=1` → 算下一个 build 号 |
| 3 二进制上传 | **不走 API，走 `xcrun altool`** | `--upload-app --type osx --file <.app/.pkg/.zip> --apiKey <KeyID> --apiIssuer <Issuer>`；需先 `export API_PRIVATE_KEYS_DIR="$(dirname "$APPLE_API_KEY_PATH")"` |
| 4 版本 | `GET/POST /v1/apps/{appId}/appStoreVersions` | `filter[platform]=MAC_OS`；`versionString` |
| 5 本地化 | `GET/PATCH/POST /v1/appStoreVersions/{id}/appStoreVersionLocalizations` | `description` / `keywords` / `promotionalText` / `supportUrl` / `marketingUrl`；首版不能填 `whatsNew` |
| 6 截图容器 | `GET/POST /v1/appStoreVersionLocalizations/{locId}/appScreenshotSets` | macOS 的 `screenshotDisplayType`：`APP_DESKTOP` / `APP_MAC_16_10` |
| 7 截图上传 | `POST /v1/appScreenshots` → 按返回的 `uploadOperations[]` PUT 到 Apple S3 → `PATCH` 带 `uploaded=true` + `sourceFileChecksum`(md5 hex) | 三段式；重传前先 `DELETE` 清空 set |
| 8 提交审核 | 网页确认 build 处理完成（5–20 分钟）→ 选 build → 提交 | mddock **未自动化**此步 |

---

## 4. MacSlim 上 Mac App Store 的三个硬差异

mddock 项目**没有任何 macOS → App Store 的先例**（其 macOS 版走 Developer ID + 公证，与 ASC 无关）。复用时必须自己补：

1. **两套签名体系互斥**：MAS 要求 `com.apple.security.app-sandbox` entitlement + 沙箱化分发；Developer ID 路线是 `signingIdentity: Developer ID Application: …` + hardened runtime。
2. **`altool --type osx`**（mddock 只用过 `--type ios`），且 **`platform` 枚举是 `MAC_OS`**（不是 `IOS`，也不是 `APP_MAC_CATALYST`）。
3. **公证 ≠ ASC 上架**：`notarytool` / `stapler` 是给站外分发用的，MAS 走 ASC 上传，**不需要**。

### 4.1 ⚠️ 与既有结论的冲突（必须先解决）

`docs/superpowers/mas-feasibility-and-e2e-release-gate.md` 已论证：**MacSlim 的核心功能与 App Sandbox 结构性冲突**（删 `~/Library/Caches`、`~/.npm`、`~/.cargo`、废纸篓、终止任意 PID、执行 npm/brew/docker CLI、自更新，全部在沙箱外或被禁止）。按该分析，MAS 版会只剩「只读体检 + 用户手动选中单个 app 卸载」。

用户现已在 ASC 建好 `com.vgoapp.macslim`，与该结论方向相反。**需要用户明确**：是接受一个功能大幅缩水、且审核风险高的 MAS 版，还是仍以 Developer ID + 公证 + GitHub Releases 为主路线、ASC 记录先留着。

---

## 5. 待办（发布阶段执行）

- [ ] 裁决 bundle ID：是否改为 `com.vgoapp.macslim`，是否写一次性数据迁移
- [ ] 若改 ID：同步 `scripts/tests/test_updater_artifact.py` 的 8 处硬编码
- [ ] 裁决 §4.1 的路线冲突
- [ ] 用 `ios-test-credentials.py` 验证 ASC 凭据（macOS 版）
- [ ] 写 macOS 版的 `app-metadata.json`（多 locale 文案 + 截图清单）
- [ ] 截图：`APP_MAC_16_10` 需要 16:10 比例，`APP_DESKTOP` 需 1:1；产出后走三段式上传
- [ ] `altool --type osx` 上传二进制
- [ ] 打包：MAS 需 `.app`/`.pkg` 沙箱签名；Developer ID 路线沿用 `npm run bundle:arm`

**纪律**：真实 Key ID / Issuer ID / 私钥路径**一律不进 git、不进本文档**（`AGENTS.md` §8.5）。本文档只记录变量名。
