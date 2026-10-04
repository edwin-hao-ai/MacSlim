#!/usr/bin/env python3
"""为**本地截图构建**签一份 Development provisioning profile。

## 为什么需要它

MAS 包带的是 App Store **生产** profile，而 macOS 只认经 App Store / Xcode
安装的产物。本地直接跑会被拒：

    taskgated: embedded provisioning profile not valid, Code=-215
               "Only Development Provisioning Profiles can be installed..."
    amfid:     -413 "No matching profile found"  → open 报 Error 162

而截图必须是**沙箱下**的真实行为：上架版没授权就读不到 `~/Library/Caches`，
所以「授权 → 数据 → 清理」这条叙事线只有在沙箱里才成立。

于是只剩一条路：用 **Development** profile 在本地跑沙箱版。macOS 接受
Development profile 本地安装，于是：

| 签名方式 | 能启动 | 有沙箱 | 截图可信 |
| --- | --- | --- | --- |
| 生产 profile（原样） | ✗ Error 162 | 有 | — |
| ad-hoc 剥掉 entitlement | ✓ | **无** | ✗ 会直接读全部缓存 |
| Development profile（本脚本） | ✓ | 有 | ✓ |

## 与 create_mas_profile.py 的分工

`create_mas_profile.py` 签的是**上架用**的生产 profile（MAC_APP_STORE，
不含设备）。本脚本签的是**本地跑**用的 Development profile
（MAC_APP_DEVELOPMENT，绑定本机）。两者 entitlement 集合必须一致 ——
否则本地跑通的沙箱权限组合和上架那份不是一回事，截图就又不可信了。

用法: python3 scripts/create_screenshot_dev_profile.py
"""
from __future__ import annotations

import base64
import json
import os
import plistlib
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from create_mas_profile import (  # noqa: E402
    BUNDLE_ID,
    await_profile_content,
    call,
    desired_entitlements,
    fail,
    find_bundle_id,
    get_all,
    info,
    load_env,
)

ROOT = Path(__file__).resolve().parents[1]
OUT = Path(
    os.environ.get(
        "MAS_DEV_PROFILE",
        os.path.expanduser("~/.cargo/shared-target-mas/MacSlim-dev.provisionprofile"),
    )
)


def find_development_certificate(env: dict) -> str:
    """挑一张 Mac 开发证书。

    ASC 的 certificateType 里 macOS 开发证书是 `MAC_APP_DEVELOPMENT`；
    纯 iOS 通用开发证书是 `DEVELOPMENT`，带不上 macOS entitlement，所以不用。
    """
    for wanted in ("MAC_APP_DEVELOPMENT", "DEVELOPMENT"):
        for item in get_all("/certificates", env):
            if item.get("attributes", {}).get("certificateType") == wanted:
                info(f"用证书 {item['id']}（{wanted}）")
                return item["id"]
    fail("证书库里没有 Mac 开发证书（MAC_APP_DEVELOPMENT）")


def find_this_mac(env: dict) -> str:
    """找到本机在开发者后台注册的设备 id。

    Development profile 必须绑定设备，否则签名和设备不匹配会被拒
    （「certificate is not valid for this device」类问题）。多台机器时按
    当前主机名匹配，匹配不上就把清单打出来让人选，不猜。
    """
    devices = get_all("/devices", env)
    if not devices:
        fail("开发者后台没有注册任何设备 —— Development profile 必须绑定设备")
    if len(devices) == 1:
        info(f"只有一台注册设备，直接用：{devices[0]['id']}")
        return devices[0]["id"]

    wanted = subprocess.run(
        ["scutil", "--get", "ComputerName"],
        capture_output=True,
        text=True,
    ).stdout.strip()
    for device in devices:
        attributes = device.get("attributes", {})
        if wanted and wanted in (attributes.get("name") or ""):
            info(f"按主机名匹配到设备：{attributes.get('name')}（{device['id']}）")
            return device["id"]

    listing = "\n".join(
        f"  {d['id']}  {d.get('attributes', {}).get('name')}  "
        f"({d.get('attributes', {}).get('platform')})"
        for d in devices
    )
    fail(f"有 {len(devices)} 台注册设备，无法判断哪台是本机（主机名 {wanted!r}）：\n{listing}")


def profile_name(entitlements: dict) -> str:
    """名字带上 entitlement 指纹。

    ASC 不允许重名（实测 409），所以同一套 entitlement 必须永远算出同一个
    名字，否则重复运行会一直新建。理由同 create_mas_profile.profile_name。
    """
    import hashlib

    digest = hashlib.sha256(
        json.dumps(entitlements, sort_keys=True).encode("utf-8")
    ).hexdigest()[:8]
    return f"MacSlim screenshot dev {digest}"


def find_reusable(env: dict, name: str) -> str | None:
    """同名且已签发的 profile 直接复用。

    重复运行必须幂等：ASC 不允许重名（实测 409），而 profile 又无法用 API
    删除（DELETE 返回 405），所以每次都新建会在后台堆出一串同名冲突。
    """
    for item in get_all("/profiles", env):
        attributes = item.get("attributes", {})
        if attributes.get("name") == name and attributes.get("profileContent"):
            info(f"复用已存在的 profile：{name}")
            return attributes["profileContent"]
    return None


def create(env: dict, name: str) -> str:
    body = {
        "data": {
            "type": "profiles",
            "attributes": {"name": name, "profileType": "MAC_APP_DEVELOPMENT"},
            "relationships": {
                "bundleId": {"data": {"type": "bundleIds", "id": find_bundle_id(env)}},
                "certificates": {
                    "data": [
                        {"type": "certificates", "id": find_development_certificate(env)}
                    ]
                },
                "devices": {"data": [{"type": "devices", "id": find_this_mac(env)}]},
            },
        }
    }
    created = call("POST", "/profiles", env, body)
    profile_id = created["data"]["id"]
    info(f"Development profile 已创建：{profile_id}（{name}）")
    return created["data"].get("attributes", {}).get("profileContent") or (
        await_profile_content(env, profile_id)
    )


def verify(decoded: dict) -> None:
    """自检：只核对**身份字段**。

    一开始这里拿去和 entitlements.mas.plist 全量比对，报「profile 缺
    app-sandbox / bookmarks.app-scope」—— 而上架那份 MAC_APP_STORE profile
    里同样没有这几条：沙箱 entitlement 是签名时给的，不进 profile。
    真正会静默出事的是身份字段对不上（签名用 A 证书、profile 绑 B 证书），
    所以只查这部分。
    """
    entitlements = decoded.get("Entitlements", {})
    for required in (
        "com.apple.application-identifier",
        "com.apple.developer.team-identifier",
    ):
        if not entitlements.get(required):
            fail(f"profile 里没有 {required}")
    if not decoded.get("ProvisionedDevices"):
        fail("profile 没有绑定设备 —— Development profile 不带设备签不出可运行的包")
    info(f"身份字段核对通过：{entitlements['com.apple.application-identifier']}")
    info(f"绑定设备：{decoded['ProvisionedDevices']}")
    info(f"过期时间：{decoded.get('ExpirationDate')}")


def main() -> None:
    env = load_env()
    name = profile_name(desired_entitlements())
    content = find_reusable(env, name) or create(env, name)

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(base64.b64decode(content))
    info(f"已写入 {OUT}")

    decoded = plistlib.loads(
        subprocess.run(
            ["security", "cms", "-D", "-i", str(OUT)], capture_output=True
        ).stdout
    )
    verify(decoded)


if __name__ == "__main__":
    os.environ.setdefault("PYTHONDONTWRITEBYTECODE", "1")
    main()
