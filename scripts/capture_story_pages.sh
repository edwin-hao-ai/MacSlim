#!/usr/bin/env bash
# 拍五拍故事线里「不需要点击」的那些画面。
#
# ## 为什么要分两套工具
#
# 「授权 → 扫描 → 二次确认 → 清理」这条线里，前两步和最后一步都能用文件系统
# 与重建来达成，只有**二次确认弹窗**必须真的点一下主按钮。所以：
#
# - 这一支（capture_story_pages.sh）：用「改初始视图 + 重建」切页面，
#   用增删 folder-grants.json 切授权状态。不碰 NSOpenPanel，也就不碰
#   那个会弹系统密码框的路径。
# - capture_story_beats.py：负责需要点击的那一拍。
#
# ## 授权状态为什么可以直接写文件
#
# folder-grants.json 落在应用自己的 container 里，格式是明文 JSON。已授权
# 状态**不需要**真的走过 NSOpenPanel 才能得到 —— 拿到过一次真实 bookmark
# 之后，它就长期有效，删掉应用才会消失。所以「已授权」这个画面可以稳定
# 复现，而截图要的正是这个状态。
#
# 但**不**去伪造 bookmark：bookmark 是二进制凭证，编一个出来只会被
# `bookmark_is_plausible` 丢掉。这里只做「有 / 没有」的切换。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:?用法: capture_story_pages.sh <输出目录> <locale> <view> [granted|none]}"
LOCALE="${2:?}"
VIEW="${3:?}"
GRANTED="${4:-none}"

MAS_TARGET="${CARGO_TARGET_DIR:-$HOME/.cargo/shared-target-mas}"
APP="$MAS_TARGET/aarch64-apple-darwin/release/bundle/macos/MacSlim.app"
SHOT_APP="${TMPDIR:-/tmp}/MacSlimShot.app"
GRANTS="$HOME/Library/Containers/com.vgoapp.macslim/Data/Library/Application Support/folder-grants.json"
GRANTS_BACKUP="/tmp/macslim-grants-backup.json"
REGION="0,0,1440,900"

cleanup() {
  cd "$ROOT"
  git checkout -- src/App.tsx src/i18n/index.tsx src-tauri/tauri.conf.json 2>/dev/null || true
  # 授权状态是**借来的**：拍完必须还原，不能把用户的授权留在截图专用的状态上
  if [ -f "$GRANTS_BACKUP" ]; then
    mkdir -p "$(dirname "$GRANTS")"
    mv "$GRANTS_BACKUP" "$GRANTS"
  else
    rm -f "$GRANTS"
  fi
}
trap cleanup EXIT

case "$LOCALE" in
  zh-Hans) LOCALE_CODE="zh-CN" ;;
  en-US)   LOCALE_CODE="en" ;;
  *) echo "locale 只接受 zh-Hans 或 en-US" >&2; exit 2 ;;
esac

# 备份真实授权状态（只在存在时备份；不存在则记住「本来就没有」）
if [ -f "$GRANTS" ]; then
  cp "$GRANTS" "$GRANTS_BACKUP"
else
  rm -f "$GRANTS_BACKUP"
fi

# 切授权状态
if [ "$GRANTED" = "granted" ]; then
  [ -f "$GRANTS_BACKUP" ] || {
    echo "错误: 需要已授权状态，但没有可用的真实 bookmark 备份" >&2
    exit 2
  }
  mkdir -p "$(dirname "$GRANTS")"
  cp "$GRANTS_BACKUP" "$GRANTS"
else
  rm -f "$GRANTS"
fi

# 构建 + ad-hoc 重签交给共享脚本，避免和 capture_story_beats.py 各写一份
# 而两份迟早漂移 —— 漂移的后果是「脚本跑完，截出一屏白」。
"$ROOT/scripts/_prepare_shot_app.sh" "$VIEW" "$LOCALE_CODE"

# 截屏不能用固定秒数：应用卸载页要递归统计每个 .app 的体积，本机实测
# 30 秒才出列表。固定 9 秒会截到一屏骨架屏 —— 而骨架屏正是审核指南 2.1
# 里最典型的「不完整」形态。
MIN_SETTLE=5
MAX_SETTLE=120
IDLE_DELTA=0.30
cpu_time() {
  ps -o time= -p "$(pgrep -x macslim | head -1)" 2>/dev/null |
    awk '{ gsub("-", "", $1); n = split($1, p, ":"); s = 0;
           for (i = 1; i <= n; i++) s = s * 60 + p[i];
           printf "%.2f\n", s }'
}

# 直接 exec，不用 `open`。`open` 走 LaunchServices，对放在 $TMPDIR
# （/var/folders/.../T）里的 Development 签名 app 会直接失败：
#   RBSRequestErrorDomain Code=5 / NSUnderlyingError 162 "Launchd job spawn failed"
# 同一个包直接 exec 完全正常 —— 用 `open` 时症状是「脚本跑完，截出一屏白」。
"$SHOT_APP/Contents/MacOS/macslim" >/tmp/macslim-shot-run.log 2>&1 &
sleep "$MIN_SETTLE"
waited=$MIN_SETTLE
prev="$(cpu_time)"
while [ "$waited" -lt "$MAX_SETTLE" ]; do
  sleep 2
  curr="$(cpu_time)"
  waited=$((waited + 2))
  # 取不到进程说明它还没起来或刚退出，**不能**当成「已空闲」——
  # 之前这里直接 break，等于完全没等，截图全落在 webview 还没画完的白窗上。
  [ -z "$curr" ] && continue
  [ -z "$prev" ] && { prev="$curr"; continue; }
  if awk "BEGIN{exit !($curr - $prev < $IDLE_DELTA)}"; then break; fi
  prev="$curr"
done

mkdir -p "$OUT"
out="$OUT/$VIEW.png"

# 这台机器上还有别的窗口，截图抓的是**屏幕上那块区域**，不是应用自己的窗口。
# 应用没在前台时截出来的是一片空白（实测：整张纯白、96 KB，连侧栏都没有）。
# 所以：先激活、等窗口真的出现，再截；截完还要**验证不是空白**，失败就重试。
activate_and_shoot() {
  osascript -e 'tell application "MacSlimShot" to activate' >/dev/null 2>&1 || true
  sleep 3
  screencapture -x -R "$REGION" -t png "$out"
}

# 空白判据：灰度标准差。纯白/纯色画面 stddev 恰好为 0，真实界面（侧栏、
# 文字、色块、深浅底色并存）必然显著大于 0。
# 之前用「暗像素计数」是错的：这套界面是浅色底，02 那张正常截图的暗像素
# 只有 2000，阈值一旦定高就会把好图判成坏图，定低又拦不住真空白。
has_content() {
  python3 - "$1" <<'PY'
import sys
from PIL import Image, ImageStat
im = Image.open(sys.argv[1]).convert("L")
print("%.2f" % ImageStat.Stat(im).stddev[0])
PY
}

attempt=0
while [ "$attempt" -lt 6 ]; do
  attempt=$((attempt + 1))
  activate_and_shoot
  std="$(has_content "$out" | tail -1)"
  if awk "BEGIN{exit !($std > 3)}"; then
    break
  fi
  echo "  第 ${attempt} 次截到空白（stddev ${std}），重新激活再试" >&2
  sleep 5
done

std="$(has_content "$out" | tail -1)"
if ! awk "BEGIN{exit !($std > 3)}"; then
  echo "错误: 连试 6 次都截到空白画面（stddev ${std}）" >&2
  exit 1
fi
echo "  灰度标准差 ${std}（确认不是空白）"

size=$(sips -g pixelWidth -g pixelHeight "$out" \
  | awk '/pixelWidth/{w=$2}/pixelHeight/{h=$2}END{print w"x"h}')
echo "  $out ($size)"
pkill -x macslim 2>/dev/null || true
