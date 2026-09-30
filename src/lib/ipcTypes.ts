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

import type { I18nParams } from "@/lib/tauri";

export type ProcessKind = "zombie" | "idle" | "hog" | "dev" | "system" | "foreground";

/**
 * 后端下发的插值参数：`[参数名, 参数值]` 二元组数组
 * （Rust `Vec<(String, String)>` 的 JSON 形状）。
 *
 * 渲染时用 `paramsToRecord` 转成 `t()` 认的对象，或直接用 `tText()`。
 */


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
  key: string;
  /** 相对真实 home 的路径，仅用于展示「你要授权哪个目录」 */
  relativePath: string;
  reasonKey: string;
  granted: boolean;
  grantedPath: string | null;
};
