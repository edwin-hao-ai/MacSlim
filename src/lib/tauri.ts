import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppInfo,
  DockerInventory,
  FdaStatus,
  FolderTarget,
  HistoryEntry,
  I18nParams,
  InstalledApp,
  Keyed,
  OperationErrorInfo,
  OperationResult,
  PreparedOperation,
  ProcessRow,
  ResidueAppGroup,
  ScanResult,
  SnapshotResult,
  StageUpdate,
  SystemHealth,
  TauriError,
  WhitelistEntry,
} from "@/lib/ipcTypes";

/**
 * 数据契约类型统一在 `ipcTypes.ts` 里定义，这里原样回导。
 *
 * 仓库门禁规定**只有本文件可以调 `invoke`**，所以桥接函数必须留在这里；
 * 但纯类型定义不调 `invoke`，可以拆出去给 800 行硬上限腾余量。回导保证
 * 调用方 `import ... from "@/lib/tauri"` 的路径一个字都不用改。
 * 背景见 `ipcTypes.ts` 的模块注释。
 */
export type {
  AppChildProcess,
  AppGracefulQuitReport,
  AppInfo,
  AppResidue,
  CleanReport,
  CleanSummary,
  DockerBuilderCache,
  DockerContainer,
  DockerExecutionReport,
  DockerImage,
  DockerInventory,
  DockerVolume,
  FdaStatus,
  FolderTarget,
  HistoryEntry,
  I18nParams,
  InstalledApp,
  Keyed,
  MoveResult,
  OperationErrorInfo,
  OperationErrorKind,
  OperationResult,
  PreparedOperation,
  ProcessInfo,
  ProcessKillDetail,
  ProcessKillReport,
  ProcessKind,
  ProcessRow,
  ResidueAppGroup,
  ResidueItem,
  ScanResult,
  SnapshotResult,
  StageUpdate,
  SystemHealth,
  TauriError,
  UninstallReport,
  WhitelistEntry,
} from "@/lib/ipcTypes";

export type { UnlistenFn };

// ========== 操作准备请求 ==========
//
// ⚠️ 这几个类型**刻意留在 `tauri.ts`，不搬去 `ipcTypes.ts`**：
// `scripts/operation_surface_checks.py` 的前端契约门禁只解析 `tauri.ts`
// 的源码文本来核对 `PrepareOperationRequest` 变体与 `CacheItem` 字段，
// 搬走会让门禁读到空集合而失败。它们不调 `invoke`，但契约校验依赖
// 它们出现在本文件里。

export type ProcessMode = "graceful" | "force";

export type DockerAction =
  | "remove_image"
  | "remove_container"
  | "remove_volume"
  | "prune";

export type PrepareOperationRequest =
  | { type: "cache"; snapshot_id: string; item_keys: string[] }
  | {
      type: "process";
      snapshot_id: string;
      process_keys: string[];
      mode: ProcessMode;
    }
  | {
      type: "app_terminate";
      snapshot_id: string;
      app_keys: string[];
      mode: ProcessMode;
    }
  | {
      type: "app_graceful_quit";
      snapshot_id: string;
      app_keys: string[];
    }
  | {
      type: "uninstall";
      app_snapshot_id: string;
      residue_snapshot_id: string;
      app_keys: string[];
      residue_keys: string[];
      quit_running: boolean;
    }
  | {
      type: "docker";
      snapshot_id: string;
      action: DockerAction;
      target_keys: string[];
    };

// ========== 缓存 ==========

export type CacheCategory =
  | "npm"
  | "pnpm"
  | "yarn"
  | "docker"
  | "homebrew"
  | "xcode"
  | "cocoapods"
  | "cargo"
  | "pip"
  | "go"
  | "system";

export type Safety = "safe" | "low" | "medium";

/**
 * 缓存项的可读文案由前端词典翻译：后端只发 i18n key + 插值参数。
 * 形状与 `ProcessInfo` / `PreparedOperation` 完全一致。
 */
export type CacheItem = {
  id: string;
  category: CacheCategory;
  label_key: string;
  label_params: I18nParams;
  description_key: string;
  description_params: I18nParams;
  path: string | null;
  size_bytes: number;
  safety: Safety;
  default_select: boolean;
  recover_hint: string;
};

export type CacheSnapshotView = {
  items: Keyed<CacheItem>[];
  total_bytes: number;
  scanned_at_ms: number;
};

/**
 * 「快照/操作已失效」类 code。
 *
 * ⚠️ 这份清单是**从旧的中文子串匹配逐条换算**过来的，不是重新挑的：
 * 改造前 `classifyOperationError` 拿 `STALE_MARKERS`（5 个中文子串）判断，
 * 凡是消息里含其中任一串的旧文案都归 `stale`。现在按 code 判断，所以
 * **集合必须逐条等价** —— `error-i18n.test.ts` 的
 * `每个旧文案样本的分类结果与改造前逐条相同` 就是这条约束的护栏。
 */
const STALE_CODES = new Set([
  "snapshot_stale",
  "operation_used_or_expired",
  "operation_snapshot_stale",
  "selection_missing",
  "app_no_quittable_process",
  "process_gone",
  "process_pid_reused",
  "process_identity_changed",
  "process_protection_changed",
  "app_no_terminable_process",
  "app_child_not_in_app",
  "residue_recheck_count_mismatch",
  "app_gone",
  "app_bundle_id_changed",
  "app_name_changed",
  "residue_not_in_app",
  "residue_gone",
  "residue_path_changed",
  "docker_inventory_changed",
  "docker_resource_gone",
  "docker_id_reused",
  "docker_referenced_changed",
]);

/** 旧文案「写入历史记录失败」→ `history`。 */
const HISTORY_CODES = new Set(["history_write_failed"]);

/** 旧文案「受保护进程只能强制终止」→ `forceOnly`。 */
const FORCE_ONLY_CODES = new Set(["protected_force_only"]);

/** 旧文案「白名单进程不能终止」→ `whitelisted`。 */
const WHITELISTED_CODES = new Set(["whitelisted_process_cannot_terminate"]);

const NO_CODES = new Set<string>();

/**
 * 认出后端的结构化错误。
 *
 * 非后端来源的错误（Tauri 自己的报错、`Error`、`null`…）一律落成
 * `internal` + 原始文本 —— `internal` **刻意不在 i18n 词典里**，
 * `errorText()` 会因此回落到 `message`，用户看到原始文本而不是裸 code。
 */
export function toTauriError(error: unknown): TauriError {
  if (
    typeof error === "object" &&
    error !== null &&
    typeof (error as TauriError).code === "string" &&
    typeof (error as TauriError).message === "string"
  ) {
    const raw = error as TauriError;
    return {
      code: raw.code,
      message: raw.message,
      params: Array.isArray(raw.params) ? raw.params : [],
    };
  }
  return {
    code: "internal",
    message: error instanceof Error ? error.message : String(error ?? ""),
    params: [],
  };
}

/**
 * 按 `code` 给错误分类 —— **不再匹配任何文案字符串**。
 *
 * 改造前这里是 `message.includes("请重新扫描")` 这类中文子串匹配，也就是
 * **文案参与了控制流**：文案一改，分类静默失效，所有错误退化成 `failed`。
 * 现在 `code` 与文案彻底分离，两者互不影响。
 */
export function classifyOperationError(error: unknown): OperationErrorInfo {
  const info = toTauriError(error);
  if (HISTORY_CODES.has(info.code)) {
    return { ...info, kind: "history" };
  }
  if (FORCE_ONLY_CODES.has(info.code)) {
    return { ...info, kind: "forceOnly" };
  }
  if (WHITELISTED_CODES.has(info.code)) {
    return { ...info, kind: "whitelisted" };
  }
  if (STALE_CODES.has(info.code)) {
    return { ...info, kind: "stale" };
  }
  return { ...info, kind: "failed" };
}

/** `tText` 的最小签名 —— 让 `tauri.ts` 不必依赖 i18n 上下文。 */
export type TextRenderer = (
  key: string,
  params?: ReadonlyArray<readonly [string, string]>,
) => string;

/**
 * 错误 / 结果消息的最终文案：`error.<code>` 查词典，查不到就回落到中文兜底。
 *
 * 必须自己判断「拿到的还是不是 key」：`i18n/index.tsx` 的 `get()` 找不到条目时
 * **静默返回 key 本身**，直接渲染就会让用户看到裸 `error.some_code`。
 */
export function errorText(info: TauriError, tText: TextRenderer): string {
  const key = `error.${info.code}`;
  const rendered = tText(key, info.params);
  return rendered === key ? info.message : rendered;
}

/** 保留空集合引用，让「四类 code 集合互不重叠」这条断言有个可比较的锚点。 */
export const CLASSIFICATION_CODE_SETS = {
  stale: STALE_CODES,
  history: HISTORY_CODES,
  forceOnly: FORCE_ONLY_CODES,
  whitelisted: WHITELISTED_CODES,
  failed: NO_CODES,
};

export function cacheOperation(
  snapshotId: string,
  itemKeys: string[],
): PrepareOperationRequest {
  return { type: "cache", snapshot_id: snapshotId, item_keys: itemKeys };
}

export function processOperation(
  snapshotId: string,
  processKeys: string[],
  mode: ProcessMode,
): PrepareOperationRequest {
  return {
    type: "process",
    snapshot_id: snapshotId,
    process_keys: processKeys,
    mode,
  };
}

export function appTerminateOperation(
  snapshotId: string,
  appKeys: string[],
  mode: ProcessMode,
): PrepareOperationRequest {
  return {
    type: "app_terminate",
    snapshot_id: snapshotId,
    app_keys: appKeys,
    mode,
  };
}

export function appGracefulQuitOperation(
  snapshotId: string,
  appKeys: string[],
): PrepareOperationRequest {
  return {
    type: "app_graceful_quit",
    snapshot_id: snapshotId,
    app_keys: appKeys,
  };
}

export function uninstallOperation(
  appSnapshotId: string,
  residueSnapshotId: string,
  appKeys: string[],
  residueKeys: string[],
  quitRunning: boolean,
): PrepareOperationRequest {
  return {
    type: "uninstall",
    app_snapshot_id: appSnapshotId,
    residue_snapshot_id: residueSnapshotId,
    app_keys: appKeys,
    residue_keys: residueKeys,
    quit_running: quitRunning,
  };
}

export function dockerOperation(
  snapshotId: string,
  action: DockerAction,
  targetKeys: string[],
): PrepareOperationRequest {
  return {
    type: "docker",
    snapshot_id: snapshotId,
    action,
    target_keys: targetKeys,
  };
}

export async function getSystemHealth(): Promise<SystemHealth> {
  return invoke("get_system_health");
}

/**
 * 当前构建形态：`"developer_id"` 或 `"mas"`。
 *
 * 后端 `flavor::CURRENT` 在**编译期**定死（`--features mas`），这里只是把它读
 * 出来。`src/lib/flavor.ts` 用它决定「终止进程」这类沙箱里做不到的入口要不要
 * 出现。
 */
export async function getBuildFlavor(): Promise<"developer_id" | "mas"> {
  return invoke("get_build_flavor");
}

/**
 * 探测「能不能读到用户数据」。
 *
 * 注意它现在**不是**单纯的 FDA 探测：后端拼路径用的是 passwd 里的真实
 * home，而不是 `$HOME` —— 沙箱会把 `$HOME` 指向 container，用它拼出来的
 * 路径落在那个空目录上，会把「读不到」误报成「已授权」。
 */
export async function getFdaStatus(): Promise<FdaStatus> {
  return invoke("get_fda_status");
}

/**
 * 打开「完全磁盘访问权限」设置页。
 *
 * 那个 URL scheme 是系统私有但稳定的（CleanMyMac 等工具也用它）；
 * 打不开时返回 false，由调用方退回「手动导航」的文案。
 */
export async function openFullDiskAccessSettings(): Promise<boolean> {
  try {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(
      "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
    );
    return true;
  } catch {
    return false;
  }
}

export async function scanAll(): Promise<SnapshotResult<ScanResult>> {
  return invoke("scan_all");
}

export async function prepareOperation(
  request: PrepareOperationRequest,
): Promise<PreparedOperation> {
  return invoke("prepare_operation", { request });
}

export async function executeOperation(
  operationId: string,
): Promise<OperationResult> {
  return invoke("execute_operation", { operationId });
}

// ========== 进程管理视图 ==========

export async function listAllProcesses(): Promise<SnapshotResult<ProcessRow[]>> {
  return invoke("list_all_processes");
}

// ========== 应用程序管理 ==========

export async function listApplications(): Promise<SnapshotResult<AppInfo[]>> {
  return invoke("list_applications");
}

// ========== Docker 深度视图 ==========

export async function dockerAvailable(): Promise<boolean> {
  return invoke("docker_available");
}

export async function dockerInventory(): Promise<SnapshotResult<DockerInventory>> {
  return invoke("docker_inventory");
}

// ========== Cache ==========

export async function scanCache(): Promise<SnapshotResult<CacheSnapshotView>> {
  return invoke("scan_cache");
}

// ========== 文件夹访问授权（App Store 版） ==========
//
// 三个函数刻意写得很短：本文件离 800 行硬上限只剩十几行余量，
// 而「只有 tauri.ts 能调 invoke」是仓库门禁，硬约束。
// 背景与取舍见 `ipcTypes.ts` 的模块注释。

export async function listFolderAccess(): Promise<FolderTarget[]> {
  return invoke("list_folder_access");
}

/** 弹系统文件选择框让用户授权。返回 false = 用户取消。 */
export async function grantFolderAccess(
  targetKey: string,
  prompt: string,
): Promise<boolean> {
  return invoke("grant_folder_access", { targetKey, prompt });
}

export async function revokeFolderAccess(targetKey: string): Promise<void> {
  return invoke("revoke_folder_access", { targetKey });
}

// ========== 扫描阶段进度事件 ==========

export async function onCacheScanProgress(
  cb: (u: StageUpdate) => void,
): Promise<UnlistenFn> {
  return listen<StageUpdate>("cache-scan-progress", (e) => cb(e.payload));
}

export async function onResidueScanProgress(
  cb: (u: StageUpdate) => void,
): Promise<UnlistenFn> {
  return listen<StageUpdate>("residue-scan-progress", (e) => cb(e.payload));
}

// ========== History & Whitelist ==========

export async function getHistory(limit = 200): Promise<HistoryEntry[]> {
  return invoke("get_history", { limit });
}

export async function getWhitelist(): Promise<WhitelistEntry[]> {
  return invoke("get_whitelist");
}

export async function addWhitelist(
  kind: string,
  value: string,
  note = "",
): Promise<void> {
  return invoke("add_whitelist", { kind, value, note });
}

export async function removeWhitelist(id: number): Promise<void> {
  return invoke("remove_whitelist", { id });
}

// ========== 应用卸载 ==========

export async function scanInstalledApps(): Promise<SnapshotResult<InstalledApp[]>> {
  return invoke("scan_installed_apps");
}

export async function scanAppResiduesBatch(
  appSnapshotId: string,
  appKeys: string[],
): Promise<SnapshotResult<ResidueAppGroup[]>> {
  return invoke("scan_app_residues_batch", {
    appSnapshotId,
    appKeys,
  });
}

export async function checkAppRunning(bundlePath: string): Promise<boolean> {
  return invoke("check_app_running", { bundlePath });
}
