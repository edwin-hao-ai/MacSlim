#!/usr/bin/env bash
# MacSlim 更新发布脚本
#
# 用法:
#   ./scripts/publish-update.sh <version> "<release notes>" [arm|intel|universal]
#
# 用法:
#   ./scripts/publish-update.sh 0.2.0 "修复若干安全问题，新增 Rust 缓存扫描" arm
#
# 前置:
#   1. 已经跑过一次带 updater artifacts 的 Tauri release 打包
#   2. 环境变量里有 TAURI_SIGNING_PRIVATE_KEY 和 TAURI_SIGNING_PRIVATE_KEY_PASSWORD
#      （或者 TAURI_SIGNING_PRIVATE_KEY_PATH 指向密钥文件）
#
# 输出:
#   - landing/updates/latest.json  新客户端更新清单
#   - landing/updates/<platform>/0.1.0.json、0.2.2.json  旧客户端兼容清单
#   - landing/downloads/MacSlim_<version>_<arch>.app.tar.gz(.sig,.build-stamp.json)
#
# 部署:
#   把 landing/ 整目录推到 https://edwin-hao-ai.github.io/MacSlim，并把版本化 updater 资产上传到对应 GitHub Release。

set -euo pipefail

VERSION="${1:-}"
NOTES="${2:-}"
TARGET="${3:-arm}"

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "错误: version 必须是三段 SemVer" >&2
  exit 1
fi

CONFIGURED_VERSION="$(python3 -c 'import json; print(json.load(open("src-tauri/tauri.conf.json", encoding="utf-8"))["version"])')"
if [ "$VERSION" != "$CONFIGURED_VERSION" ]; then
  echo "错误: 发布版本与 tauri.conf.json 不一致" >&2
  exit 1
fi

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
    echo "用法: $0 <version> \"<release notes>\" [arm|intel|universal]" >&2
    exit 1
    ;;
esac

if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ] && [ -z "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" ]; then
  if [ -f "$HOME/.tauri/macslim-updater.key" ]; then
    export TAURI_SIGNING_PRIVATE_KEY_PATH="$HOME/.tauri/macslim-updater.key"
    if [ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD+x}" ] || [ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD-}" ]; then
      echo "错误: 使用默认 updater 密钥时必须设置 TAURI_SIGNING_PRIVATE_KEY_PASSWORD" >&2
      exit 1
    fi
    echo "==> 使用 Keychain/环境变量提供的 updater 签名密码"
  else
    echo "错误: 未找到 updater 私钥" >&2
    exit 1
  fi
fi

if [ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD+x}" ] || [ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD-}" ]; then
  echo "错误: 使用 updater 私钥时必须设置 TAURI_SIGNING_PRIVATE_KEY_PASSWORD" >&2
  exit 1
fi

if [ -n "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" ] && [ ! -f "$TAURI_SIGNING_PRIVATE_KEY_PATH" ]; then
  echo "错误: updater 私钥文件不存在" >&2
  exit 1
fi

UPDATER_SRC="src-tauri/target/${RUST_TARGET}/release/bundle/macos/MacSlim.app.tar.gz"
if [ ! -f "$UPDATER_SRC" ]; then
  echo "错误: 找不到 Tauri updater artifact $UPDATER_SRC" >&2
  echo "请先执行带 bundle.createUpdaterArtifacts 的 Tauri build" >&2
  exit 1
fi
UPDATER_SRC_STAMP_PATH="${UPDATER_SRC}.build-stamp.json"

platforms_for_arch() {
  case "$1" in
    aarch64) printf '%s' "darwin-aarch64" ;;
    x64) printf '%s' "darwin-x86_64" ;;
    universal) printf '%s' "darwin-aarch64 darwin-x86_64" ;;
    *) return 1 ;;
  esac
}

target_for_arch() {
  case "$1" in
    aarch64) printf '%s' "aarch64-apple-darwin" ;;
    x64) printf '%s' "x86_64-apple-darwin" ;;
    universal) printf '%s' "universal-apple-darwin" ;;
    *)
      echo "错误: updater 资产架构不受支持: $2" >&2
      return 1
      ;;
  esac
}

verify_existing_asset() {
  local asset="$1"
  local asset_name="$2"
  local asset_arch="$3"
  local asset_target="$4"
  local seen_arches="$5"
  if [ ! -f "${asset}.build-stamp.json" ]; then
    echo "错误: updater 资产缺少 build stamp sidecar: $asset_name" >&2
    return 1
  fi
  case " $seen_arches " in
    *" $asset_arch "*)
      echo "错误: updater 资产架构重复: $asset_arch" >&2
      return 1
      ;;
  esac
  python3 scripts/updater_artifact.py verify \
    --archive "$asset" \
    --target "$asset_target" \
    --stamp "${asset}.build-stamp.json"
}

append_asset_platforms() {
  local seen_platforms="$1"
  local asset_platforms="$2"
  for platform in $asset_platforms; do
    case " $seen_platforms " in
      *" $platform "*)
        echo "错误: updater 平台匹配重复: $platform" >&2
        return 1
        ;;
    esac
    seen_platforms="${seen_platforms}${platform} "
  done
  printf '%s' "$seen_platforms"
}

check_current_platforms() {
  local current_arch="$1"
  local seen_arches="$2"
  local seen_platforms="$3"
  local current_platforms
  current_platforms="$(platforms_for_arch "$current_arch")"
  local current_arch_seen=0
  case " $seen_arches " in
    *" $current_arch "*) current_arch_seen=1 ;;
  esac
  if [ "$current_arch_seen" -eq 0 ]; then
    for platform in $current_platforms; do
      case " $seen_platforms " in
        *" $platform "*)
          echo "错误: 当前 target 与既有 updater 平台冲突: $platform" >&2
          return 1
          ;;
      esac
    done
  fi
  return 0
}

verify_existing_assets() {
  local root_dir="$1"
  local current_arch="$2"
  local downloads_dir="${root_dir}/landing/downloads"
  shopt -s nullglob
  local assets=("${downloads_dir}"/MacSlim_"${VERSION}"_*.app.tar.gz)
  if [ "${#assets[@]}" -eq 0 ]; then
    return 0
  fi
  local seen_arches=""
  local seen_platforms=""
  for asset in "${assets[@]}"; do
    local asset_name="${asset##*/}"
    local asset_arch="${asset_name#MacSlim_${VERSION}_}"
    asset_arch="${asset_arch%.app.tar.gz}"
    local asset_target
    asset_target="$(target_for_arch "$asset_arch" "$asset_name")" || return 1
    verify_existing_asset "$asset" "$asset_name" "$asset_arch" "$asset_target" "$seen_arches" || return 1
    seen_arches="${seen_arches}${asset_arch} "
    local asset_platforms
    asset_platforms="$(platforms_for_arch "$asset_arch")"
    seen_platforms="$(append_asset_platforms "$seen_platforms" "$asset_platforms")" || return 1
  done
  check_current_platforms "$current_arch" "$seen_arches" "$seen_platforms" || return 1
  return 0
}

python3 scripts/updater_artifact.py verify \
  --archive "$UPDATER_SRC" \
  --target "$RUST_TARGET"
verify_existing_assets "." "$ARCH"

if [ "${MACSlim_PUBLISH_PREFLIGHT_ONLY:-}" = "1" ]; then
  echo "==> Updater 发布预检通过"
  exit 0
fi

STAGING_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/macslim-publish.XXXXXX")"
cleanup() {
  local status=$?
  if [ -n "${STAGING_ROOT:-}" ] && [ -d "$STAGING_ROOT" ]; then
    rm -rf "$STAGING_ROOT"
  fi
  exit "$status"
}
trap cleanup EXIT
STAGING_DOWNLOAD_DIR="${STAGING_ROOT}/landing/downloads"
STAGING_UPDATE_DIR="${STAGING_ROOT}/landing/updates"
mkdir -p "$STAGING_DOWNLOAD_DIR" "$STAGING_UPDATE_DIR"
UPDATER_BASENAME="MacSlim_${VERSION}_${ARCH}.app.tar.gz"
UPDATER_PATH="${STAGING_DOWNLOAD_DIR}/${UPDATER_BASENAME}"
UPDATER_SIG_PATH="${UPDATER_PATH}.sig"
UPDATER_STAMP_PATH="${UPDATER_PATH}.build-stamp.json"

copy_existing_assets() {
  shopt -s nullglob
  local assets=(landing/downloads/MacSlim_"${VERSION}"_*.app.tar.gz)
  if [ "${#assets[@]}" -eq 0 ]; then
    return 0
  fi
  for asset in "${assets[@]}"; do
    cp "$asset" "$STAGING_DOWNLOAD_DIR/${asset##*/}"
    cp "${asset}.build-stamp.json" "$STAGING_DOWNLOAD_DIR/${asset##*/}.build-stamp.json"
    if [ -f "${asset}.sig" ]; then
      cp "${asset}.sig" "$STAGING_DOWNLOAD_DIR/${asset##*/}.sig"
    fi
  done
}

copy_existing_assets
cp "$UPDATER_SRC" "$UPDATER_PATH"
cp "$UPDATER_SRC_STAMP_PATH" "$UPDATER_STAMP_PATH"
verify_existing_assets "$STAGING_ROOT" "$ARCH"

sign_staged_assets() {
  shopt -s nullglob
  local assets=("$STAGING_DOWNLOAD_DIR"/MacSlim_"${VERSION}"_*.app.tar.gz)
  if [ "${#assets[@]}" -eq 0 ]; then
    return 0
  fi
  for asset in "${assets[@]}"; do
    rm -f "${asset}.sig"
    bun tauri signer sign "$asset"
    if [ ! -f "${asset}.sig" ]; then
      echo "错误: 签名文件 ${asset}.sig 未生成" >&2
      return 1
    fi
  done
  return 0
}

sign_staged_assets

PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
VERSION="$VERSION" \
NOTES="$NOTES" \
PUB_DATE="$PUB_DATE" \
DOWNLOAD_DIR="$STAGING_DOWNLOAD_DIR" \
UPDATE_DIR="$STAGING_UPDATE_DIR" \
python3 - <<'PY'
import json
import os
import pathlib
import re
import subprocess
import sys
from urllib.parse import quote

version = os.environ["VERSION"]
download_dir = pathlib.Path(os.environ["DOWNLOAD_DIR"])
update_dir = pathlib.Path(os.environ["UPDATE_DIR"])
platforms = {}
assets = {}
pattern = re.compile(rf"^MacSlim_{re.escape(version)}_([A-Za-z0-9_-]+)\.app\.tar\.gz$")
arch_platforms = {
    "aarch64": ("darwin-aarch64",),
    "x64": ("darwin-x86_64",),
    "universal": ("darwin-aarch64", "darwin-x86_64"),
}

for path in sorted(download_dir.iterdir()):
    if not path.is_file():
        continue
    if path.name.endswith(".build-stamp.json"):
        continue
    match = pattern.fullmatch(path.name)
    if match is None:
        if path.name.startswith(f"MacSlim_{version}_") and not path.name.endswith(".sig"):
            raise SystemExit(f"错误: updater 资产 basename 不符合契约: {path.name}")
        continue
    arch = match.group(1)
    if arch not in arch_platforms:
        raise SystemExit(f"错误: updater 资产架构不受支持: {path.name}")
    if arch in assets:
        raise SystemExit(f"错误: updater 资产架构重复: {arch}")
    stamp_path = pathlib.Path(f"{path}.build-stamp.json")
    if not stamp_path.is_file():
        raise SystemExit(f"错误: updater 资产缺少 build stamp sidecar: {path.name}")
    target = {
        "aarch64": "aarch64-apple-darwin",
        "x64": "x86_64-apple-darwin",
        "universal": "universal-apple-darwin",
    }[arch]
    verified = subprocess.run(
        [
            sys.executable,
            "scripts/updater_artifact.py",
            "verify",
            "--archive",
            str(path),
            "--target",
            target,
            "--stamp",
            str(stamp_path),
        ],
        cwd=pathlib.Path.cwd(),
        capture_output=True,
        text=True,
        check=False,
    )
    if verified.returncode != 0:
        raise SystemExit(f"错误: updater archive 校验失败: {path.name}")
    signature_path = pathlib.Path(f"{path}.sig")
    if not signature_path.is_file():
        raise SystemExit(f"错误: 缺少 updater 签名: {signature_path.name}")
    signature = signature_path.read_text(encoding="utf-8").strip()
    if not signature:
        raise SystemExit(f"错误: updater 签名为空: {signature_path.name}")
    assets[arch] = path.name
    for platform in arch_platforms[arch]:
        if platform in platforms:
            raise SystemExit(f"错误: updater 平台匹配重复: {platform}")
        platforms[platform] = {
            "signature": signature,
            "url": f"https://github.com/edwin-hao-ai/MacSlim/releases/download/v{version}/{quote(path.name)}",
        }

if not platforms:
    raise SystemExit("错误: 没有找到版本化 updater artifact")

manifest = {
    "version": version,
    "notes": os.environ["NOTES"],
    "pub_date": os.environ["PUB_DATE"],
    "platforms": platforms,
}


def write_manifest(path: pathlib.Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(manifest, indent=2, ensure_ascii=False), encoding="utf-8")


write_manifest(update_dir / "latest.json")
for platform in platforms:
    for old_version in ("0.1.0", "0.2.2"):
        write_manifest(update_dir / platform / f"{old_version}.json")
PY

python3 scripts/publish_assets.py commit \
  --staging-root "$STAGING_ROOT" \
  --live-root "." \
  --version "$VERSION"

echo ""
echo "✅ Updater 更新包已生成"
echo "   Asset:     landing/downloads/${UPDATER_BASENAME}"
echo "   Manifest:  landing/updates/latest.json"
echo "   Compat:    landing/updates/darwin-aarch64/0.1.0.json, landing/updates/darwin-aarch64/0.2.2.json"
echo ""
echo "下一步：把 landing/ 部署到 GitHub Pages，并把版本化 updater 资产上传到对应 GitHub Release"
echo "已安装 MacSlim 的用户在「设置」→「检查更新」会看到新版本"
