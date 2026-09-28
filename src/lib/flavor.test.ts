import { beforeEach, describe, expect, it, vi } from "vitest";

const getBuildFlavor = vi.fn();

vi.mock("./tauri", () => ({
  getBuildFlavor: () => getBuildFlavor(),
}));

const importFresh = async () => {
  vi.resetModules();
  return import("./flavor");
};

describe("flavor", () => {
  beforeEach(() => {
    getBuildFlavor.mockReset();
  });

  it("默认按 Developer ID 处理 —— 拿不到形态时不能少给功能", async () => {
    const flavor = await importFresh();
    // 未初始化时：全功能。这是刻意的保守默认，见模块注释。
    expect(flavor.getFlavor()).toBe("developer_id");
    expect(flavor.canTerminateProcesses()).toBe(true);
  });

  it("拿到 mas 时关掉终止进程能力", async () => {
    getBuildFlavor.mockResolvedValue("mas");
    const flavor = await importFresh();
    expect(await flavor.initFlavor()).toBe("mas");
    expect(flavor.getFlavor()).toBe("mas");
    expect(flavor.canTerminateProcesses()).toBe(false);
    expect(flavor.canExecExternalTools()).toBe(false);
  });

  it("拿到 developer_id 时能力全开", async () => {
    getBuildFlavor.mockResolvedValue("developer_id");
    const flavor = await importFresh();
    expect(await flavor.initFlavor()).toBe("developer_id");
    expect(flavor.canTerminateProcesses()).toBe(true);
    expect(flavor.canExecExternalTools()).toBe(true);
  });

  it("后端抛错时退回全功能，不能反过来误报成 mas", async () => {
    // 误报成 mas 会让全功能版用户凭空丢掉进程管理，比多显示一个坏入口更糟。
    getBuildFlavor.mockRejectedValue(new Error("no ipc"));
    const flavor = await importFresh();
    expect(await flavor.initFlavor()).toBe("developer_id");
    expect(flavor.canTerminateProcesses()).toBe(true);
  });

  it("后端返回未知值时忽略，仍按全功能", async () => {
    getBuildFlavor.mockResolvedValue("something_else");
    const flavor = await importFresh();
    expect(await flavor.initFlavor()).toBe("developer_id");
  });
});
