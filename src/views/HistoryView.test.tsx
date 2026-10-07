import { cleanup, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ getHistory: vi.fn() }));

vi.mock("@/lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/tauri")>();
  return { ...actual, getHistory: mocks.getHistory };
});

vi.mock("@/i18n", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/i18n")>();
  const { zhCN } = await import("@/i18n/zh-CN");
  const lookup = (key: string): string => {
    const parts = key.split(".");
    let cur: unknown = zhCN;
    for (const part of parts) {
      if (typeof cur !== "object" || cur === null) return key;
      cur = (cur as Record<string, unknown>)[part];
    }
    return typeof cur === "string" ? cur : key;
  };
  const { fakeI18n } = await import("@/i18n/fake-i18n");
  // 上下文在工厂里建好（不是每次调用时）：`useI18n()` 必须同步返回对象，
  // 返回 Promise 会让组件里的 `t(...)` 直接炸。
  const ctx = fakeI18n({ t: lookup });
  return { ...actual, useI18n: () => ctx };
});

import HistoryView from "@/views/HistoryView";
import { zhCN } from "@/i18n/zh-CN";
import { en } from "@/i18n/en";

type HistoryRow = {
  id: number;
  timestamp: string;
  operation: string;
  target: string;
  freed_bytes: number;
  success: boolean;
  detail: string;
};

const row = (id: number, operation: string): HistoryRow => ({
  id,
  timestamp: "2026-03-01 10:00:00",
  operation,
  target: `目标 ${id}`,
  freed_bytes: 0,
  success: true,
  detail: "详情",
});

const OPERATIONS = [
  "cache",
  "process",
  "app_terminate",
  "app_graceful_quit",
  "uninstall",
  "docker",
];

describe("HistoryView operation labels", () => {
  beforeEach(() => {
    mocks.getHistory.mockReset();
    mocks.getHistory.mockResolvedValue(OPERATIONS.map((operation, index) => row(index + 1, operation)));
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("labels every broker operation kind in Chinese", async () => {
    render(() => <HistoryView active />);
    await waitFor(() => expect(mocks.getHistory).toHaveBeenCalled());
    const expected = [
      zhCN.history.opCache,
      zhCN.history.opProcess,
      zhCN.history.opAppTerminate,
      zhCN.history.opAppGracefulQuit,
      zhCN.history.opUninstall,
      zhCN.history.opDocker,
    ];
    for (const label of expected) {
      await waitFor(() =>
        expect(document.body.textContent).toContain(label),
      );
    }
  });

  it("never renders a raw operation key or an unknown-operation fallback", async () => {
    render(() => <HistoryView active />);
    await waitFor(() => expect(mocks.getHistory).toHaveBeenCalled());
    const text = document.body.textContent ?? "";
    for (const operation of OPERATIONS) {
      expect(text).not.toContain(`"${operation}"`);
      expect(text).not.toContain(`· ${operation}`);
    }
    expect(text).not.toContain("未知操作");
  });

  it("keeps graceful quit distinct from forced termination", async () => {
    expect(zhCN.history.opAppGracefulQuit).not.toBe(zhCN.history.opAppTerminate);
    expect(en.history.opAppGracefulQuit).not.toBe(en.history.opAppTerminate);
    expect(zhCN.history.opAppGracefulQuit).toBe("应用优雅退出");
    expect(en.history.opAppGracefulQuit).toBe("Graceful app quit");
  });

  it("renders the graceful quit row with its own label", async () => {
    mocks.getHistory.mockResolvedValue([row(1, "app_graceful_quit")]);
    const view = render(() => <HistoryView active />);
    await waitFor(() =>
      expect(view.container.textContent).toContain(zhCN.history.opAppGracefulQuit),
    );
    expect(view.container.textContent).not.toContain("app_graceful_quit");
    expect(view.container.textContent).not.toContain(zhCN.history.opAppTerminate);
  });

  it("reloads every time the view becomes active", async () => {
    // 这条抓的是「清理成功但历史页显示空」：TabPanel 只是 display:none，
    // 所有视图从启动起常驻，若只在 onMount 读一次，用户清理完切到历史记录
    // 看到的是启动那一刻的空列表 —— 会以为刚才什么都没发生。
    mocks.getHistory.mockResolvedValue([row(1, "cache")]);
    const [active, setActive] = createSignal(false);
    render(() => <HistoryView active={active()} />);

    expect(mocks.getHistory).not.toHaveBeenCalled();

    setActive(true);
    await waitFor(() => expect(mocks.getHistory).toHaveBeenCalledTimes(1));

    // 清理之后再进历史页：必须重新读一次，而不是复用启动时的旧结果
    setActive(false);
    setActive(true);
    await waitFor(() => expect(mocks.getHistory).toHaveBeenCalledTimes(2));
  });
});
