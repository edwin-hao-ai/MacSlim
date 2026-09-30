import { describe, expect, it, vi } from "vitest";

// 这里的被测对象就是 `@/lib/flavor` **本身**，所以不能像
// `flavor.mas.test.ts` 那样把它 mock 掉 —— 那个文件正是靠 mock 才能对
// 同一个模块问出「MAS 与否」两种答案。代价是测不到真实实现，
// 所以这个文件专门补上那一半。
//
// 存在的理由：能力门禁写对了、`initFlavor()` 也确实调了，**界面却没变**。
// 根因是 `current` 原本是普通模块变量配普通赋值 —— Solid 追踪不到，
// 于是任何在 `initFlavor()` 完成前渲染过一次的组件会永远停在
// developer_id，侧栏里 MAS 该藏的入口全都还在。
//
// 这类失效极其隐蔽：门禁测试全绿（直接设值后同步断言），只有真机看得见。
// 所以这里断言 setter 之后**依赖它的计算会重算**，而不只是「值对」。

const getBuildFlavor = vi.fn();

vi.mock("@/lib/tauri", () => ({
  getBuildFlavor: () => getBuildFlavor(),
}));

type FlavorModule = typeof import("@/lib/flavor");

/**
 * 每次都拿一份**全新的**模块实例。
 *
 * 不这么做的话模块级的 `current` 会在用例之间串：第一个用例把它设成 mas，
 * 后面三个用例的「初值」断言就全都不成立了 —— 而这三个用例恰恰是在断言
 * 「失败时/未知值时必须保持初值」。
 */
const freshModule = async (): Promise<FlavorModule> => {
  vi.resetModules();
  return (await import("@/lib/flavor")) as FlavorModule;
};

describe("形态切换的响应式语义", () => {
  it("初值是 developer_id —— 保守的一侧：宁可多显示入口", async () => {
    // 与 initFlavor 失败时的处理一致：拿不到就按全功能处理。
    // 反过来（失败即 mas）会让全功能版用户凭空丢掉进程管理。
    getBuildFlavor.mockResolvedValue("mas");
    const { getFlavor, canTerminateProcesses } = await freshModule();
    expect(getFlavor()).toBe("developer_id");
    expect(canTerminateProcesses()).toBe(true);
  });

  it("initFlavor 之后读到新值，且能力查询跟着变", async () => {
    getBuildFlavor.mockResolvedValue("mas");
    const { getFlavor, initFlavor, can, canTerminateProcesses } =
      await freshModule();
    expect(await initFlavor()).toBe("mas");
    expect(getFlavor()).toBe("mas");
    expect(can("terminateProcess")).toBe(false);
    expect(canTerminateProcesses()).toBe(false);
  });

  it("initFlavor 失败时保持 developer_id，而不是变成 mas", async () => {
    getBuildFlavor.mockRejectedValue(new Error("no ipc"));
    const { initFlavor, can } = await freshModule();
    expect(await initFlavor()).toBe("developer_id");
    expect(can("terminateProcess")).toBe(true);
  });

  it("后端给出未知值时保持初值 —— 不能把拼写错误当成一种形态", async () => {
    getBuildFlavor.mockResolvedValue("ios");
    const { initFlavor } = await freshModule();
    expect(await initFlavor()).toBe("developer_id");
  });

  it("能力清单是随形态变的计算值，不是加载时的快照", async () => {
    getBuildFlavor.mockResolvedValue("mas");
    const { initFlavor, visibleCapabilities } = await freshModule();
    const before = visibleCapabilities();
    await initFlavor();
    const after = visibleCapabilities();
    expect(after).not.toEqual(before);
    expect(after).toContain("folderGrant");
    expect(before).toContain("terminateProcess");
  });
});
