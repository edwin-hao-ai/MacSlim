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

const GRANTED = { userCache: true, systemDirs: true };
const NEED_CACHE = { userCache: false, systemDirs: true };
const NEED_SYSTEM = { userCache: true, systemDirs: false };
const NEED_BOTH = { userCache: false, systemDirs: false };

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
