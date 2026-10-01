#!/usr/bin/env python3
"""把截屏上传到 App Store Connect。

## 为什么要有这个脚本

ASC 的截屏要走三段式资源模型（`appStoreVersionLocalizations` →
`appStoreScreenshotSets` → `appScreenshots`），而 **GET_COLLECTION 全被禁用**，
意味着「先列出现有的再补缺」这条路走不通 —— 只能靠本地状态判断，
或者干脆每次重建。

所以这个脚本采取的策略是：**本地记状态 + 幂等重建**。
`state/screenshots.json` 记录「这个本地化已经建过哪几个 set」，
重复运行不会重复创建。

## 尺寸要求

macOS 平台只接受这几档（点单位）：

- 1280 x 800
- 1440 x 900
- 2560 x 1600
- 2880 x 1800

Retina 屏上 `screencapture` 出的全屏图就是 2880 x 1800，正好合规。
本脚本会**先校验尺寸**，不合规就直接停下并报出实际尺寸 ——
让 Apple 回一个含糊的 400 不如自己说清楚。

## 用法

```bash
python3 scripts/upload_mas_screenshots.py --dir /tmp/ms-shot/final
```
"""
from __future__ import annotations

import argparse
import base64
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "mas_profile", ROOT / "scripts/create_mas_profile.py"
)
mas = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(mas)

spec2 = importlib.util.spec_from_file_location(
    "publish_meta", ROOT / "scripts/publish_mas_metadata.py"
)
meta = importlib.util.module_from_spec(spec2)
assert spec2.loader is not None
spec2.loader.exec_module(meta)

VERSION_ID = meta.VERSION_ID
STATE = ROOT / "state/screenshots.json"

ALLOWED_SIZES = [(1280, 800), (1440, 900), (2560, 1600), (2880, 1800)]

# macOS 只有一个截图类型：APP_DESKTOP。
#
# 之前这里抄了 iOS 的两套（APP_DESKTOP + APP_DETAILS）并把图交错分进两个
# set，那是从 iPhone「6.5"/6.7" 两档展示」搬过来的形状，macOS 根本没有这回事。
# 后果不只是多两个空 set：`APP_DETAILS` 在 macOS 版本下建出来永远不会被
# 审核看到，却会让 App Store Connect 里出现两套互相矛盾的图。
#
# 属性名也是实测出来的：ASC 对 `appScreenshotSets` 只认 `screenshotDisplayType`
# 一个字段。写 `appStoreScreenshotType` / `appStoreScreenshotDisplayType`
# 会被 409 挡回：
#   ENTITY_ERROR.ATTRIBUTE.UNKNOWN
#   "'appStoreScreenshotType' is not an attribute on the resource 'appScreenshotSets'"
SCREENSHOT_DISPLAY_TYPE = "APP_DESKTOP"


def fail(message: str) -> None:
    print(f"错误: {message}")
    raise SystemExit(1)


def png_size(path: Path) -> tuple[int, int]:
    """读 PNG 头里的宽高。不解码像素 —— 截图动辄几 MB，没必要。"""
    with path.open("rb") as handle:
        header = handle.read(24)
    if header[:8] != b"\x89PNG\r\n\x1a\n":
        fail(f"{path.name} 不是 PNG")
    width = int.from_bytes(header[16:20], "big")
    height = int.from_bytes(header[20:24], "big")
    return width, height


def load_state() -> dict:
    if STATE.exists():
        return json.loads(STATE.read_text(encoding="utf-8"))
    return {}


def save_state(state: dict) -> None:
    STATE.parent.mkdir(parents=True, exist_ok=True)
    STATE.write_text(json.dumps(state, ensure_ascii=False, indent=2), encoding="utf-8")


def find_localization(env: dict, locale: str) -> str:
    found = mas.request(
        "GET",
        f"/appStoreVersions/{VERSION_ID}/appStoreVersionLocalizations?filter[locale]={locale}",
        env,
    )["data"]
    if isinstance(found, list):
        if not found:
            fail(f"版本下没有 {locale} 本地化 —— 先跑 publish_mas_metadata.py")
        return found[0]["id"]
    return found["id"]


def find_set(env: dict, localization_id: str, type_name: str) -> str | None:
    found = mas.request(
        "GET",
        f"/appStoreVersionLocalizations/{localization_id}/appScreenshotSets"
        f"?filter[screenshotDisplayType]={type_name}",
        env,
    )["data"]
    if isinstance(found, list):
        return found[0]["id"] if found else None
    return None


def ensure_set(env: dict, localization_id: str, type_name: str) -> str:
    existing = find_set(env, localization_id, type_name)
    if existing:
        print(f"  复用 screenshotSet {existing} ({type_name})")
        return existing
    created = mas.request(
        "POST",
        "/appScreenshotSets",
        env,
        {
            "data": {
                "type": "appScreenshotSets",
                "attributes": {"screenshotDisplayType": type_name},
                "relationships": {
                    "appStoreVersionLocalization": {
                        "data": {
                            "type": "appStoreVersionLocalizations",
                            "id": localization_id,
                        }
                    }
                },
            }
        },
    )
    new_id = created["data"]["id"]
    print(f"  创建 screenshotSet {new_id} ({type_name})")
    return new_id


def create_screenshot(env: dict, set_id: str, path: Path) -> tuple[str, dict]:
    """先建资源，拿回上传指令。

    ASC 不会用一个请求收下图片 —— 它先返回一组 `uploadOperations`
    （每条自带 URL / method / headers），二进制要照着它们逐条 PUT 上去。
    所以这是两阶段，缺任何一步都是 400。
    """
    created = mas.request(
        "POST",
        "/appScreenshots",
        env,
        {
            "data": {
                "type": "appScreenshots",
                "attributes": {
                    "fileSize": path.stat().st_size,
                    "fileName": path.name,
                },
                "relationships": {
                    "appStoreScreenshotSet": {
                        "data": {"type": "appStoreScreenshotSets", "id": set_id}
                    }
                },
            }
        },
    )
    attributes = created["data"]["attributes"]
    operations = attributes.get("uploadOperations") or []
    if not operations:
        fail(f"{path.name} 资源已建但没给上传操作，无法上传")
    # 把占位的两个字段清成 null，ASC 才认为这次上传是完整的。
    # 不发这一步资源会一直停在「上传中」。
    screenshot_id = created["data"]["id"]
    mas.request(
        "PATCH",
        f"/appScreenshots/{screenshot_id}",
        env,
        {
            "data": {
                "type": "appScreenshots",
                "id": screenshot_id,
                "attributes": {"assetToken": None, "sourceFileChecksum": None},
            }
        },
    )
    return screenshot_id, {"operations": operations}


def put_operations(operations: list, path: Path) -> str:
    """按上传指令逐条 PUT 二进制。"""
    payload_bytes = path.read_bytes()
    for operation in operations:
        headers = operation.get("requestHeaders") or {}
        if isinstance(headers, list):
            # 历史上 ASC 有时返回 [[key, value], ...] 的形式
            headers = {pair[0]: pair[1] for pair in headers}
        status = _raw(
            operation.get("method", "PUT"), operation["url"], headers, payload_bytes
        )
        if status not in (200, 201, 204):
            fail(f"{path.name} 上传失败：HTTP {status}")
    return "、".join(o.get("method", "PUT") for o in operations)


def upload(env: dict, set_id: str, path: Path) -> None:
    _, plan = create_screenshot(env, set_id, path)
    print(f"  已上传 {path.name}（{put_operations(plan['operations'], path)}）")


def _raw(method: str, url: str, headers: dict, body: bytes) -> int:
    import requests

    for attempt in range(4):
        try:
            return requests.request(method, url, headers=headers, data=body, timeout=180).status_code
        except requests.RequestException as error:
            last = error
            import time

            time.sleep(2**attempt)
    fail(f"上传网络失败（已重试 4 次）：{last}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dir", required=True, help="截屏目录（文件名按 ASCII 排序即展示顺序）")
    parser.add_argument("--locale", default="zh-Hans")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    directory = Path(args.dir)
    shots = sorted(p for p in directory.glob("*.png"))
    if not shots:
        fail(f"{directory} 里没有 PNG")

    print(f"截屏 {len(shots)} 张，规范尺寸 {ALLOWED_SIZES[-1]}")
    for shot in shots:
        size = png_size(shot)
        mark = "OK" if size in ALLOWED_SIZES else "不合规"
        print(f"  [{mark}] {shot.name}: {size[0]}x{size[1]}（{shot.stat().st_size // 1024} KB）")
    bad = [s for s in shots if png_size(s) not in ALLOWED_SIZES]
    if bad:
        fail(
            "以下截图尺寸不合规，ASC 只接受 " + " / ".join(f"{w}x{h}" for w, h in ALLOWED_SIZES)
            + "：" + "、".join(s.name for s in bad)
        )

    if args.dry_run:
        print("\ndry-run，未写入")
        return

    env = mas.load_env()
    localization_id = find_localization(env, args.locale)
    state = load_state()

    print(f"\n[{SCREENSHOT_DISPLAY_TYPE}] {len(shots)} 张")
    set_id = ensure_set(env, localization_id, SCREENSHOT_DISPLAY_TYPE)
    uploaded: list[str] = []
    for shot in shots:
        upload(env, set_id, shot)
        uploaded.append(shot.name)

    state[args.locale] = {
        "set": set_id,
        "screenshots": uploaded,
    }
    save_state(state)
    print(f"\n完成，set {set_id} 共 {len(uploaded)} 张。")


if __name__ == "__main__":
    main()