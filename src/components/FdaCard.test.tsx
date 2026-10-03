import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const getFdaStatus = vi.fn();
const openFullDiskAccessSettings = vi.fn();

vi.mock("@/lib/tauri", () => ({
  getFdaStatus: () => getFdaStatus(),
  openFullDiskAccessSettings: () => openFullDiskAccessSettings(),
}));

vi.mock("@/i18n", () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));

import FdaCard from "@/components/FdaCard";

const GRANTED = { userCache: true, systemDirs: true, homeRedirected: false, flavor: "developer_id" };
const NEED_CACHE = { userCache: false, systemDirs: true, homeRedirected: false, flavor: "developer_id" };
const NEED_SYSTEM = { userCache: true, systemDirs: false, homeRedirected: false, flavor: "developer_id" };
const NEED_BOTH = { userCache: false, systemDirs: false, homeRedirected: false, flavor: "developer_id" };

describe("FdaCard 完全磁盘访问引导", () => {
  beforeEach(() => {
    getFdaStatus.mockReset();
    openFullDiskAccessSettings.mockReset();
    openFullDiskAccessSettings.mockResolvedValue(true);
  });
  afterEach(cleanup);

  it("已授权时显示绿色对勾且没有按钮 —— 不打扰", async () => {
    // 引导卡片的定位是「需要时才出现」。已授权还天天弹按钮的向导会
    // 变成这个品类最招人烦的设计。
    getFdaStatus.mockResolvedValue(GRANTED);
    render(() => <FdaCard />);
    await screen.findByTestId("fda-card");
    expect(screen.getByText("settings.fda.granted")).toBeTruthy();
    expect(
      screen.queryByRole("button", { name: "settings.fda.openSettings" }),
    ).toBeNull();
  });

  it("缺用户缓存权限：给缓存相关的引导文案", async () => {
    // 这一类挡的是「缓存清理」。笼统说「请授权」用户不知道授权后能干什么。
    getFdaStatus.mockResolvedValue(NEED_CACHE);
    render(() => <FdaCard />);
    expect(await screen.findByText("settings.fda.needUserCache")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "settings.fda.openSettings" }),
    ).toBeTruthy();
  });

  it("缺系统目录权限：给进程相关的引导文案", async () => {
    getFdaStatus.mockResolvedValue(NEED_SYSTEM);
    render(() => <FdaCard />);
    expect(await screen.findByText("settings.fda.needSystem")).toBeTruthy();
  });

  it("两类都缺：给合并文案", async () => {
    getFdaStatus.mockResolvedValue(NEED_BOTH);
    render(() => <FdaCard />);
    expect(await screen.findByText("settings.fda.needBoth")).toBeTruthy();
  });

  it("点按钮会调打开系统设置", async () => {
    getFdaStatus.mockResolvedValue(NEED_BOTH);
    render(() => <FdaCard />);
    const button = await screen.findByRole("button", {
      name: "settings.fda.openSettings",
    });
    fireEvent.click(button);
    await waitFor(() => expect(openFullDiskAccessSettings).toHaveBeenCalledTimes(1));
  });

  it("打不开设置页时给出手动路径，而不是静默失败", async () => {
    // 深链那个 URL scheme 是系统私有但稳定的，但不是保证可用。
    // 打不开时必须说清「去哪儿手动找」，否则用户点了没反应就走了。
    openFullDiskAccessSettings.mockResolvedValue(false);
    getFdaStatus.mockResolvedValue(NEED_BOTH);
    render(() => <FdaCard />);
    const button = await screen.findByRole("button", {
      name: "settings.fda.openSettings",
    });
    fireEvent.click(button);
    expect(await screen.findByText("settings.fda.openFailed")).toBeTruthy();
  });

  it("探针报错时按「缺权限」处理（安全侧保守默认）", async () => {
    // 反过来（探不到就当作有权限）会让用户对着空列表永远看不到引导。
    getFdaStatus.mockRejectedValue(new Error("no ipc"));
    render(() => <FdaCard />);
    expect(await screen.findByText("settings.fda.needBoth")).toBeTruthy();
  });
});

// ===== MAS 版不该把用户领去开 FDA =====
//
// 实测（2026-09-30，MAS 包真机）：沙箱把 `$HOME` 重定向到应用自己的
// container，用户数据路径根本不在我们能碰的范围里。**授权解决不了。**
// 而 Apple 文档也明说 App Store 应用即使拿到 FDA，沙箱仍强制执行自己的
// 文件限制。
//
// 所以 MAS 版的卡片必须换掉：说清边界、列出真实可用能力、把「要完整
// 缓存清理」这件事引流到完整版。继续显示「打开系统设置」按钮只会把用户
// 领去做一件无效的事 —— 那是比没有引导更糟的引导。

const MAS_REDIRECTED = {
  userCache: false,
  systemDirs: true,
  homeRedirected: true,
  flavor: "mas" as const,
};

const DEV_OK = {
  userCache: true,
  systemDirs: true,
  homeRedirected: false,
  flavor: "developer_id" as const,
};

describe("FdaCard 在 MAS 版的行为", () => {
  beforeEach(() => {
    getFdaStatus.mockReset();
    openFullDiskAccessSettings.mockReset();
    openFullDiskAccessSettings.mockResolvedValue(true);
  });
  afterEach(cleanup);

  it("不显示「打开系统设置」按钮 —— 授权在 MAS 版无效", async () => {
    getFdaStatus.mockResolvedValue(MAS_REDIRECTED);
    render(() => <FdaCard />);
    await screen.findByTestId("fda-card");
    expect(
      screen.queryByRole("button", { name: "settings.fda.openSettings" }),
    ).toBeNull();
  });

  it("说清真实原因：沙箱把用户目录换掉了，而不是「你没授权」", async () => {
    getFdaStatus.mockResolvedValue(MAS_REDIRECTED);
    render(() => <FdaCard />);
    expect(await screen.findByText("settings.fda.sandboxed")).toBeTruthy();
    expect(screen.queryByText("settings.fda.needBoth")).toBeNull();
    expect(screen.queryByText("settings.fda.needUserCache")).toBeNull();
  });

  it("沙箱形态下不给任何站外入口 —— 只陈述边界，不导流", async () => {
    // 这条用例的上一版断言的是「有链接、指向 vgoapp.com」，也就是把导流
    // 写成了期望行为。现在反过来：App Store 版里不允许出现任何站外链接
    // 或升级入口。
    //
    // 判据不是「文案温和」，而是**结构上没有可点的去处** —— 换个措辞
    // 仍然是导流，而审核看的是行为。
    getFdaStatus.mockResolvedValue(MAS_REDIRECTED);
    const { container } = render(() => <FdaCard />);
    await screen.findByText("settings.fda.sandboxed");

    expect(container.querySelectorAll("a").length).toBe(0);
    expect(
      screen.queryByRole("link", { name: "settings.fda.getFullVersion" }),
    ).toBeNull();
    // 也不能有 button 形态的升级入口
    expect(
      screen.queryByRole("button", { name: "settings.fda.getFullVersion" }),
    ).toBeNull();
    expect(openFullDiskAccessSettings).not.toHaveBeenCalled();
  });

  it("完整版行为不变：已授权就是绿勾、无按钮", async () => {
    getFdaStatus.mockResolvedValue(DEV_OK);
    render(() => <FdaCard />);
    expect(await screen.findByText("settings.fda.granted")).toBeTruthy();
    expect(document.querySelectorAll("a").length).toBe(0);
  });
});
