#!/usr/bin/env bash
# 手动对已打包好的 .app / DMG 签名 + notarize
# 用法：./scripts/sign.sh [arm|intel|universal]

set -euo pipefail

TARGET="${1:-arm}"
VERSION="$(python3 -c 'import json, pathlib; print(json.loads(pathlib.Path("src-tauri/tauri.conf.json").read_text(encoding="utf-8"))["version"])')"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "错误: tauri.conf.json 中的 version 无效" >&2
  exit 1
fi
TEAM_ID="5XNDF727Y6"
SIGNING_ID="${APPLE_SIGNING_IDENTITY:-Developer ID Application: Beijing VGO Co;Ltd (${TEAM_ID})}"
PROFILE_NAME="${MACSlim_NOTARY_PROFILE:-macslim-notary}"
ENTITLEMENTS="src-tauri/entitlements.plist"

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
  *) echo "用法: $0 [arm|intel|universal]" >&2; exit 1 ;;
esac

APP_PATH="src-tauri/target/${RUST_TARGET}/release/bundle/macos/MacSlim.app"
DMG_PATH="src-tauri/target/${RUST_TARGET}/release/bundle/dmg/MacSlim_${VERSION}_${ARCH}.dmg"
UPDATER_PATH="src-tauri/target/${RUST_TARGET}/release/bundle/macos/MacSlim.app.tar.gz"
UPDATER_SIG_PATH="${UPDATER_PATH}.sig"
UPDATER_STAMP_PATH="${UPDATER_PATH}.build-stamp.json"
APP_ARCHIVE_PATH="${APP_PATH}.zip"
rm -f "$UPDATER_PATH" "$UPDATER_SIG_PATH" "$UPDATER_STAMP_PATH"

if [ ! -d "$APP_PATH" ]; then
  echo "错误: 找不到 $APP_PATH，请先跑 bun run tauri build --target $RUST_TARGET" >&2
  exit 1
fi

if [ ! -f "$DMG_PATH" ]; then
  echo "错误: 找不到待签名和公证的 DMG" >&2
  exit 1
fi

IDENTITIES="$(security find-identity -v -p codesigning)"
if [[ "$IDENTITIES" != *"$SIGNING_ID"* ]]; then
  echo "错误: Keychain 中未找到签名证书 $SIGNING_ID" >&2
  exit 1
fi

trap 'echo "错误: Apple notary 凭证预检失败，请检查本机 Keychain" >&2' ERR
xcrun notarytool history --keychain-profile "$PROFILE_NAME" >/dev/null 2>&1
trap - ERR

TIMESTAMP_FLAG="--timestamp"

echo "==> 签名 .app 内所有二进制..."
for f in "$APP_PATH/Contents/MacOS"/*; do
  if [ -f "$f" ] && [ -x "$f" ]; then
    codesign --force --options runtime $TIMESTAMP_FLAG \
      --entitlements "$ENTITLEMENTS" \
      --sign "$SIGNING_ID" "$f"
  fi
done

echo "==> 签名整个 .app..."
codesign --force --options runtime $TIMESTAMP_FLAG \
  --entitlements "$ENTITLEMENTS" \
  --sign "$SIGNING_ID" "$APP_PATH"

echo "==> 验证签名..."
codesign --verify --deep --strict --verbose=2 "$APP_PATH"

echo "==> 签名 DMG..."
codesign --force $TIMESTAMP_FLAG --sign "$SIGNING_ID" "$DMG_PATH"
codesign --verify --verbose=4 "$DMG_PATH"

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

echo "==> 装订并验证公证票据..."
xcrun stapler staple "$DMG_PATH"
xcrun stapler staple "$APP_PATH"
xcrun stapler validate "$APP_PATH"
xcrun stapler validate "$DMG_PATH"

echo "==> 签名与 Gatekeeper 最终验证..."
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

echo ""
echo "✅ 签名流程结束"
ls -lh "$APP_PATH" "$DMG_PATH" 2>/dev/null | awk '{print "   " $5 "  " $9}'
