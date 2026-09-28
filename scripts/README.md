# 发布脚本

| 脚本 | 做什么 | 什么时候用 |
| :--- | :--- | :--- |
| `release.sh` | 编译、签名、公证并验证安装包 | 每次要给用户下载新 DMG 时 |
| `sign.sh` | 对现有 `.app` 和 DMG 单独签名、公证并验证 | `release.sh` 的签名步骤失败后重试 |
| `publish-update.sh` | 给 Tauri `.app.tar.gz` 签 Updater 签名 + 生成 manifest | 发布自动更新 |

## 发版工作流

```bash
# 1. 更新版本号
vim src-tauri/Cargo.toml src-tauri/tauri.conf.json package.json
```

```bash
# 2. 运行统一验证并完成 release/sign（二选一）
bun run verify
bun run tauri build --debug --bundles app
./scripts/release.sh arm
# 已有产物仅重试签名、公证时：./scripts/sign.sh arm
```

```bash
# 3. release/sign 成功后，确认当前 target 的 archive 由当前 app rebuild
python3 scripts/updater_artifact.py rebuild --app "src-tauri/target/aarch64-apple-darwin/release/bundle/macos/MacSlim.app" --archive "src-tauri/target/aarch64-apple-darwin/release/bundle/macos/MacSlim.app.tar.gz" --target "aarch64-apple-darwin"
```

```bash
# 4. 仅在版本参数等于 tauri.conf.json 后发布 updater
./scripts/publish-update.sh 0.2.2 "本次更新说明" arm
# 之后才部署 landing/ 到 GitHub Pages
```

## Updater 发布门禁

正式发布必须按以下顺序执行；`release.sh` 或 `sign.sh` 返回成功前不得运行 `publish-update.sh`。两个脚本的成功路径都会从当前已签名、公证并通过 Gatekeeper 验证的 `.app` 调用 `rebuild`；下面命令是独立的完整性确认/重试入口，也是正式 updater 唯一允许的 stamp 生成路径。

```bash
TARGET=arm
RUST_TARGET="aarch64-apple-darwin"
APP_PATH="src-tauri/target/${RUST_TARGET}/release/bundle/macos/MacSlim.app"
ARCHIVE_PATH="src-tauri/target/${RUST_TARGET}/release/bundle/macos/MacSlim.app.tar.gz"
./scripts/release.sh "$TARGET"
# 或：./scripts/sign.sh "$TARGET"（二选一）
python3 scripts/updater_artifact.py rebuild --app "$APP_PATH" --archive "$ARCHIVE_PATH" --target "$RUST_TARGET"
./scripts/publish-update.sh 0.2.2 "本次更新说明" "$TARGET"
```

`publish-update.sh` 的版本参数必须严格等于 `src-tauri/tauri.conf.json` 的 `version`；不匹配必须以非零状态退出。target 只能使用以下映射，不能把一个 target 的 archive、sidecar 或签名复用到另一个 target：

| 发布参数 | Rust target | asset 架构 |
| :--- | :--- | :--- |
| `arm` | `aarch64-apple-darwin` | `aarch64` |
| `intel` | `x86_64-apple-darwin` | `x64` |
| `universal` | `universal-apple-darwin` | `universal` |

| 门禁项目 | 必须满足 |
| :--- | :--- |
| sidecar | 每个 asset 必须携带并校验 `build-stamp.json` sidecar |
| staging | publish 只能复制到临时 staging 后再签名和生成 manifest |
| 事务 | 最后执行 `python3 scripts/publish_assets.py commit` |
| config version | 版本参数必须严格等于 `src-tauri/tauri.conf.json` 的 `version` |
| 禁止旧资产 | 不得手工复制旧 tar、不得手工修改 manifest、不得跨 target 复用 stamp |

正式 updater 的 `.build-stamp.json` 只能由 `rebuild --app` 在重建当前 archive 后内部生成；不得使用公开 stamp CLI/API，不得手工编辑 stamp，也不得跨 target 复用 stamp。Updater 发布对象是版本化的 `MacSlim_<version>_<arch>.app.tar.gz(.sig,.build-stamp.json)`，不是 DMG；新客户端读取 `updates/latest.json`，旧客户端读取对应 platform 下的 `0.1.0.json` 与 `0.2.2.json`。

`publish-update.sh` 必须携带并校验每个 asset 的 `build-stamp.json` sidecar；正式发布先执行只读 preflight，再复制到临时 staging，逐资产重签并生成 manifest，最后通过 `scripts/publish_assets.py commit` 事务提交当前版本文件。不得手工复制旧 tar、不得手工修改 manifest，也不得绕过 sidecar、staging 或事务 commit；失败会恢复旧 live 状态并保留其他版本资产。

自动验证只运行本地静态检查和测试，不执行真实 release、signer、notary、上传或发布命令；正式发布命令只能由操作者在上文门禁全部成功后手动运行。

## 发布前检查

```bash
security find-identity -v -p codesigning
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD
xcrun notarytool store-credentials macslim-notary
```

Updater 私钥默认位于 `~/.tauri/macslim-updater.key`，必须通过环境变量提供密码，不存在默认密码。私钥绝不提交；公钥已嵌入 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。更换公钥需要重新发布所有平台的初始安装包，老用户才能验证新签名。

发布前可执行无副作用预检；该模式只检查 key、密码和 Tauri updater artifact，不签名、不复制文件、不生成 manifest：

```bash
MACSlim_PUBLISH_PREFLIGHT_ONLY=1 ./scripts/publish-update.sh 0.2.2 "预检" arm
```

## Apple 签名

- 证书 Common Name：`Developer ID Application: Beijing VGO Co;Ltd (5XNDF727Y6)`
- Team ID：`5XNDF727Y6`
- 默认 Keychain profile：`macslim-notary`

`release.sh` 与 `sign.sh` 会分别使用默认 profile `macslim-notary` 提交 `.app` 的 `ditto` ZIP 归档和 DMG；两项均成功后才装订票据并执行最终验证：

```bash
codesign --verify --deep --strict --verbose=2 "$APP_PATH"
SIGNATURE_DETAILS="$(codesign -dvvv "$APP_PATH" 2>&1)"
if [[ "$SIGNATURE_DETAILS" != *"Timestamp="* ]]; then exit 1; fi
codesign --verify --verbose=4 "$DMG_PATH"
xcrun stapler validate "$APP_PATH"
xcrun stapler validate "$DMG_PATH"
spctl -a -t exec -vv "$APP_PATH"
spctl -a -t install -vv "$DMG_PATH"
```
