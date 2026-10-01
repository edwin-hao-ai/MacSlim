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
                    "appScreenshotSet": {
                        "data": {"type": "appScreenshotSets", "id": set_id}
                    }
                },
            }
        },
    )
    attributes = created["data"]["attributes"]
    operations = attributes.get("uploadOperations") or []
    if not operations:
        fail(f"{path.name} 资源已建但没给上传操作，无法上传")
    screenshot_id = created["data"]["id"]
    return screenshot_id, {"operations": operations}


def commit(env: dict, screenshot_id: str) -> str | None:
    """把资源标记为「上传完毕」，并回读交付状态。

    ## 这一步不能省

    PUT 全部返回 2xx 之后，资源状态仍然是 `AWAITING_UPLOAD` —— 实测等 45 秒
    也不变，说明不是异步生效，而是**确实还差一次提交**。不清掉的话，App Store
    上会留下一排坏掉的截图位。

    ## 属性名又是从 409 里挖出来的

    先试过两种直觉写法，都被挡回：

        ENTITY_ERROR.ATTRIBUTE.NOT_ALLOWED
        The attribute 'assetToken' can not be included in a 'UPDATE' operation

        ENTITY_ERROR.ATTRIBUTE.INVALID   （空 attributes 的 UPDATE）
        'Uploaded flag is not set!'  pointer: /data/attributes/uploaded

    真名是 `uploaded`。Apple 的文档 markdown 不列字段，409 的 detail 才是
    唯一可信来源 —— 所以这里保留「提交后回读状态」，让判据来自服务端。
    """
    mas.request(
        "PATCH",
        f"/appScreenshots/{screenshot_id}",
        env,
        {
            "data": {
                "type": "appScreenshots",
                "id": screenshot_id,
                "attributes": {"uploaded": True},
            }
        },
    )
    return delivery_state(env, screenshot_id)


def delivery_state(env: dict, screenshot_id: str) -> str | None:
    """回读一张截图的交付状态，用来证明「真的上传完了」。

    没有这一步的话，脚本在 PUT 返回 200 之后就宣布成功 —— 但 PUT 成功只说明
    字节进了 Apple 的暂存区，不代表资源已被接受。判据要来自服务端。

    状态取值有**两个**都算成功：提交 PATCH 的响应里先是 `UPLOAD_COMPLETE`，
    稍后再查就变成 `COMPLETE`。只认一个会让脚本在另一种时序下误报失败 ——
    而误报失败的代价是重跑一次（重跑会往集合里塞重复图）。
    """
    try:
        got = mas.request("GET", f"/appScreenshots/{screenshot_id}", env)
    except SystemExit:
        return None
    raw = got["data"].get("attributes", {}).get("assetDeliveryState")
    if isinstance(raw, dict):
        return raw.get("state")
    return raw


DONE_STATES = ("UPLOAD_COMPLETE", "COMPLETE")


def normalize_headers(raw) -> dict:
    """把 `requestHeaders` 的三种历史形态统一成 dict。

    ASC 实测返回的是 **list of dict**：

        "requestHeaders": [ { "name": "Content-Type", "value": "image/png" } ]

    但历史上见过另外两种：直接给 dict，以及给 [[key, value], ...]。三种都收下，
    遇到不认识的形状就当场报错 —— 静默返回空 dict 会变成「不带 Content-Type
    上传」，Apple 可能收下却把图当损坏资产，而我们要到看图才发现。
    """
    if raw is None:
        return {}
    if isinstance(raw, dict):
        return raw
    headers = {}
    if isinstance(raw, list):
        for item in raw:
            if isinstance(item, dict) and "name" in item:
                headers[item["name"]] = item.get("value")
            elif isinstance(item, (list, tuple)) and len(item) == 2:
                headers[item[0]] = item[1]
            else:
                fail(f"看不懂的 requestHeaders 形状：{item!r}")
        return headers
    return fail(f"看不懂的 requestHeaders 类型：{type(raw).__name__}")


def put_operations(operations: list, path: Path) -> str:
    """按上传指令逐条 PUT 二进制。"""
    payload_bytes = path.read_bytes()
    for operation in operations:
        headers = normalize_headers(operation.get("requestHeaders"))
        offset = operation.get("offset") or 0
        length = operation.get("length")
        chunk = (
            payload_bytes
            if length is None
            else payload_bytes[offset : offset + length]
        )
        status = _raw(
            operation.get("method", "PUT"), operation["url"], headers, chunk
        )
        if status not in (200, 201, 204):
            fail(f"{path.name} 上传失败：HTTP {status}")
    return "、".join(o.get("method", "PUT") for o in operations)


def upload(env: dict, set_id: str, path: Path) -> str:
    screenshot_id, plan = create_screenshot(env, set_id, path)
    methods = put_operations(plan["operations"], path)
    state = commit(env, screenshot_id)
    if state not in DONE_STATES:
        fail(
            f"{path.name} 已提交但交付状态是 {state or '未知'}，"
            "这张图不会被 App Store 采用"
        )
    print(f"  已上传 {path.name}（{methods}）交付状态={state}")
    return screenshot_id


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
    ids: list[str] = []
    for shot in shots:
        ids.append(upload(env, set_id, shot))
        uploaded.append(shot.name)

    state[args.locale] = {
        "set": set_id,
        "screenshots": uploaded,
        "ids": ids,
    }
    save_state(state)
    print(f"\n完成，set {set_id} 共 {len(uploaded)} 张。")


if __name__ == "__main__":
    main()