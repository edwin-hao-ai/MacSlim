#!/usr/bin/env bash
# 抓 App Store 用的截屏。
#
# ## 为什么需要单独一个脚本
#
# 提交用的那个 .app **在本地根本跑不起来**：
#
#   App Store 要求签名里带 profile 的 application-identifier
#   （少一条就 ASC 拒收 90886/90230，见 release-mas.sh 的 sign_entitlements）。
#   而 macOS 一旦看到签名里有这条，就按「App Store 应用」对待，
#   本地未公证的包会被 Gatekeeper 直接 SIGKILL —— 实测连二进制直接执行都是
#   exit 137，`spctl -a` 报 rejected。
#
# 所以截屏用一份**同二进制、不同签名**的副本：只签沙箱 entitlement、不签
# 身份字段。这样它能本地启动，而沙箱行为、界面、可用能力与真包完全一致
# （entitlement 不影响前端）。
#
# ## 用法
#
#   ./scripts/capture_mas_screenshots.sh <输出目录>
#
# 逐页切换靠临时改 `src/App.tsx` 的初始页 + 重新构建 + 还原，
# 因为不用 AppleScript —— 那会触发系统的「Developer Tools Access」
# 授权弹窗，挡在截图前面还得手动清掉。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:?用法: capture_mas_screenshots.sh <输出目录>}"
SRC="${MAS_APP:-$HOME/.cargo/shared-target-mas/aarch64-apple-darwin/release/bundle/macos/MacSlim.app}"
SHOT_APP="${TMPDIR:-/tmp}/MacSlimShot.app"
# 与 App Store 认可的尺寸对应：1280x800 点 = 2560x1600 像素
WINDOW_W=1280
WINDOW_H=800
# 全屏区域。菜单栏在截屏时会自动隐藏（鼠标不在顶部），所以画面里不会出现
# 无关的系统文字。
REGION="0,0,1440,900"

# 四个要入镜的页面。scan 打头是因为它是默认首屏。
# 顺序即 App Store 里的展示顺序，第一张是列表首图。
# 进程管理放第一：它是唯一一张信息密度足够的画面（进程数 / 内存 / CPU /
# 按 .app 分组的进程树），而智能扫描页去掉假数据之后画面偏空。
PAGES=("process:进程管理" "cache:缓存清理" "uninstaller:应用卸载" "scan:智能扫描")
idx=0

cleanup() {
  cd "$ROOT"
  git checkout -- src/App.tsx src-tauri/tauri.conf.json 2>/dev/null || true
  pkill -x macslim 2>/dev/null || true
  rm -rf "$SHOT_APP"
}
trap cleanup EXIT

mkdir -p "$OUT"

launch_shootable() {
  pkill -x macslim 2>/dev/null || true
  sleep 1
  rm -rf "$SHOT_APP"
  cp -R "$SRC" "$SHOT_APP"
  # profile 一并去掉：留着一个与签名不匹配的 profile 只会让 Gatekeeper 更糊涂
  rm -f "$SHOT_APP/Contents/embedded.provisionprofile"
  codesign --force --deep \
    --sign "3rd Party Mac Developer Application: Beijing VGO Co;Ltd (5XNDF727Y6)" \
    --options runtime --timestamp=none \
    --entitlements "$ROOT/src-tauri/entitlements.mas.plist" \
    "$SHOT_APP" >/dev/null 2>&1
  open "$SHOT_APP"
  sleep 9
}

cd "$ROOT"
# 全屏捕获：屏幕本身就是 1440x900 点 = 2880x1800 像素，App Store 也认这档。
#
# 之前用「窗口 1280x800 + 猜窗口位置再截那一块」，结果右侧和上沿都漏进桌面
# 壁纸 —— 窗口居中的位置是估出来的，屏幕尺寸一变就偏。改成全屏就没有对齐
# 问题可出了。
python3 scripts/_set_shot_window_size.py fullscreen

for entry in "${PAGES[@]}"; do
  page="${entry%%:*}"
  label="${entry#*:}"
  python3 - "$page" <<'PY'
import pathlib, sys
page = sys.argv[1]
p = pathlib.Path("src/App.tsx")
s = p.read_text(encoding="utf-8")
import re
s = re.sub(r'createSignal<ViewId>\("[a-z]+"\)',
           f'createSignal<ViewId>("{page}")', s, count=1)
p.write_text(s, encoding="utf-8")
PY
  "$ROOT/scripts/release-mas.sh" build >/dev/null 2>&1
  launch_shootable
  # 文件名用 ASCII 排序，uploader 按顺序入位
  idx=$((idx + 1))
  out="$OUT/$(printf '%02d' "$idx")-$page.png"
  screencapture -x -R "$REGION" -t png "$out"
  size=$(sips -g pixelWidth -g pixelHeight "$out" \
    | awk '/pixelWidth/{w=$2}/pixelHeight/{h=$2}END{print w"x"h}')
  echo "  $label → $(basename "$out") ($size)"
  pkill -x macslim 2>/dev/null || true
done

echo
echo "完成：$OUT"
ls -1 "$OUT"