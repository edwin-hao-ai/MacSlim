# MacSlim Updater Artifact Integrity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 确保 updater manifest 中的版本、target、签名资产和实际 `.app.tar.gz` 内嵌 bundle 身份完全一致，拒绝陈旧或错误版本的重命名 tarball。

**Architecture:** 新增纯 Python 标准库工具读取 Tauri 配置与 gzip tar 内的唯一 `MacSlim.app/Contents/Info.plist`，校验 identifier/version、目录/符号链接结构与 Mach-O 架构并计算 SHA-256。release/sign 在公证、staple、Gatekeeper 全部成功后从当前 `.app` 重建 archive，并由唯一公开写入口 `rebuild --app` 内部生成 build stamp；publish-update 在 staging 中验证所有同版本资产、逐资产重签并生成 manifest，最后通过 `publish_assets.py` 事务提交。测试使用真实临时 tarball 与 Mach-O fixture，不执行真实发布。

**Tech Stack:** Python 3 标准库（`tarfile`、`plistlib`、`hashlib`、`json`）、Bash、Tauri updater artifacts、Vitest 无关的 Python unittest。

**Spec:** `docs/superpowers/specs/2026-09-24-macslim-product-hardening-app-store-design.md`

## Global Constraints

- 不读取、打印或写入真实 updater 私钥/密码；测试只用临时 sentinel。
- 不执行真实 signer、release、签名、公证、上传或 commit/add/push。
- 不新增代码注释。
- `publish-update.sh` 的版本参数必须与 `src-tauri/tauri.conf.json` 一致。
- `.app.tar.gz` 必须包含且只包含一个安全的 `MacSlim.app` 根、唯一可验证的 `Contents/Info.plist` 与目标架构 Mach-O executable；所有路径/祖先/symlink 必须安全。
- manifest 版本、asset basename、archive 内 `CFBundleShortVersionString`、bundle identifier 与 build stamp 必须一致。
- `MACSlim_PUBLISH_PREFLIGHT_ONLY=1` 必须保持无复制、无签名、无 manifest 副作用。
- 当前发布内容继续使用稳定 `updates/latest.json` 与旧客户端 `0.1.0/0.2.2` 兼容 manifest。

---

### Task 1: 实现 archive inspector 与 build stamp

**Files:**
- Create: `scripts/updater_artifact.py`
- Create: `scripts/tests/test_updater_artifact.py`

**Interfaces:**
- Consumes: `src-tauri/tauri.conf.json`、gzip tar archive、target triple。
- Produces: `inspect_archive(archive: Path, expected_version: str, expected_identifier: str, expected_target: str | None = None) -> dict`、`stamp_path`、`rebuild_archive(app_path: Path, archive: Path, target: str) -> Path`、`verify_stamp(archive: Path, target: str) -> None`，CLI `rebuild|verify`。`write_stamp` 不公开；build stamp 只能由 `rebuild --app` 内部生成。

- [ ] **Step 1: 写真实 tarball 失败测试**

测试 helper 必须创建合法 gzip tar：

```python
def make_archive(path: pathlib.Path, version: str = "0.2.2", identifier: str = "com.macslim.desktop") -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "CFBundleIdentifier": identifier,
        "CFBundleShortVersionString": version,
        "CFBundleVersion": version,
    }
    info = plistlib.dumps(payload)
    with tarfile.open(path, "w:gz") as archive:
        data = info.encode("utf-8")
        item = tarfile.TarInfo("MacSlim.app/Contents/Info.plist")
        item.size = len(data)
        archive.addfile(item, io.BytesIO(data))
```

覆盖：合法 bundle、wrong version、wrong identifier、0/2 个 Info.plist、symlink Info.plist、tamper after stamp、wrong target、缺 stamp。

- [ ] **Step 2: 运行测试确认 RED**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.tests.test_updater_artifact -v
```

Expected: ERROR，因为 `scripts/updater_artifact.py` 尚不存在。

- [ ] **Step 3: 实现严格 archive inspector**

核心规则：

```python
INFO_RE = re.compile(r"^[^/]+\.app/Contents/Info\.plist$")

members = [member for member in archive.getmembers() if member.isfile() and INFO_RE.fullmatch(member.name)]
if len(members) != 1:
    raise ValueError("updater archive 必须包含且只包含一个 app Info.plist")
```

读取后校验 `CFBundleIdentifier`、`CFBundleShortVersionString`、非空 `CFBundleVersion`；SHA-256 必须流式读取 archive。stamp 路径固定为：

```python
def stamp_path(archive: pathlib.Path) -> pathlib.Path:
    return pathlib.Path(f"{archive}.build-stamp.json")
```

stamp 至少包含：`schema_version`、`product`、`version`、`target`、`identifier`、`bundle_version`、`archive_sha256`、`created_at`。

- [ ] **Step 4: 实现 rebuild/verify CLI**

CLI：

```text
python3 scripts/updater_artifact.py rebuild --app <MacSlim.app> --archive <path> --target <triple>
python3 scripts/updater_artifact.py verify --archive <path> --target <triple> [--stamp <path>]
```

不提供公开 `stamp` CLI/API；错误必须是中文，不输出 archive 内容、签名或凭据。

- [ ] **Step 5: 运行测试确认 GREEN**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.tests.test_updater_artifact -v
```

Expected: 全部通过，测试后不存在 `__pycache__`。

---

### Task 2: 在 release/sign 重建 archive，在 publish 前验证

**Files:**
- Modify: `scripts/release.sh`
- Modify: `scripts/sign.sh`
- Modify: `scripts/publish-update.sh`
- Modify: `scripts/verify-security-config.py`
- Create: `scripts/publish_assets.py`
- Modify: `scripts/tests/test_verify_security_config.py`
- Create: `scripts/tests/test_publish_assets.py`
- Create: `scripts/tests/test_publish_update_integrity.py`
- Create: `scripts/tests/test_release_contract.py`

**Interfaces:**
- Consumes: Task 1 的 `rebuild|verify` CLI。
- Produces: post-notarization archive 与内部 build stamp；所有同版本资产 sidecar；staging manifest；`publish_assets.py commit` 的原子提交/回滚结果。

- [ ] **Step 1: 写 updater identity 失败测试**

把现有测试的 `PUBLISH_VERSION = "9.9.9"` 改为从 `tauri.conf.json` 读取的 `0.2.2`。更新 fake archive 为 Task 1 的真实 tar helper，并先生成 stamp。

新增测试：

- `publish_update_rejects_config_version_mismatch`
- `publish_update_rejects_missing_stamp`
- `publish_update_rejects_wrong_bundle_version`
- `publish_update_rejects_tampered_archive`
- `publish_update_rejects_wrong_target`
- `release_and_sign_require_post_notarization_stamp`

- [ ] **Step 2: 运行测试确认 RED**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.tests.test_verify_security_config -v
```

Expected: 新测试失败，旧 `9.9.9` fake tar 成功路径被移除。

- [ ] **Step 3: release/sign 从当前 app 重建 archive**

`release.sh` 与 `sign.sh` 在 `.app`、DMG 都完成 notarize/staple/Gatekeeper 后执行：

```bash
python3 scripts/updater_artifact.py rebuild \
  --app "$APP_PATH" \
  --archive "$UPDATER_PATH" \
  --target "$RUST_TARGET"
```

`UPDATER_PATH` 必须是同 target 下精确的 `bundle/macos/MacSlim.app.tar.gz`；流程开始先失效旧 archive、签名和 stamp，失败时不得留下可验证旧 stamp。

- [ ] **Step 4: publish-update 强制 config version、sidecar 与 staging**

在脚本开始处读取并严格比较 `tauri.conf.json` 的真实 `version`；在任何 preflight/staging 副作用前验证 source archive、target Mach-O 和内部 stamp。既有同版本资产必须逐一具备 sidecar 并通过对应 target/平台冲突检查。

正式发布先复制到 `mktemp` staging，逐资产 verify、重签并生成 latest 与 `0.1.0/0.2.2` compat manifest；最后执行：

```bash
python3 scripts/publish_assets.py commit \
  --staging-root "$STAGING_ROOT" \
  --live-root "." \
  --version "$VERSION"
```

`publish_assets.py` 使用同目录临时文件、fsync、`os.replace`、备份与回滚；失败不得留下半更新 live 状态。`MACSlim_PUBLISH_PREFLIGHT_ONLY=1` 只允许读取与校验，body 仅允许提示输出和 `exit 0`。

- [ ] **Step 5: 强化静态 validator**

validator 必须以真实 shell 命令、Markdown code block、Python AST 和 package token 校验：禁止公开 `stamp` API/CLI，锁定 config version 读取与 target 映射，验证 Mach-O/target/sidecar，要求 staging/preflight/manifest/commit 顺序与事务 helper 实际调用；注释、别名、吞错与死代码不得满足契约。

- [ ] **Step 6: 运行专项与全量测试**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts.tests.test_updater_artifact scripts.tests.test_verify_security_config -v
python3 scripts/verify-security-config.py
bash -n scripts/release.sh scripts/sign.sh scripts/publish-update.sh
```

Expected: 全部退出 0，无真实 signer 调用。

---

### Task 3: 文档与统一验证

**Files:**
- Modify: `scripts/README.md`
- Modify: `scripts/verify-security-config.py`
- Create: `scripts/release_docs_contract.py`
- Modify: `scripts/tests/test_verify_security_config.py`
- Create: `scripts/tests/test_task3_fixwave.py`

**Interfaces:**
- Consumes: Tasks 1–2。
- Produces: 可执行发布 SOP 与完整 `bun run verify` 证据。

- [ ] **Step 1: 更新发布文档**

明确：必须先成功执行 release/sign，由 `rebuild --app` 从当前已签名/公证 app 重建 archive 并生成内部 stamp，再运行 publish-update；版本参数必须等于 tauri config；stamp 不得手工编辑或跨 target 复用。README 的真实 bash 命令必须按 release/sign → rebuild → publish 顺序位于 Updater 门禁章节。

- [ ] **Step 2: 增加无缓存测试约束**

确认 `test:python` 继续包含 `PYTHONDONTWRITEBYTECODE=1`；运行后两个 `__pycache__` 路径都不存在。

- [ ] **Step 3: 运行完整验证**

Run:

```bash
bun run verify
git diff --check
```

Expected: 全部退出 0；不执行真实 updater signer、release 或上传。

- [ ] **Step 4: 原生发布前人工命令（不在自动测试中执行）**

正式发布时由操作者运行：

```bash
./scripts/release.sh arm
./scripts/publish-update.sh 0.2.2 "本次更新说明" arm
```

Expected: release/sign 公证成功后由 `rebuild --app` 生成与 target/version/hash/架构匹配的 archive；publish-update 在 staging 中重签并事务提交该 tarball。
