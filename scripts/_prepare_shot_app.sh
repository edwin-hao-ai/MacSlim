#!/usr/bin/env bash
# 构建「截图专用」的 MAS app 并放到 /tmp/MacSlimShot.app。
#
# 由 capture_story_beats.py（五拍故事线）和 capture_story_pages.sh（单页补拍）
# 共用。抽出来是因为「构建 + 重签」是一整套有坑的步骤，复制两份必然漂移 ——
# 而漂移的后果是「脚本照常跑完，截出来的却是一屏白」。
#
# 用法: _prepare_shot_app.sh <view> <locale-code>   例: _prepare_shot_app.sh cache zh-CN
#
# 三件事，每件都有踩过的坑：
#   1. 改源码里的首屏视图与语言（截图要固定在某一页、某一种语言）
#   2. 窗口改全屏（1440x900 pt = 2880x1800 px，App Store 认可）
#   3. ad-hoc 重签并**剥掉沙箱 entitlement**，否则本机根本起不来
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VIEW="${1:?用法: _prepare_shot_app.sh <view> <locale-code>}"
LOCALE="${2:?}"

MAS_TARGET="${CARGO_TARGET_DIR:-$HOME/.cargo/shared-target-mas}"
APP="$MAS_TARGET/aarch64-apple-darwin/release/bundle/macos/MacSlim.app"
SHOT_APP="${TMPDIR:-/tmp}/MacSlimShot.app"

cd "$ROOT"

python3 - "$VIEW" "$LOCALE" <<'PY'
import pathlib, re, sys

view, locale = sys.argv[1], sys.argv[2]

app = pathlib.Path("src/App.tsx")
s = app.read_text(encoding="utf-8")
s, n = re.subn(r'createSignal<ViewId>\("[a-z]+"\)', f'createSignal<ViewId>("{view}")', s, count=1)
if n != 1:
    raise SystemExit(f"首屏替换命中 {n} 处（需要 1）—— App.tsx 被重构过，别截到别的页面")
app.write_text(s, encoding="utf-8")

i18n = pathlib.Path("src/i18n/index.tsx")
t = i18n.read_text(encoding="utf-8")
# 两条出口都要改：localStorage 命中那条与兜底那条。只改兜底会被上一轮残留
# 的存储值盖掉，然后我们就会把中文截图当英文传上去。
t, a = re.subn(r'return v;', f'return "{locale}";', t, count=1)
t, b = re.subn(r'return "auto";', f'return "{locale}";', t, count=1)
if (a, b) != (1, 1):
    raise SystemExit(f"语言补丁命中 {a}/{b} 处（需要各 1）")
i18n.write_text(t, encoding="utf-8")
print(f"patched: view={view} locale={locale}")
PY

python3 scripts/_set_shot_window_size.py fullscreen >/dev/null

# 构建失败必须看得见。之前这里 `>/dev/null 2>&1` 让整支脚本只打印一句
# "patched" 就静默退出 —— 排查时完全不知道是构建挂了还是后面挂了。
if ! "$ROOT/scripts/release-mas.sh" build >/tmp/story-build.log 2>&1; then
  echo "构建失败，最后 20 行：" >&2
  tail -20 /tmp/story-build.log >&2
  exit 1
fi
[ -d "$APP" ] || {
  echo "构建没产出 $APP" >&2
  tail -20 /tmp/story-build.log >&2
  exit 1
}

# 本地要跑**沙箱版**才拍得出可信截图：上架版没授权就读不到 ~/Library/Caches，
# 「授权 → 数据 → 清理」这条叙事只有在沙箱里成立。跑无沙箱版会看到 app 直接
# 把 DerivedData 13 GB 列出来 —— 那不是上架版的行为，交上去是骗审核。
#
# MAS 包自带的 profile 是 App Store **生产** profile，macOS 只认经 App Store /
# Xcode 安装的产物，本地直接跑会被拒：
#   taskgated: embedded provisioning profile not valid, Code=-215
#              "Only Development Provisioning Profiles can be installed..."
#   amfid:     -413 "No matching profile found"  → open 报 Error 162
#
# 所以本地用 Development profile（macOS 接受它本地安装）+ Apple Development
# 证书签一份副本。二进制不变，仍然是 --features mas 编出来的上架版。
DEV_PROFILE="${MAS_DEV_PROFILE:-$HOME/.cargo/shared-target-mas/MacSlim-dev.provisionprofile}"
[ -f "$DEV_PROFILE" ] || "$ROOT/scripts/create_screenshot_dev_profile.py"

# 签名要跟 profile 里那张证书一致，否则设备/证书不匹配会被拒。
#
# `-noout` 不是可选的洁癖：不给它，openssl 会连同完整 PEM 一起打到 stdout，
# 于是 DEV_CERT_SHA 变成「指纹 + 一整段证书」的多行字符串。codesign 拿到这个
# 垃圾身份会失败，而失败被 `>/dev/null 2>&1` 吞掉 → 副本仍是 release-mas.sh
# 留下的 MAS 签名，但 profile 已经被换成 Development 的 → taskgated 报
# 「Unsatisfied entitlements」把 app 杀掉。整个链条静默，最难查的就是这一处。
DEV_CERT_SHA="$(python3 - "$DEV_PROFILE" <<'PY'
import plistlib, subprocess, sys
der = plistlib.loads(subprocess.run(
    ["security", "cms", "-D", "-i", sys.argv[1]], capture_output=True
).stdout)["DeveloperCertificates"][0]
out = subprocess.run(
    ["openssl", "x509", "-inform", "DER", "-noout", "-fingerprint", "-sha1"],
    input=der, capture_output=True, check=True,
).stdout.decode()
fingerprint = out.strip().split("=", 1)[1].strip()
if not fingerprint or len(fingerprint.replace(":", "")) != 40:
    raise SystemExit(f"没拿到合法的证书 SHA1 指纹：{out!r}")
print(fingerprint.replace(":", ""))
PY
)"

# entitlements = 沙箱那份 + profile 里的三个身份字段。
# 身份字段必须补：缺了它们，沙箱内的 XPC 服务（WebKit 的 GPU 进程首当其冲）
# 查不到注册信息，表现为「进程反复崩溃 → 整个 webview 纯白」，而不是签名报错。
ENTITLEMENTS_TMP="$(mktemp -t macslim-shot-ent)"
python3 - "$ROOT/src-tauri/entitlements.mas.plist" "$DEV_PROFILE" "$ENTITLEMENTS_TMP" <<'PY'
import plistlib, subprocess, sys
ent = plistlib.loads(open(sys.argv[1], "rb").read())
prof = plistlib.loads(subprocess.run(
    ["security", "cms", "-D", "-i", sys.argv[2]], capture_output=True
).stdout)["Entitlements"]
for key in ("com.apple.application-identifier",
            "com.apple.developer.team-identifier",
            "keychain-access-groups"):
    ent[key] = prof[key]
open(sys.argv[3], "wb").write(plistlib.dumps(ent))
PY

rm -rf "$SHOT_APP"
cp -R "$APP" "$SHOT_APP"
# 换成 Development profile：生产那份留着会被 taskgated 判为「不适用」。
cp "$DEV_PROFILE" "$SHOT_APP/Contents/embedded.provisionprofile"

# 重签失败必须在这里就炸出来。`codesign -v` 兜不住：签名本来就是完整的
# （MAS 那份），只是证书身份错了，它照样校验通过 —— 于是失败被推迟到启动时
# 才变成「app 启动 9 秒后被杀」，跟签名八竿子打不着。
if ! codesign --force --deep --sign "$DEV_CERT_SHA" \
  --entitlements "$ENTITLEMENTS_TMP" \
  "$SHOT_APP"; then
  rm -f "$ENTITLEMENTS_TMP"
  echo "错误: 用 Development 证书（${DEV_CERT_SHA}）重签失败" >&2
  exit 1
fi
rm -f "$ENTITLEMENTS_TMP"

# 再确认签出来的确实是这把证书带的身份字段，而不是碰巧保留了 MAS 签名。
# 判据选「签名里的 keychain-access-groups 与 profile 一致」：这正是
# taskgated 报 Unsatisfied entitlements 时逐条比的东西，也是唯一能提前
# 抓住「重签没生效」的读回方式。
python3 - "$SHOT_APP" "$DEV_PROFILE" <<'PY'
import plistlib, subprocess, sys

app, profile = sys.argv[1], sys.argv[2]
raw = subprocess.run(["codesign", "-d", "--entitlements", "-", "--xml", app],
                     capture_output=True).stdout
signed = plistlib.loads(raw[raw.find(b"<?xml"):])
prof = plistlib.loads(subprocess.run(
    ["security", "cms", "-D", "-i", profile], capture_output=True
).stdout)["Entitlements"]

for key in ("com.apple.application-identifier",
            "com.apple.developer.team-identifier",
            "keychain-access-groups"):
    if signed.get(key) != prof.get(key):
        raise SystemExit(
            f"签名与 profile 对不上：{key} 签名={signed.get(key)!r} "
            f"profile={prof.get(key)!r}。重签多半没生效。"
        )
if not signed.get("com.apple.security.app-sandbox"):
    raise SystemExit("签名里没有 app-sandbox —— 截图会拍到无沙箱行为，不能用")
print("  签名自检通过：Development 证书 + 沙箱 + 与 profile 一致的身份字段")
PY

echo "截图副本就绪: ${SHOT_APP}（view=${VIEW} locale=${LOCALE} 全屏，沙箱 + Development profile）"
