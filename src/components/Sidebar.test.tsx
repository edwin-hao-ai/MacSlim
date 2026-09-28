import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  startDragging: vi.fn(async () => undefined),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startDragging: tauriMocks.startDragging }),
}));

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn(async () => "9.8.7"),
}));

vi.mock("@/i18n", () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));

import Sidebar from "@/components/Sidebar";

const dispatchMouseDown = (target: Element) => {
  const event = new MouseEvent("mousedown", {
    bubbles: true,
    cancelable: true,
    button: 0,
  });
  target.dispatchEvent(event);
  return event;
};

describe("Sidebar 拖动区域", () => {
  beforeEach(() => tauriMocks.startDragging.mockClear());
  afterEach(cleanup);

  it("侧栏空白可拖；版本号文字让位给拖选", async () => {
    const onChange = vi.fn();
    render(() => <Sidebar current="scan" onChange={onChange} />);

    const surface = screen.getByTestId("sidebar-drag-surface");
    const version = await screen.findByText("v9.8.7");

    const surfaceEvent = dispatchMouseDown(surface);
    const versionEvent = dispatchMouseDown(version);

    // 侧栏空白（Logo 区、导航项间隙、导航项之间的 padding）可拖。
    expect(tauriMocks.startDragging).toHaveBeenCalledTimes(1);
    expect(surfaceEvent.defaultPrevented).toBe(true);
    // 版本号那一格自身带文字，交给拖选，不再拖窗口。这是「文字让位给拖选」
    // 这条分界线的直接后果，不是缺陷 —— 那一格只有一行 10px 的字，拖不到也无妨。
    expect(versionEvent.defaultPrevented).toBe(false);
  });

  it("导航项仍然可以正常点击而不触发拖动", () => {
    const onChange = vi.fn();
    render(() => <Sidebar current="scan" onChange={onChange} />);

    const settings = screen.getByRole("button", { name: "nav.settings" });
    const event = dispatchMouseDown(settings);
    fireEvent.click(settings);

    expect(tauriMocks.startDragging).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
    expect(onChange).toHaveBeenCalledWith("settings");
  });
});
