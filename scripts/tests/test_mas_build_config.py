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
import json
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


def strip_shell_comments(source: str) -> str:
    """去掉 shell 里的整行 `#` 注释。"""
    return "\n".join(
        line for line in source.splitlines() if not line.lstrip().startswith("#")
    )


def strip_py_comments(source: str) -> str:
    """把 Python 的 `#` 注释替换成等长空格，**保持行结构不变**。

    为什么要保行：门禁断言的是整行代码（比如
    `SCREENSHOT_DISPLAY_TYPE = "APP_DESKTOP"`）。若把 token 逐个 join 起来
    拼成一行，源码里所有换行都没了，这些断言会全部变成「找不到」——
    门禁就成了永远红的假信号。

    换成等长空格而不是直接删：删会让后面的行列号全部前移，注释掉的那行
    也会与下一行粘在一起，破坏后续按行做的解析。

    用 `tokenize` 而不是正则：正则分不清 `#` 在不在字符串里。判据里就
    含 `"filter[screenshotDisplayType]"`、`"#appStoreScreenshotType"` 这类
    字面量，正则会把字符串内容当注释吃掉，判据直接错位。

    词法分析失败时断言失败，**不**退回「当作无注释」—— 那会让
    「注释里没有该词」变成假阳性通过，正是这类门禁要防的事。
    """
    import io
    import tokenize

    try:
        tokens = list(tokenize.generate_tokens(io.StringIO(source).readline))
    except (tokenize.TokenError, IndentationError, SyntaxError) as error:
        raise AssertionError(f"该 Python 源文件无法词法分析：{error}")

    lines = source.splitlines()
    for token in tokens:
        if token.type != tokenize.COMMENT:
            continue
        # 单行注释 token 不跨行（跨行的是 NL/NEWLINE），所以只会落在一行里
        row = token.start[0] - 1
        lines[row] = (
            lines[row][: token.start[1]] + " " * (token.end[1] - token.start[1])
            + lines[row][token.end[1] :]
        )
    return "\n".join(lines)


def strip_rust_comments(source: str) -> str:
    """去掉 Rust 的行注释与文档注释，只留代码。

    「只读路径不许出现 sysinfo」这类门禁必须查**代码**而不是散文 ——
    而这些模块的文档注释里恰恰要写清「为什么不用 sysinfo、它被沙箱拦」，
    把注释一起判掉会让正确的说明反过来把门禁弄挂。
    """
    out: list[str] = []
    for line in source.splitlines():
        stripped = line.lstrip()
        if stripped.startswith("//"):
            continue
        # 行尾注释：不在字符串里的第一个 //。这里用「// 前不得出现奇数个引号」
        # 判断，够用且不会为了严谨去写一个 Rust 词法分析器。
        quoted = line.count('"') % 2 == 1
        if not quoted and "//" in line:
            line = line[: line.index("//")]
        out.append(line)
    return "\n".join(out)


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
        # `files.user-selected.read-write`：用户用文件选择框授权的路径。
        self.assertTrue(
            self.entitlements.get("com.apple.security.files.user-selected.read-write")
        )
        # `files.bookmarks.app-scope`：让那次授权能跨启动持久化。
        # 少了它，上面那次授权每次启动都要重做一遍。
        self.assertTrue(
            self.entitlements.get("com.apple.security.files.bookmarks.app-scope"),
            "缺少 bookmarks.app-scope：用户授权无法持久化，每次启动都要重新选",
        )

    def test_does_not_claim_file_access_it_can_never_get(self) -> None:
        # `com.apple.security.files.all`（读用户系统所有文件）**只在 Developer ID
        # 分发里存在**，App Store 的 profile 不会授权它。挂着它有三个坏处：
        # 让 MAS 构建自认为有它实际没有的能力（历史上 fda.rs 的错误归因就是
        # 这么来的）、签名里多一条「请求了但没拿到」的 entitlement、以及审核时
        # 多一条「为什么要这么多权限」的质疑 —— QA1773 明确把「多要权限」
        # 列为比「少做功能」更容易被拒的理由。
        self.assertNotIn(
            "com.apple.security.files.all",
            self.entitlements,
            "MAS 版不该请求 files.all —— 那是 Developer ID 专属，写在这里只会误导",
        )

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


class MasProcessMonitoringTests(unittest.TestCase):
    """MAS 版必须有只读进程列表，而且必须走 sysctl 那条路。

    实测（2026-09-30，MAS 包真机）：`sysinfo` 在沙箱里数出 **0** 个进程，
    因为它枚举进程走 libproc 的 `proc_listallpids`，那个调用被 App Sandbox
    拦掉了。交接文档里「进程管理 0 项」「应用程序 0」都是这么来的 ——
    **不是权限问题，是依赖交白卷**。

    换成 `sysctl(KERN_PROC_ALL)` + `proc_pidinfo` 后沙箱内能拿到 200+ 个
    进程，且 `proc_pidpath` 还能给出完整 bundle 路径。
    """

    def setUp(self) -> None:
        self.lib = strip_rust_comments((TAURI / "src/lib.rs").read_text(encoding="utf-8"))
        self.monitor = strip_rust_comments(
            (TAURI / "src/process_monitor.rs").read_text(encoding="utf-8")
        )
        self.snapshot = strip_rust_comments(
            (TAURI / "src/process_snapshot.rs").read_text(encoding="utf-8")
        )

    def test_the_process_command_branches_on_flavor(self) -> None:
        # 分支必须挂在 flavor 模块上（唯一真相源），不能散落 cfg。
        body = self.lib.split("async fn list_all_processes", 1)[1]
        body = body.split("\n}", 1)[0]
        self.assertIn("flavor::CURRENT", body, "list_all_processes 没有按形态分叉")
        self.assertIn("Flavor::Mas", body, "MAS 分支缺失")
        self.assertIn("Flavor::DeveloperId", body, "Developer ID 分支缺失")

    def test_the_readonly_list_never_reaches_for_sysinfo(self) -> None:
        # 一旦有人在只读路径里用上 sysinfo，沙箱里就又变回 0 项，而且不会有
        # 任何报错 —— 只会安静地给用户一张空列表。
        for forbidden in ("sysinfo", "System::new", "refresh_processes"):
            self.assertNotIn(
                forbidden,
                self.monitor,
                f"只读进程列表里出现了 {forbidden} —— 沙箱内会被拦成 0 项",
            )

    def test_the_snapshot_reads_pids_through_sysctl_not_libproc(self) -> None:
        # 这条是整件事的技术前提。libproc 的 proc_listallpids 被沙箱拦，
        # sysctl(KERN_PROC_ALL) 不被拦。
        self.assertIn("KERN_PROC_ALL", self.snapshot)
        self.assertNotIn("proc_listallpids", self.snapshot)
        self.assertIn("PROC_PIDTASKALLINFO", self.snapshot)

    def test_the_readonly_list_never_claims_it_can_protect_or_terminate(self) -> None:
        # 只读视图里没有终止入口，**没有东西需要保护**。把 protected 设成
        # true 会让整张列表被 opacity-70 变灰，看起来像 App 坏了。
        self.assertIn("protected: false", self.monitor)
        self.assertIn("ports: Vec::new()", self.monitor)
        self.assertIn("icon_base64: None", self.monitor)

    def test_the_readonly_list_keeps_the_raw_process_name_for_identity_checks(self) -> None:
        # `revalidate_targets` 会在终止前把 `ProcessIdentity.name` 与现场读到的
        # 原始名逐字比较。full_name 一旦塞了展示名，一键清理会全部报
        # 「进程身份已变化」。
        self.assertIn("full_name: sample.name.clone()", self.monitor)

    def test_the_two_flavors_produce_the_same_row_shape(self) -> None:
        # 前端零改动的前提：MAS 行必须就是 `scanner::ProcessRow`。
        # 字段少一个编译不过，多一个也会编译不过 —— 这正是我们要的。
        self.assertIn("use crate::scanner::ProcessRow;", self.monitor)
        self.assertIn("-> Vec<ProcessRow>", self.monitor)


class MasProfileAndBookmarkTests(unittest.TestCase):
    """MAS 的签名必须自带文件夹授权能力，而且这条链路要能被自动复核。

    实测（2026-09-30，MAS 包真机，见 docs/mas-capability-matrix.md）：

    - 没有 `bookmarks.app-scope` 时，bookmark **创建成功、解析失败** ——
      症状是「代码完全正常、功能就是不工作」，不主动查根本发现不了。
    - 补上 entitlement 后，**解析成功**，卡在 `startAccessingSecurityScopedResource`
      返回 false —— 因为还没有任何用户通过文件选择框授权过那个目录。
      这是正确行为，不是 bug。

    所以这里断言的是「链路的前半段是通的」，后半段（用户点面板）由人验。
    """

    def setUp(self) -> None:
        self.entitlements = load_plist("entitlements.mas.plist")
        self.monitor = strip_rust_comments(
            (TAURI / "src/sandbox_probe.rs").read_text(encoding="utf-8")
        )
        self.script = (ROOT / "scripts/release-mas.sh").read_text(encoding="utf-8")

    def test_the_signed_entitlements_include_app_scope(self) -> None:
        self.assertTrue(self.entitlements.get("com.apple.security.app-sandbox"))
        self.assertTrue(
            self.entitlements.get("com.apple.security.files.bookmarks.app-scope")
        )

    def test_the_probe_reports_the_bookmark_chain_step_by_step(self) -> None:
        # 「创建成功但解析失败」和「解析成功但拿不到访问权」在上面的症状里
        # 长得一模一样。合成一句「授权失败」会让人一直查错方向 ——
        # 实际就踩过：明明 entitlement 已经加进签名，却被误判成
        # 「profile 缺 entitlement」，而 MAS 的 profile 本来就不该有沙箱
        # entitlement，那是个错误方向。
        for step in ("KERN_PROC_ALL", "resolve_plain", "AccessNotGranted"):
            self.assertIn(step, self.monitor, f"探针缺 {step}")

    def test_the_profile_regeneration_script_exists_and_is_documented(self) -> None:
        # release-mas.sh 的报错信息指向 scripts/create_mas_credentials.py，
        # 而那个文件**从来不存在**（实测：脚本报错让人去跑一个不存在的脚本）。
        # 真正需要的是「改了 entitlement 要重新签 profile」，所以补了这个。
        script = ROOT / "scripts/create_mas_profile.py"
        self.assertTrue(script.exists(), "缺少 MAS profile 生成脚本")
        text = script.read_text(encoding="utf-8")
        self.assertIn("entitlements.mas.plist", text)
        self.assertIn("MAC_APP_STORE", text)

    def test_release_script_points_at_the_script_that_actually_exists(self) -> None:
        # 别再指向 create_mas_credentials.py
        self.assertNotIn("create_mas_credentials.py", self.script)
        self.assertIn("create_mas_profile.py", self.script)


class MasSigningTests(unittest.TestCase):
    """MAS 签名必须自带 profile 的身份字段。

    实测（2026-10-01 首次真实上传）：`codesign --entitlements <file>` **只**用
    给的那一份，不与 provisioning profile 合并。而 Xcode 生成的包能用，是因为
    它在签名时把 profile 的三项身份字段也写进了签名。

    缺了它们的后果不是「沙箱失效」，而是 ASC 直接拒收：
      90886 signature ... is missing an application identifier but has an
            application identifier in the provisioning profile
      90230 Invalid product archive metadata ... product-identifier

    这两条被 altool 的输出淹没，而 `upload` 模式此前**从未跑通**
    （`local` 之前就引用了未声明的变量，`set -u` 下直接中止），所以
    build 全绿的 CI 从来没发现过。
    """

    def setUp(self) -> None:
        self.script = (ROOT / "scripts/release-mas.sh").read_text(encoding="utf-8")

    def test_the_release_script_merges_profile_identity_into_the_signature(self) -> None:
        self.assertIn("sign_entitlements", self.script)
        for key in (
            "com.apple.application-identifier",
            "com.apple.developer.team-identifier",
            "keychain-access-groups",
        ):
            self.assertIn(key, self.script, f"签名时没有带上 {key}")

    def test_codesign_is_never_pointed_at_the_bare_sandbox_plist(self) -> None:
        # 直接 `--entitlements src-tauri/entitlements.mas.plist` 就是漏掉
        # 身份字段的写法，必须指向合并后的那份。
        self.assertNotIn(
            '--entitlements src-tauri/entitlements.mas.plist', self.script
        )
        self.assertIn('--entitlements "$merged"', self.script)

    def test_upload_declares_its_env_file_before_using_it(self) -> None:
        # `local` 必须先于第一次使用。之前是反过来的，于是 upload 模式
        # 一进去就 `unbound variable` 中止 —— 而 build 模式不走这段代码，
        # 所以 CI 全绿。
        body = self.script.split("upload() {", 1)[1].split("\n}", 1)[0]
        local_at = body.index("local ENV_PATH")
        first_use = body.index('"$ENV_PATH"')
        self.assertLess(
            local_at,
            first_use,
            "upload() 里 local ENV_PATH 出现在第一次使用它之后",
        )

    def test_profile_fields_are_read_with_plistbuddy_not_plutil(self) -> None:
        # entitlement 名里带点号，而 plutil 的 -extract 会把点当键路径分隔符，
        # 于是读出来是空 → 合并出一份没有身份字段的签名 → 又是 90886。
        #
        # 只看代码不看注释：这段的注释里**要**写清「为什么不用 plutil」，
        # 把注释一起判会让正确的说明反过来把门禁弄挂。
        body = strip_shell_comments(
            self.script.split("sign_entitlements() {", 1)[1].split("\n}\n", 1)[0]
        )
        self.assertIn("PlistBuddy", body)
        self.assertNotIn("plutil -extract", body)


class MasDeadUiTests(unittest.TestCase):
    """MAS 版不许出现「显示了但用不了」的交互元素。

    这类问题的特征是：**测试全绿、真机截图才看出来**。本轮就靠列表截图
    抓到两处：

    - 进程列表左侧一整列复选框。它们的唯一用途是勾选后点「终止」，
      而 MAS 版终止不了任何进程 —— 点下去毫无反应。
    - 列表底部那句「受保护进程只能强制终止」的提示。MAS 版的只读列表里
      `protected` 恒为 false（没有东西需要保护），所以它是纯噪音，
      而且出现在列表底部很容易被误读成一条错误信息。

    更麻烦的是复选框有**两套实现**（`ProcessList.tsx` 一份、
    `ProcessView.tsx` 排序后的行一份），只改一处会漏。这条门禁就是为了
    钉住「两处都要门禁」。
    """

    FILES = {
        "ProcessList.tsx": ROOT / "src/components/ProcessList.tsx",
        "ProcessView.tsx": ROOT / "src/views/ProcessView.tsx",
    }

    def setUp(self) -> None:
        self.sources = {
            name: path.read_text(encoding="utf-8") for name, path in self.FILES.items()
        }

    def test_every_checkbox_is_behind_the_terminate_capability_gate(self) -> None:
        for name, source in self.sources.items():
            code = strip_tsx_comments(source)
            checkboxes = code.count('type="checkbox"')
            if not checkboxes:
                continue
            gates = code.count("canTerminateProcesses()")
            self.assertGreaterEqual(
                gates,
                checkboxes,
                f"{name} 有 {checkboxes} 个复选框但只有 {gates} 处能力门禁 —— "
                "MAS 版会露出点不动的选择框",
            )

    def test_the_protected_hint_is_not_shown_when_nothing_is_protected(self) -> None:
        code = strip_tsx_comments(self.sources["ProcessView.tsx"])
        self.assertIn("process.protectedHint", code)
        # 它必须落在某个 Show/when 的门禁里，而不是无条件渲染
        self.assertIn(
            "canTerminateProcesses()",
            code,
            "受保护提示必须按能力门禁：MAS 的只读列表里 protected 恒为 false",
        )

    def test_mas_has_no_empty_action_column(self) -> None:
        """只读列表里不能留一整列空白的「操作」表头。

        这是截屏抓出来的第三处：表头照旧写着「操作」，而列里除了 hover
        才浮现的白名单按钮之外什么都没有。截图上它就是一列**永远空白**的
        表头 —— 比留个坏按钮更像「这页坏了」。

        而且那一列里唯一的控件是「加入白名单」，在 MAS 版同样是死功能：
        白名单的作用是保护进程不被终止，而 MAS 终止不了任何进程。所以表头
        与整列都要按能力门禁，grid 模板也得跟着少一列，否则表头与数据
        会错位。
        """
        code = strip_tsx_comments(self.sources["ProcessView.tsx"])
        header_at = code.index("process.columnAction")
        window = code[max(0, header_at - 400) : header_at + 200]
        self.assertIn(
            "canTerminateProcesses()",
            window,
            "「操作」表头必须落在 canTerminateProcesses() 门禁内",
        )
        self.assertIn(
            "whitelist(",
            code,
            "白名单按钮应当仍在（完整版要用），但必须被同一道门禁包住",
        )
        button_at = code.index("whitelist(")
        button_window = code[max(0, button_at - 600) : button_at]
        self.assertIn(
            "canTerminateProcesses()",
            button_window,
            "行内白名单按钮必须按能力门禁 —— MAS 终止不了进程，白名单无意义",
        )
        # 两套 grid 模板都要以字面量出现：Tailwind 只扫源码里的完整类名，
        # 靠字符串拼出来的 grid-cols-[...] 不会被生成，列宽会全部塌掉。
        self.assertIn('"grid-cols-[1fr_72px_72px_64px_56px_96px]"', code)
        self.assertIn('"grid-cols-[1fr_72px_72px_64px_56px]"', code)
        # 表头与数据行必须用同一个条件，否则两边列数对不上
        self.assertGreaterEqual(
            code.count("canTerminateProcesses() ? GRID_FULL : GRID_READONLY"),
            2,
            "表头与数据行都要用同一个能力条件挑 grid 模板",
        )


class MasDockerSectionGateTests(unittest.TestCase):
    """缓存页的 Docker 分区在 MAS 版必须整块藏起来。

    这是截图抓出来的第三处死 UI，而且**位置最靠下、最容易被漏审**：
    `DockerSection` 无论 inventory 有没有都渲染一张卡片，沙箱里
    `docker system df` 跑不通，用户会看到一张永远转圈/永远空的卡片，
    外加一个点了没反应的「一键清理」。

    前后端必须对齐：后端 `cache_scanner.rs` 已经在
    `can_exec_external_tools()` 门禁下**不产出** docker 分类，前端再显示
    这块就纯属自相矛盾。
    """

    CACHE_VIEW = ROOT / "src/views/CacheView.tsx"
    FLAVOR = ROOT / "src/lib/flavor.ts"
    SCANNER = ROOT / "src-tauri/src/cache_scanner.rs"

    def test_the_docker_section_is_not_rendered_unconditionally(self) -> None:
        code = strip_tsx_comments(self.CACHE_VIEW.read_text(encoding="utf-8"))
        self.assertIn("DockerSection", code, "缓存页仍应保留完整版的 Docker 分区")
        self.assertIn(
            'can("dockerCleanup")',
            code,
            "DockerSection 必须落在 can(\"dockerCleanup\") 门禁里 —— "
            "MAS 版沙箱 exec 不了 docker CLI",
        )
        self.assertRegex(
            code,
            r'<Show\s+when=\{can\("dockerCleanup"\)\}\s*>\s*<DockerSection',
            "门禁要包住 <DockerSection> 本身，而不是只在它内部判空",
        )

    def test_docker_cleanup_is_a_declared_capability_and_mas_lacks_it(self) -> None:
        flavor = strip_tsx_comments(self.FLAVOR.read_text(encoding="utf-8"))
        self.assertIn('"dockerCleanup"', flavor)
        # MAS 的能力表里不能出现它 —— 出现即等于又放出了这块死 UI
        mas_block = flavor.split("const MAS: readonly Capability[] = [", 1)
        self.assertEqual(len(mas_block), 2, "flavor.ts 里找不到 MAS 能力表")
        self.assertNotIn(
            '"dockerCleanup"',
            mas_block[1].split("];", 1)[0],
            "dockerCleanup 不能进 MAS 能力表",
        )

    def test_the_backend_agrees_that_mas_produces_no_docker_inventory(self) -> None:
        scanner = strip_rust_comments(self.SCANNER.read_text(encoding="utf-8"))
        self.assertIn(
            "can_exec_external_tools",
            scanner,
            "后端也必须在 can_exec_external_tools 下跳过 docker 扫描，"
            "否则会出现「前端藏了、后端还在算」的另一种不一致",
        )


def strip_tsx_comments(source: str) -> str:
    """去掉 TSX 里的行注释与块注释，只留代码。"""
    import re

    without_blocks = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    out = []
    for line in without_blocks.splitlines():
        stripped = line.lstrip()
        if stripped.startswith("//"):
            continue
        if line.count('"') % 2 == 0 and "//" in line:
            line = line[: line.index("//")]
        out.append(line)
    return "\n".join(out)


class MasFolderGrantI18nTests(unittest.TestCase):
    """文件夹授权的文案 key 必须与后端声明的一致。

    实测（2026-10-01 抓截屏时）：授权清单里显示的是
    `access.target.user_caches` 这种**原始 key** —— 字典里根本没有这一条，
    于是界面把 key 当文案直接印出来了。

    根因是两边命名不一致：后端的 `target.key` 是 snake_case
    （`user_caches`，也是落盘时的稳定标识），而 i18n 字典的 key 是
    camelCase（`userCaches`）。前端从 `key` 拼出 `access.target.user_caches`
    自然查不到。

    修法是前端改用后端下发的 `reason_key`，并由这里钉住「前端不许从
    target.key 拼文案 key」。
    """

    CARD = ROOT / "src/components/FolderAccessCard.tsx"

    def test_the_card_uses_the_reason_key_supplied_by_the_backend(self) -> None:
        source = strip_tsx_comments(self.CARD.read_text(encoding="utf-8"))
        self.assertIn("target.reasonKey", source)

    def test_the_card_never_builds_an_i18n_key_out_of_the_target_key(self) -> None:
        source = strip_tsx_comments(self.CARD.read_text(encoding="utf-8"))
        self.assertNotIn(
            "access.target.${", source, "前端仍在从 target.key 拼文案 key —— 会显示原始 key"
        )

    def test_every_offered_target_has_a_reason_key_that_exists_in_both_dictionaries(self) -> None:
        rust = (TAURI / "src/folder_access.rs").read_text(encoding="utf-8")
        declared = set(re.findall(r'reason_key:\s*"([^"]+)"', rust))
        self.assertGreaterEqual(len(declared), 6, "授权清单少于 6 项，与文案不匹配")
        for dictionary in ("src/i18n/zh-CN.ts", "src/i18n/en.ts"):
            text = (ROOT / dictionary).read_text(encoding="utf-8")
            for key in declared:
                leaf = key.split(".")[-1]
                self.assertIn(
                    f"{leaf}:", text, f"{dictionary} 里缺 {leaf}（{key}）"
                )


class MasWhitelistNoiseTests(unittest.TestCase):
    """MAS 版不该给「不建议终止」这类暗示 —— 它没有对应的操作入口。

    实测（抓截屏时）：进程列表第一行下面挂着「命中白名单，默认不建议
    终止」。那不是「受保护」提示，是**另一套** —— 白名单命中。列表里出现
    「不建议动某个进程」却又不给任何操作入口，是最难解释的一种界面。

    根因：`snapshot_process_rows` 会用白名单策略覆盖 `process_monitor`
    设好的 `protected: false`。白名单回答的是「该不该被终止」，而 MAS 版
    终止不了任何进程，所以整件事没有意义。
    """

    LIB = TAURI / "src/lib.rs"

    def test_the_mas_flavor_uses_a_policy_that_never_matches(self) -> None:
        body = strip_rust_comments(self.LIB.read_text(encoding="utf-8"))
        fn = body.split("fn whitelist_policy", 1)[1].split("\n}", 1)[0]
        self.assertIn("Flavor::Mas", fn, "whitelist_policy 没有区分 MAS 形态")
        self.assertIn(
            "false", fn, "MAS 分支必须返回一个恒为 false 的策略（没有进程该被终止）"
        )

    def test_the_readonly_rows_are_not_relabelled_as_protected(self) -> None:
        monitor = strip_rust_comments(
            (TAURI / "src/process_monitor.rs").read_text(encoding="utf-8")
        )
        self.assertIn("protected: false", monitor)
        self.assertIn("protected_reason_key: None", monitor)


class FolderGrantPrivacyTruthTests(unittest.TestCase):
    """隐私政策与 ASC 隐私文本不得谎称 bookmark「退出即失效」。

    ## 这条错在哪

    security-scoped bookmark 的**设计目的就是跨启动持久**：用户在文件选择框
    点一次授权，之后每次启动都能直接读，不该反复骚扰。原来的文案写成
    「退出即失效 / valid only while the app runs」，与实现相反，也与
    应用内文案「授权一次即长期有效」自相矛盾。

    为什么要在意措辞准不准：审核会人工读隐私政策。一条**低估**自己权限
    的描述，比一条准确的描述更容易被当成误导 —— 尤其它旁边紧跟着
    「我们不保存、也不上传被授权目录里的任何内容」这种承诺，读起来像是
    「反正拿到也没用」。真实边界要说清楚：记录持久、读取按需、卸载即消失。
    """

    SOURCES = {
        "docs/privacy.html": ROOT / "docs/privacy.html",
        "scripts/publish_mas_metadata.py": ROOT / "scripts/publish_mas_metadata.py",
    }

    FORBIDDEN = ("退出即失效", "只在应用运行期间生效", "valid only while the app runs")

    def test_no_source_claims_the_grant_dies_when_the_app_quits(self) -> None:
        for name, path in self.SOURCES.items():
            text = path.read_text(encoding="utf-8")
            for phrase in self.FORBIDDEN:
                self.assertNotIn(
                    phrase,
                    text,
                    f"{name} 声称授权「{phrase}」，与 bookmark 跨启动持久的事实相反",
                )

    def test_the_policy_states_the_real_lifetime(self) -> None:
        html = self.SOURCES["docs/privacy.html"].read_text(encoding="utf-8")
        self.assertIn("跨启动保留", html, "隐私政策应说明授权记录跨启动保留")
        metadata = self.SOURCES["scripts/publish_mas_metadata.py"].read_text(encoding="utf-8")
        self.assertIn("跨启动保留", metadata, "ASC 中文隐私文本应同步")
        self.assertIn(
            "persists across launches",
            metadata,
            "ASC 英文隐私文本应同步",
        )


class MasCacheEmptyStateTruthTests(unittest.TestCase):
    """空态不许用「这个形态有没有授权能力」当判据。

    ## 这条门禁来自一句假话

    缓存页空态原先是 `needsFolderGrant()` —— 一个**静态能力位**，在 MAS 下
    恒为 true。于是即使用户六个目录全授权了、Mac 也确实干净，界面照样显示
    「还没授权任何目录，所以看不到可清理的缓存」：用户点遍六个「授权」按钮
    之后，这句话一个字都不会变。

    这不只是上架截图难看 —— 它是对真实用户撒谎，而且属于 AGENTS §7.2
    明令禁止的那类「用户可见错误必须是人话且如实」。

    后端 `folder_access.rs` 的注释早就写明了该怎么修：

        UI 必须据此告诉用户去授权，而不是显示「没有发现可清理的缓存」

    它为此造的 `RootsReason` 从没被前端接过，`scan_roots*` 至今是死代码。
    而判据其实前端本来就拿得到：`FolderAccessCard` 为了画卡片一直在调
    `listFolderAccess()`，把 granted 数报出来就够，不必改后端 IPC 形状。
    """

    VIEW = ROOT / "src/views/CacheView.tsx"
    CARD = ROOT / "src/components/FolderAccessCard.tsx"

    def test_the_judgement_is_the_granted_count_not_the_capability(self) -> None:
        code = strip_tsx_comments(self.VIEW.read_text(encoding="utf-8"))
        self.assertIn(
            'grantedCount() > 0 ? "cache.noCleanable" : "cache.noAccess"',
            code,
            "空态判据必须是「到底授权了没有」",
        )
        # 能力位只能决定「这一形态有没有授权这回事」，不能决定「用户授权了没有」
        self.assertNotRegex(
            code,
            r'when=\{needsFolderGrant\(\)\}[\s\S]{0,200}cache\.noAccess',
            "cache.noAccess 不能再挂在能力位门禁下 —— 它是「用户没授权」的意思",
        )

    def test_the_card_reports_its_granted_count(self) -> None:
        code = strip_tsx_comments(self.CARD.read_text(encoding="utf-8"))
        self.assertIn("onGrantedCount", code, "卡片要把已授权数量报给调用方")
        self.assertIn(
            "createEffect",
            code,
            "上报要走 effect：首次加载完成、以及 grant/revoke 之后都要自动补发",
        )

    def test_both_branches_have_real_copy_in_both_dictionaries(self) -> None:
        """两句必须是两句**不同**的话，而且都要有中英文。"""
        for name in ("zh-CN.ts", "en.ts"):
            keys = _flatten_dict(load_ts_dict(ROOT / "src/i18n" / name))
            for key in ("cache.noAccess", "cache.noCleanable", "cache.noItems"):
                self.assertIn(key, keys, f"{name} 缺 {key}")

        zh = load_ts_dict(ROOT / "src/i18n/zh-CN.ts")
        en = load_ts_dict(ROOT / "src/i18n/en.ts")
        self.assertNotEqual(
            zh["cache"]["noAccess"],
            zh["cache"]["noCleanable"],
            "「没授权」和「已授权但干净」不能是同一句话",
        )
        self.assertNotEqual(
            en["cache"]["noAccess"],
            en["cache"]["noCleanable"],
            "同上（英文）",
        )


class MasNoOffPlatformSteeringTests(unittest.TestCase):
    """App Store 版不得把用户导向站外的「完整版」。

    ## 为什么这是最可能被拒的一条

    原先 `FdaCard.tsx` 在 MAS 构建里渲染一个 **`btn-primary` 主按钮**
    「获取完整版（支持完整缓存与开发缓存清理）」，点开 `https://vgoapp.com`。
    三重叠加，审核看到的就是「这个 App Store 版是残废的，去我们官网下真的那个」：

    - 用主按钮样式 —— 设置页上最扎眼的元素
    - 形态上就是导流：App Store 版 → 外部网站 → 下载同款完整版
    - 指南 **4.0** 把「功能太少」列为下架原因第一位，**2.1** 会把自认残缺的
      构建当成不完整；再加一条导流

    「免费版 + 官网下完整版」被拒是行业里反复发生的事，因为它同时踩中这三条。

    ## 删掉它不损失任何诚实

    `settings.fda.sandboxed` 那段本来就如实列出生效的四项能力
    （进程监控、系统健康、应用体积分析、应用卸载不受影响），并说明「这是平台
    限制，授权也解决不了」。**诚实陈述和导流是两件事**，原先混在一起了。

    所以判据只拦「去别处拿」这个动作，不拦「这里做不到」这个事实。
    """

    # 「去别处拿」的措辞。中英都要拦：只拦中文等于英文版照样被导流。
    FORBIDDEN = (
        "获取完整版",
        "请用 Developer ID 版",
        "或改用完整版",
        "use the full edition",
        "Use the Developer ID build",
        "Get the full edition",
    )
    DICTIONARIES = (ROOT / "src/i18n/zh-CN.ts", ROOT / "src/i18n/en.ts")
    COMPONENTS = (ROOT / "src/components/FdaCard.tsx", ROOT / "src/views/ProcessView.tsx")

    def test_no_user_facing_string_points_at_another_edition(self) -> None:
        """扫的是**代码**，注释不算。

        不剥注释的话，这条门禁会先被开发者自己的解释性注释挡下 ——
        而那些注释恰恰应该保留：删掉导流按钮的理由必须留在原地，
        否则过几个月有人会「补回一个更温和的升级入口」。
        """
        for path in (*self.DICTIONARIES, *self.COMPONENTS):
            text = strip_tsx_comments(path.read_text(encoding="utf-8"))
            for phrase in self.FORBIDDEN:
                self.assertNotIn(
                    phrase,
                    text,
                    f"{path.name} 仍出现「{phrase}」—— 在 App Store 里把用户导向站外完整版"
                    "是最可能被拒的一条（指南 2.1 / 4.0 + 导流）",
                )

    def test_there_is_no_external_link_left_in_the_frontend(self) -> None:
        """整个前端只允许营销页那种固定链接，且必须在 MAS 构建里不可达。

        实测原先全前端只有一处站外链接，就是那个导流按钮。
        """
        offenders = []
        for path in ROOT.joinpath("src").rglob("*.tsx"):
            if ".test." in path.name:
                continue
            text = path.read_text(encoding="utf-8")
            if 'href="http' in text or 'target="_blank"' in text:
                offenders.append(path.name)
        self.assertEqual(
            offenders,
            [],
            f"这些文件里还有站外链接：{offenders}。App Store 版里的每一个都要有明确理由",
        )

    def test_the_honest_scope_statement_stays(self) -> None:
        """删掉的必须是导流，不是诚实。

        `settings.fda.sandboxed` 那段是化解审核疑虑的关键：它说明这是平台
        限制，并逐项列出不受影响的能力。少了它，剩下的话就成了「功能太少」
        的自认。
        """
        for path in self.DICTIONARIES:
            keys = _flatten_dict(load_ts_dict(path))
            self.assertIn("settings.fda.sandboxed", keys, f"{path.name} 缺 sandboxed 说明")
        zh = load_ts_dict(ROOT / "src/i18n/zh-CN.ts")
        self.assertIn(
            "不受影响",
            zh["settings"]["fda"]["sandboxed"],
            "sandboxed 说明必须继续列出哪些能力不受影响",
        )

    def test_the_terminate_limitation_is_still_stated(self) -> None:
        """「终止不了进程」这个事实要留着，只是不要再指路。"""
        zh = load_ts_dict(ROOT / "src/i18n/zh-CN.ts")
        self.assertIn("无法终止", zh["process"]["masTerminateUnsupported"])
        self.assertIn(
            "这一页仍可用来查看",
            zh["process"]["masTerminateUnsupported"],
            "要留下这一页能做什么，而不是只说不能做什么",
        )

    def test_the_store_description_does_not_advertise_the_other_edition(self) -> None:
        """上架描述末句原本是「两个版本……官网提供完整版」。

        那是给每位用户看、审核逐字读的文案 —— 等于在商店页面里公开导流，
        比应用内的那个按钮更严重。改成陈述沙箱边界与「其余功能不受影响」，
        诚实度不降反升。
        """
        source = (ROOT / "scripts/publish_mas_metadata.py").read_text(encoding="utf-8")
        for phrase in ("官网提供完整版", "两个版本", "Two editions", "完整版"):
            self.assertNotIn(
                phrase, source, f"上架描述里仍出现「{phrase}」—— 公开导流"
            )

    def test_the_description_still_declares_the_scope_honestly(self) -> None:
        source = (ROOT / "scripts/publish_mas_metadata.py").read_text(encoding="utf-8")
        self.assertIn(
            "进程监控",
            source,
            "描述仍要写明这个版本提供什么（诚实陈述范围，不是导流）",
        )


class ScreenshotStoryTests(unittest.TestCase):
    """五拍故事线：文案要齐、两种语言要对得上、叠字不许盖住侧栏。

    ## 为什么这些也要钉

    叠字是**唯一**能让 App Store 有故事线的手段（Apple 自己不会给 macOS 截图
    加标题），所以文案和工具就成了关键路径的一部分。而它失败得很安静：叠字
    脚本照样退出 0、PNG 尺寸照样 2880x1800，只有把图打开看才发现问题 ——
    第一版就出了两个：渐变太透导致白字和应用文字叠在一起，以及整幅盖满把侧栏
    顶部盖掉、导航列表从中间开始，看起来像应用坏了。
    """

    SCRIPT = ROOT / "scripts/annotate_screenshots.py"
    STORIES = {
        "zh-Hans": ROOT / "scripts/screenshot_story.zh-Hans.json",
        "en-US": ROOT / "scripts/screenshot_story.en-US.json",
    }
    # 五拍：授权 → 看得见 → 敢删 → 删了 → 可追溯
    BEATS = (
        "01-authorized.png",
        "02-scanned.png",
        "03-confirm.png",
        "04-cleaned.png",
        "05-history.png",
    )

    def setUp(self) -> None:
        self.stories = {
            locale: json.loads(path.read_text(encoding="utf-8"))
            for locale, path in self.STORIES.items()
        }

    def test_both_locales_cover_exactly_the_five_beats(self) -> None:
        for locale, story in self.stories.items():
            self.assertEqual(
                tuple(sorted(story)),
                tuple(sorted(self.BEATS)),
                f"{locale} 的文案条目必须正好是这五拍",
            )

    def test_every_beat_has_a_headline_and_a_subline(self) -> None:
        for locale, story in self.stories.items():
            for beat, copy in story.items():
                for field in ("headline", "subline"):
                    self.assertTrue(
                        copy.get(field, "").strip(),
                        f"{locale}/{beat} 缺 {field}",
                    )

    def test_no_two_beats_say_the_same_thing(self) -> None:
        """五句话必须真的在推进故事，而不是同一句换了五个说法。"""
        for locale, story in self.stories.items():
            headlines = [story[beat]["headline"] for beat in self.BEATS]
            self.assertEqual(
                len(set(headlines)),
                len(headlines),
                f"{locale} 有两拍标题完全相同",
            )

    def test_the_english_copy_has_no_chinese_left_in_it(self) -> None:
        """抓「漏译」的残留：英文那套里混进了中文字符。

        一开始我想用「中英标题长度倍数」来近似判断是否等义翻译 —— 那是个坏代理：
        英文天然比中文长得多（一句 12 字的中文对应 39 个字符的英文很常见），于是
        正常的文案被判成不等义。

        字符级互查才是真正能自动抓到的问题：**复制粘贴时忘了翻译**，会在英文
        那套里留下中文字符，而这种错误在缩略图尺寸下极难被肉眼发现。
        等义与否最终仍要人读一遍，但至少漏译能被挡住。
        """
        cjk = re.compile(r"[一-鿿　-〿＀-￯]")
        for beat, copy in self.stories["en-US"].items():
            for field in ("headline", "subline"):
                found = cjk.findall(copy[field])
                self.assertEqual(
                    found,
                    [],
                    f"en-US/{beat} 的 {field} 里混进了中文{found} —— 漏译",
                )

        for beat, copy in self.stories["zh-Hans"].items():
            for field in ("headline", "subline"):
                self.assertTrue(
                    cjk.search(copy[field]),
                    f"zh-Hans/{beat} 的 {field} 没有中文 —— 可能贴错了语言",
                )

    def test_the_tool_refuses_to_upload_a_shot_without_copy(self) -> None:
        """缺文案必须直接失败，而不是叠一张没故事的图上去。"""
        source = self.SCRIPT.read_text(encoding="utf-8")
        self.assertIn("宁可不叠", source)
        self.assertIn("missing", source)

    def test_the_caption_never_covers_the_sidebar(self) -> None:
        """盖住侧栏顶部会让导航列表从中间开始，一眼看着像应用坏了。"""
        source = self.SCRIPT.read_text(encoding="utf-8")
        self.assertIn("GUTTER", source)
        gutter = int(
            re.search(r"GUTTER\s*=\s*(\d+)", source).group(1)
        )
        # 侧栏实测宽 280 点 @2x = 560 像素（截屏里「缓存清理」选中态的右边缘
        # 就在 x≈560）。gutter 必须不小于它，否则会啃掉侧栏。
        self.assertGreaterEqual(gutter, 560, "文案栏起点必须在侧栏右侧")
        self.assertIn(
            "draw.rectangle([(GUTTER, 0)",
            source,
            "实心栏只能画在内容区，不能整幅盖满",
        )

    def test_the_band_is_opaque_with_a_fade_below(self) -> None:
        """半透明渐变会让下面应用文字与我们的文字叠在一起，两边都不可读。"""
        source = self.SCRIPT.read_text(encoding="utf-8")
        self.assertIn("fill=(*BAR_TOP, 255)", source, "实心部分必须不透明")
        self.assertIn("FADE", source, "下沿要留柔化，避免硬边像贴纸")
        self.assertIn("CANVAS = (2880, 1800)", source, "尺寸必须正好是 ASC 认可的那档")

    def test_the_originals_are_never_overwritten(self) -> None:
        """原始截屏是无价的事实：叠坏了要能重做。"""
        source = self.SCRIPT.read_text(encoding="utf-8")
        self.assertIn("-shot", source, "输出应落到单独的目录")
        self.assertIn("原图不动", source)


class ScreenshotFixtureTests(unittest.TestCase):
    """造截图用的假缓存：只许「移开」，绝不许「删除」用户真实数据。

    ## 为什么风险集中在这个脚本

    「清理完成」和「历史记录」这两张图要求真的清理过一次。但所有缓存扫描器
    都是**整目录一个条目**的粒度（`scan_app_logs` 给整个 `~/Library/Logs`
    出一个，`scan_npm` 给整个 `~/.npm` 出一个），所以往真实目录里塞 fixture
    会被连锅端。

    于是唯一安全的机制是移开 + 还原。真实数据全程只被改名。这条一旦写错，
    损失的是用户真实的日志目录 —— 而且**不可撤销**（AGENTS §4.1）。
    """

    SCRIPT = ROOT / "scripts/screenshot_fixture.sh"

    def setUp(self) -> None:
        self.raw = self.SCRIPT.read_text(encoding="utf-8")
        self.source = strip_shell_comments(self.raw)

    def test_it_empties_the_directory_instead_of_renaming_it(self) -> None:
        """只能搬空目录，不能改名 —— `~/Library/*` 受 TCC 保护，改名被拒。

        实测 `mv ~/Library/Logs ~/Library/Logs.shot-backup` 返回
        `Permission denied`，而往里写文件是可以的。所以机制只能是
        「把内容移出去、目录原地留空」。
        """
        self.assertIn(
            'find "$TARGET_DIR" -mindepth 1 -maxdepth 1 -exec mv',
            self.source,
            "要连隐藏项一起搬走（glob 会漏 .DS_Store）",
        )
        self.assertNotRegex(
            self.source,
            r'mv\s+"\$TARGET_DIR"',
            "目录本身不可改名 —— 实测被 TCC 拒绝",
        )

    def test_the_backup_is_only_removed_after_a_successful_merge(self) -> None:
        """备份可以在 rsync 成功**之后**删，但不能之前删。

        一开始我把这条写成「备份绝不能删」—— 那是我自己写错了：rsync 是
        **复制**不是移动，合并完备份里还留着同一份内容，不删就等于在用户
        家目录里留一份日志副本，而且下次 `up` 会因为「备份已存在」直接拒绝。
        真正的约束只有顺序：`set -e` 保证 rsync 出错就不会走到 rm。
        """
        rsync_at = self.source.index('rsync -a "$BACKUP_DIR/"')
        rm_at = self.source.index('rm -rf "$BACKUP_DIR"')
        self.assertLess(rsync_at, rm_at, "必须先合并成功，才允许删备份")
        self.assertEqual(
            self.source.count('rm -rf "$BACKUP_DIR"'),
            1,
            "备份只应在还原流程末尾删一次，别处出现就是漏了保护",
        )
        # 删备份之前不允许有任何针对它的删除
        for line in self.source[:rm_at].splitlines():
            self.assertNotIn(
                'rm -rf "$BACKUP_DIR"', line, "合并之前不许删备份"
            )

    def test_restore_merges_instead_of_overwriting(self) -> None:
        """目录被搬空的那几分钟里，系统照常会往 Logs 写新日志。

        直接覆盖会把期间产生的新日志弄丢；只移回旧的又会把新的挤掉。必须
        rsync 合并 —— 目录项也要递归合并，`CrashReporter` 这种期间又被写
        过的目录才不会丢。
        """
        self.assertIn("rsync -a", self.source)
        self.assertIn("期间新产生的文件", self.source, "要提示操作者目录不再干净")

    def test_it_only_deletes_its_own_fixture_files(self) -> None:
        self.assertIn('rm -f "$TARGET_DIR"/"$FIXTURE_PREFIX"-*', self.source)
        self.assertNotIn(
            'rm -rf "$TARGET_DIR"',
            self.source,
            "绝不能整目录删：期间新产生的日志就在里面",
        )

    def test_it_probes_writability_before_moving_anything(self) -> None:
        """先确认能写，再搬空。

        顺序反了会出现「备份已建、fixture 建不了」的半成品状态 ——
        用户的日志已经在备份里，而脚本报错退出。
        """
        probe = self.source.index('-probe" || {')
        move = self.source.index("-exec mv {} \"$BACKUP_DIR/\"")
        self.assertLess(probe, move, "可写性探测必须早于搬空")

    def test_it_refuses_to_start_when_a_backup_already_exists(self) -> None:
        self.assertIn("上一次可能没还原干净", self.source)

    def test_the_fixture_is_sparse_so_it_costs_almost_no_disk(self) -> None:
        """`dir_size` 量的是 metadata().len()，所以稀疏文件的标称长度会被当真。

        这不是取巧：`len()` 确实是这个文件的真实标称长度，扫描器也确实测出了
        它，清理也真的删了它、历史也真的记下了它。稀疏只是让「5 GB」这个数字
        不必真的占 5 GB 磁盘。
        """
        self.assertIn("truncate -s", self.source)
        self.assertIn("稀疏文件", self.raw)

    def test_it_targets_app_logs_on_purpose(self) -> None:
        """目标是 ~/Library/Logs —— 换目标必须重新论证，不能随手改。

        这里读**原文**（含注释）而不是剥掉注释的代码：守的正是「选择旁边必须
        留着理由」。改目标而不更新理由，通常意味着有人跳过了「沙箱里这个
        目录到底扫不扫得到」这个验证 —— 而那正是选它的唯一理由。
        """
        self.assertIn(
            'TARGET_DIR="${SHOT_FIXTURE_DIR:-$HOME/Library/Logs}"', self.raw
        )
        self.assertIn("纯路径扫描", self.raw, "要保留「为什么不选 Caches」的论证")
        self.assertIn("受 TCC/SIP 保护", self.raw, "要保留「为什么不能改名」的实测结论")


class LongCopyLayoutTests(unittest.TestCase):
    """不得对会随语言变长的文案使用 `whitespace-nowrap`。

    ## 为什么需要

    出英文截屏时才暴露：缓存页右下角那句「此操作不可撤销，请确认后执行」
    在中文下够短，加了 `whitespace-nowrap` 也没事；英文是
    "This action cannot be undone, confirm before proceeding"，nowrap 让它撑出
    容器右沿，被**裁掉**了最后几个字母。

    这类问题中文界面永远看不见、测试也不会红（没有像素断言），只有把英文
    版截屏打开看才发现。所以对「两种语言长度差别大」的文案直接禁掉 nowrap，
    改成允许换行 + 右对齐。

    判据只针对这一句：给全局加「禁止 nowrap」会误伤表格里该单行显示的短
    标签（`总内存`、`PID`），那属于矫枉过正。
    """

    VIEW = ROOT / "src/views/CacheView.tsx"

    def test_the_irreversible_notice_may_wrap(self) -> None:
        code = strip_tsx_comments(self.VIEW.read_text(encoding="utf-8"))
        anchor = code.index("common.notice_irreversible")
        window = code[max(0, anchor - 500) : anchor]
        self.assertNotIn(
            "whitespace-nowrap",
            window,
            "不可撤销提示不得 nowrap：英文长度约为中文两倍，会被裁掉",
        )
        self.assertIn(
            "text-right",
            window,
            "改成允许换行后要右对齐，否则换行后左沿会参差不齐",
        )

    def test_the_english_notice_really_is_much_longer(self) -> None:
        # 钉住「英文更长」这个前提：哪天有人把英文文案改短了、或者两边
        # 换过来，这条门禁的前提就该重新评估而不是继续盲守。
        zh = _dict_value_line(
            (ROOT / "src/i18n/zh-CN.ts").read_text(encoding="utf-8"),
            "notice_irreversible",
        )
        en = _dict_value_line(
            (ROOT / "src/i18n/en.ts").read_text(encoding="utf-8"),
            "notice_irreversible",
        )
        self.assertIsNotNone(zh)
        self.assertIsNotNone(en)
        self.assertGreater(
            len(en),
            len(zh),
            "英文提示理应明显长于中文；若不再如此，说明 nowrap 的风险前提变了",
        )


class MasScreenshotCaptureTimingTests(unittest.TestCase):
    """截屏脚本不得用「一个固定秒数」等所有页面。

    ## 这条门禁来自一次真实翻车

    脚本原先每页一律 `sleep 9`。而各页耗时差两个数量级：进程页枚举
    260 个进程约 1 秒就绪，应用卸载页要给每个 .app 递归统计体积 ——
    实测 12 秒仍是骨架屏、30 秒才出列表。于是上架图里混进了一屏**骨架屏**，
    而「停在加载中」正是审核指南 2.1 里最典型的「不完整」形态。

    这类失败特别阴险：脚本照常打印「完成」、退出码 0、文件名和尺寸全部
    正常，只有把图打开看才发现。所以必须钉住「不许再回到固定 sleep」。
    """

    SCRIPT = ROOT / "scripts/capture_mas_screenshots.sh"

    def setUp(self) -> None:
        self.source = strip_shell_comments(
            self.SCRIPT.read_text(encoding="utf-8")
        )

    def test_it_waits_for_the_app_to_go_idle_instead_of_a_fixed_sleep(self) -> None:
        self.assertIn("wait_until_idle()", self.source, "启动后应等进程空闲")
        self.assertIn("cpu_time()", self.source, "空闲信号应取自进程 CPU 时间")
        self.assertIn("IDLE_DELTA", self.source)
        self.assertIn("MAX_SETTLE", self.source, "要有上限，避免进程异常时空转")

    def test_no_page_is_served_by_a_bare_sleep(self) -> None:
        body = self.source.split("launch_shootable()", 1)
        self.assertEqual(len(body), 2, "找不到 launch_shootable")
        launch = body[1].split("\n}", 1)[0]
        self.assertNotIn(
            "sleep 9",
            launch,
            "别再回到固定 9 秒 —— 卸载页要 30 秒，它会截到骨架屏",
        )
        self.assertNotIn(
            "screencapture",
            launch,
            "等待逻辑应放在 launch_shootable 里，截图前必须已经等过",
        )

    def test_the_idle_wait_keeps_a_floor_and_reports_a_timeout(self) -> None:
        # 下限：数据秒回时空闲检测会立刻通过，那时窗口还没稳定
        self.assertIn("sleep \"$MIN_SETTLE\"", self.source)
        # 上限要有告警，否则「等到天荒地老」和「卡死」看起来一模一样
        self.assertIn("骨架屏", self.source, "超时必须明说可能截到骨架屏")

    def test_partial_capture_keeps_the_original_file_numbering(self) -> None:
        """补截时编号必须沿用完整列表的位置，不能按子集重排。

        uploader 是按文件名 ASCII 序入位的。只补 cache 一张时若从 01 开始数，
        它会变成 `01-cache.png` 排到进程图前面 —— 上架页的展示顺序就反了，
        而且这一切不会有任何报错。
        """
        self.assertIn("SHOT_PAGES", self.source, "应支持只截其中几页")
        self.assertIn(
            'ALL_PAGES[$i]%%:*}" = "$page"',
            self.source,
            "编号应按完整列表里的位置算",
        )
        self.assertNotIn(
            "idx=$((idx + 1))",
            self.source,
            "不能用子集自增编号 —— 补截会把顺序排错",
        )

    def test_the_two_source_patches_must_actually_land(self) -> None:
        """首屏与语言两处补丁都要校验命中数。

        这两处都是「文本替换式」注入，一旦目标被重构（换个写法、改个名字），
        替换会静默变成 no-op：截图脚本照样打印「完成」、退出码 0，只是画面
        全错 —— 最坏情况是把中文图当英文图传上 en-US。宁可当场失败。
        """
        self.assertIn("loadStored 大概被重构过", self.source)
        self.assertIn("App.tsx 大概被重构过", self.source)

    def test_an_unknown_page_name_fails_before_building_anything(self) -> None:
        self.assertIn("不是已知页面", self.source, "SHOT_PAGES 写错要当场报错")
        # 校验必须发生在构建循环之前
        self.assertLess(
            self.source.index("不是已知页面"),
            self.source.index('"$ROOT/scripts/release-mas.sh" build'),
            "页面名校验要早于第一次构建，否则白等好几分钟才报错",
        )


class MasBuildNumberTests(unittest.TestCase):
    """CFBundleVersion 必须与商店版本独立，且每次上传变大。

    ## 为什么会踩到

    Tauri 只从 `tauri.conf.json` 的 `version` 生成**两个**键，于是 MAS 包里
    CFBundleVersion == CFBundleShortVersionString == 1.0.0。第一次上传成功，
    之后每次都被 ASC 挡回：

        ENTITY_ERROR.ATTRIBUTE.INVALID.DUPLICATE  (-19232 / -19241)
        The bundle version must be higher than the previously uploaded version: '1.0.0'

    这是流程性的坑，不是编译错误 —— build 全绿、CI 全绿、签名自检全过，
    只有真的传那一刻才炸，而且是在传了 20 分钟之后。而 ASC **不允许**同一
    build 号覆盖，所以「改了代码想重传」这件事根本做不到。

    默认值按日期（1.YYYYMMDD）而不是人工递增整数：人工递增迟早会忘，忘的
    那次就是又一次 409。
    """

    SCRIPT = ROOT / "scripts/release-mas.sh"

    def setUp(self) -> None:
        self.source = strip_shell_comments(self.SCRIPT.read_text(encoding="utf-8"))

    def test_it_stamps_a_build_number_distinct_from_the_store_version(self) -> None:
        self.assertIn(
            "stamp_build_number",
            self.source,
            "必须有一个抬 build 号的步骤",
        )
        self.assertIn("Set :CFBundleVersion", self.source)
        self.assertIn("MAS_BUILD_NUMBER", self.source, "同一天重传时要能显式指定")
        self.assertIn("date +%Y%m%d", self.source, "默认按日期，保证逐日变大")

    def test_the_self_check_rejects_a_build_number_equal_to_the_store_version(self) -> None:
        self.assertIn(
            "CFBundleVersion 与商店版本相同",
            self.source,
            "自检里就要拦住，别等传完 20 分钟才看到 ASC 的 409",
        )
        self.assertIn("Print :CFBundleVersion", self.source)

    def test_stamping_runs_before_signing(self) -> None:
        stamp = self.source.index("stamp_build_number \"$(store_version)\"")
        sign = self.source.index("; sign; verify;")
        self.assertLess(
            stamp, sign,
            "必须在签名前改 Info.plist —— 改在签名之后会破坏签名完整性",
        )

    def test_the_pkg_name_is_not_hardcoded(self) -> None:
        """文件名写死版本号，改了版本就与产物对不上。

        而文件名根本不影响 ASC（ASC 只读包内 Info.plist），所以写死它既没好处
        又会在版本变更后给出误导性的产物名。
        """
        self.assertIn('PKG_PATH="${ZIP_DIST}/MacSlim-$(store_version)-mas.pkg"', self.source)
        self.assertNotIn(
            'PKG_PATH="${ZIP_DIST}/MacSlim-1.0.0-mas.pkg"',
            self.source,
            "pkg 文件名不该写死版本号",
        )


class AscApiHelperTests(unittest.TestCase):
    """ASC 请求助手必须容忍空正文响应。

    ## 为什么

    ASC 的 DELETE 返回 204 无正文，硬解 JSON 会把成功表现成崩溃。踩过：
    清理上传失败留下的孤儿截图时，DELETE 明明成功（204），却抛
    `JSONDecodeError: Expecting value: line 1 column 1` —— 一堆 requests 的
    栈，完全看不出「删除其实成功了」，反而像凭据坏了。

    这不是边角情况：DELETE 在这套脚本里是常规操作（孤儿资源清理、重建
    资源），所以凡是成功路径都会撞上。
    """

    def test_it_survives_an_empty_response_body(self) -> None:
        source = (ROOT / "scripts/create_mas_profile.py").read_text(encoding="utf-8")
        self.assertIn(
            "response.status_code in (204, 205)",
            source,
            "空正文响应必须短路返回，不能交给 response.json()",
        )
        self.assertIn(
            "not response.content.strip()",
            source,
            "除状态码外也应按正文是否为空兜底 —— 有的端点回 200 空正文",
        )


class MasScreenshotUploadContractTests(unittest.TestCase):
    """截图上传脚本必须只建 macOS 的那一个 set，且字段名与 ASC 实测一致。

    ## 为什么钉得这么细

    字段名是**探活 API 试出来的**，不是照文档抄的：Apple 的文档页
    （`AppScreenshotSetCreateRequest`）在 markdown 渲染里根本不列属性，
    而按「iOS 老形状」直觉写的两个属性都会被 409 挡回：

        ENTITY_ERROR.ATTRIBUTE.UNKNOWN
        'appStoreScreenshotType' is not an attribute on the resource
        'appScreenshotSets'

    `screenshotDisplayType` 才是真名，filter 也叫
    `filter[screenshotDisplayType]`。这类「文档不告诉你、只能试」的
    字段最需要门禁钉住 —— 否则改回去要再赔一次 409。

    另一半是**别把 iOS 的形状搬过来**：macOS 只有 APP_DESKTOP 一种展示
    尺寸。原来交错建 APP_DESKTOP + APP_DETAILS 两个 set，多出来的那套
    审核根本看不到，却会让 ASC 里出现两套互相矛盾的图。
    """

    SCRIPT = ROOT / "scripts/upload_mas_screenshots.py"

    def setUp(self) -> None:
        # 只看代码不看注释：解释「为什么不用 APP_DETAILS」的那段注释里
        # 必然会出现 APP_DETAILS 这个词，拿它当判据就成了自伤。
        self.source = strip_py_comments(self.SCRIPT.read_text(encoding="utf-8"))

    def _has(self, needle: str) -> bool:
        return needle in self.source

    def test_it_uses_the_field_name_asc_actually_accepts(self) -> None:
        self.assertTrue(
            self._has('"screenshotDisplayType"'),
            "appScreenshotSets 的属性名是 screenshotDisplayType（实测）",
        )
        self.assertTrue(
            self._has("filter[screenshotDisplayType]"),
            "filter 参数名同样是 filter[screenshotDisplayType]",
        )

    def test_never_sends_the_two_fields_asc_rejects(self) -> None:
        for wrong in ("appStoreScreenshotType", "appStoreScreenshotDisplayType"):
            self.assertFalse(
                self._has(f'"{wrong}"'),
                f"{wrong} 不是 appScreenshotSets 的属性，POST 会吃 409",
            )

    def test_the_screenshot_points_at_its_set_by_the_right_relationship(self) -> None:
        """relationship 叫 `appScreenshotSet`，不叫 `appStoreScreenshotSet`。

        和上面的属性名是同一类坑：文档 markdown 不列字段，只能靠 409 的
        detail 反推，而它这次把两个错一起说了：

            ENTITY_ERROR.RELATIONSHIP.UNKNOWN
            'appStoreScreenshotSet' is not a relationship on 'appScreenshots'
            ENTITY_ERROR.RELATIONSHIP.REQUIRED
            You must provide a value for the relationship 'appScreenshotSet'
        """
        self.assertTrue(
            self._has('"appScreenshotSet"'),
            "relationship 名应为 appScreenshotSet",
        )
        self.assertFalse(
            self._has('"appStoreScreenshotSet"'),
            "appStoreScreenshotSet 不是 appScreenshots 的 relationship，POST 会吃 409",
        )

    def test_macos_gets_exactly_one_screenshot_set(self) -> None:
        self.assertTrue(
            self._has('SCREENSHOT_DISPLAY_TYPE = "APP_DESKTOP"'),
            "macOS 只有 APP_DESKTOP 一种展示尺寸",
        )
        self.assertFalse(
            self._has("APP_DETAILS"),
            "APP_DETAILS 是 iOS 的展示尺寸，macOS 不该出现第二个 set",
        )
        self.assertFalse(
            self._has("SCREENSHOT_SETS"),
            "单 set 之后不应再有 set 列表与交错分配逻辑",
        )

    def test_it_commits_with_the_uploaded_flag_and_verifies_delivery_state(self) -> None:
        """必须发 `uploaded: true` 的提交，且提交后回读交付状态。

        ## 为什么这一步不能省

        PUT 全部返回 2xx 之后，资源状态仍是 `AWAITING_UPLOAD` —— 实测等 45 秒
        也不变，所以不是异步生效，而是**确实差一次提交**。不清掉，App Store
        上会留下一排坏掉的截图位，而本地一切看起来都正常。

        ## 属性名是从 409 里挖出来的

        两种直觉写法都被挡回：

            ENTITY_ERROR.ATTRIBUTE.NOT_ALLOWED
            The attribute 'assetToken' can not be included in a 'UPDATE' operation

            ENTITY_ERROR.ATTRIBUTE.INVALID   （发空 attributes 的 UPDATE）
            'Uploaded flag is not set!'   pointer: /data/attributes/uploaded
        """
        self.assertTrue(
            self._has('"uploaded": True'),
            "提交属性是 uploaded: true（字段名来自 409，不是文档）",
        )
        self.assertFalse(
            self._has('"assetToken"'),
            "assetToken 不允许出现在 UPDATE 里",
        )
        self.assertTrue(
            self._has("UPLOAD_COMPLETE"),
            "提交后必须校验交付状态到达 UPLOAD_COMPLETE / COMPLETE",
        )
        self.assertTrue(
            self._has('"COMPLETE"'),
            "COMPLETE 也是成功态：提交响应里是 UPLOAD_COMPLETE，稍后再查变 COMPLETE，"
            "只认一个会在另一种时序下误报失败 —— 而误报失败会诱发重跑、重跑塞重复图",
        )
        self.assertTrue(
            self._has("assetDeliveryState"),
            "完成与否应读服务端状态，而不是自己宣布成功",
        )

    def test_every_shot_goes_into_that_one_set_in_sorted_order(self) -> None:
        self.assertTrue(
            self._has('shots = sorted(p for p in directory.glob("*.png"))'),
            "展示顺序 = 文件名 ASCII 序，截屏脚本靠 01-04 前缀控制",
        )
        self.assertFalse(
            self._has("[index::"),
            "不应再有把图交错拆进多个 set 的切片",
        )

    def test_it_sends_no_commit_patch_but_reads_back_the_delivery_state(self) -> None:
        """不许发那个 commit PATCH，但必须回读交付状态。

        把 assetToken / sourceFileChecksum 清成 null 的 PATCH 会被 ASC 挡回：

            ENTITY_ERROR.ATTRIBUTE.NOT_ALLOWED
            The attribute 'assetToken' can not be included in a 'UPDATE' operation

        而去掉它之后，脚本唯一的「成功」依据就只剩 PUT 返回 200 —— 那只说明
        字节进了 Apple 的暂存区。所以必须回读 assetDeliveryState，让判据来自
        服务端而不是来自我们自己。
        """
        self.assertFalse(
            self._has('"assetToken"'),
            "assetToken 不允许出现在 UPDATE 里，这一步只会吃 409",
        )
        self.assertTrue(
            self._has("assetDeliveryState"),
            "上传后应回读服务端交付状态，而不是自己宣布成功",
        )


class MasCacheCardCopyTests(unittest.TestCase):
    """缓存页那张卡的标题/副标题，在 MAS 版不得承诺做不到的事。

    ## 为什么盯两行小字

    截图抓完 Docker 那一节后，副标题还留着「NPM / **Docker** / Xcode /
    **Homebrew** / Cargo」。隐藏了分区却在标题里继续点名它，是最容易被
    漏审的一种不一致 —— 审核看的是文字与截图整体，点不点得到分区反而
    不一定查。而这两行恰恰是这一页最醒目、最会被读到的文案。

    实情是：Docker 要 `docker` CLI + socket，Homebrew 要 `brew`，沙箱里
    都拿不到；而 npm / Cargo 只是目录，经授权就能读，**可以**列进去。
    所以 MAS 版该列的是授权清单里真实存在的那六项。

    判据也要求这两个 key 真的被 i18n 覆盖到 —— 用直接字面量 `t("cache.titleMas")`
    而不是三元里塞变量，否则静态扫描扫不到，门禁就形同虚设。
    """

    VIEW = ROOT / "src/views/CacheView.tsx"

    def test_the_card_picks_a_flavor_specific_title_and_subtitle(self) -> None:
        code = strip_tsx_comments(self.VIEW.read_text(encoding="utf-8"))
        for key in ("cache.titleMas", "cache.subtitleMas"):
            self.assertIn(f't("{key}")', code, f"{key} 应以字面量形式被调用，才能被 i18n 门禁扫到")
        self.assertIn(
            'can("dockerCleanup") ? t("cache.title") : t("cache.titleMas")',
            code.replace("\n", " ").replace("  ", " "),
            "标题应按 dockerCleanup 能力二选一，而不是无条件用完整版文案",
        )

    def test_the_mas_copy_does_not_name_docker_or_homebrew(self) -> None:
        for name in ("zh-CN.ts", "en.ts"):
            source = (ROOT / "src/i18n" / name).read_text(encoding="utf-8")
            for key in ("titleMas", "subtitleMas"):
                line = _dict_value_line(source, key)
                self.assertIsNotNone(line, f"{name} 缺少 cache.{key}")
                for forbidden in ("Docker", "Homebrew"):
                    self.assertNotIn(
                        forbidden,
                        line,
                        f"{name} 的 cache.{key} 提到了 {forbidden} —— "
                        "MAS 版沙箱里这两项做不到",
                    )

    def test_both_dictionaries_still_define_the_mas_copy(self) -> None:
        for name in ("zh-CN.ts", "en.ts"):
            keys = _flatten_dict(load_ts_dict(ROOT / "src/i18n" / name))
            for key in ("cache.titleMas", "cache.subtitleMas"):
                self.assertIn(key, keys, f"{name} 缺 {key}")


def _dict_value_line(source: str, key: str) -> str | None:
    """取 `key: "..."` 那一行的值部分。

    只取那一行而不是整份词典：断言失败时把几百行 i18n 全 dump 出来，
    等于用噪声淹没真正的问题。
    """
    for line in strip_tsx_comments(source).splitlines():
        if line.strip().startswith(f"{key}:"):
            return line
    return None


class I18nKeyAvailabilityTests(unittest.TestCase):
    """组件里 `t("…")` 用到的 key，中英两份词典里都必须能查到。

    ## 为什么需要这条

    抓截屏时发现进程页底部直接显示着 `process.masTerminateUnsupported`
    —— 原始 key 印在界面上。查下去是**命名空间错位**：那条文案被写在
    `scan:` 下面，而组件按 `process.` 去取。

    这类问题极难靠常规手段发现：

    - TypeScript 不报错（`t` 收的是任意 string）
    - 组件测试也抓不到 —— 它们普遍 mock 了 `t: (key) => key`，于是
      「查不到 key」和「正常显示 key」在测试里长得一模一样
    - 词典的类型只约束自己那份，`tsc` 也过

    只有真机截图才看得见。而截图是要上架的东西。
    """

    SRC = ROOT / "src"
    PATTERN = re.compile(r"\bt(?:ext)?\(\s*\"([A-Za-z][A-Za-z0-9_.]+)\"")

    def setUp(self) -> None:
        self.zh = _flatten_dict(load_ts_dict(ROOT / "src/i18n/zh-CN.ts"))
        self.en = _flatten_dict(load_ts_dict(ROOT / "src/i18n/en.ts"))

    def _used_keys(self) -> dict[str, set[str]]:
        used: dict[str, set[str]] = {}
        for path in sorted(self.SRC.rglob("*.ts")) + sorted(self.SRC.rglob("*.tsx")):
            name = path.name
            if ".test." in name or name == "navItems.ts":
                continue
            text = path.read_text(encoding="utf-8")
            for match in self.PATTERN.finditer(text):
                used.setdefault(match.group(1), set()).add(str(path.relative_to(ROOT)))
        return used

    def test_the_scanner_finds_enough_keys_to_be_worth_running(self) -> None:
        self.assertGreater(len(self._used_keys()), 100)

    def test_every_used_key_exists_in_both_dictionaries(self) -> None:
        used = self._used_keys()
        missing = sorted(
            f"{key}  ({', '.join(sorted(files))})"
            for key, files in used.items()
            if key not in self.zh or key not in self.en
        )
        self.assertEqual(
            missing,
            [],
            "这些 key 在词典里查不到，界面会直接把 key 印出来：\n  " + "\n  ".join(missing),
        )

    def test_the_two_dictionaries_have_identical_key_sets(self) -> None:
        only_zh = sorted(self.zh - self.en)
        only_en = sorted(self.en - self.zh)
        message = (
            "中英词典 key 不一致 —— 缺哪边都会在某种语言下显示原始 key："
            f"\n  仅中文有：{only_zh}"
            f"\n  仅英文有：{only_en}"
        )
        self.assertEqual(only_zh + only_en, [], message)


def load_ts_dict(path: Path) -> dict:
    """把 `export const xx = { … };` 的字面量读成 dict。

    不引入 TS 运行时：这两个词典是纯字面量，用括号配平截出来再交给
    `json.loads`（尾逗号先去掉）。词典里若出现注释或函数，这个做法会失效
    —— 那时这条门禁会**明显报错**而不是悄悄放过，比悄悄放过好。
    """
    import json

    text = path.read_text(encoding="utf-8")
    start = text.index("=", text.index("export const")) + 1
    depth = 0
    end = None
    for index in range(start, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                end = index + 1
                break
    if end is None:
        raise AssertionError(f"{path.name} 里没找到配平的字典字面量")
    body = re.sub(r"//[^\n]*", "", text[start:end])
    body = re.sub(r"/\*.*?\*/", "", body, flags=re.S)
    body = re.sub(r",(\s*[}\]])", r"\1", body)
    # JS 允许 `appName:` 这种不带引号的 key，JSON 不允许 —— 补上。
    body = re.sub(
        r"([,{]\s*)([A-Za-z_$][A-Za-z0-9_$]*)\s*:", r'\1"\2":', body
    )
    return json.loads(body)


def _flatten_dict(value, prefix: str = "") -> set[str]:
    out: set[str] = set()
    if not isinstance(value, dict):
        return out
    for key, child in value.items():
        path = f"{prefix}.{key}" if prefix else key
        out.add(path)
        out |= _flatten_dict(child, path)
    return out
