#!/usr/bin/env bash
# Mac App Store 打包 + 签名 + 上传。
#
# 用法: ./scripts/release-mas.sh [build|upload|all]
#
#   build   产出并签名 MAS .app + .zip（不上传）
#   upload  上传到 App Store Connect
#   all     两步都做
#
# ## 为什么需要这个脚本：Tauri v2 不再支持 Mac App Store
#
# Tauri 2.10.3 的 `BundleType` 只有 `deb / rpm / appimage / msi / nsis / app / dmg`
# —— **没有 `mas`**。所以 `tauri build --config ... ` 无法产出 App Store 产物，
# 那个 target 在 schema 校验阶段就会被拒：
# `Error ["app","mas"] is not valid under any of the schemas`。
#
# 但 MAS 需要的额外动作全都是标准 macOS 工具，本脚本把它们串起来：
#   1. `tauri build` 产出普通 .app（带 `mas` cargo feature）
#   2. 修掉 MAS 审核会挑的 Info.plist 键
#   3. 复制 PrivacyInfo.xcprivacy（必需，缺了会被拒）
#   4. **先签嵌套可执行文件、再签主程序** —— 顺序反了主签名会被嵌套的
#      签名弄坏（`code object is not signed at all` / `bundle format unrecognized`）
#   5. 挂 provisioning profile 并二次签名（App Store 要求 profile 出现在
#      `Contents/embedded.provisionprofile`）
#   6. `ditto -c -k --keepParent` 打成 zip（**不能用 zip 命令**，它不保
#      symlink 与权限位，altool 会报 `The archive is not in the correct format`）
#   7. `altool --upload-app --type osx`
#
# ## 凭据
#
# 证书与 profile 由 `scripts/create_mas_credentials.py` 通过 App Store Connect
# API 创建。缺任何一样都在这里直接停下并说明，不静默降级。
set -euo pipefail

MODE="${1:-all}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TEAM_ID="5XNDF727Y6"
# 「3rd Party Mac Developer Application」= Apple Distribution 证书在钥匙串里的显示名
SIGN_IDENTITY="${MAS_SIGN_IDENTITY:-3rd Party Mac Developer Application: Beijing VGO Co;Ltd (${TEAM_ID})}"
MAS_KEY="${MAS_KEY:-$HOME/.config/mddock/MacSlim_MAS_key.pem}"
PROFILE="${MAS_PROFILE:-$HOME/.config/mddock/MacSlim_MAS.mobileprovision}"
PRIV="${PRIVACY_MANIFEST:-src-tauri/PrivacyInfo.xcprivacy}"
TARGET="aarch64-apple-darwin"

APP="src-tauri/target/${TARGET}/release/bundle/macos/MacSlim.app"
ZIP_DIST="dist/mas"
ZIP_PATH="${ZIP_DIST}/MacSlim-1.0.0-mas.zip"

# MAS 凭据里的 API key
ENV_FILE="${MAS_ASC_ENV:-$HOME/.config/mddock/ios-release.env}"

info()  { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }
fail()  { printf '\033[1;31m错误: %s\033[0m\n' "$*" >&2; exit 1; }
warn()  { printf '\033[1;33m注意: %s\033[0m\n' "$*"; }

preflight() {
  info "发布前检查"
  security find-identity -v -p codesigning | grep -q "$SIGN_IDENTITY" \
    || fail "钥匙串里没有签名身份：$SIGN_IDENTITY
    先跑 python3 scripts/create_mas_credentials.py"
  [ -f "$PROFILE" ] || fail "找不到 provisioning profile：$PROFILE
    先跑 python3 scripts/create_mas_credentials.py"
  [ -f "$PRIV" ] || fail "找不到 $PRIV —— App Store 要求隐私清单"
  echo "  签名身份: $SIGN_IDENTITY"
  echo "  profile:   $PROFILE"
  echo "  隐私清单:  $PRIV"
}

build() {
  info "构建 .app（mas feature）"
  # MAS 不该有自更新，显式清空以免 Tauri 尝试签 updater 产物
  export TAURI_SIGNING_PRIVATE_KEY="" TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
  bunx tauri build --target "$TARGET" \
    --config src-tauri/tauri.mas.conf.json --features mas

  [ -d "$APP" ] || fail "构建没产出 $APP"
  echo "  产物: $APP ($(du -sh "$APP" | cut -f1))"
}

sanitize_info_plist() {
  # MAS 会挑的键，逐个说明见下方注释
  info "修整 Info.plist"
  local plist="$APP/Contents/Info.plist"

  # LSRequiresCarbon=true：Carbon 早已移除，MAS 审核明确会拒。Tauri 生成的
  # Info.plist 带着它，我们直接删掉而不是设 false（留着键本身就没意义）。
  if /usr/libexec/PlistBuddy -c "Print :LSRequiresCarbon" "$plist" >/dev/null 2>&1; then
    /usr/libexec/PlistBuddy -c "Delete :LSRequiresCarbon" "$plist"
    echo "  删除 LSRequiresCarbon（Carbon 已废弃，MAS 会拒）"
  fi

  # NSAppleEventsUsageDescription 只服务于 cache_cleaner 的 osascript 优雅退出，
  # 而 MAS 版不走那条路（沙箱不能发 AppleEvent，entitlements 里也没有
  # automation.apple-events）。留着会与实际能力不符。
  if /usr/libexec/PlistBuddy -c "Print :NSAppleEventsUsageDescription" "$plist" >/dev/null 2>&1; then
    /usr/libexec/PlistBuddy -c "Delete :NSAppleEventsUsageDescription" "$plist"
    echo "  删除 NSAppleEventsUsageDescription（MAS 不发 AppleEvent）"
  fi
}

install_privacy_manifest() {
  info "安装 PrivacyInfo.xcprivacy"
  mkdir -p "$APP/Contents/Resources"
  cp "$PRIV" "$APP/Contents/Resources/PrivacyInfo.xcprivacy"
  echo "  已复制到 Contents/Resources/"
}

sign() {
  info "签名（App Store 证书 + 沙箱 entitlements + profile）"
  local identity="$SIGN_IDENTITY"

  # 顺序：**从里往外**。App Store 的校验是递归的，主程序签名会把整个
  # bundle 的哈希固化下来；若先签主程序再签嵌套，主签名立刻失效
  # （`code object is not signed at all in subcomponent`）。
  # 用 --deep 是给 Tauri 这类多可执行文件 bundle 的常规做法，但会丢掉
  # 嵌套各自的 entitlements —— 这里两个可执行文件都不需要额外 entitlement，
  # 所以 --deep 是安全的。
  codesign --force --deep --sign "$identity" \
    --options runtime \
    --timestamp=none \
    --entitlements src-tauri/entitlements.mas.plist \
    "$APP"

  # profile 必须出现在 bundle 里（App Store 校验会找它）
  cp "$PROFILE" "$APP/Contents/embedded.provisionprofile"
  # 复制 profile 改了 bundle 内容 → 必须**重签一次**
  codesign --force --deep --sign "$identity" \
    --options runtime \
    --timestamp=none \
    --entitlements src-tauri/entitlements.mas.plist \
    "$APP"

  echo "  签名完成"
}

verify() {
  info "签名自检"
  local out
  out="$(codesign -dvvv "$APP" 2>&1)"
  echo "$out" | grep -E "Identifier=|TeamIdentifier=|flags=" | sed 's/^/  /'

  echo "$out" | grep -q "$TEAM_ID" || fail "签名里没有 TeamIdentifier=$TEAM_ID"
  codesign --verify --deep --strict --verbose=2 "$APP" 2>&1 | sed 's/^/  /' \
    || fail "codesign --verify 未通过"

  # 沙箱必须真的开着 —— 这是 MAS 最核心的一条，漏了会在上传后被拒
  local ent
  ent="$(codesign -d --entitlements - --xml "$APP" 2>/dev/null | plutil -convert xml1 -o - - 2>/dev/null)"
  echo "$ent" | grep -q "com.apple.security.app-sandbox" \
    || fail "entitlements 里没有 app-sandbox —— MAS 强制要求"
  echo "  app-sandbox: 已启用"
  echo "$ent" | grep -q "temporary-exception" \
    && fail "entitlements 含 temporary-exception（MAS 不支持这个键）"
  echo "  temporary-exception: 无"

  [ -f "$APP/Contents/Resources/PrivacyInfo.xcprivacy" ] \
    || fail "缺少 PrivacyInfo.xcprivacy"
  echo "  PrivacyInfo: 已就位"
  [ -f "$APP/Contents/embedded.provisionprofile" ] \
    || fail "缺少 embedded.provisionprofile"
  echo "  provisioning profile: 已就位"

  # 体积门槛：App Store 对下载包有上限，超了要等 Apple 批
  local size
  size="$(du -sm "$APP" | cut -f1)"
  echo "  体积: ${size} MB"
}

archive() {
  info "打包 zip"
  mkdir -p "$ZIP_DIST"
  rm -f "$ZIP_PATH"
  # 必须用 ditto 而不是 zip：zip 不保 symlink 与权限位，
  # altool 会报 "The archive is not in the correct format"
  ditto -c -k --keepParent "$APP" "$ZIP_PATH"
  echo "  $ZIP_PATH ($(du -h "$ZIP_PATH" | cut -f1))"
}

upload() {
  info "上传 App Store Connect"
  [ -f "$ENV_PATH" ] || :
  local ENV_PATH="$ENV_FILE"
  [ -f "$ENV_PATH" ] || fail "找不到 ASC 凭据：$ENV_PATH"
  set -a; . "$ENV_PATH"; set +a
  export APPLE_API_KEY_PATH="${APPLE_API_KEY_PATH/#\$HOME/$HOME}"
  export API_PRIVATE_KEYS_DIR="$(dirname "$APPLE_API_KEY_PATH")"

  xcrun altool --upload-app --type osx \
    --file "$ZIP_PATH" \
    --apiKey "$APPLE_API_KEY" \
    --apiIssuer "$APPLE_API_ISSUER"
}

case "$MODE" in
  build)  preflight; build; sanitize_info_plist; install_privacy_manifest; sign; verify; archive ;;
  upload) upload ;;
  all)    preflight; build; sanitize_info_plist; install_privacy_manifest; sign; verify; archive; upload ;;
  *)     fail "用法: $0 [build|upload|all]" ;;
esac
