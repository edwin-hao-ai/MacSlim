import type { Safety } from "@/lib/tauri";

export type SafetyFilter = "all" | "safe" | "check";

/** safe → 可直接清理；low/medium → 需要复核。 */
export function safetyBucket(safety: Safety): "safe" | "check" {
  return safety === "safe" ? "safe" : "check";
}

export function filterBySafety<T extends { safety: Safety }>(
  items: T[],
  filter: SafetyFilter,
): T[] {
  if (filter === "all") return items;
  return items.filter((item) => safetyBucket(item.safety) === filter);
}
