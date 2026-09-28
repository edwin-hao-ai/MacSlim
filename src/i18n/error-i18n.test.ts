/**
 * 错误消息 i18n 的门禁（第 4 片）：`error.<code>` 词条 + **分类行为等价性**。
 *
 * ## 为什么要有一份「行为等价性」测试
 *
 * 改造前 `classifyOperationError` 拿**中文字符串**做分类：
 *
 * ```ts
 * const STALE_MARKERS = ["操作已使用或已失效", "快照不存在或已失效", …];
 * if (message.includes("请重新扫描")) → kind: "stale"
 * ```
 *
 * 也就是说**文案参与了控制流**。本片把错误升级为结构化 `code`（后端
 * `src-tauri/src/user_error.rs`），分类改成比 `code`。这件事做对做错的判据
 * 只有一个：**同一个错误，改造前后的分类结果必须完全一样**。
 *
 * 所以本文件把「旧的中文子串匹配逻辑」原样抄在这里当**参照物**，再用一张
 * 覆盖全部旧文案样本的对照表，逐条断言 `legacyClassify(old) ===
 * classifyByCode(code)`。任何一侧漂移（新增 code 却忘了它的旧分类、或者旧
 * 分类集合与 `STALE_CODES` 不一致）都会立刻红。
 *
 * 附带守三条：
 *
 * 1. 中英 `error.*` 词条集合逐条相等（漏翻 / 错翻）；
 * 2. 英文侧不含任何 CJK 字符（英文 Mac 用户看到中文即失败）；
 * 3. 同 key 两侧插值参数名完全一致（占位符改名 = 静默丢值）。
 */

import { describe, expect, it } from "vitest";
import userErrorSource from "../../src-tauri/src/user_error.rs?raw";
import operationCommandsSource from "../../src-tauri/src/operation_commands.rs?raw";
import {
  CLASSIFICATION_CODE_SETS,
  classifyOperationError,
  errorText,
  toTauriError,
  type OperationErrorKind,
  type TauriError,
} from "@/lib/tauri";
import { en } from "./en";
import { zhCN } from "./zh-CN";

function flatten(node: unknown, prefix: string): Record<string, string> {
  if (typeof node === "string") return prefix ? { [prefix]: node } : {};
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(node as Record<string, unknown>)) {
    Object.assign(out, flatten(value, prefix ? `${prefix}.${key}` : key));
  }
  return out;
}

const EN = flatten(en, "");
const ZH = flatten(zhCN, "");

// ========== 参照物：改造前的中文子串分类逻辑 ==========

/** 一字不改地抄自改造前的 `src/lib/tauri.ts`。 */
const LEGACY_STALE_MARKERS = [
  "操作已使用或已失效",
  "快照不存在或已失效",
  "操作所属快照已失效",
  "选择项不存在",
  "请重新扫描",
];

function legacyClassify(message: string): OperationErrorKind {
  if (message.includes("写入历史记录失败")) return "history";
  if (message.includes("受保护进程只能强制终止")) return "forceOnly";
  if (message.includes("白名单进程不能终止")) return "whitelisted";
  if (LEGACY_STALE_MARKERS.some((marker) => message.includes(marker))) {
    return "stale";
  }
  return "failed";
}

// ========== 对照表：每一条旧文案样本 → 现在的 code ==========

/**
 * `old` 必须是**改造前代码里真实存在的**错误消息（占位符已按真实运行填好），
 * `code` 必须是后端现在为它下发的 `ErrorCode` wire 名。
 *
 * 完整性由下面两条断言兜住：
 * - `STALE_CODES` 里的每一枚 code 都必须在本表出现（漏一条 = 少验了一个旧分类）；
 * - 本表每一行的 code 都必须在 `en` / `zh-CN` 里有词条（= 后端真的会发它）。
 */
const EQUIVALENCE: ReadonlyArray<{ old: string; code: string }> = [
  // ---- broker：快照 / 操作生命周期 ----
  { old: "快照不存在或已失效", code: "snapshot_stale" },
  { old: "快照类型与载荷不匹配", code: "snapshot_payload_mismatch" },
  {
    old: "残留快照必须使用专用注册接口",
    code: "snapshot_dedicated_entry_required",
  },
  {
    old: "已安装应用快照必须使用专用注册接口",
    code: "snapshot_dedicated_entry_required",
  },
  {
    old: "Docker 快照必须使用专用注册接口",
    code: "snapshot_dedicated_entry_required",
  },
  { old: "操作已使用或已失效", code: "operation_used_or_expired" },
  { old: "操作所属快照已失效", code: "operation_snapshot_stale" },
  { old: "操作时间无效", code: "operation_time_invalid" },
  { old: "操作所有者不匹配", code: "operation_owner_mismatch" },
  { old: "操作所有者不能为空", code: "operation_owner_empty" },
  { old: "操作缺少所属快照", code: "operation_missing_snapshot" },
  {
    old: "操作计划与请求的 operation_id 不匹配",
    code: "operation_id_mismatch",
  },
  { old: "操作锁已失效", code: "operation_lock_broken" },
  { old: "选择不能为空", code: "selection_empty" },
  { old: "选择项不能重复", code: "selection_duplicated" },
  { old: "选择项不存在", code: "selection_missing" },
  { old: "应用选择项不存在", code: "selection_missing" },
  { old: "生成随机 ID 失败", code: "random_id_failed" },
  { old: "生成选择项随机 key 失败", code: "selection_key_generation_failed" },
  { old: "操作计划不是缓存计划", code: "plan_kind_mismatch" },
  { old: "操作计划不是进程计划", code: "plan_kind_mismatch" },
  { old: "操作计划不是应用终止计划", code: "plan_kind_mismatch" },
  { old: "操作计划不是卸载计划", code: "plan_kind_mismatch" },
  { old: "操作计划不是 Docker 计划", code: "plan_kind_mismatch" },
  { old: "进程操作必须通过阻塞执行通道", code: "process_plan_requires_blocking" },
  {
    old: "阻塞执行通道只接受进程与应用终止计划",
    code: "blocking_channel_plan_mismatch",
  },
  {
    old: "操作已执行，但写入历史记录失败（缓存清理 · 2 项缓存）：历史数据库不可写，请到历史页核对",
    code: "history_write_failed",
  },
  {
    old: "本地存储不可用，无法写入历史记录（cache_clean · 2 项缓存）：操作结果未被审计",
    code: "history_storage_unavailable",
  },
  { old: "操作结果类型与缓存清理不一致", code: "result_not_cache_summary" },
  { old: "操作结果类型与进程终止不一致", code: "result_not_kill_report" },

  // ---- 进程终止 ----
  { old: "白名单进程不能终止", code: "whitelisted_process_cannot_terminate" },
  { old: "受保护进程只能强制终止", code: "protected_force_only" },
  { old: "白名单进程不会被退出", code: "whitelisted_app_not_quit" },
  {
    old: "目标应用没有可退出的进程，请重新扫描",
    code: "app_no_quittable_process",
  },
  { old: "没有可终止的进程目标", code: "no_terminable_process_targets" },
  { old: "没有可退出的应用目标", code: "no_quittable_app_targets" },
  { old: "没有可卸载的应用目标", code: "no_uninstallable_app_targets" },
  { old: "进程已不存在（PID 11），请重新扫描", code: "process_gone" },
  { old: "PID 11 已被其他进程复用，请重新扫描", code: "process_pid_reused" },
  { old: "进程身份已变化（PID 11），请重新扫描", code: "process_identity_changed" },
  {
    old: "进程保护状态已变化（PID 11），请重新扫描",
    code: "process_protection_changed",
  },
  { old: "进程操作执行失败: task panicked", code: "process_execution_failed" },
  { old: "已终止", code: "kill_terminated" },
  { old: "进程已不存在", code: "kill_already_gone" },
  {
    old: "权限不足（通常是 root 或系统进程，MacSlim 不应该看到这类进程）",
    code: "kill_permission_denied",
  },
  {
    old: "原进程已终止，但一个 supervisor 立刻以新 PID 22 重启了 `node`。请从上游启动器（launchd agent / pm2 / nvm / Cursor / VS Code 等）停止，或把此进程名加入白名单屏蔽显示。",
    code: "kill_respawned",
  },
  {
    old: "SIGKILL 已发送，但系统报告进程仍存活。可能是僵死进程或受内核保护。",
    code: "kill_still_alive",
  },
  { old: "失败: EPERM", code: "kill_failed" },
  { old: "失败: SIGTERM 发送失败: EPERM", code: "kill_failed" },

  // ---- 应用 / 残留 / 卸载 ----
  { old: "应用 Notes 已不存在，请重新扫描", code: "app_gone" },
  {
    old: "应用 Notes 的 bundle ID 已变化，请重新扫描",
    code: "app_bundle_id_changed",
  },
  { old: "应用 Notes 的名称已变化，请重新扫描", code: "app_name_changed" },
  {
    old: "应用 Notes 的子进程 node 不属于该应用，请重新扫描",
    code: "app_child_not_in_app",
  },
  {
    old: "应用 Notes 没有可终止的进程，请重新扫描",
    code: "app_no_terminable_process",
  },
  {
    old: "子进程选择 key 数量与快照不一致",
    code: "app_child_selection_mismatch",
  },
  { old: "子进程选择 key 不属于该应用", code: "app_child_selection_mismatch" },
  {
    old: "应用选择 key 数量与快照不一致",
    code: "app_selection_count_mismatch",
  },
  { old: "同一应用被重复选择", code: "app_selected_twice" },
  { old: "同名应用无法精确定位，请分开卸载", code: "app_ambiguous_name" },
  { old: "系统核心应用不能卸载", code: "system_app_cannot_uninstall" },
  { old: "残留批次不能为空", code: "residue_batch_empty" },
  { old: "残留快照不能为空", code: "residue_snapshot_empty" },
  { old: "残留批次缺少应用选择项", code: "residue_batch_missing_app_key" },
  { old: "残留快照引用了未知应用选择项", code: "residue_unknown_app_key" },
  {
    old: "残留选择 key 数量与快照不一致",
    code: "residue_selection_count_mismatch",
  },
  { old: "残留选择项不属于所选应用", code: "residue_not_in_selected_app" },
  {
    old: "残留 /Library/…/Cache 不属于该应用，请重新扫描",
    code: "residue_not_in_app",
  },
  { old: "残留路径重复", code: "residue_path_duplicated" },
  { old: "残留路径重复：/Library/…/Cache", code: "residue_path_duplicated" },
  {
    old: "残留复核结果数量不一致，请重新扫描",
    code: "residue_recheck_count_mismatch",
  },
  {
    old: "残留 /Library/…/Cache 已不存在，请重新扫描",
    code: "residue_gone",
  },
  {
    old: "残留 /Library/…/Cache 路径已变化，请重新扫描",
    code: "residue_path_changed",
  },
  {
    old: "残留路径 /Library/…/Cache 不在允许的清理范围内，已拒绝",
    code: "residue_path_out_of_scope",
  },
  { old: "扫描残留失败: task panicked", code: "residue_scan_failed" },
  { old: "文件不存在", code: "move_to_trash_failed" },
  { old: "移动失败: No such file", code: "move_to_trash_failed" },
  { old: "授权移动失败: User canceled", code: "move_to_trash_failed" },
  { old: "用户取消授权", code: "authorization_cancelled" },
  { old: "启动 osascript 失败: ETIMEDOUT", code: "move_to_trash_failed" },
  { old: "无法获取用户主目录", code: "move_to_trash_failed" },

  // ---- 缓存清理 ----
  { old: "缓存选择 key 数量与快照不一致", code: "cache_selection_count_mismatch" },
  {
    old: "检测到 Xcode 正在运行，已跳过清理以防止损坏",
    code: "cache_busy_app_skipped",
  },
  { old: "Docker 缓存操作类型不匹配", code: "cache_docker_action_mismatch" },
  { old: "缓存项没有清理路径", code: "cache_item_missing_path" },
  { old: "路径无法访问: No such file", code: "cache_item_missing_path" },
  { old: "拒绝清理包含符号链接祖先的路径", code: "refuse_symlink_ancestor" },
  { old: "拒绝清理符号链接", code: "refuse_symlink" },
  { old: "无法检查路径类型: NotADirectory", code: "refuse_symlink" },
  { old: "stale node_modules 路径不是目录", code: "stale_path_not_dir" },
  { old: "stale 缓存项缺少 canonical path 快照", code: "stale_missing_canonical" },
  { old: "stale 路径已变化", code: "stale_path_changed" },
  { old: "stale node_modules 路径类型无效", code: "stale_path_kind_invalid" },
  { old: "stale 路径必须指向 node_modules", code: "stale_path_not_node_modules" },
  { old: "stale 路径缺少父目录", code: "stale_path_missing_parent" },
  { old: "stale 路径不在允许的项目目录内", code: "stale_path_outside_projects" },
  { old: "stale 缓存项缺少所有权快照", code: "stale_missing_ownership" },
  { old: "缓存项所有权已变化", code: "cache_ownership_changed" },
  { old: "无法检查路径祖先: EACCES", code: "cache_ancestor_check_failed" },
  { old: "无法读取路径所有权: EACCES", code: "cache_ancestor_check_failed" },
  { old: "无法检查路径目录: EACCES", code: "cache_ancestor_check_failed" },
  { old: "路径不在白名单内，拒绝删除: /etc/hosts", code: "path_not_whitelisted" },
  {
    old: "执行前二次校验失败，拒绝删除: /etc/hosts",
    code: "pre_delete_recheck_failed",
  },
  { old: "删除失败: Directory not empty", code: "delete_failed" },
  { old: "任务失败: task cancelled", code: "delete_failed" },
  { old: "用户取消了授权", code: "authorization_cancelled" },
  { old: "需要管理员权限才能删除此目录: (-128)", code: "admin_privileges_required" },
  { old: "osascript 启动失败: ETIMEDOUT", code: "admin_privileges_required" },
  { old: "启动失败 (/bin/zsh): ETIMEDOUT", code: "cache_command_failed" },
  { old: "命令退出码 Some(1)", code: "cache_command_exit_nonzero" },
  { old: "`python3 -m pip cache purge` 失败: boom", code: "cache_pip_broken" },
  {
    old: "Pip 环境损坏（shebang 指向已卸载的 Python）：boom。\n请手动执行 `python3 -m pip cache purge`，或重新安装 pip。",
    code: "cache_pip_broken",
  },

  // ---- Docker ----
  { old: "Docker 未运行，无法执行操作，请先启动 Docker", code: "docker_not_running" },
  { old: "没有可删除的 Docker 资源", code: "no_docker_targets" },
  {
    old: "Docker 资源清单已变化，请重新扫描后再清理",
    code: "docker_inventory_changed",
  },
  {
    old: "Docker 镜像 nginx:latest 已不存在，请重新扫描",
    code: "docker_resource_gone",
  },
  {
    old: "Docker 镜像 ID abc123 已被其他资源占用，请重新扫描",
    code: "docker_id_reused",
  },
  {
    old: "Docker 镜像 nginx:latest 引用状态已变化，请重新扫描",
    code: "docker_referenced_changed",
  },
  {
    old: "Docker 选择 key 数量与快照不一致",
    code: "docker_selection_count_mismatch",
  },
  {
    old: "Docker 镜像 选择 key 数量与快照不一致",
    code: "docker_selection_count_mismatch",
  },
  { old: "Docker prune 不接受资源目标", code: "docker_prune_rejects_target" },
  {
    old: "Docker prune 不接受资源选择项",
    code: "docker_prune_rejects_selection",
  },
  { old: "Docker 选择项类型不匹配", code: "docker_selection_type_mismatch" },
  { old: "Docker 操作类型错误", code: "docker_selection_type_mismatch" },
  { old: "启动 docker 失败: ETIMEDOUT", code: "docker_cli_failed" },
  { old: "Cannot connect to the Docker daemon", code: "docker_cli_failed" },
];

/** 后端 `ErrorCode::ALL` 里的 wire 名（从 Rust 源码抽取）。 */
function backendCodeSet(): Set<string> {
  const all = userErrorSource.match(
    /pub const ALL: &.static \[ErrorCode\] = &\[(.*?)\];/s,
  );
  expect(all, "user_error.rs 里必须还有 ErrorCode::ALL 清单").not.toBeNull();
  const names = [...(all as RegExpMatchArray)[1].matchAll(/Self::(\w+)/g)];
  expect(names.length, "ALL 至少要有一项（抽取正则失效会让本门禁空转）")
    .toBeGreaterThan(50);
  const wire = new Map<string, string>();
  for (const m of userErrorSource.matchAll(
    /pub const (\w+): Self =\s*Self\x28\x22([a-z0-9_]+)\x22\x29;/g,
  )) {
    wire.set(m[1], m[2]);
  }
  return new Set(names.map((m) => wire.get(m[1]) ?? `??${m[1]}`));
}

describe("错误分类：改造前后的行为等价性", () => {
  it("对照表本身非空（抽取/维护失效时本组断言会空转成全过）", () => {
    expect(EQUIVALENCE.length).toBeGreaterThan(100);
  });

  it("每条旧文案样本的分类结果与改造前完全相同", () => {
    const mismatches: string[] = [];
    for (const { old, code } of EQUIVALENCE) {
      const before = legacyClassify(old);
      const after = classifyOperationError({ code, message: old, params: [] })
        .kind;
      if (before !== after) {
        mismatches.push(`${old} → ${code}: 旧=${before} 新=${after}`);
      }
    }
    expect(mismatches).toEqual([]);
  });

  it("STALE_CODES 里的每一枚 code 都在对照表里出现过", () => {
    // 反向门禁：有人往 STALE_CODES 里塞一枚 code 却没有对应的旧文案样本，
    // 那枚 code 的分类就等于没被验证过。
    const covered = new Set(
      EQUIVALENCE.filter(
        (row) => legacyClassify(row.old) === "stale",
      ).map((row) => row.code),
    );
    const missing = [...CLASSIFICATION_CODE_SETS.stale].filter(
      (code) => !covered.has(code),
    );
    expect(missing).toEqual([]);
  });

  it("四类分类 code 集合互不重叠（重叠会让上面的逐条断言失去意义）", () => {
    const sets = CLASSIFICATION_CODE_SETS;
    const names = ["stale", "history", "forceOnly", "whitelisted"] as const;
    for (const a of names) {
      for (const b of names) {
        if (a >= b) continue;
        for (const code of sets[a]) {
          expect(sets[b].has(code), `${code} 同时属于 ${a} 和 ${b}`).toBe(
            false,
          );
        }
      }
    }
  });

  it("分类只认 code：同样的 code 配任意语言的消息都给出同样分类", () => {
    for (const code of CLASSIFICATION_CODE_SETS.stale) {
      expect(
        classifyOperationError({ code, message: "totally unrelated", params: [] })
          .kind,
      ).toBe("stale");
    }
    expect(
      classifyOperationError({
        code: "protected_force_only",
        message: "whatever",
        params: [],
      }).kind,
    ).toBe("forceOnly");
  });

  it("反向门禁：中文消息里带旧标记、但 code 不在那批里的一律归 failed", () => {
    // 这条是「有人又把中文匹配加回来」的专测：只要 `classifyOperationError`
    // 恢复成匹配 `message.includes("请重新扫描")`，这里立刻红。
    const smuggled: TauriError = {
      code: "internal",
      message: "进程已不存在（PID 11），请重新扫描",
      params: [],
    };
    expect(classifyOperationError(smuggled).kind).toBe("failed");
    expect(
      classifyOperationError({
        code: "internal",
        message: "写入历史记录失败（x · y）：z",
        params: [],
      }).kind,
    ).toBe("failed");
  });

  it("非后端来源的错误落成 internal，并保留原始文本", () => {
    for (const thrown of [
      new Error("Docker 未运行"),
      "plain string",
      null,
      undefined,
      42,
      { code: 1 },
    ]) {
      const info = toTauriError(thrown);
      expect(info.code).toBe("internal");
      expect(classifyOperationError(thrown).kind).toBe("failed");
    }
    expect(toTauriError(new Error("boom")).message).toBe("boom");
  });

  it("errorText 查不到词条时回落到中文兜底，绝不显示裸 code", () => {
    // 假词典：任何 key 都原样返回（等价于「词典里没有这一条」）
    const passthrough = (key: string) => key;
    expect(
      errorText({ code: "totally_made_up", message: "兜底中文", params: [] }, passthrough),
    ).toBe("兜底中文");
    expect(
      errorText(
        { code: "internal", message: "internal 兜底", params: [] },
        passthrough,
      ),
    ).toBe("internal 兜底");
  });

  it("errorText 命中词条时用译文（并把 params 传下去）", () => {
    const seen: Array<{ key: string; params: unknown }> = [];
    const tText = (key: string, params?: ReadonlyArray<readonly [string, string]>) => {
      seen.push({ key, params });
      return key === "error.snapshot_stale" ? "Snapshot expired" : key;
    };
    const text = errorText(
      { code: "snapshot_stale", message: "快照不存在或已失效", params: [["pid", "11"]] },
      tText,
    );
    expect(text).toBe("Snapshot expired");
    expect(seen[0].key).toBe("error.snapshot_stale");
    expect(seen[0].params).toEqual([["pid", "11"]]);
  });
});

describe("error.* 词条", () => {
  const backend = backendCodeSet();

  it("后端每一枚 code 在中英两侧都有词条", () => {
    const missing: string[] = [];
    for (const code of backend) {
      const key = `error.${code}`;
      if (!(key in EN)) missing.push(`en 缺少 ${key}`);
      if (!(key in ZH)) missing.push(`zh-CN 缺少 ${key}`);
    }
    expect(missing).toEqual([]);
  });

  it("中英两侧的 error 命名空间 key 集合完全相等", () => {
    const enKeys = Object.keys(EN).filter((k) => k.startsWith("error."));
    const zhKeys = Object.keys(ZH).filter((k) => k.startsWith("error."));
    expect(enKeys.sort()).toEqual(zhKeys.sort());
  });

  it("不存在「已翻译但后端从不发出」的孤儿词条", () => {
    const orphans = Object.keys(EN)
      .filter((k) => k.startsWith("error."))
      .filter((k) => !backend.has(k.slice("error.".length)));
    expect(orphans).toEqual([]);
  });

  it("英文侧不含任何 CJK 字符（英文用户绝对不能看到中文）", () => {
    const offenders = Object.entries(EN)
      .filter(([key]) => key.startsWith("error."))
      .filter(([, value]) => /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u.test(value))
      .map(([key]) => key);
    expect(offenders).toEqual([]);
  });

  it("中文侧的 error 词条必须是中文（防止 en 侧被整段复制到 zh 侧）", () => {
    const offenders = Object.entries(ZH)
      .filter(([key]) => key.startsWith("error."))
      .filter(([, value]) => !/[\p{Script=Han}]/u.test(value))
      .map(([key]) => key);
    expect(offenders).toEqual([]);
  });

  it("同 key 两侧插值参数名完全一致", () => {
    const placeholders = (text: string) =>
      [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
    const mismatched: string[] = [];
    for (const key of Object.keys(EN).filter((k) => k.startsWith("error."))) {
      const a = placeholders(EN[key]);
      const b = placeholders(ZH[key]);
      if (JSON.stringify(a) !== JSON.stringify(b)) {
        mismatched.push(`${key}: en=${a} zh=${b}`);
      }
    }
    expect(mismatched).toEqual([]);
  });

  it("英文译文里的花括号都是合法占位符（抓漏转义与占位符当括号）", () => {
    const bad: string[] = [];
    for (const key of Object.keys(EN).filter((k) => k.startsWith("error."))) {
      const value = EN[key];
      for (const m of value.matchAll(/\{([^}]*)\}/g)) {
        if (!/^\w+$/.test(m[1])) bad.push(`${key}: {${m[1]}}`);
      }
      const opens = (value.match(/\{/g) ?? []).length;
      const closes = (value.match(/\}/g) ?? []).length;
      if (opens !== closes) bad.push(`${key}: 花括号不配对`);
    }
    expect(bad).toEqual([]);
  });

  it("后端源码层：没有把中文塞进 code 位（code 必须纯 ASCII snake_case）", () => {
    // `ErrorCode` 走的是「私有字段 + 关联常量」，所以构造点写的是常量名而不是
    // 字符串字面量。这里的门禁是**反向**的：确认 `ALL` 与常量定义都还是
    // 纯 ASCII，防止有人图省事把 code 改成 `Self("快照失效")`。
    const literals = [
      ...userErrorSource.matchAll(/Self\(\s*\x22([^\x22]*)\x22\s*,?\s*\x29/g),
    ].map((m) => m[1]);
    expect(literals.length).toBeGreaterThan(50);
    for (const literal of literals) {
      expect(literal, `code ${literal} 含非 ASCII 字符`).toMatch(
        /^[a-z][a-z0-9_]*$/,
      );
    }
  });

  it("后端源码层：history 消息仍然由 code 承载，不靠文案", () => {
    // 双重失败（操作失败 + 历史写不进去）是唯一带「原错误前缀」的错误，
    // 它的文案里必然含前一条错误的中文 —— 所以**绝不能**再用
    // `includes("写入历史记录失败")` 去认它。这里钉住 code 出现在构造点上。
    expect(operationCommandsSource).toContain("ErrorCode::HISTORY_WRITE_FAILED");
  });
});
