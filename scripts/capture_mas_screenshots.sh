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
# zh-CN | en。默认中文。
#
# ## 为什么需要这个开关
#
# 上架页要按语言分别给图：en-US 那条 listing 的副标题是英文的，配一屏中文
# 截图会很显眼。而截屏只能靠「改源码 → 重新构建 → 还原」逐页切过去，没法
# 在运行时点设置页改语言（不用 AppleScript 的理由见文件末尾）。
#
# 这里改的是**默认值**（loadStored 原本回落到 "auto"，即跟随系统语言），
# 不是改任何一条文案 —— 画面上的每个英文串都仍然来自 src/i18n/en.ts 本身，
# 所以截图如实反映一个英文用户看到的东西。
SHOT_LOCALE="${SHOT_LOCALE:-zh-CN}"
case "$SHOT_LOCALE" in
  zh-CN | en) ;;
  *)
    echo "SHOT_LOCALE 只接受 zh-CN 或 en，收到的是：$SHOT_LOCALE" >&2
    exit 2
    ;;
esac
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
  git checkout -- src/App.tsx src-tauri/tauri.conf.json src/i18n/index.tsx 2>/dev/null || true
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
  wait_until_idle
}

# 等 App 真的把数据算完再截，而不是拍一个固定秒数。
#
# ## 为什么不能固定 sleep
#
# 各页耗时差了两个数量级：进程页枚举 260 个进程约 1 秒，而应用卸载页要给
# 每个 .app 递归统计体积，本机实测 12 秒时还是骨架屏、30 秒才出列表。原先
# 一律 `sleep 9`，于是卸载页截到的是**一屏骨架屏** —— 而骨架屏正是审核指南
# 2.1 里最典型的「不完整」形态，拿它当上架图等于自己交把柄。
#
# 拉长成 `sleep 40` 也能糊过去，但那是把一个已知问题用更大的常数盖住：
# 换台机器、装的东西一多，40 秒照样不够，而且没人会注意到。
#
# ## 用什么信号
#
# app 的扫描是纯 CPU 的目录遍历，所以「进程累计 CPU 时间不再增长」就等价于
# 「算完了」。这个信号不需要读窗口，也就不用 AppleScript —— 那会触发系统的
# 「Developer Tools Access」授权弹窗，正好挡在截图前面。
#
# 保留一个下限：窗口弹出与首屏动画本身也要时间，空闲检测会在数据秒回时立刻
# 通过，那一刻画面还没稳定。
MIN_SETTLE=5
MAX_SETTLE=120
# 连续两次采样之间，允许的 CPU 时间增量（秒）。低于它即视为已空闲。
IDLE_DELTA=0.30

cpu_time() {
  # macOS 的 ps TIME 是 [[dd-]hh:]mm:ss.ss，取小数秒部分统一成数字
  ps -o time= -p "$(pgrep -x macslim | head -1)" 2>/dev/null |
    awk '{ gsub("-", "", $1); n = split($1, p, ":");
           s = 0;
           for (i = 1; i <= n; i++) s = s * 60 + p[i];
           printf "%.2f\n", s }'
}

wait_until_idle() {
  sleep "$MIN_SETTLE"
  local waited=$MIN_SETTLE
  local prev curr
  prev="$(cpu_time)"
  while [ "$waited" -lt "$MAX_SETTLE" ]; do
    sleep 2
    curr="$(cpu_time)"
    waited=$((waited + 2))
    # 进程没了（崩了/退出了）就别再等，否则这里会空转到上限
    [ -z "$curr" ] && return 0
    if awk "BEGIN{exit !($curr - $prev < $IDLE_DELTA)}"; then
      return 0
    fi
    prev="$curr"
  done
  echo "  警告：等待 ${MAX_SETTLE}s 后仍未空闲，截屏可能是骨架屏" >&2
}

cd "$ROOT"
# 全屏捕获：屏幕本身就是 1440x900 点 = 2880x1800 像素，App Store 也认这档。
#
# 之前用「窗口 1280x800 + 猜窗口位置再截那一块」，结果右侧和上沿都漏进桌面
# 壁纸 —— 窗口居中的位置是估出来的，屏幕尺寸一变就偏。改成全屏就没有对齐
# 问题可出了。
python3 scripts/_set_shot_window_size.py fullscreen

# 语言补丁：必须在每页循环**之前**打一次，且要同时盖掉两条出口。
#
# ## 为什么不能只改默认值
#
# loadStored() 有两条 return：一条是 localStorage 命中时 `return v`，一条是
# 兜底的 `return "auto"`。而截屏副本与真包 **bundle id 相同**，WebKit 的
# localStorage 因此跨次运行残留 —— 上一轮中文截图写进去的 "zh-CN" 会把
# 只改兜底值的补丁整个盖掉，然后我们就会把一屏中文当成英文图传上 en-US。
#
# 所以两条出口都要强制成目标语言。而且**补丁必须校验命中数**：哪天有人
# 重构了 loadStored，替换数不对就得当场失败，绝不能静默出一屏错语言的图。
echo "语言：$SHOT_LOCALE"
python3 - "$SHOT_LOCALE" <<'PY'
import pathlib, re, sys

want = sys.argv[1]
path = pathlib.Path("src/i18n/index.tsx")
source = path.read_text(encoding="utf-8")

patched, hit_v, hit_auto = source, 0, 0
patched, hit_v = re.subn(r'return v;', f'return "{want}";', patched, count=1)
patched, hit_auto = re.subn(r'return "auto";', f'return "{want}";', patched, count=1)

if not (hit_v == 1 and hit_auto == 1):
    raise SystemExit(
        f"i18n 语言补丁只命中 {hit_v}/{hit_auto} 处（需要各 1 处）。"
        "loadStored 大概被重构过 —— 停下来，别把错语言的图传上去。"
    )
path.write_text(patched, encoding="utf-8")
PY

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