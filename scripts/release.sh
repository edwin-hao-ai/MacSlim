#!/usr/bin/env bash
# MacSlim release build + notarization 脚本
# 用法: ./scripts/release.sh [arm|intel|universal]

set -euo pipefail

TARGET="${1:-arm}"
VERSION="$(python3 -c 'import json, pathlib; print(json.loads(pathlib.Path("src-tauri/tauri.conf.json").read_text(encoding="utf-8"))["version"])')"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "错误: tauri.conf.json 中的 version 无效" >&2
  exit 1
fi
TEAM_ID="5XNDF727Y6"
SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-Developer ID Application: Beijing VGO Co;Ltd (${TEAM_ID})}"
PROFILE_NAME="${MACSlim_NOTARY_PROFILE:-macslim-notary}"

case "$TARGET" in
  arm)
    RUST_TARGET="aarch64-apple-darwin"
    ARCH="aarch64"
    ;;
  intel)
    RUST_TARGET="x86_64-apple-darwin"
    ARCH="x64"
    ;;
  universal)
    RUST_TARGET="universal-apple-darwin"
    ARCH="universal"
    ;;
  *)
    echo "用法: $0 [arm|intel|universal]" >&2
    exit 1
    ;;
esac

APP_PATH="src-tauri/target/${RUST_TARGET}/release/bundle/macos/MacSlim.app"
DMG_PATH="src-tauri/target/${RUST_TARGET}/release/bundle/dmg/MacSlim_${VERSION}_${ARCH}.dmg"
UPDATER_PATH="src-tauri/target/${RUST_TARGET}/release/bundle/macos/MacSlim.app.tar.gz"
UPDATER_SIG_PATH="${UPDATER_PATH}.sig"
UPDATER_STAMP_PATH="${UPDATER_PATH}.build-stamp.json"
APP_ARCHIVE_PATH="${APP_PATH}.zip"
rm -f "$UPDATER_PATH" "$UPDATER_SIG_PATH" "$UPDATER_STAMP_PATH"

echo "==> 目标架构: $RUST_TARGET"

IDENTITIES="$(security find-identity -v -p codesigning)"
if [[ "$IDENTITIES" != *"$SIGNING_IDENTITY"* ]]; then
  echo "错误: Keychain 中未找到签名证书 $SIGNING_IDENTITY" >&2
  exit 1
fi

trap 'echo "错误: Apple notary 凭证预检失败，请检查本机 Keychain" >&2' ERR
xcrun notarytool history --keychain-profile "$PROFILE_NAME" >/dev/null 2>&1
trap - ERR

echo "==> 签名证书: $SIGNING_IDENTITY"
echo "==> Notary profile: $PROFILE_NAME"
export APPLE_SIGNING_IDENTITY="$SIGNING_IDENTITY"

# 1. Tauri release build (自动用 tauri.conf.json 里配置的签名身份)
bun run tauri build --target "$RUST_TARGET"

if [ ! -d "$APP_PATH" ] || [ ! -f "$DMG_PATH" ]; then
  echo "错误: 构建产物不完整，缺少 .app 或 DMG" >&2
  exit 1
fi

echo "==> 生成的 .app: $APP_PATH"
echo "==> 生成的 DMG: $DMG_PATH"

# 2. 验证签名
echo "==> 验证签名..."
codesign --verify --deep --strict --verbose=2 "$APP_PATH"

# 3. 分别提交 .app 归档和 DMG 到 Apple 公证
echo "==> 打包 .app 用于公证..."
rm -f "$APP_ARCHIVE_PATH"
ditto -c -k --sequesterRsrc --keepParent "$APP_PATH" "$APP_ARCHIVE_PATH"

echo "==> 提交 .app 归档到 Apple 公证..."
xcrun notarytool submit "$APP_ARCHIVE_PATH" \
  --keychain-profile "$PROFILE_NAME" \
  --wait

echo "==> 提交 DMG 到 Apple 公证..."
xcrun notarytool submit "$DMG_PATH" \
  --keychain-profile "$PROFILE_NAME" \
  --wait

# 4. Staple ticket 到 DMG 和 .app
echo "==> 装订并验证公证票据..."
xcrun stapler staple "$DMG_PATH"
xcrun stapler staple "$APP_PATH"
xcrun stapler validate "$APP_PATH"
xcrun stapler validate "$DMG_PATH"

# 5. 最终验证
echo "==> 签名与 Gatekeeper 验证..."
codesign --verify --deep --strict --verbose=2 "$APP_PATH"
SIGNATURE_DETAILS="$(codesign -dvvv "$APP_PATH" 2>&1)"
if [[ "$SIGNATURE_DETAILS" != *"Timestamp="* ]]; then
  echo "错误: .app 签名缺少 secure timestamp" >&2
  exit 1
fi
codesign --verify --verbose=4 "$DMG_PATH"
spctl -a -t exec -vv "$APP_PATH"
spctl -a -t install -vv "$DMG_PATH"
python3 scripts/updater_artifact.py rebuild \
  --app "$APP_PATH" \
  --archive "$UPDATER_PATH" \
  --target "$RUST_TARGET"

# 6. 给重建后的 updater 归档补签
#
# 顺序上这是必须的：脚本开头 `rm -f` 掉了 tauri 自己产出的 `.sig`，随后
# `updater_artifact.py rebuild` 又重新打了 tar 归档 —— 归档内容变了，tauri 那个
# 签名立刻作废。**必须重建完再签，顺序不能反**（先签后重建同样会失效）。
# 漏掉这一步的话，`scripts/publish-update.sh:335` 会报「缺少 updater 签名」，
# 而 release.sh 本身一路绿灯，看起来一切正常。
echo "==> 签署 updater 归档..."
SIGNING_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
if [ -n "$SIGNING_PASSWORD" ] || [ -f "$HOME/.tauri/macflow-updater.key" ]; then
  bunx tauri signer sign "$UPDATER_PATH" >/dev/null
fi
if [ ! -s "$UPDATER_SIG_PATH" ]; then
  echo "错误: updater 归档签名未生成（$UPDATER_SIG_PATH 缺失或为空）" >&2
  echo "      请检查 TAURI_SIGNING_PRIVATE_KEY / TAURI_SIGNING_PRIVATE_KEY_PASSWORD" >&2
  exit 1
fi
echo "    签名: $UPDATER_SIG_PATH"

echo ""
echo "✅ Release 完成"
echo "   DMG: $DMG_PATH"
echo "   .app: $APP_PATH"
ls -lh "$DMG_PATH" | awk '{print "   大小: " $5}'
