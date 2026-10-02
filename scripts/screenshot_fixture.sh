#!/usr/bin/env bash
# 为截屏造一份一次性的假缓存数据。
#
# ## 为什么需要它
#
# 「清理完成」和「历史记录」这两张截图要求真的清理过一次，否则画面是假的。
# 但所有缓存扫描器都是**整目录一个条目**的粒度（`scan_app_logs` 给整个
# `~/Library/Logs` 出一个条目，`scan_npm` 给整个 `~/.npm` 出一个），
# 所以往真实目录里塞 fixture 会被连锅端 —— 清理会连带删掉用户真实数据。
#
# 做法是**把真实内容移出去**，而不是删除：
#
#   1. 把 ~/Library/Logs 下的内容（含隐藏项）移进 ~/Library/macslim-shot-backup
#   2. 在空出来的 ~/Library/Logs 里造稀疏文件 fixture
#   3. 用户走完授权 → 扫描 → 清理
#   4. 删掉 fixture 残留，用 rsync 把真实内容合并回原位
#
# 真实数据全程只被移动，从未被删除。
#
# ## 为什么是「移内容」而不是「移目录」
#
# 试过 `mv ~/Library/Logs ~/Library/Logs.shot-backup`，被系统拒绝：
# `~/Library/*` 与 `~/.Trash` 受 TCC/SIP 保护，**改名**不行，但往里
# **写**是可以的。所以只能保持目录本身不动，把内容搬空。
#
# ## 为什么选 ~/Library/Logs
#
# - 纯路径扫描，不依赖 `find_tool`：npm/docker/homebrew 那些要先 exec 外部
#   命令，沙箱里未必找得到，而 Logs 一定是纯读目录
# - 日志本身会自动再生（扫描器自己的 recover_hint 就写着「日志会在应用运行时
#   自动重新生成」），万一还原失败也不至于长期损失
# - 比 `~/Library/Caches` 轻：后者清掉会让部分应用重新下载资源、个别要重新登录
#
# ## 期间新产生的日志怎么办
#
# 目录被搬空的那几分钟里，系统和应用照常会往 Logs 里写东西。所以还原用
# `rsync -a` **合并**而不是直接覆盖：新产生的文件留在原地，旧的补回去。
#
# ## 为什么 fixture 用稀疏文件
#
# `dir_size` 用 WalkDir + `metadata().len()`，量的是**文件标称长度**而不是
# 占用块数。所以 `truncate -s 5G` 显示 5 GB 而几乎不占磁盘 —— 截图里每个数字
# 都是真实测量（文件真的存在、真的被删、真的回填历史），代价接近零。
#
# ## 用法
#
#   ./scripts/screenshot_fixture.sh up      # 造
#   ./scripts/screenshot_fixture.sh status   # 看现状
#   ./scripts/screenshot_fixture.sh down    # 还原
set -euo pipefail

TARGET_DIR="${SHOT_FIXTURE_DIR:-$HOME/Library/Logs}"
BACKUP_DIR="${SHOT_FIXTURE_BACKUP:-$HOME/Library/macslim-shot-backup}"
FIXTURE_PREFIX="macslim-shot-fixture"
FIXTURE_SIZE="${SHOT_FIXTURE_SIZE:-5G}"

fixture_file() { printf '%s/%s' "$TARGET_DIR" "$FIXTURE_PREFIX"; }

# 搬走真实内容之前先记一份基线，还原后核对。
#
# 没有基线时，「都还原了」只能靠看：目录项数量对得上、体积差不多。但 10M
# 变 9.9M 这种差别，分不清是 du 舍入还是真丢了东西。这不是理论担忧 ——
# 第一次实测往返就是 10M → 9.9M，而我无法说明它。
#
# manifest 放在 ~/Library 下、**不放**备份目录里：备份目录会被 rsync 进
# Logs，manifest 混进去就变成 ~/Library/Logs/macslim-shot-fixture.manifest
# 这种垃圾，还要再清理一次。
MANIFEST="${SHOT_FIXTURE_MANIFEST:-$HOME/Library/macslim-shot-fixture.manifest}"

# 目录项数（含目录与隐藏项）+ 文件字节总量。两项一起比才够：
# 只比字节数会把「一个大文件换成两个小文件」这种变化漏掉。
measure_entries() { find "$1" -mindepth 1 \( -type f -o -type d -o -type l \) | wc -l | tr -d ' '; }
measure_bytes() {
  find "$1" -type f -exec stat -f '%z' {} + 2>/dev/null |
    awk '{s+=$1} END {printf "%d", s+0}'
}

write_manifest() {
  local entries bytes
  entries="$(measure_entries "$TARGET_DIR")"
  bytes="$(measure_bytes "$TARGET_DIR")"
  printf '%s %s\n' "$entries" "$bytes" > "$MANIFEST"
  printf '  已记基线: %s 个目录项 / %s 字节\n' "$entries" "$bytes"
}

verify_manifest() {
  [ -f "$MANIFEST" ] || {
    printf '注意: 没有基线可比对（%s 不存在），无法核对是否完整还原。\n' "$MANIFEST" >&2
    return 0
  }
  local want want_bytes got got_bytes
  read -r want want_bytes < "$MANIFEST"
  got="$(measure_entries "$TARGET_DIR")"
  got_bytes="$(measure_bytes "$TARGET_DIR")"
  if [ "$want" = "$got" ] && [ "$want_bytes" = "$got_bytes" ]; then
    printf '  核对通过: %s 个目录项 / %s 字节，与搬走前一致\n' "$got" "$got_bytes"
  else
    # 期间新产生的日志会让两个数都比基线大，那属于正常。只在**变少**时报警 ——
    # 变少意味着有东西丢了，那才是不可逆的事故。
    if [ "$got" -lt "$want" ] || [ "$got_bytes" -lt "$want_bytes" ]; then
      printf '错误: 还原后比搬走前少东西了（目录项 %s→%s，字节 %s→%s）\n' \
        "$want" "$got" "$want_bytes" "$got_bytes" >&2
      return 1
    fi
    printf '  核对: 目录项 %s→%s、字节 %s→%s（只多不少，期间有新日志写入）\n' \
      "$want" "$got" "$want_bytes" "$got_bytes"
  fi
}

restore() {
  if [ ! -d "$BACKUP_DIR" ]; then
    # 没有备份目录 = 从没 up 过，或者已经还原完。绝不能在这里去删 TARGET_DIR：
    # 那就是删用户真实数据。
    return 0
  fi

  # 只清我们自己造的那几个文件，且必须按前缀精确匹配。
  # 不用 `rm -rf "$TARGET_DIR"`：期间系统新产生的日志会落在同一个目录里。
  local stray
  stray="$(find "$TARGET_DIR" -maxdepth 1 -type f \
    -not -name "$FIXTURE_PREFIX-*" -print -quit 2>/dev/null || true)"
  if [ -n "$stray" ]; then
    # 有外来文件是**正常**的（应用照常写日志）。rsync 合并能处理共存，
    # 所以这里不是错误，只是要告诉操作者别以为目录是干净的。
    printf '注意: %s 里有期间新产生的文件（%s），会用合并方式还原。\n' \
      "$TARGET_DIR" "$(basename "$stray")"
  fi

  if compgen -G "$TARGET_DIR/$FIXTURE_PREFIX-*" > /dev/null; then
    rm -f "$TARGET_DIR"/"$FIXTURE_PREFIX"-*
  fi

  # rsync 合并：新产生的文件留在原地，备份里的旧文件补回去。
  # 目录项会递归合并，所以 `CrashReporter` 这类期间又被写过的目录也不会丢。
  # set -e 保证 rsync 出错时不会走到下面的 rm。
  rsync -a "$BACKUP_DIR/" "$TARGET_DIR/"
  rm -rf "$BACKUP_DIR"
  # 先核对再删基线：核对失败时 manifest 留在原地，那正是排障需要的证据。
  verify_manifest
  rm -f "$MANIFEST"
  printf '已还原: %s\n' "$TARGET_DIR"
}

fixture_up() {
  if [ -d "$BACKUP_DIR" ]; then
    printf '错误: %s 已存在 —— 上一次可能没还原干净。先跑 down。\n' "$BACKUP_DIR" >&2
    return 1
  fi
  [ -d "$TARGET_DIR" ] || {
    printf '错误: %s 不存在，无法造 fixture。\n' "$TARGET_DIR" >&2
    return 1
  }
  # 往 TARGET_DIR 里写必须先验证得过，否则会出现「备份已建、fixture 建不了」
  # 的半成品状态。
  : > "$(fixture_file)-probe" || {
    printf '错误: 无法在 %s 里创建文件，放弃（尚未移动任何内容）。\n' "$TARGET_DIR" >&2
    return 1
  }
  rm -f "$(fixture_file)-probe"

  mkdir -p "$BACKUP_DIR"
  write_manifest
  # 包含隐藏项：`-mindepth 1` + find -exec mv，glob 会漏掉 .DS_Store 之类
  find "$TARGET_DIR" -mindepth 1 -maxdepth 1 -exec mv {} "$BACKUP_DIR/" \;

  truncate -s "$FIXTURE_SIZE" "$(fixture_file)-a.log"
  truncate -s "$FIXTURE_SIZE" "$(fixture_file)-b.log"
  truncate -s 512M "$(fixture_file)-c.log"

  printf '已造假缓存: %s\n' "$TARGET_DIR"
  printf '  真实内容暂存在 %s\n' "$BACKUP_DIR"
  printf '  标称总量 %s（稀疏文件，实际占盘接近 0）\n' "$FIXTURE_SIZE"
}

fixture_status() {
  if [ -d "$BACKUP_DIR" ]; then
    printf '状态: fixture 在位，真实内容在 %s\n' "$BACKUP_DIR"
    rtk ls -1 "$(fixture_file)"-* 2>/dev/null || true
  elif [ -d "$TARGET_DIR" ]; then
    printf '状态: 已还原（%s 是真实目录）\n' "$TARGET_DIR"
  else
    printf '状态: 异常 —— 目标目录不存在\n' >&2
    return 1
  fi
}

case "${1:-}" in
  up)     fixture_up ;;
  down)   restore ;;
  status) fixture_status ;;
  *)      printf '用法: %s [up|down|status]\n' "$0" >&2; exit 2 ;;
esac