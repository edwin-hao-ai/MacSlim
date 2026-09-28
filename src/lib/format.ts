export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

export function fmtDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

import type { Safety } from "./tauri";

/** `t()` 的最小签名 —— 让 `format.ts` 不必依赖 solid 的 context。 */
export type Translate = (key: string, params?: Record<string, string | number>) => string;

/**
 * 相对时间（历史页 / 白名单列表）。
 *
 * 两条硬约束：
 * 1. 文案全部走 i18n key，**不许**在这里硬编码中文 —— 英文用户看到中文日期即失败。
 * 2. 超过 7 天走绝对日期，locale 必须跟着界面语言走。这里原来把日期 locale
 *    写死成中文，英文界面会显示 `2026/9/1` 这种中文日期格式，
 *    是切到英文后必然暴露的混排。
 */
export function fmtRelativeTime(iso: string, t: Translate, locale: string): string {
  const d = new Date(iso);
  const diff = Date.now() - d.getTime();
  const min = Math.floor(diff / 60000);
  if (min < 1) return t("timeFormat.justNow");
  if (min < 60) return t("timeFormat.minutesAgo", { min });
  const h = Math.floor(min / 60);
  if (h < 24) return t("timeFormat.hoursAgo", { h });
  const day = Math.floor(h / 24);
  if (day < 7) return t("timeFormat.daysAgo", { day });
  return d.toLocaleDateString(locale);
}

/**
 * 缓存分类的展示名。
 *
 * 品牌类分类（NPM / Docker / Xcode…）两种语言写法相同，直接内联；只有
 * `system`（系统类缓存）需要翻译 —— 它原来硬编码中文，英文界面会露出
 * 「系统」两个字。
 */
export const CATEGORY_LABELS: Record<string, string> = {
  npm: "NPM",
  pnpm: "PNPM",
  yarn: "Yarn",
  docker: "Docker",
  homebrew: "Homebrew",
  xcode: "Xcode",
  cocoapods: "CocoaPods",
  cargo: "Cargo",
  pip: "Pip",
  go: "Go",
  system: "__i18n__:cacheCategory.system",
};

/** 分类标签：能内联就内联，需要翻译的查词典。 */
export function categoryLabel(category: string, t: Translate): string {
  const label = CATEGORY_LABELS[category];
  if (label === undefined) return category;
  if (label.startsWith("__i18n__:")) return t(label.slice("__i18n__:".length));
  return label;
}

export const CATEGORY_COLORS: Record<string, string> = {
  npm: "bg-red-500/15 text-red-600",
  pnpm: "bg-yellow-500/15 text-yellow-600",
  yarn: "bg-blue-500/15 text-blue-600",
  docker: "bg-sky-500/15 text-sky-600",
  homebrew: "bg-amber-500/15 text-amber-600",
  xcode: "bg-indigo-500/15 text-indigo-600",
  cocoapods: "bg-pink-500/15 text-pink-600",
  cargo: "bg-orange-500/15 text-orange-600",
  pip: "bg-green-500/15 text-green-600",
  go: "bg-cyan-500/15 text-cyan-600",
  system: "bg-zinc-500/15 text-zinc-600",
};

/** 判定「可清理」所需的最小字段面 —— 与 `CacheItem` 结构解耦，便于单测。 */
export type ReclaimableProbe = { safety: Safety };

/**
 * 某个分类分组是不是「可清理空间」—— 也就是该不该给它加斜线网格。
 *
 * **判据来自数据，不来自分类名单**：`CacheItem.safety` 是后端逐项算出来的
 * 风险等级，`medium`（界面上的「注意」）意味着这一项清理前需要用户自己拿
 * 主意。规则是「**分组里一项 `medium` 都没有**才算可清理」：
 *
 * - 宁可少标，不可多标。混进一项「注意」级的条目还整组标可清理，等于替用户
 *   做了那个不该替他做的判断（`AGENTS.md` §4.3 的安全边界）。
 * - 规则随数据变化：Xcode 分组里唯一一项 `medium` 是 `xcode-archives`
 *   （>1GB 的发布存档才列出），它不在列表里时整个 Xcode 分组会自动变成可清理。
 * - `default_select` 刻意**不**参与：它表达的是「默认勾选」（重新获取成本
 *   < 5 分钟），而斜线纹理表达的是「这里确实是能清的空间」。废纸篓
 *   `default_select: false` 但确实可清，两者语义不同，混用会说错话。
 */
export function isCategoryReclaimable(
  items: ReadonlyArray<ReclaimableProbe>,
): boolean {
  return items.length > 0 && items.every((item) => item.safety !== "medium");
}

