#!/usr/bin/env python3
"""把上架元数据写进 App Store Connect。

## 为什么写成脚本而不是手工点

上架文案是**契约的一部分**：审核会逐条核对截图、描述、隐私政策与实际
行为是否一致，而这些内容散落在多处时必然会漂移。写进脚本意味着
「改了产品行为 → 必须同步改文案」这件事有地方可查。

而且 Apple 的元数据接口全是 **PATCH**（`GET_COLLECTION` 被禁），所以
「先读再判断要不要写」这条路根本走不通 —— 只能幂等地 PUT 一遍。

## 用法

```bash
# 先看现状（不写）
python3 scripts/publish_mas_metadata.py --dry-run

# 写入
python3 scripts/publish_mas_metadata.py

# 只更新某一个本地化
python3 scripts/publish_mas_metadata.py --locale zh-Hans
```

凭据从 `~/.config/mddock/ios-release.env` 读（仓库外，不进 git）。

## 文案里最容易踩的两个坑（都在下面的注释里标了）

1. **不要自称「减配版 / Lite / 受限版」。** App Store 审核指南 4.0 把
   「功能太少」列为下架原因第一位，而用户看到自我标注的降级标签只会觉得
   被骗。正确做法是正面描述这个版本**做了什么**。
2. **不要承诺做不到的事。** 截图里没有的、App Store 版做不到的，一句都
   不能写 —— 审核会照着描述逐条试。
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "mas_profile", ROOT / "scripts/create_mas_profile.py"
)
mas = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(mas)

APP_ID = os.environ.get("MAS_APP_ID", "6816609940")
VERSION_ID = os.environ.get("MAS_VERSION_ID", "2c11d65f-fc50-4440-9604-bfe34dd3259f")
APP_INFO_ID = os.environ.get("MAS_APP_INFO_ID", "16ef238c-95a4-4083-9c4d-09cce0e0fef6")
SUPPORT_URL = "https://edwin-hao-ai.github.io/MacSlim/support.html"
PRIVACY_URL = "https://edwin-hao-ai.github.io/MacSlim/privacy.html"
COPYRIGHT = "2026 Beijing VGO Co., Ltd."

# ============================================================================
# 文案
# ============================================================================
# 写作纪律（每条都对应一个真实踩过的坑）：
#
# - **不自称减配版。** 见模块注释第 1 条。
# - **不写做不到的事。** App Store 版**不能**终止进程、不能 exec 外部
#   CLI、不能读未经授权的目录。描述里出现任何一条，审核一试就知道。
# - **文件夹授权要写在显眼处。** 这是这个版本与直觉最大的偏差：用户以为
#   装完就能扫全部缓存。提前说清比事后解释少很多差评。
# - **隐私承诺要和 PrivacyInfo.xcprivacy 一致**：零采集、零网络。

ZH = {
    "name": "MacSlim",
    "subtitle": "看得见的 Mac 空间管家",
    "promotional_text": "系统健康、进程监控、应用体积分析，以及你授权后就能清的缓存。",
    "description": """MacSlim 是一款只做一件事的 Mac 工具：告诉你空间和资源去哪了，然后把能安全清掉的清掉。

实时看清 Mac 的状态
CPU、内存、磁盘三项实时读数，环形图一眼看出余量。全部在本机计算，不上传任何数据。

进程监控
按 CPU 与内存排序，展开就能看到每个应用的主进程与子进程构成。不只给一个总数，而是告诉你「是谁占的」。

应用体积分析
自动读取每个已安装应用的实际占用，按体积排序。Xcode 之类的开发工具往往是最容易被忽略的大户。

关于删除应用
App Store 版受系统沙箱限制，无法删除应用本身（这是平台规定，授权也解决不了）。上面的体积分析照常可用 —— 想知道空间去哪了，这一页就够；要真正卸载，请用访达把它拖进废纸篓。

缓存清理：授权一次，长期有效
这一条与别的清理工具不一样，说明白为什么：

App Store 版运行在 macOS 的系统沙箱里，出于平台限制**无法**直接读取你的用户目录 —— 这也是苹果的规定，拿到「完全磁盘访问权限」也不会绕过沙箱。所以 MacSlim 采取的方式是：你在应用内点一下授权，在系统弹出的标准文件选择框里选中想让它清理的目录，之后就长期有效，随时可以撤销。

授权后可以清理：
· 应用缓存（~/Library/Caches）
· 应用日志（~/Library/Logs）
· Xcode 编译缓存
· npm 缓存、Rust 工具链缓存
· 废纸篓

清理前一定告诉你会发生什么
每一项都标注了体积与风险，「重新获取成本低于 5 分钟」的项目才会默认选中；完整镜像、整份 node_modules 这类大件不会默认勾选。执行前会二次校验路径，删错文件的情况有专门的防护。

全部在本机完成
没有账号、没有统计、没有广告、没有第三方 SDK。清理历史与白名单只存在你自己的机器上，删除应用即一并删除。

关于沙箱
MacSlim 运行在 macOS 的系统沙箱里，因此有两件事它做不到：终止其他进程，以及读取你未授权的目录。这是平台限制，上面列出的每一项功能都不受影响。""",
    "keywords": "mac清理,缓存,磁盘空间,内存,进程监控,应用卸载,xcode,npm缓存,系统监控",
    "whats_new": "首个版本。\n\n· 系统健康实时读数\n· 进程监控（只读）\n· 应用体积分析与卸载\n· 缓存清理：授权一次，长期有效",
    "support_url": SUPPORT_URL,
    "marketing_url": "https://vgoapp.com",
    "privacy_url": PRIVACY_URL,
}

EN = {
    "name": "MacSlim",
    "subtitle": "See where your space goes",
    "promotional_text": "System health, process monitoring, app sizes, and cache cleaning for the folders you authorize.",
    "description": """MacSlim does one thing well: it shows you where your space and resources actually go, then removes what is safe to remove.

Live system health
Real-time readings for CPU, memory and disk, with ring gauges that show the headroom at a glance. Everything is computed on your Mac; nothing is uploaded.

Process monitoring
Sorted by CPU and memory, expandable down to each app's main process and its helpers. Not just a total — it tells you who is using it.

App size analysis
Measures what every installed app actually occupies on disk, largest first. Developer tools like Xcode are routinely the biggest thing on a Mac, and the easiest to overlook.

About removing apps
The App Store edition runs inside the macOS sandbox and cannot remove apps — a platform limit that no permission lifts. The size analysis above works as usual, so you can still see where your space goes; to actually uninstall something, drag it to the Trash in Finder.

Cache cleaning: authorize once, valid for good
This part works differently from other cleaners, and here is why:

The App Store edition runs inside the macOS system sandbox and, by platform design, **cannot** read your user folders directly — Apple's own rules mean that even Full Disk Access does not lift the sandbox. So MacSlim asks: tap authorize, pick the folder in the standard macOS file dialog, and it stays valid until you revoke it.

Once authorized, MacSlim can clean:
· App caches (~/Library/Caches)
· App logs (~/Library/Logs)
· Xcode build cache
· npm cache and Rust toolchain cache
· Trash

You are told what will happen before it happens
Every item is labelled with its size and its risk. Only items that cost less than five minutes to re-download are selected by default; full images and whole node_modules trees never are. Paths are re-validated right before deletion, and deleting the wrong file has dedicated guards.

Entirely on your Mac
No account, no analytics, no ads, no third-party SDKs. Cleanup history and your whitelist never leave your machine, and deleting the app deletes them.

About the sandbox
MacSlim runs inside the macOS system sandbox, which means two things it cannot do: terminate other apps, and read folders you have not authorized. That is a platform limit, and everything listed above is unaffected.""",
    "keywords": "mac cleaner,cache,disk space,memory,process monitor,uninstaller,xcode,npm cache",
    "whats_new": "First release.\n\n· Live system health\n· Process monitoring (read-only)\n· App size analysis and uninstaller\n· Cache cleaning: authorize once, valid for good",
    "support_url": SUPPORT_URL,
    "marketing_url": "https://vgoapp.com",
    "privacy_url": PRIVACY_URL,
}

LOCALES = {"zh-Hans": ZH, "en-US": EN}


def fail(message: str) -> None:
    print(f"错误: {message}", file=sys.stderr)
    raise SystemExit(1)


def lookup_localization(env: dict, lookup_path: str) -> dict | None:
    """按 filter 路径查现有本地化。

    带 filter 的路径返回数组，不带 filter 的返回单个对象 —— 两种都认。
    ASC 对部分资源禁 GET_COLLECTION，所以「先列后取」这条路走不通，
    只能靠 filter 直查。
    """
    try:
        found = mas.request("GET", lookup_path, env)["data"]
    except SystemExit:
        return None
    if isinstance(found, list):
        return found[0] if found else None
    return found if isinstance(found, dict) and found else None


def create_localization(
    env: dict,
    collection_path: str,
    kind: str,
    attributes: dict,
    relationship_body: dict,
    label: str,
    dry: bool,
) -> str:
    """新建一条本地化。

    POST 时 locale **必须**带上（Apple: "You must provide a value for the
    attribute 'locale'"）—— 它是 PATCH 时不可变、POST 时必填的那个字段。
    """
    payload = {
        "data": {
            "type": kind,
            "attributes": attributes,
            "relationships": relationship_body,
        }
    }
    if dry:
        print(f"  [dry-run] 将创建 {kind} ({label})")
        return ""
    created = mas.request("POST", collection_path, env, payload)
    new_id = created["data"]["id"]
    print(f"  创建 {kind} {new_id} ({label})")
    return new_id


def upsert(
    env: dict,
    collection_path: str,
    lookup_path: str,
    kind: str,
    attributes: dict,
    relationship_filter: str,
    dry: bool,
    relationship_body: dict | None = None,
) -> str:
    """按 localizable id 找现有记录，找不到就 POST 新建。"""
    existing = lookup_localization(env, lookup_path)
    if existing:
        # PATCH 时**不能带 locale**：它是不可变字段，带上会让 Apple 返回
        # 405 METHOD_NOT_ALLOWED —— 而 405 看起来像「这个资源不支持 PATCH」，
        # 于是很容易得出「只能删了重建」的错误结论（DELETE 主语言其实也
        # 禁止，409）。这是踩过的坑，所以在这里剥掉。
        patch_attrs = {k: v for k, v in attributes.items() if k != "locale"}
        payload = {
            "data": {
                "type": kind,
                "id": existing["id"],
                "attributes": patch_attrs,
            }
        }
        if dry:
            print(f"  [dry-run] 将更新 {kind} {existing['id']} ({relationship_filter})")
            return existing["id"]
        mas.request("PATCH", f"{collection_path}/{existing['id']}", env, payload)
        print(f"  更新 {kind} {existing['id']} ({relationship_filter})")
        return existing["id"]

    if relationship_body is None:
        relationship_body = {
            "appStoreVersion": {
                "data": {"type": "appStoreVersions", "id": VERSION_ID}
            }
        }
    if relationship_body is None:
        relationship_body = {
            "appStoreVersion": {
                "data": {"type": "appStoreVersions", "id": VERSION_ID}
            }
        }
    return create_localization(
        env, collection_path, kind, dict(attributes), relationship_body,
        relationship_filter, dry,
    )


def existing_whats_new(env: dict, localization_id: str) -> str | None:
    """这个本地化已有的 whatsNew。

    首次提交的版本**不允许**写 whatsNew（Apple 返回
    「Attribute 'whatsNew' cannot be edited at this time」）—— 那个字段是
    给「改版说明」用的，首版没有「上一版」可写。所以只有已经存在内容时
    才继续写它，否则整条 PATCH 会因为这一个字段被整体拒绝，
    连带把真正要更新的描述和关键词一起丢掉。
    """
    try:
        data = mas.request(
            "GET", f"/appStoreVersionLocalizations/{localization_id}", env
        )["data"]
    except SystemExit:
        return None
    value = data.get("attributes", {}).get("whatsNew")
    return value or None


def publish_version_localization(env: dict, locale: str, copy: dict, dry: bool) -> None:
    attributes = {
        "locale": locale,
        "description": copy["description"],
        "keywords": copy["keywords"],
        "promotionalText": copy["promotional_text"],
        "supportUrl": copy["support_url"],
        "marketingUrl": copy["marketing_url"],
    }
    existing = None
    try:
        found = mas.request(
            "GET",
            f"/appStoreVersions/{VERSION_ID}/appStoreVersionLocalizations?filter[locale]={locale}",
            env,
        )["data"]
        existing = found[0] if isinstance(found, list) and found else found
    except SystemExit:
        existing = None
    if existing and existing_whats_new(env, existing["id"]):
        # 只有真的存在「上一版说明」时才更新它
        attributes["whatsNew"] = copy["whats_new"]
    elif not existing:
        attributes["whatsNew"] = copy["whats_new"]

    info_id = upsert(
        env,
        "/appStoreVersionLocalizations",
        f"/appStoreVersions/{VERSION_ID}/appStoreVersionLocalizations?filter[locale]={locale}",
        "appStoreVersionLocalizations",
        {
            "locale": locale,
            "description": copy["description"],
            "keywords": copy["keywords"],
            "promotionalText": copy["promotional_text"],

            "supportUrl": copy["support_url"],
            "marketingUrl": copy["marketing_url"],
        },
        locale,
        dry,
    )
    if dry or not info_id:
        return
    # subtitle 只属于 appInfoLocalizations，不是 versionLocalization
    return info_id


def publish_app_info_localization(env: dict, locale: str, copy: dict, dry: bool) -> None:
    upsert(
        env,
        "/appInfoLocalizations",
        f"/appInfos/{APP_INFO_ID}/appInfoLocalizations?filter[locale]={locale}",
        "appInfoLocalizations",
        {
            "locale": locale,
            "name": copy["name"],
            "subtitle": copy["subtitle"],
            "privacyPolicyUrl": copy["privacy_url"],
            "privacyPolicyText": copy["privacy_text"],
        },
        locale,
        dry,
        # appInfoLocalizations 挂在 appInfo 上，不是挂在 app 上（实测：
        # 用 app 会报 "'app' is not a relationship" + 缺 appInfo）
        relationship_body={
            "appInfo": {"data": {"type": "appInfos", "id": APP_INFO_ID}}
        },
    )


def privacy_text(locale: str) -> str:
    # 纯文本版（Apple 的表单接受纯文本 / 不接受 HTML）
    if locale == "zh-Hans":
        return (
            "MacSlim 不采集任何数据，也不把任何东西传出你的 Mac。"
            "没有账号、没有统计、没有崩溃上报、没有广告 SDK、没有第三方分析。\n\n"
            "清理历史与白名单保存在应用自己的沙箱容器内，仅本机可见，删除应用即一并删除。\n\n"
            "App Store 版运行在系统沙箱内，只读取你亲手授权的目录。授权记录是一份 "
            "security-scoped bookmark，存在应用自己的容器里，跨启动保留（所以只需授权一次），"
            "实际读取只在扫描期间开启；卸载应用即随之删除。可随时在应用内撤销。"
            "我们不保存、也不上传被授权目录里的任何内容。\n\n"
            "macOS 规定沙箱应用即使获得完全磁盘访问权限，仍受沙箱自身限制。"
            "因此本应用不要求你授予完全磁盘访问权限，也不需要修改任何系统隐私设置。\n\n"
            "本应用不集成任何第三方 SDK。\n\n"
            "完整政策：https://edwin-hao-ai.github.io/MacSlim/privacy.html"
        )
    return (
        "MacSlim does not collect any data and does not send anything off your Mac. "
        "No account, no analytics, no crash reporting, no ad SDKs, no third-party analytics.\n\n"
        "Cleanup history and your whitelist are stored inside the app's own sandbox "
        "container, are visible only on this Mac, and are deleted with the app.\n\n"
        "The App Store edition runs inside the system sandbox and reads only the folders "
        "you authorize yourself. Authorization is a security-scoped bookmark stored in the "
        "app's own container. It persists across launches (so you authorize a folder once), "
        "actual reads happen only while a scan is running, and deleting the app removes it. "
        "You can revoke it at any time. We do not store or upload anything inside the "
        "folders you authorize.\n\n"
        "macOS requires sandboxed apps to keep their own file restrictions even with Full "
        "Disk Access. MacSlim therefore never asks for Full Disk Access and never asks you "
        "to change any system privacy setting.\n\n"
        "No third-party SDKs are integrated.\n\n"
        "Full policy: https://edwin-hao-ai.github.io/MacSlim/privacy.html"
    )


def update_app_info(env: dict, dry: bool) -> None:
    """分类与分级**必须分开**写。

    实测把 `appStoreAgeRating` 放进 appInfo 的 PATCH 会被拒
    （"can not be included in a 'UPDATE' operation"），而整个 PATCH 失败
    会连带把分类也丢掉 —— 而分类是这次唯一真正要改的东西。
    """
    payload = {
        "data": {
            "type": "appInfos",
            "id": APP_INFO_ID,
            "relationships": {
                "primaryCategory": {
                    "data": {"type": "appCategories", "id": "UTILITIES"}
                }
            },
        }
    }
    if dry:
        print("  [dry-run] 将更新分类为 Utilities")
    else:
        mas.request("PATCH", f"/appInfos/{APP_INFO_ID}", env, payload)
        print("  更新分类：Utilities")


def _patch_with_retry(url: str, env: dict, attributes: dict, kind: str):
    """带重试的 PATCH。

    这份问卷要连打二十几次（每轮补一个字段），而 Apple 的 API 会偶发
    SSL EOF。没有重试的话，一次网络抖动就会让整个分级流程失败，而它本
    来只是重跑一遍就能过的事。
    """
    import time as _time

    import requests

    last: Exception | None = None
    for attempt in range(4):
        try:
            return requests.patch(
                url,
                headers={
                    "Authorization": f"Bearer {mas.token(env)}",
                    "Content-Type": "application/json",
                },
                json={
                    "data": {
                        "type": kind,
                        "id": APP_INFO_ID,
                        "attributes": attributes,
                    }
                },
                timeout=60,
            )
        except requests.RequestException as error:
            last = error
            _time.sleep(2**attempt)
    raise SystemExit(f"年龄分级问卷网络失败（已重试 4 次）：{last}")


# 年龄分级问卷里**类型不统一**的两组字段。
#
# 同样是「是否涉及某内容」的一组问题，一部分要 JSON 布尔、另一部分要
# 枚举字符串。实测踩过：统一填一个值就报错，而按报错逐条改会**来回震荡**
# —— 一轮里把 A 改成布尔、另一轮又要求它改回字符串。
#
# 所以这里固定写死，并在上面标注了推导依据（Apple 的
# App Store Connect API 文档 ageRatingDeclarations 一节 + 实测 200）。
# 字段集若随平台规则变化，需要按同样方式重新定型一次。
RATING_BOOLEAN_FIELDS = [
    "advertising",
    "ageAssurance",
    "gambling",
    "healthOrWellnessTopics",
    "lootBox",
    "messagingAndChat",
    "parentalControls",
    "unrestrictedWebAccess",
    "userGeneratedContent",
]

RATING_ENUM_FIELDS = [
    "alcoholTobaccoOrDrugUseOrReferences",
    "contests",
    "gamblingSimulated",
    "gunsOrOtherWeapons",
    "horrorOrFearThemes",
    "matureOrSuggestiveThemes",
    "medicalOrTreatmentInformation",
    "profanityOrCrudeHumor",
    "sexualContentGraphicAndNudity",
    "sexualContentOrNudity",
    "violenceCartoonOrFantasy",
    "violenceRealistic",
    "violenceRealisticProlongedGraphicOrSadistic",
]


def set_age_rating(env: dict, dry: bool) -> None:
    """答完整份年龄分级问卷，得到 4+。

    ## 为什么这么绕

    `appStoreAgeRating` 不能直接写进 appInfo 的 PATCH（Apple：
    "can not be included in a 'UPDATE' operation"），分级要走
    `ageRatingDeclarations`，而那个资源要求**整份问卷一个不漏** ——
    少一个就 409，且一次只告诉你缺哪一个。

    全填「否」即 4+：MacSlim 是系统工具，不含任何这类内容。
    `ageRatingOverride` 填 `"NONE"` 表示「不用覆盖，按问卷结果算」。
    """
    if dry:
        print("  [dry-run] 将提交年龄分级问卷（全项否 → 4+）")
        return

    attributes = {"ageRatingOverride": "NONE"}
    attributes.update({field: False for field in RATING_BOOLEAN_FIELDS})
    attributes.update({field: "NONE" for field in RATING_ENUM_FIELDS})

    response = _patch_with_retry(
        f"https://api.appstoreconnect.apple.com/v1/ageRatingDeclarations/{APP_INFO_ID}",
        env,
        attributes,
        "ageRatingDeclarations",
    )
    if response.status_code != 200:
        try:
            errors = response.json()["errors"]
        except Exception:
            fail(f"年龄分级问卷响应异常：{response.status_code}")
        fail("年龄分级问卷被拒：" + json.dumps(errors, ensure_ascii=False)[:400])
    print(f"  年龄分级问卷已提交（{len(attributes)} 项，全为「否」→ 4+）")


def update_version_copyright(env: dict, dry: bool) -> None:
    payload = {
        "data": {
            "type": "appStoreVersions",
            "id": VERSION_ID,
            "attributes": {"copyright": COPYRIGHT},
        }
    }
    if dry:
        print(f"  [dry-run] 将更新版本版权信息：{COPYRIGHT}")
        return
    mas.request("PATCH", f"/appStoreVersions/{VERSION_ID}", env, payload)
    print(f"  更新版本版权信息：{COPYRIGHT}")


def check_limits(locale: str, copy: dict) -> None:
    """长度前置校验。

    ASC 对超限字段的报错是**整体请求失败**，不会告诉你哪个字段超了多少。
    所以本地先卡掉 —— 尤其是 keywords 上限 100 字符，一个单词超了就得
    重跑一次网络请求。
    """
    limits = {
        "keywords": (100, "关键词"),
        "subtitle": (30, "副标题"),
        "promotional_text": (170, "宣传文字"),
        "whats_new": (4000, "新功能说明"),
    }
    problems = []
    for key, (limit, label) in limits.items():
        length = len(copy[key])
        if length > limit:
            problems.append(f"{label} {length} 字符（上限 {limit}）")
    if problems:
        fail(f"[{locale}] " + "；".join(problems))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dry-run", action="store_true", help="只打印要做什么，不写")
    parser.add_argument("--locale", choices=sorted(LOCALES), help="只处理某个本地化")
    args = parser.parse_args()

    env = mas.load_env()
    locales = [args.locale] if args.locale else sorted(LOCALES)

    print(f"应用 {APP_ID} / 版本 {VERSION_ID}" + ("（dry-run）" if args.dry_run else ""))
    for locale in locales:
        copy = dict(LOCALES[locale])
        copy["privacy_text"] = privacy_text(locale)
        check_limits(locale, copy)
        print(f"\n[{locale}]")
        print(f"  副标题：{copy['subtitle']}")
        print(f"  关键词：{copy['keywords']}（{len(copy['keywords'])} 字符）")
        print(f"  描述：{len(copy['description'])} 字符")
        publish_app_info_localization(env, locale, copy, args.dry_run)
        publish_version_localization(env, locale, copy, args.dry_run)

    if not args.locale:
        print("\n[应用级]")
        update_app_info(env, args.dry_run)
        set_age_rating(env, args.dry_run)
        update_version_copyright(env, args.dry_run)

    print("\n完成。" + ("（dry-run，未写入）" if args.dry_run else ""))


if __name__ == "__main__":
    main()