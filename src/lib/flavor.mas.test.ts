import { describe, expect, it, vi } from "vitest";

// 这些测试守的是**审核层面的一条硬规则**：App Store 版里不允许出现
// 「显示了但用不了」的功能入口。
//
// 为什么这是审核问题而不只是体验问题：
// - 指南 2.1 App Completeness：导航到一个空页面、点了报错的按钮，
//   是最典型的「不完整」形态
// - 指南 4.0 Design：「功能太少」是下架原因第一位
// 更糟的是**假数据**：MAS 版曾经显示「没有发现可优化的进程，系统运行良好」，
// 而那台机器上有 186 个进程 —— 只是枚举被沙箱拦了。对审核说这是误导，
// 对用户是骗人。

vi.mock("@/lib/flavor", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/flavor")>();
  return { ...actual, getFlavor: () => "mas", can: (c: string) => actual.masCan(c) };
});

describe("能力门禁：MAS 版不得出现用不了的入口", () => {
  it("MAS 不能终止进程", async () => {
    const { can } = await importFlavor();
    expect(can("terminateProcess")).toBe(false);
  });

  it("MAS 不做应用内自更新（updater 插件压根没注册，点了必报错）", async () => {
    const { can } = await importFlavor();
    expect(can("inAppUpdate")).toBe(false);
  });

  it("MAS 不能按 .app 聚合「运行中的应用」", async () => {
    // 这一页目前还是空的 —— 放出来就是「导航到空页面」，
    // 属于审核指南 2.1 说的不完整。
    const { can } = await importFlavor();
    expect(can("appGrouping")).toBe(false);
  });

  it("MAS 能做的要明确为 true，不能靠默认", async () => {
    const { can } = await importFlavor();
    expect(can("processMonitor")).toBe(true);
    expect(can("cacheClean")).toBe(true);
    expect(can("appSizeAnalysis")).toBe(true);
  });

  it("MAS 不能清理 Docker 缓存 —— 要 exec `docker` CLI，沙箱做不到", async () => {
    // 缓存页底部那节「Docker 缓存与资源」。它要跑 `docker system df` 之类
    // 的外部命令，沙箱里必然拿不到 inventory —— 露出来就是一整块死 UI，
    // 而且比空页面更糟：它带着「一键清理」按钮。
    const { can } = await importFlavor();
    expect(can("dockerCleanup")).toBe(false);
  });

  it("完整版必须保留 Docker 清理能力 —— 门禁不能把主产品削掉", async () => {
    const { capabilitiesOf } = (await import("@/lib/flavor")) as unknown as {
      capabilitiesOf: (f: string) => readonly string[];
    };
    expect(capabilitiesOf("developer_id")).toContain("dockerCleanup");
    expect(capabilitiesOf("mas")).not.toContain("dockerCleanup");
  });

  it("未知能力一律为 false —— 宁可藏起来，也不要露出一个坏入口", async () => {
    const { can } = await importFlavor();
    expect(can("somethingNobodyDefinedYet")).toBe(false);
  });
});

describe("侧栏只列出当前形态真正支持的页面", () => {
  it("MAS 侧栏不出现「应用程序」—— 那页还是空的", async () => {
    const { visibleNavItems } = await importSidebar();
    const ids = visibleNavItems("mas").map((i: { id: string }) => i.id);
    expect(ids).not.toContain("applications");
    expect(ids).toContain("process");
    expect(ids).toContain("cache");
  });

  it("完整版侧栏 7 项齐全 —— 门禁不能把主产品削掉", async () => {
    const { visibleNavItems } = await importSidebar();
    const ids = visibleNavItems("developer_id").map((i: { id: string }) => i.id);
    expect(ids).toEqual([
      "scan",
      "process",
      "applications",
      "cache",
      "uninstaller",
      "history",
      "settings",
    ]);
  });
});

async function importFlavor() {
  return (await import("@/lib/flavor")) as unknown as {
    can: (c: string) => boolean;
  };
}

async function importSidebar() {
  // 测 `lib/navItems` 而不是 `components/Sidebar`：后者会连带加载整个图标
  // 库与 Tauri API，实测光 import 就超过 20 秒、直接把测试拖超时。
  return (await import("@/lib/navItems")) as unknown as {
    visibleNavItems: (f: string) => { id: string }[];
  };
}
describe("页面内不得出现用不了的控件", () => {
  it("MAS 的智能扫描页不出现「一键优化」—— 沙箱里终止不了任何进程", async () => {
    // 这不是「藏一个按钮」那么轻：它背后那张列表在 MAS 下是空的，
    // 空列表会显示「没有发现可优化的进程，系统运行良好」——
    // 那台机器上有 186 个进程，这句话是假的。审核看到的是误导。
    const { can } = await importFlavor();
    expect(can("terminateProcess")).toBe(false);
  });

  it("MAS 的设置页不出现「检查更新」—— updater 插件压根没注册", async () => {
    // MAS 走 App Store 更新。留着这个按钮，点了必然报错。
    const { can } = await importFlavor();
    expect(can("inAppUpdate")).toBe(false);
  });
});
