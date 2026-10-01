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
#   6. `xcrun productbuild --component` 打成自包含 pkg
#      （**不能**用 ditto 打 zip —— 实测被 ASC 以 90270 拒：Mac App Store
#      现在只认 productbuild 的产物，理由见 archive() 的注释）
#   7. `altool --upload-app --type osx`
#   8. 签名时把 profile 的身份字段合并进 entitlements（少一条就 90886/90230，
#      理由见 sign_entitlements 的注释）
#   9. pkg 用一把**临时钥匙串**里的 installer 私钥签名（用登录钥匙串里那把会
#      挂死，理由见 sign_pkg_with_installer 的注释）
#
# ## 凭据
#
# 证书由钥匙串提供；profile 由 `scripts/create_mas_profile.py` 通过 App Store
# Connect API 签发。缺任何一样都在这里直接停下并说明，不静默降级。
#
# **每次给 entitlements.mas.plist 加/删一条 entitlement，都必须重跑一次
# create_mas_profile.py。** 签名与 profile 是两层，少一层不会报错、只会静默
# 失效 —— 实测踩过：`bookmarks.app-scope` 加进签名后 bookmark 仍然解析失败，
# 查了一圈才发现是 profile 是加之前签的。
set -euo pipefail

MODE="${1:-all}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TEAM_ID="5XNDF727Y6"
# 「3rd Party Mac Developer Application」= Apple Distribution 证书在钥匙串里的显示名
SIGN_IDENTITY="${MAS_SIGN_IDENTITY:-3rd Party Mac Developer Application: Beijing VGO Co;Ltd (${TEAM_ID})}"
MAS_KEY="${MAS_KEY:-$HOME/.config/mddock/MacSlim_MAS_key.pem}"
PROFILE="${MAS_PROFILE:-$HOME/.config/mddock/MacSlim_MAS.mobileprovision}"
ENTITLEMENTS="${MAS_ENTITLEMENTS:-$ROOT/src-tauri/entitlements.mas.plist}"
# installer 证书（pkg 自身签名用，由 scripts/create_mas_installer_cert.py 签发）
INSTALLER_P12="${MAS_INSTALLER_P12:-$HOME/.config/mddock/MacSlim_MAS_installer.p12}"
INSTALLER_P12_PASSWORD="${MAS_INSTALLER_P12_PASSWORD:-$HOME/.config/mddock/MacSlim_MAS_installer_p12_password.txt}"
PRIV="${PRIVACY_MANIFEST:-src-tauri/PrivacyInfo.xcprivacy}"
TARGET="aarch64-apple-darwin"

# MAS 构建**必须**用独立的 target 目录。
#
# 踩过：两个构建共用 `src-tauri/target` 时，MAS 构建会把完整版的 release 产物
# 整个顶掉 —— 实测 Developer ID 签名的 .app 被 MAS 的 3rd Party 版覆盖，
# 连已构建好的 dmg 都没了。完整版是主产品，不能被 MAS 的调试反复破坏。
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cargo/shared-target-mas}"
mkdir -p "$CARGO_TARGET_DIR"

APP="${CARGO_TARGET_DIR}/${TARGET}/release/bundle/macos/MacSlim.app"
ZIP_DIST="dist/mas"

# 商店版本（给用户看的那一串）。从 tauri.conf.json 读而不是硬编码：
# 之前 PKG_PATH 里写死了 1.0.0，版本一改文件名就与产物对不上，而文件名本身
# 又不影响 ASC（ASC 只看包内的 Info.plist），纯属自己骗自己。
#
# 定义必须早于 PKG_PATH 的赋值 —— 那是文件里第一次调用它的地方。
store_version() {
  python3 -c "import json;print(json.load(open('src-tauri/tauri.conf.json'))['version'])"
}

PKG_PATH="${ZIP_DIST}/MacSlim-$(store_version)-mas.pkg"

# MAS 凭据里的 API key
ENV_FILE="${MAS_ASC_ENV:-$HOME/.config/mddock/ios-release.env}"

info()  { printf '\n\033[1;34m==> %s\033[0m\n' "$*"; }
fail()  { printf '\033[1;31m错误: %s\033[0m\n' "$*" >&2; exit 1; }
warn()  { printf '\033[1;33m注意: %s\033[0m\n' "$*"; }

preflight() {
  info "发布前检查"
  security find-identity -v -p codesigning | grep -q "$SIGN_IDENTITY" \
    || fail "钥匙串里没有签名身份：$SIGN_IDENTITY
    先跑 python3 scripts/create_mas_profile.py"
  [ -f "$PROFILE" ] || fail "找不到 provisioning profile：$PROFILE
    先跑 python3 scripts/create_mas_profile.py"
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

strip_cli() {
  # 从 MAS bundle 里删掉嵌套的 macslim-cli。
  #
  # 实测（2026-09-29）：**带沙箱的 macslim-cli 一起签名后启动即 SIGTRAP**，
  # 崩溃栈全在 dyld 初始化阶段：
  #   _libsecinit_appsandbox → _os_activity_initicate_impl → libSystemInitializer
  #   → dyld::MachOAnalyzer::forEachInitializer
  # 连 `--version`（只做 println!）都打不出来，所以与业务代码无关，是
  # 沙箱 profile 在进程启动极早期校验失败后主动 trap。
  #
  # 留着一个必定崩溃的可执行文件有两个坏处：审核阶段可能因此直接拒；
  # 而且它对 App Store 用户**毫无价值** —— 沙箱里 exec 不了外部工具，
  # 而 CLI 的存在意义正是驱动它们。桌面端是自带 CLI 的（不走沙箱）。
  info "剔除嵌套 CLI"
  local cli="$APP/Contents/MacOS/macslim-cli"
  if [ -f "$cli" ]; then
    rm -f "$cli"
    # 注意 `${cli}` 的花括号是必需的，不是风格问题。
    # macOS 自带的 bash 3.2 在 UTF-8 locale 下用 locale 感知的 isalnum()
    # 判断变量名合法性，多字节字符（这里是全角左括号）的字节会被判为
    # 「字母」并吞进变量名 —— 紧跟全角左括号的裸引用于是变成查一个
    # 名字里带三个字节的变量，在 `set -u` 下直接报 `unbound variable`。
    # 实测踩过：MAS 构建卡死在这一行，报
    #   release-mas.sh: line 107: cli?: unbound variable
    echo "  已删除 ${cli}（沙箱下启动即 SIGTRAP，且对 App Store 用户无价值）"
  fi
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

# 签名用的 entitlements：沙箱权限 + profile 里的身份字段。
#
# ## 为什么必须合并，而不是直接用 entitlements.mas.plist
#
# `codesign --entitlements <file>` **只**用你给的那一份，不与
# provisioning profile 合并。而 Xcode 生成的包之所以能用，是因为它在
# 签名时把 profile 的三项身份字段也写进了签名。
#
# 缺了它们的后果不是「沙箱失效」，而是 ASC 直接拒收：
#   90886 signature ... is missing an application identifier but has an
#         application identifier in the provisioning profile
#   90230 Invalid product archive metadata ... product-identifier
#
# 这个 warning 之前被 altool 淹没在输出里，而 upload 模式此前**从未跑通**
# （`local` 之前就引用了未声明的变量），所以没人看到过它。
sign_entitlements() {
  local out="${CARGO_TARGET_DIR}/MacSlim-entitlements.plist"
  local decoded="$CARGO_TARGET_DIR/MacSlim-profile.plist"

  # profile 是 CMS 签名包，先解出明文 plist。
  security cms -D -i "$PROFILE" > "$decoded" 2>/dev/null || true
  [ -s "$decoded" ] || fail "无法解出 profile：$PROFILE"

  # entitlement 的名字里带点号（com.apple.developer.team-identifier）。
  # `plutil -extract` 把点当键路径分隔符，必须转义成 `\.`；PlistBuddy 不按点
  # 分割，用它更省心。这里用 PlistBuddy，读数组也顺带能拿到。
  local team app_id
  team="$(/usr/libexec/PlistBuddy -c "Print :Entitlements:com.apple.developer.team-identifier" "$decoded")"
  app_id="$(/usr/libexec/PlistBuddy -c "Print :Entitlements:com.apple.application-identifier" "$decoded")"
  [ -n "$team" ] && [ -n "$app_id" ] || fail "profile 里读不到身份字段：$PROFILE"

  # 沙箱权限那份做底，再补上身份字段
  cp "$ENTITLEMENTS" "$out"
  local fields=(
    "com.apple.application-identifier|$app_id"
    "com.apple.developer.team-identifier|$team"
  )
  local entry key value
  for entry in "${fields[@]}"; do
    key="${entry%%|*}"
    value="${entry#*|}"
    /usr/libexec/PlistBuddy -c "Delete :$key" "$out" 2>/dev/null || true
    /usr/libexec/PlistBuddy -c "Add :$key string $value" "$out"
  done

  # keychain-access-groups 是数组，逐项搬
  /usr/libexec/PlistBuddy -c "Delete :keychain-access-groups" "$out" 2>/dev/null || true
  /usr/libexec/PlistBuddy -c "Add :keychain-access-groups array" "$out"
  local raw index=1
  raw="$(/usr/libexec/PlistBuddy -c "Print :Entitlements:keychain-access-groups" "$decoded" \
    | tr -d '{}' | tr ',' '\n' | sed 's/^ *//;s/ *$//')"
  for value in $raw; do
    [ -n "$value" ] || continue
    /usr/libexec/PlistBuddy -c "Add :keychain-access-groups:-$index string $value" "$out" || true
    index=$((index + 1))
  done

  echo "$out"
}

sign() {
  info "签名（App Store 证书 + 沙箱 entitlements + profile）"
  local identity="$SIGN_IDENTITY"

  # 顺序：**从里往外**。App Store 的校验是递归的，主程序签名会把整个
  # bundle 的哈希固化下来；若先签主程序再签嵌套，主签名立刻失效
  # （`code object is not signed at all in subcomponent`）。
  # 用 --deep 是给 Tauri 这类多可执行文件 bundle 的常规做法，但会丢掉
  # 签名用的 entitlements 是「沙箱权限 + profile 的身份字段」合并出来的，
  # 理由见 sign_entitlements 的注释。少那三条 ASC 会拒（90886 / 90230）。
  local merged
  merged="$(sign_entitlements)"
  echo "  合并后的 entitlements：$merged"

  # 嵌套各自的 entitlements —— 这里两个可执行文件都不需要额外 entitlement，
  # 所以 --deep 是安全的。
  codesign --force --deep --sign "$identity" \
    --options runtime \
    --timestamp=none \
    --entitlements "$merged" \
    "$APP"

  # profile 必须出现在 bundle 里（App Store 校验会找它）
  cp "$PROFILE" "$APP/Contents/embedded.provisionprofile"
  # 复制 profile 改了 bundle 内容 → 必须**重签一次**
  codesign --force --deep --sign "$identity" \
    --options runtime \
    --timestamp=none \
    --entitlements "$merged" \
    "$APP"

  echo "  签名完成"
}

stamp_build_number() {
  # CFBundleVersion（build 号）必须**每次上传都变大**，而 CFBundleShortVersionString
  # 是给用户看的商店版本，两者要能独立变化。
  #
  # ## 为什么会踩到
  #
  # Tauri 只从 tauri.conf.json 的 `version` 生成**两个**键，于是 MAS 包里
  # CFBundleVersion == CFBundleShortVersionString == 1.0.0。第一次上传成功，
  # 之后每次都是同一个 build 号，ASC 直接挡回：
  #
  #   ENTITY_ERROR.ATTRIBUTE.INVALID.DUPLICATE  (-19232 / -19241)
  #   The bundle version must be higher than the previously uploaded version: '1.0.0'
  #
  # 而 ASC **不会**让同一 build 号覆盖 —— 所以「改了代码想重传」根本做不到，
  # 必须抬号。这是流程性的坑，不是编译错误，build 全绿也照样撞。
  #
  # ## 默认值按日期
  #
  # 默认 1.<YYYYMMDD>：每天自动变大（比人工维护一个递增整数可靠得多 ——
  # 人工递增迟早会忘，忘的那次就是又一次 409），且在数值上确实大于 1.0.0。
  # 同一天要再传时用 MAS_BUILD_NUMBER 显式指定。
  local short="$1"
  local build="${MAS_BUILD_NUMBER:-1.$(date +%Y%m%d)}"
  local plist="$APP/Contents/Info.plist"
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $build" "$plist"
  local got
  got="$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$plist")"
  [ "$got" = "$build" ] || fail "CFBundleVersion 写入失败（期望 ${build}，实得 ${got}）"
  echo "  build 号: ${got}（商店版本仍为 ${short}）"
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

  # build 号必须与商店版本不同：两者相同时第一次能传，之后每次都被 ASC 以
  # ENTITY_ERROR.ATTRIBUTE.INVALID.DUPLICATE 挡回。在自检里就说清楚，别等到
  # 传了 20 分钟才看到那条错。
  local bundle_short bundle_build
  bundle_short="$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$APP/Contents/Info.plist")"
  bundle_build="$(/usr/libexec/PlistBuddy -c "Print :CFBundleVersion" "$APP/Contents/Info.plist")"
  echo "  CFBundleShortVersionString=$bundle_short  CFBundleVersion=$bundle_build"
  if [ "$bundle_short" = "$bundle_build" ]; then
    fail "CFBundleVersion 与商店版本相同（都是 ${bundle_short}）—— ASC 不允许重复上传同一 build 号"
  fi

  # 体积门槛：App Store 对下载包有上限，超了要等 Apple 批
  local size
  size="$(du -sm "$APP" | cut -f1)"
  echo "  体积: ${size} MB"
}

archive() {
  # 格式：必须 productbuild，不能 ditto 打 zip（理由见下）。
  mkdir -p "$ZIP_DIST"
  rm -f "$PKG_PATH"
  xcrun productbuild \
    --component "$APP" \
    /Applications \
    "$PKG_PATH" || fail "productbuild 失败"
  [ -f "$PKG_PATH" ] || fail "productbuild 没产出 $PKG_PATH"

  local installer="${MAS_INSTALLER_IDENTITY:-3rd Party Mac Developer Installer: Beijing VGO Co;Ltd (${TEAM_ID})}"
  security find-identity -v | grep -Fq "$installer" \
    || fail "钥匙串里没有 installer 身份：$installer
  先跑 python3 scripts/create_mas_installer_cert.py"

  sign_pkg_with_installer "$PKG_PATH" "$installer"
  echo "  $PKG_PATH ($(du -h "$PKG_PATH" | cut -f1))，已用 installer 证书签名"
}

# 拿一把**临时钥匙串**给 productsign 用。
#
# ## 为什么不能用登录钥匙串里那把
#
# 实测（2026-10-01）：直接用登录钥匙串里的 installer 私钥时，productsign
# 会**挂死**。用 `sample` 抓到的调用栈是：
#
#   SecKeyCreateSignature → SecKeyRunAlgorithmAndCopyResult
#     → CSSM_SignData → SecurityServer::ClientSession::generateSignature
#       → mach_msg2_trap        ← 无限等 securityd
#
# 被外部超时掐断后留下一个 Bom/PackageInfo/Payload 全是 0 字节的半成品，
# `pkgutil --check-signature` 报 invalid signature。
#
# 换一把我自己知道密码的临时钥匙串（同样的证书与私钥）就立刻签成功。
# 所以问题出在登录钥匙串里那把私钥的状态，不在 productsign 本身。
#
# 顺带避开另一个坑：`security unlock-keychain <path>` 找不到不在搜索列表里的
# 钥匙串，所以要先 `list-keychains -d user -s` 临时加进去、结束后还原。

BUILD_KEYCHAIN=""
BUILD_KEYCHAIN_PASSWORD=""

build_signing_keychain() {
  BUILD_KEYCHAIN="${TMPDIR:-/tmp}/macslim-build.keychain-db"
  BUILD_KEYCHAIN_PASSWORD="msl$(date +%s)"
  security delete-keychain "$BUILD_KEYCHAIN" 2>/dev/null || true
  security create-keychain -p "$BUILD_KEYCHAIN_PASSWORD" "$BUILD_KEYCHAIN" \
    || fail "无法创建临时钥匙串"

  local original_search
  original_search="$(security list-keychains -d user | tr -d ' "')"
  security list-keychains -d user -s "$BUILD_KEYCHAIN" $original_search

  security import "$INSTALLER_P12" -k "$BUILD_KEYCHAIN" \
    -P "$(cat "$INSTALLER_P12_PASSWORD")" -A || fail "临时钥匙串导入证书失败"
  security set-key-partition-list -S apple-tool:,apple:,codesign:,productsign: \
    -s -k "$BUILD_KEYCHAIN" "$BUILD_KEYCHAIN_PASSWORD" >/dev/null 2>&1 || true
}

restore_keychain_search() {
  [ -n "$BUILD_KEYCHAIN" ] || return 0
  security list-keychains -d user -s \
    "$HOME/Library/Keychains/login.keychain-db" "/Library/Keychains/System.keychain"
  security delete-keychain "$BUILD_KEYCHAIN" 2>/dev/null || true
}

sign_pkg_with_installer() {
  local pkg="$1" installer="$2"
  build_signing_keychain
  # shellcheck disable=SC2064  # 密码要在这条命令执行的那一刻展开，不是定义时
  trap "restore_keychain_search" RETURN

  local out="$pkg.signed"
  rm -f "$out"
  if ! xcrun productsign --keychain "$BUILD_KEYCHAIN" --sign "$installer" \
       --timestamp=none "$pkg" "$out"; then
    fail "productsign 失败"
  fi
  # 验签：productsign 失败时会留下一个所有条目都是 0 字节的半成品包，
  # 而它**不报错** —— 不验这一步就会把损坏的包送上去，然后收到一句
  # 「signature is invalid」，完全指不到真正原因。
  pkgutil --check-signature "$out" | grep -Fq "Status: signed" \
    || fail "productsign 产出的包状态不是 signed（详见注释：失败时不报错，只留空包）"
  mv "$out" "$pkg"
}

upload() {
  info "上传 App Store Connect"
  # local 必须**先于**第一次使用。这里原本多了一行在 local 之前就引用
  # $ENV_PATH 的判断，在 set -u 下直接 `unbound variable` 中止 ——
  # 于是 upload 模式从来没跑通过，而 build 模式全绿，CI 也就没发现。
  local ENV_PATH="$ENV_FILE"
  [ -f "$ENV_PATH" ] || fail "找不到 ASC 凭据：$ENV_PATH"
  set -a; . "$ENV_PATH"; set +a
  export APPLE_API_KEY_PATH="${APPLE_API_KEY_PATH/#\$HOME/$HOME}"
  export API_PRIVATE_KEYS_DIR="$(dirname "$APPLE_API_KEY_PATH")"

  # 上传的是 pkg 而不是 zip —— 理由见 archive()。
  xcrun altool --upload-app --type osx \
    --file "$PKG_PATH" \
    --apiKey "$APPLE_API_KEY" \
    --apiIssuer "$APPLE_API_ISSUER"
}

case "$MODE" in
  build)  preflight; build; strip_cli; sanitize_info_plist; stamp_build_number "$(store_version)"; install_privacy_manifest; sign; verify; archive ;;
  upload) upload ;;
  all)    preflight; build; strip_cli; sanitize_info_plist; stamp_build_number "$(store_version)"; install_privacy_manifest; sign; verify; archive; upload ;;
  *)     fail "用法: $0 [build|upload|all]" ;;
esac
