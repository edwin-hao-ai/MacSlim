/**
 * 侧栏导航项的**数据**部分。
 *
 * ## 为什么和 Sidebar.tsx 分开
 *
 * 导航项上挂着图标组件，而图标库是个大模块。把「哪些页面该出现」这条
 * 规则和图标放在同一个文件里，测那条规则就必须把整个图标库 + Tauri API
 * 一起加载进来 —— 实测单是 import 就超过 20 秒，测试直接超时。
 *
 * 规则本身（按能力过滤）是纯数据，放到这里就能被秒级测到；
 * Sidebar 只负责把图标贴上去。
 */

import { capabilitiesOf, type Capability, type Flavor } from "@/lib/flavor";

export type ViewId =
  | "scan"
  | "process"
  | "applications"
  | "cache"
  | "uninstaller"
  | "history"
  | "settings";

export type NavItemSpec = {
  id: ViewId;
  /**
   * 该页面依赖的能力。缺了就渲染。
   *
   * **硬要求**，不是锦上添花：没有对应能力的页面导航过去就是空页面 ——
   * 那是 App Store 指南 2.1 说的不完整形态，也是审核最容易挑的点。
   */
  needs?: Capability;
  /** 图标名，由 Sidebar 映射到具体组件 */
  icon: "activity" | "cpu" | "package" | "drive" | "trash" | "history" | "settings";
};

export const ALL_NAV_ITEMS: NavItemSpec[] = [
  { id: "scan", icon: "activity" },
  { id: "process", icon: "cpu", needs: "processMonitor" },
  { id: "applications", icon: "package", needs: "appGrouping" },
  { id: "cache", icon: "drive", needs: "cacheClean" },
  { id: "uninstaller", icon: "trash", needs: "appSizeAnalysis" },
  { id: "history", icon: "history" },
  { id: "settings", icon: "settings" },
];

/**
 * 当前形态下应该出现的导航项。
 *
 * 能力清单的唯一真相源在 `@/lib/flavor`，这里只做映射。
 */
export const visibleNavItems = (flavor: Flavor): NavItemSpec[] => {
  const supported = capabilitiesOf(flavor);
  return ALL_NAV_ITEMS.filter(
    (item) => item.needs === undefined || supported.includes(item.needs),
  );
};