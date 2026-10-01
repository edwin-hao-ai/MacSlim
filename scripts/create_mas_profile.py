#!/usr/bin/env python3
"""通过 App Store Connect API 生成/刷新 Mac App Store 的 provisioning profile。

## 为什么需要它

MAS 签名有两层：**我们自己的 entitlements 文件** 和 **profile 里授权的
entitlements**。两层必须一致，缺一不可。

实测踩过（2026-09-30）：给 `entitlements.mas.plist` 加了
`com.apple.security.files.bookmarks.app-scope` 并重新签名，
`codesign -d --entitlements` 里能看到它 —— 但 profile 是**加之前**生成的，
里面没有这条。于是内核不认，`URLByResolvingBookmarkData:` 静默返回 nil：

    创建书签 → 成功
    解析书签 → 失败

也就是说**每加一条 entitlement 就必须重签一次 profile**，否则症状是
「代码看起来完全正常，功能就是不工作」。

## 为什么不自动跑

profile 只能通过 App Store Connect API 签发，属于需要网络与凭据的操作。
本脚本是显式调用的，不挂在任何构建/提交钩子上。

## 用法

```bash
# 从 src-tauri/entitlements.mas.plist 读取 entitlements 并生成 profile
python3 scripts/create_mas_profile.py

# 只看现状，不联网（顺带核对签名里的沙箱权限）
python3 scripts/create_mas_profile.py --show --app /path/to/MacSlim.app
```

## 一个容易查错方向的地方

**Mac App Store 的 provisioning profile 不带沙箱 entitlement。**
它只带 `application-identifier` / `keychain-access-groups` / `team-id`。
MAS 应用的沙箱权限来自**签名**（`codesign --entitlements`），由 App Store
在处理包时校验。

所以：不要因为「profile 里没有 app-sandbox」就去重新签 profile ——
本脚本第一版正是犯了这个错，还顺手把排查引到了「重新签 profile」上。
`--show` 现在会把两处分开列。

凭据从 `~/.config/mddock/ios-release.env` 读（仓库外，不进 git）。
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import plistlib
import subprocess
import sys
import time
from pathlib import Path

import jwt
import requests

ROOT = Path(__file__).resolve().parents[1]
ENTITLEMENTS = ROOT / "src-tauri" / "entitlements.mas.plist"
PROFILE = Path(
    os.environ.get(
        "MAS_PROFILE", Path.home() / ".config/mddock/MacSlim_MAS.mobileprovision"
    )
)
ENV_FILE = Path(
    os.environ.get("MAS_ASC_ENV", Path.home() / ".config/mddock/ios-release.env")
)

BUNDLE_ID = os.environ.get("MAS_BUNDLE_ID", "com.vgoapp.macslim")
API_BASE = "https://api.appstoreconnect.apple.com/v1"

# 这些 entitlement 是签名/沙箱机制本身，由系统隐含提供，不该也不能
# 在 profile 申请里出现。
IMPLICIT = {
    "com.apple.security.get-task-allow",
    "com.apple.application-identifier",
    "com.apple.developer.team-identifier",
    "keychain-access-groups",
}


def info(message: str) -> None:
    print(f"  {message}")


def fail(message: str) -> None:
    print(f"错误: {message}", file=sys.stderr)
    raise SystemExit(1)


def desired_entitlements() -> dict:
    """从 entitlements.mas.plist 读出需要在 profile 里授权的那部分。

    只取沙箱权限。签名（codesign -o runtime）之类的由签名参数控制，
    写进 profile 反而会被 ASC 拒。
    """
    with ENTITLEMENTS.open("rb") as handle:
        source = plistlib.load(handle)
    wanted = {
        key: value
        for key, value in source.items()
        if key.startswith("com.apple.security.") and key not in IMPLICIT
    }
    if not wanted:
        fail(f"{ENTITLEMENTS} 里一条 com.apple.security.* 都没有")
    return wanted


def load_env() -> dict:
    if not ENV_FILE.exists():
        fail(f"找不到 ASC 凭据：{ENV_FILE}")
    values = {}
    for line in ENV_FILE.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        value = value.strip().strip("'\"")
        # env 文件里习惯写 $HOME/... 而不是绝对路径（换机器不用改）。
        # 不展开的话这里会拿着字面量 ${HOME}/... 去 open，报「找不到私钥」
        # —— 一个看起来像凭据坏了、其实只是没做变量展开的错误。
        value = os.path.expandvars(value)
        values[key.strip()] = value
    for required in ("APPLE_API_ISSUER", "APPLE_API_KEY"):
        if required not in values:
            fail(f"{ENV_FILE} 里缺 {required}")
    return values


def token(env: dict) -> str:
    key_path = Path(env.get("APPLE_API_KEY_PATH", "")).expanduser()
    if not key_path.exists():
        fail(f"找不到私钥文件：{key_path}")
    now = int(time.time())
    claims = {
        "iss": env["APPLE_API_ISSUER"],
        "exp": now + 1200,
        "aud": "appstoreconnect-v1",
    }
    private_key = key_path.read_text(encoding="utf-8")
    return jwt.encode(claims, private_key, algorithm="ES256", headers={"kid": env["APPLE_API_KEY"]})


def request(method: str, path: str, env: dict, body: dict | None = None) -> dict:
    url = f"{API_BASE}{path}"
    headers = {
        "Authorization": f"Bearer {token(env)}",
        "Content-Type": "application/json",
    }
    # 网络调用必须有重试：Apple 的 API 会偶发 SSL EOF / 502，不重试就会把
    # 「网络抖了一下」表现成「凭据无效」这种完全错误的结论。
    #
    # 按方法**分别派发**，不要写成 `if GET else POST` —— 那会把 PATCH
    # 悄悄发成 POST，于是对「{resource}/{id}」这种只接受 PATCH 的路径
    # 返回 405 METHOD_NOT_ALLOWED，看起来像「这个资源不支持 PATCH」，
    # 实际是方法发错了。踩过：据此得出「只能删了重建」的错误结论，
    # 而 DELETE 主语言本地化其实也是禁止的（409）。
    senders = {
        "GET": lambda: requests.get(url, headers=headers, timeout=60),
        "POST": lambda: requests.post(url, headers=headers, json=body, timeout=60),
        "PATCH": lambda: requests.patch(url, headers=headers, json=body, timeout=60),
        "DELETE": lambda: requests.delete(url, headers=headers, timeout=60),
    }
    if method not in senders:
        fail(f"不支持的 HTTP 方法：{method}")
    last_error: Exception | None = None
    for attempt in range(4):
        try:
            response = senders[method]()
            break
        except requests.RequestException as error:
            last_error = error
            time.sleep(2**attempt)
    else:
        fail(f"{method} {path} 网络失败（已重试 4 次）：{last_error}")
    if response.status_code >= 400:
        fail(f"{method} {path} → HTTP {response.status_code}\n{response.text[:800]}")
    # 204 / 205 没有正文，硬解 JSON 会抛 JSONDecodeError，把「删除成功」表现成
    # 一堆无关的 requests 栈 —— 而 DELETE 在这里恰恰是常规操作（清理上传失败
    # 留下的孤儿资源）。空正文就返回空 dict，与有正文时同样可链式使用。
    if response.status_code in (204, 205) or not response.content.strip():
        return {}
    return response.json()


def call(method: str, path: str, env: dict, body: dict | None = None) -> dict:
    return request(method, path, env, body)


def get_all(path: str, env: dict, limit: int = 200) -> list:
    """翻完所有分页。

    ASC 的列表接口默认只给 20 条。团队里 bundleId 多到几十个（这里就有
    20+），只看第一页会得出「没有 com.vgoapp.macslim」这种**看起来很确定
    的错误结论** —— 而它只是翻页没翻完。
    """
    separator = "&" if "?" in path else "?"
    collected: list = []
    url = f"{path}{separator}limit={limit}"
    while url:
        page = request("GET", url, env)
        collected.extend(page.get("data", []))
        url = page.get("links", {}).get("next")
    return collected


def find_bundle_id(env: dict) -> str:
    for item in get_all("/bundleIds", env):
        if item.get("attributes", {}).get("identifier") == BUNDLE_ID:
            return item["id"]
    fail(f"App Store Connect 里没有 bundleId {BUNDLE_ID}")


def find_certificate_id(env: dict) -> str:
    for item in get_all("/certificates", env):
        if item.get("attributes", {}).get("certificateType") == "MAC_APP_DISTRIBUTION":
            return item["id"]
    fail("钥匙串里有 Mac App Distribution 证书，但 App Store Connect 里没有对应记录")


def profile_name(entitlements: dict) -> str:
    """profile 名里带上 entitlement 集合的指纹。

    为什么不用固定名：ASC 不允许重名（实测报 409「Multiple profiles found」），
    而且**无法用 API 删除** —— 只有 `DELETE /profiles/{id}` 返回 405，
    只能去网页后台删。所以「改了 entitlement 就重新签一份」这条常见做法
    在纯 API 下走不通。

    把集合的短指纹编进名字，问题就消失了：同一套 entitlement 永远算出
    同���个名字（重复运行只会复用），改了 entitlement 自然是新名字、
    不与旧的冲突。旧 profile 留着无害 —— 它没过期前不影响任何东西。
    """
    digest = hashlib.sha256(
        json.dumps(entitlements, sort_keys=True).encode("utf-8")
    ).hexdigest()[:8]
    return f"MacSlim MAS 1.0 {digest}"


def find_reusable_profile(env: dict, name: str) -> dict | None:
    """同名且已签发完成的 profile 可以直接复用，不必再建。"""
    for item in get_all("/profiles", env):
        if item.get("attributes", {}).get("name") != name:
            continue
        fetched = request("GET", f"/profiles/{item['id']}", env)
        if fetched["data"].get("attributes", {}).get("profileContent"):
            return fetched["data"]["attributes"]
    return None


def signed_entitlements(app: Path) -> dict:
    """读已构建 .app 的**签名里**真正带的 entitlements。"""
    result = subprocess.run(
        ["codesign", "-d", "--entitlements", "-", "--xml", str(app)],
        capture_output=True,
    )
    try:
        return plistlib.loads(result.stdout)
    except plistlib.InvalidFileException:
        return {}


def report_profile(profile_plist: dict) -> None:
    info(f"name: {profile_plist.get('Name')}  uuid: {profile_plist.get('UUID')}")
    info(f"过期: {profile_plist.get('ExpirationDate')}")
    info("profile 授权（App Store 下只有这三类是正常的）：")
    for key, value in sorted(profile_plist.get("Entitlements", {}).items()):
        info(f"  {key} = {value}")


def report_signature(app: Path | None) -> None:
    wanted = desired_entitlements()
    info("")
    info(
        f"entitlements.mas.plist 声明的 {len(wanted)} 条沙箱权限由**签名**提供，不进 profile。"
    )
    if app is None:
        info("（没给 --app，跳过签名检查）")
        return
    if not app.exists():
        info(f"（{app} 不存在，跳过签名检查）")
        return
    signed = signed_entitlements(app)
    info(f"签名里的沙箱权限（{app.name}）：")
    for key in sorted(wanted):
        mark = "有" if signed.get(key) else "缺"
        info(f"  [{mark}] {key}")
    absent = sorted(key for key in wanted if not signed.get(key))
    if absent:
        info("")
        info("!! 签名里缺以下权限 —— 对应功能会静默失效：")
        for key in absent:
            info(f"  {key}")
        return
    info("")
    info("签名与 entitlements.mas.plist 一致。")


def show_current(app: Path | None = None) -> None:
    """打印现状，并说明**该去哪里看**什么。

    这里曾经写错过一次，而且错得很贵：它把「profile 里没有
    `com.apple.security.app-sandbox`」当成缺失项报警。

    实际上 **Mac App Store 的 profile 本来就不带沙箱 entitlement** ——
    它只带 app identifier / keychain-access-groups / team-id。MAS 应用的
    沙箱权限来自**签名**（`codesign --entitlements`），由 App Store 在处理
    包时校验。把 Developer ID 的心智模型套到 MAS 上，就会一路查错方向。

    所以正确的检查是分两处看：
    - 签名 → 沙箱 entitlement（app-sandbox / bookmarks.app-scope / files.*）
    - profile → app identifier / keychain / team
    """
    if not PROFILE.exists():
        fail(f"找不到 {PROFILE}")
    decoded = subprocess.run(
        ["security", "cms", "-D", "-i", str(PROFILE)],
        capture_output=True,
        check=True,
    ).stdout
    info(f"profile: {PROFILE}")
    report_profile(plistlib.loads(decoded))
    report_signature(app)


def await_profile_content(env: dict, profile_id: str) -> str:
    """ASC 是异步签发的：建完立刻取常常还拿不到内容，短轮询几次。"""
    for attempt in range(10):
        fetched = call("GET", f"/profiles/{profile_id}", env)
        content = fetched["data"].get("attributes", {}).get("profileContent")
        if content:
            return content
        time.sleep(2 ** min(attempt, 3))
    fail("profile 创建了但拿不到内容；稍后用 --show 再查一次")
    raise AssertionError("fail() 会退出，这里只是为了满足类型推断")


def create_profile(env: dict, name: str, entitlements: dict) -> str:
    bundle_id = find_bundle_id(env)
    certificate_id = find_certificate_id(env)
    info(f"bundleId 内部 id: {bundle_id}")
    info(f"证书内部 id:     {certificate_id}")

    body = {
        "data": {
            "type": "profiles",
            "attributes": {"name": name, "profileType": "MAC_APP_STORE"},
            "relationships": {
                "bundleId": {"data": {"type": "bundleIds", "id": bundle_id}},
                "certificates": {
                    "data": [{"type": "certificates", "id": certificate_id}]
                },
                "devices": {"data": []},
            },
        }
    }
    created = call("POST", "/profiles", env, body)
    profile_id = created["data"]["id"]
    info(f"profile 已创建: {profile_id}")
    info(f"请求的 entitlements: {json.dumps(entitlements, ensure_ascii=False)}")
    content = created["data"].get("attributes", {}).get("profileContent")
    return content or await_profile_content(env, profile_id)


def write_profile(content_base64: str) -> None:
    PROFILE.parent.mkdir(parents=True, exist_ok=True)
    PROFILE.write_bytes(base64.b64decode(content_base64))
    info(f"已写入 {PROFILE}")


def regenerate() -> None:
    env = load_env()
    wanted = desired_entitlements()
    name = profile_name(wanted)
    info(f"profile 名: {name}")

    reused = find_reusable_profile(env, name)
    if reused is not None:
        info("同名 profile 已存在且已签发，直接复用")
        write_profile(reused["profileContent"])
    else:
        write_profile(create_profile(env, name, wanted))
    show_current(Path(os.environ["MAS_APP"]) if os.environ.get("MAS_APP") else None)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--show", action="store_true", help="只打印现状，不联网"
    )
    parser.add_argument(
        "--app",
        default=os.environ.get("MAS_APP"),
        help="已构建的 .app 路径；给了就一并核对签名里的沙箱权限",
    )
    args = parser.parse_args()
    if args.show:
        show_current(Path(args.app) if args.app else None)
    else:
        regenerate()


if __name__ == "__main__":
    main()
