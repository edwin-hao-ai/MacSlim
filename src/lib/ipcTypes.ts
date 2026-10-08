/**
 * 跨进程的数据契约类型。
 *
 * ## 为什么从 `tauri.ts` 里拆出来
 *
 * 仓库有一条门禁：**只有 `tauri.ts` 可以调 `invoke`**（破坏性入口要集中
 * 在一处，便于审计）。所以新功能的前端桥接函数只能加在 `tauri.ts` 里 ——
 * 而它已经逼近 800 行的硬上限（AGENTS.md §7.1），只剩十几行余量，
 * 每加一个功能都会被卡住。
 *
 * **类型定义不调 `invoke`**，所以它们可以拆出去：`tauri.ts` 只留一行
 * `export type { ... } from`，调用方的 import 路径一个字都不用改。
 * 这是「加功能」与「改架构」解耦的地方 —— 真正需要重构的是等 `invoke`
 * 也该分层的时候，而不是现在。
 */

import type { Flavor } from "@/lib/flavor";

export type ProcessKind = "zombie" | "idle" | "hog" | "dev" | "system" | "foreground";

/**
 * 后端下发的插值参数：`[参数名, 参数值]` 二元组数组
 * （Rust `Vec<(String, String)>` 的 JSON 形状）。
 *
 * 渲染时用 `paramsToRecord` 转成 `t()` 认的对象，或直接用 `tText()`。
 */
export type I18nParams = [string, string][];

export type ProcessInfo = {
  selection_key: string;
  pid: number;
  name: string;
  exe: string;
  start_time: number;
  cpu_percent: number;
  memory_mb: number;
  kind: ProcessKind;
  risk: "safe" | "low" | "dev" | "hidden";
  default_select: boolean;
  /**
   * 分类理由的 i18n key + 插值参数，译文在前端词典里（`tText`）。
   *
   * 「受保护（原因）」与「端口 N（运行中的服务，请确认）」这两段**不在**这里 ——
   * 它们分别由 `protected_reason_key` 和 `ports` 独立携带，句子由前端拼。
   * 后端只负责判定「这个进程为什么是这一类」。
   */
  reason_key: string;
  reason_params: I18nParams;
  ports: number[];
  icon_base64: string | null;
  protected: boolean;
  protected_reason_key: string | null;
  protected_reason_params: I18nParams;
  whitelisted: boolean;
};

/**
 * 一个可授权的目录（App Store 版）。
 *
 * App Store 版读用户目录的**唯一合规入口**：用户在标准文件选择框里亲手
 * 选定一个目录。沙箱里没有任何 entitlement 能让我们直接读
 * `~/Library/Caches` —— 给完全磁盘访问权限也不行。
 */
export type FolderTarget = {
  /** 稳定标识，落盘时用它对上号。**snake_case**。 */
  key: string;
  /** 相对真实 home 的路径，仅用于展示「你要授权哪个目录」 */
  relativePath: string;
  /**
   * 展示文案的 i18n key，由后端下发。
   *
   * 不要从 `key` 拼：`key` 是 snake_case（`user_caches`），而字典里是
   * camelCase（`userCaches`）。拼错的话界面会把 `access.target.user_caches`
   * 这样的 key 直接当文案显示出来 —— 实测截图里就是这么发现的。
   */
  reasonKey: string;
  granted: boolean;
  grantedPath: string | null;
};

// ========== 系统健康与快照 ==========

export type SystemHealth = {
  cpu_percent: number;
  memory_used_mb: number;
  memory_total_mb: number;
  memory_percent: number;
  disk_used_gb: number;
  disk_total_gb: number;
  disk_percent: number;
};

export type ScanResult = {
  health: SystemHealth;
  processes: ProcessInfo[];
  scanned_at_ms: number;
};

export type SnapshotResult<T> = {
  snapshot_id: string;
  expires_at_ms: number;
  value: T;
};

export type Keyed<T> = T & { selection_key: string };

/** 完全磁盘访问权限的探测结果。分两类，因为它们挡住的**能力**不同。 */
export type FdaStatus = {
  /** 能不能读**真实 home**下的 `~/Library/Caches` —— 决定缓存清理是否可用 */
  userCache: boolean;
  /** 能不能读 `/Library/*` —— 决定进程枚举等是否可用 */
  systemDirs: boolean;
  /**
   * `$HOME` 是否被沙箱重定向到了应用自己的 container。
   *
   * 这一条决定引导该指向哪里：为 true 时**授权解决不了任何问题**
   * （实测 2026-09-30，MAS 包里真实 `~/Library/Caches` 的 read_dir 直接
   * EPERM，不是空的），继续显示「打开系统设置」按钮只会把用户领去做一件
   * 无效的事。
   */
  homeRedirected: boolean;
  /** 构建形态，与 `src/lib/flavor.ts` 的 `Flavor` 一致 */
  flavor: Flavor;
};

// ========== 操作执行结果 ==========

export type PreparedOperation = {
  operation_id: string;
  kind: string;
  expires_at_ms: number;
  item_count: number;
  estimated_bytes: number;
  /** 操作摘要的 i18n key + 插值参数（`opSummary.*`），译文在前端词典里。 */
  summary_key: string;
  summary_params: I18nParams;
};

export type CleanReport = {
  id: string;
  /** i18n key（与 CacheItem 同一套约定），由前端词典翻译 */
  label_key: string;
  label_params: I18nParams;
  success: boolean;
  /** 实际删除的字节数。**不等于释放**：删除成功不代表磁盘空间已回收。 */
  deleted_bytes: number;
  duration_ms: number;
  /** 失败原因：结构化错误（`error.<code>` 查词典），不是裸中文串。 */
  error: TauriError | null;
};

export type CleanSummary = {
  reports: CleanReport[];
  /** 实际删除的字节数（诚实口径，不再用扫描体积冒充释放）。 */
  deleted_bytes: number;
  /**
   * 实测回收的字节数：删除前后同一磁盘卷可用空间之差。
   * `null` = 未能测量（例如删除项不在同一卷 / 读取失败），**不得伪装成 0**。
   */
  reclaimed_bytes: number | null;
  success_count: number;
  fail_count: number;
};

export type ProcessKillDetail = {
  pid: number;
  name: string;
  success: boolean;
  /** 结果行文案：结构化消息（`error.<code>` 查词典），不是裸中文串。 */
  message: TauriError;
};

export type ProcessKillReport = {
  killed: number[];
  failed: number[];
  details: ProcessKillDetail[];
};

export type MoveResult = {
  path: string;
  success: boolean;
  /** 失败原因：结构化错误（`error.<code>` 查词典），不是裸中文串。 */
  error: TauriError | null;
  size_bytes: number;
};

export type UninstallReport = {
  app_name: string;
  bundle_id: string;
  /** 已移入废纸篓的字节数。文件还在废纸篓里，**尚未释放**。 */
  trashed_bytes: number;
  /**
   * 实测回收的字节数；`null` = 未能测量（移入废纸篓不改变已用空间，
   * 通常为 null），**不得伪装成 0**。
   */
  reclaimed_bytes: number | null;
  moved_count: number;
  failed_count: number;
  details: MoveResult[];
  quit_error: TauriError | null;
};

export type AppGracefulQuitReport = {
  app_name: string;
  bundle_id: string;
  quit_error: TauriError | null;
};

export type DockerExecutionReport = {
  action: string;
  succeeded: string[];
  failed: [string, string][];
  output: string;
  /**
   * 实测回收的字节数：操作前后同一磁盘卷可用空间之差。
   * `null` = 未能测量（例如报告未采集 / 读取失败），**不得伪装成 0**。
   */
  reclaimed_bytes: number | null;
};

export type OperationResult =
  | { kind: "cache"; value: CleanSummary }
  | { kind: "process"; value: ProcessKillReport }
  | { kind: "app_terminate"; value: ProcessKillReport }
  | { kind: "app_graceful_quit"; value: AppGracefulQuitReport[] }
  | { kind: "uninstall"; value: UninstallReport[] }
  | { kind: "docker"; value: DockerExecutionReport };

export type OperationErrorKind =
  | "stale"
  | "history"
  | "forceOnly"
  | "whitelisted"
  | "failed";

/**
 * 后端 `UserError` 的线上形状（`src-tauri/src/user_error.rs`）。
 *
 * `code` 是**稳定标识**（纯 ASCII snake_case），`message` 是中文兜底
 * （CLI 用；英文界面不该看到它），`params` 是插值参数。
 */
export type TauriError = {
  code: string;
  message: string;
  params: I18nParams;
};

export type OperationErrorInfo = TauriError & {
  kind: OperationErrorKind;
};

// ========== 进程管理视图 ==========

export type ProcessRow = {
  selection_key: string;
  pid: number;
  parent_pid: number | null;
  /** 给用户看的可读名（bundle 名 / bundle id 末段 / 固定映射） */
  name: string;
  /** `name` 的 i18n key（目前只有 WebKit 三个固定映射）。有值时用译文替换 `name`。 */
  name_key: string | null;
  /** 未经解析的原始进程名，用于 tooltip 与消歧副标题 */
  full_name: string;
  exe: string;
  start_time: number;
  cpu_percent: number;
  memory_mb: number;
  uptime_secs: number;
  /** 进程状态的 i18n key（`process.status.*`），不是中文状态名。 */
  status_key: string;
  ports: number[];
  icon_base64: string | null;
  protected: boolean;
  protected_reason_key: string | null;
  protected_reason_params: I18nParams;
  whitelisted: boolean;
};

// ========== 应用程序管理 ==========

export type AppChildProcess = {
  selection_key: string;
  pid: number;
  parent_pid: number | null;
  name: string;
  exe: string;
  start_time: number;
  memory_mb: number;
  cpu_percent: number;
  ports: number[];
  is_main: boolean;
  depth: number;
  protected: boolean;
  protected_reason_key: string | null;
  protected_reason_params: I18nParams;
  whitelisted: boolean;
};

export type AppInfo = {
  selection_key: string;
  bundle_path: string;
  name: string;
  bundle_id: string;
  icon_base64: string | null;
  main_pid: number;
  all_pids: number[];
  children: AppChildProcess[];
  memory_mb: number;
  cpu_percent: number;
  uptime_secs: number;
  ports: number[];
  is_system: boolean;
  protected_process_count: number;
  whitelisted_process_count: number;
};

// ========== Docker 深度视图 ==========

export type DockerImage = {
  selection_key: string;
  id: string;
  repository: string;
  tag: string;
  size_bytes: number;
  created: string;
  dangling: boolean;
  in_use: boolean;
};

export type DockerContainer = {
  selection_key: string;
  id: string;
  name: string;
  image: string;
  status: string;
  running: boolean;
  size_bytes: number;
  created: string;
};

export type DockerVolume = {
  selection_key: string;
  name: string;
  driver: string;
  size_bytes: number;
  in_use: boolean;
};

export type DockerBuilderCache = {
  total_bytes: number;
  reclaimable_bytes: number;
};

export type DockerInventory = {
  daemon_running: boolean;
  images: DockerImage[];
  containers: DockerContainer[];
  volumes: DockerVolume[];
  builder: DockerBuilderCache;
  reclaimable_bytes: number;
};

// ========== 扫描阶段进度事件 ==========

/**
 * 后端逐阶段发出的扫描进度。
 *
 * `stage` 原样透传，前端**不加字段**、不改形状（安全约束，见
 * `src-tauri/src/lib_progress_tests.rs`）。但 `stage` 装的内容是 i18n key
 * （`scanStage.*`）而不是中文阶段名：固定阶段名走 `t()` 翻译；
 * 残留扫描那条事件装的是**应用名**（用户数据），原样显示。
 * 分流靠 `tStage()` 里的前缀判断。
 */
export type StageUpdate = {
  stage: string;
  state: "running" | "done";
  item_count: number;
  found_bytes: number;
};

// ========== History & Whitelist ==========

export type HistoryEntry = {
  id: number;
  timestamp: string;
  operation: string;
  /** 拼好的中文文本，旧数据与兜底用；界面优先用下面的结构化字段。 */
  target: string;
  /**
   * 旧字段：缓存=deleted_bytes、卸载=trashed_bytes、docker=实测释放（无则 0）。
   * 仅兜底，新界面按 operation 用下面的诚实口径字段。
   */
  freed_bytes: number;
  success: boolean;
  /** 同上，中文文本兜底。 */
  detail: string;
  /** 结构化计数：界面据此本地化渲染。旧数据为 0，此时回退到 target/detail。 */
  item_count: number;
  ok_count: number;
  fail_count: number;
  /** 失败原因的错误码（`error.<code>` 词条）；空串表示无。 */
  reason_code: string;
  /** 缓存清理：实际删除的字节数。 */
  deleted_bytes: number;
  /** 卸载：已移入废纸篓的字节数。 */
  trashed_bytes: number;
  /** 实测回收的字节数；`null` = 未能测量，不得伪装成 0。 */
  reclaimed_bytes: number | null;
};

export type WhitelistEntry = {
  id: number;
  kind: string;
  value: string;
  added_at: string;
  note: string;
};

// ========== 应用卸载 ==========

export type InstalledApp = {
  selection_key: string;
  bundle_path: string;
  name: string;
  bundle_id: string;
  icon_base64: string | null;
  bundle_size_bytes: number;
  is_system: boolean;
  is_running: boolean;
  estimated_residue_bytes: number;
};

export type ResidueItem = {
  selection_key: string;
  path: string;
  category: string;
  size_bytes: number;
  is_dev_tool: boolean;
  selected: boolean;
};

export type AppResidue = {
  bundle_id: string;
  app_name: string;
  items: ResidueItem[];
  total_bytes: number;
  scan_complete: boolean;
};

export type ResidueAppGroup = {
  app_key: string;
  residue: AppResidue;
};
