#!/usr/bin/env python3
"""Mac App Store 构建形态的配置门禁。

核心不变式只有一条：**Developer ID 版和 MAS 版必须是两份分离的配置**，
任何一份被改坏、或者两边串了，MAS 就会产出一个「开了沙箱但还想要全功能」
或者「没开沙箱被审核直接拒」的包。所以这里逐项断言：

- 两份 entitlements 分离，且 Developer ID 那份**没有**沙箱
- MAS 那份**必须**有 app-sandbox（缺了直接过不了审核）
- MAS 那份**不得**含 temporary-exception（MAS 不支持这个键）
- tauri.mas.conf.json 只选 mas.json 一份 capability（多份会被 Tauri 合并）
- default.json 仍带 updater 权限、mas.json 不带
- PrivacyInfo 声明了用到的 required-reason API，且 tracking 为 false
"""
from __future__ import annotations
import plistlib
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TAURI = ROOT / "src-tauri"


def load_plist(name: str) -> dict:
    with (TAURI / name).open("rb") as handle:
        return plistlib.load(handle)


def load_json(name: str) -> dict:
    import json
    return json.loads((TAURI / name).read_text(encoding="utf-8"))


class DeveloperIdEntitlementsTests(unittest.TestCase):
    """Developer ID 公证版：必须**没有**沙箱。"""

    def setUp(self) -> None:
        self.entitlements = load_plist("entitlements.plist")

    def test_never_enables_the_sandbox(self) -> None:
        # 套上沙箱等于把产品阉了：全功能版的价值建立在能读 ~/Library/Caches、
        # 能终止任意进程、能调 npm/brew/docker 上。加了它 Developer ID 版就没意义。
        self.assertNotIn("com.apple.security.app-sandbox", self.entitlements)

    def test_keeps_the_jit_and_apple_events_it_actually_uses(self) -> None:
        # 这两条是现有的、非沙箱版的既有内容，MAS 化过程中不能被顺手删掉
        self.assertTrue(self.entitlements["com.apple.security.cs.allow-jit"])
        self.assertTrue(
            self.entitlements["com.apple.security.automation.apple-events"]
        )


class MasEntitlementsTests(unittest.TestCase):
    """MAS 版：必须有沙箱，且不得出现 MAS 不支持的键。"""

    def setUp(self) -> None:
        self.entitlements = load_plist("entitlements.mas.plist")

    def test_enables_the_sandbox(self) -> None:
        # App Store 上架的硬性要求，缺了直接被拒
        self.assertTrue(self.entitlements["com.apple.security.app-sandbox"])

    def test_has_no_temporary_exception_keys(self) -> None:
        # MAS 沙箱**不支持** temporary-exception.* 键（那是 Developer ID 版的
        # 机制）。带着它打包会直接失败或被审核挑出来。
        offenders = [
            key for key in self.entitlements
            if key.startswith("com.apple.security.temporary-exception")
        ]
        self.assertEqual(offenders, [], f"MAS 不支持这些键：{offenders}")

    def test_requests_the_file_permissions_the_product_needs(self) -> None:
        for key in (
            "com.apple.security.files.user-selected.read-write",
            "com.apple.security.files.all",
        ):
            self.assertTrue(self.entitlements.get(key), f"缺少 {key}")

    def test_does_not_claim_apple_events(self) -> None:
        # MAS 版不该发 AppleEvent（优雅退出应用的路径在 MAS 下不走）
        self.assertNotIn("com.apple.security.automation.apple-events", self.entitlements)


class MasConfigTests(unittest.TestCase):
    def setUp(self) -> None:
        self.config = load_json("tauri.mas.conf.json")
        self.base = load_json("tauri.conf.json")

    def test_selects_exactly_one_capability(self) -> None:
        # Tauri 会**合并**多份 capability。两份同时生效 = updater 权限还在 =
        # 分叉没做。所以必须显式只列一份。
        #
        # 引用的是 capability 的 **identifier**，不是文件路径。写成
        # "src-tauri/capabilities/mas.json" 会报
        # `capability with identifier ... not found`（实测踩过）。
        capabilities = self.config["app"]["security"]["capabilities"]
        self.assertEqual(capabilities, ["mas"])

    def test_points_at_the_mas_entitlements_not_the_shared_one(self) -> None:
        # 用错文件的话，MAS 包会拿 Developer ID 的 entitlements（无沙箱）去签，
        # 审核阶段才会暴露 —— 那时候已经浪费了一整轮。
        # 注意 entitlements 挂在 bundle.macOS 下面，不是 bundle 顶层。
        self.assertEqual(
            self.config["bundle"]["macOS"]["entitlements"], "entitlements.mas.plist"
        )
        self.assertNotEqual(
            self.config["bundle"]["macOS"]["entitlements"],
            self.base["bundle"]["macOS"]["entitlements"],
        )

    def test_does_not_claim_a_mas_bundle_target(self) -> None:
        # Tauri v2 **移除了** Mac App Store 支持：BundleType 只有
        # deb/rpm/appimage/msi/nsis/app/dmg，没有 `mas`。写上去会在 schema
        # 校验阶段直接失败（`["app","mas"] is not valid under any of the
        # schemas`）。MAS 产物由 scripts/release-mas.sh 手工串步骤产出。
        self.assertEqual(self.config["bundle"]["targets"], ["app"])
        self.assertNotIn("mas", self.base["bundle"]["targets"])

    def test_ships_the_privacy_manifest(self) -> None:
        self.assertIn("PrivacyInfo.xcprivacy", self.config["bundle"]["resources"])


class CapabilityForkTests(unittest.TestCase):
    def setUp(self) -> None:
        self.default = load_json("capabilities/default.json")
        self.mas = load_json("capabilities/mas.json")

    def test_mas_drops_the_updater_permissions(self) -> None:
        # MAS 走 App Store 更新，没有自更新。留着这两条等于 MAS 包带一个用不到的
        # 权限，且让 capability 校验和实际行为对不上。
        self.assertIn("updater:allow-check", self.default["permissions"])
        self.assertIn("updater:allow-download-and-install", self.default["permissions"])
        self.assertNotIn("updater:allow-check", self.mas["permissions"])
        self.assertNotIn(
            "updater:allow-download-and-install", self.mas["permissions"]
        )

    def test_mas_keeps_everything_the_ui_still_needs(self) -> None:
        for permission in (
            "core:app:allow-version",
            "core:event:allow-listen",
            "core:event:allow-unlisten",
            "core:window:allow-start-dragging",
            "process:allow-restart",
        ):
            self.assertIn(permission, self.mas["permissions"], f"MAS 缺 {permission}")

    def test_mas_is_a_strict_subset_of_default(self) -> None:
        # MAS 只能少、不能多。多出来的权限在审核里是纯粹的额外风险。
        extra = set(self.mas["permissions"]) - set(self.default["permissions"])
        self.assertEqual(extra, set(), f"MAS 版多出了权限：{extra}")

    def test_both_are_restricted_to_the_main_window(self) -> None:
        self.assertEqual(self.default["windows"], ["main"])
        self.assertEqual(self.mas["windows"], ["main"])


class PrivacyManifestTests(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest = load_plist("PrivacyInfo.xcprivacy")

    def test_declares_no_tracking(self) -> None:
        # MacSlim 无遥测、无第三方分析 SDK、无数据出本机
        self.assertFalse(self.manifest["NSPrivacyTracking"])
        self.assertEqual(self.manifest["NSPrivacyTrackingDomains"], [])

    def test_declares_no_collected_data(self) -> None:
        self.assertEqual(self.manifest["NSPrivacyCollectedDataTypes"], [])

    def test_declares_the_required_reason_apis_we_actually_use(self) -> None:
        # 两个都是 Apple 要求必须声明的类别，少一个就在上传时被拒
        declared = {
            entry["NSPrivacyAccessedAPIType"]: entry["NSPrivacyAccessedAPITypeReasons"]
            for entry in self.manifest["NSPrivacyAccessedAPITypes"]
        }
        # 读文件时间戳（判定扫描快照是否过期）
        self.assertIn(
            "NSPrivacyAccessedAPICategoryFileTimestamp", declared
        )
        self.assertTrue(declared["NSPrivacyAccessedAPICategoryFileTimestamp"])
        # 查磁盘可用空间（智能扫描的磁盘环形图）
        self.assertIn("NSPrivacyAccessedAPICategoryDiskSpace", declared)
        self.assertTrue(declared["NSPrivacyAccessedAPICategoryDiskSpace"])

    def test_reason_codes_are_the_documented_ones(self) -> None:
        declared = {
            entry["NSPrivacyAccessedAPIType"]: entry["NSPrivacyAccessedAPITypeReasons"]
            for entry in self.manifest["NSPrivacyAccessedAPITypes"]
        }
        self.assertEqual(
            declared["NSPrivacyAccessedAPICategoryFileTimestamp"], ["C617.1"]
        )
        self.assertEqual(declared["NSPrivacyAccessedAPICategoryDiskSpace"], ["E174.1"])


class MasTargetIsolationTests(unittest.TestCase):
    """MAS 构建**必须**与完整版物理隔离。

    踩过的坑：两个构建共用 `src-tauri/target` 时，MAS 构建把完整版的 release
    产物整个顶掉了 —— Developer ID 签名的 .app 变成 3rd Party 签名，连已构建
    好的 dmg 都没了。完整版是主产品，不能被 MAS 的反复调试破坏。
    """

    def setUp(self) -> None:
        self.script = (ROOT / "scripts/release-mas.sh").read_text(encoding="utf-8")
        self.gitignore = (ROOT / ".gitignore").read_text(encoding="utf-8")

    def test_pins_its_own_cargo_target_dir(self) -> None:
        self.assertIn("CARGO_TARGET_DIR", self.script)
        self.assertIn("shared-target-mas", self.script)

    def test_does_not_derive_the_app_path_from_the_shared_target(self) -> None:
        # 路径里若出现裸的 src-tauri/target，说明还是共用 —— 那正是坑的成因
        self.assertNotIn('APP="src-tauri/target/', self.script)

    def test_mas_target_dir_is_git_ignored(self) -> None:
        self.assertIn("target-mas", self.gitignore)

    def test_strips_the_cli_that_crashes_under_sandbox(self) -> None:
        # 实测：带沙箱的 macslim-cli 一起签名后启动即 SIGTRAP，崩溃栈全在
        # dyld 初始化阶段（_libsecinit_appsandbox → forEachInitializer），
        # 连 `--version` 都打不出来。留着只会被审核拒，且对 App Store 用户无价值。
        self.assertIn("strip_cli", self.script)
        self.assertIn("macslim-cli", self.script)
        # 且必须在签名**之前**执行 —— 删文件改了 bundle，签名要在删完之后做
        strip_at = self.script.find("strip_cli;")
        sign_at = self.script.find("sanitize_info_plist;")
        self.assertLess(strip_at, sign_at)

    def test_excluded_bundle_target_is_not_mas(self) -> None:
        # Tauri v2 的 BundleType 里没有 `mas`（只有 deb/rpm/appimage/msi/
        # nsis/app/dmg），写上去会在 schema 校验阶段直接失败
        self.config = load_json("tauri.mas.conf.json")
        self.assertEqual(self.config["bundle"]["targets"], ["app"])


class ShellInterpolationTests(unittest.TestCase):
    """所有 shell 脚本里都不许出现「裸 `$var` 紧跟非 ASCII 字节」。

    这不是风格问题，是一个会真炸的 macOS 专属 bug：

    macOS 自带的 bash 3.2 用 locale 感知的 `isalnum()` 判断标识符的
    合法性，而中文全角标点（`，（）：` 等）的 UTF-8 字节在 UTF-8 locale 下
    `isalnum()` 为真，于是被并进变量名。`$APP_PATH，请先跑` 会被解析成查
    一个名字里含 `，` 三个字节的变量，在 `set -u` 下报
    `APP_PATH，请先跑: unbound variable`。

    危害在于它**只在该错误分支触发**：构建顺利时永远看不到，一旦出事，
    用户拿到的是「unbound variable」而不是脚本本来想给的错误信息。
    实测在 scripts/release-mas.sh 与 scripts/sign.sh 各中过一次。
    """

    # 注释里的同样写法是无害的（bash 不解析注释），但为了让这条门禁
    # 简单可靠、也为了让注释本身不误导后来人，注释里也一律用花括号。
    PATTERN = re.compile(rb"\$([A-Za-z_][A-Za-z0-9_]*)(?=[\x80-\xff])")

    def _scripts(self) -> list[Path]:
        found = []
        for path in sorted(ROOT.rglob("*.sh")):
            relative = path.relative_to(ROOT).as_posix()
            if relative.startswith(("node_modules/", "dist/")) or "/target" in relative:
                continue
            found.append(path)
        return found

    def test_no_bare_variable_is_followed_by_a_multibyte_character(self) -> None:
        offenders: list[str] = []
        for path in self._scripts():
            data = path.read_bytes()
            for match in self.PATTERN.finditer(data):
                line = data[: match.start()].count(b"\n") + 1
                name = match.group(1).decode()
                context = data[match.start() : match.start() + 24].decode(
                    "utf-8", "replace"
                )
                offenders.append(
                    f"{path.relative_to(ROOT).as_posix()}:{line} "
                    f"${name} → {context!r}"
                )
        self.assertEqual(
            offenders,
            [],
            "这些裸变量引用紧跟非 ASCII 字节，bash 3.2 会把多字节并入变量名，"
            f"set -u 下报 unbound variable（改用 ${{var}}）：\n  " + "\n  ".join(offenders),
        )

    def test_the_check_actually_catches_the_known_offenders(self) -> None:
        # 门禁自己得先证明有效，否则「一条都没报」可能只是正则写坏了。
        # bytes 字面量不能直接写非 ASCII，用 UTF-8 字节显式拼：
        #   ，= \xef\xbc\x8c   （= \xef\xbc\x88
        should_match = [
            b'echo "$APP_PATH\xef\xbc\x8cx"',
            b'echo "$cli\xef\xbc\x88x"',
            b'echo "$A\xef\xbc\x9ax"',
        ]
        should_not_match = [
            b'echo "${APP_PATH}\xef\xbc\x8cx"',
            b'echo " $APP_PATH x"',
            b'echo "$APP_PATH"',
            b'echo "$APP_PATH "',
        ]
        for probe in should_match:
            with self.subTest(probe=probe):
                self.assertIsNotNone(self.PATTERN.search(probe), probe)
        for probe in should_not_match:
            with self.subTest(probe=probe):
                self.assertIsNone(self.PATTERN.search(probe), probe)


class CargoFeatureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.manifest = (TAURI / "Cargo.toml").read_text(encoding="utf-8")

    def test_declares_a_mas_feature_that_is_off_by_default(self) -> None:
        # 必须 off by default —— 忘了加 --features mas 就该得到全功能版，
        # 反过来（一不小心给全功能版带上沙箱）会静默阉掉主产品。
        self.assertIn("[features]", self.manifest)
        self.assertIn("default = []", self.manifest)
        self.assertIn("mas = []", self.manifest)


if __name__ == "__main__":
    unittest.main()
