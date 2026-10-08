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
  // 必须走真实的 `interpolate`：直接返回词条会让带参数的文案渲染成
  // 字面量 `{count} 项缓存`，测试就测不出「计数有没有被填进去」。
  const ctx = fakeI18n({
    t: (key: string, params?: Record<string, string | number>) =>
      actual.interpolate(lookup(key), params),
  });
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
  item_count: number;
  ok_count: number;
  fail_count: number;
  reason_code: string;
  deleted_bytes: number;
  trashed_bytes: number;
  reclaimed_bytes: number | null;
};

const row = (id: number, operation: string): HistoryRow => ({
  id,
  timestamp: "2026-03-01 10:00:00",
  operation,
  target: `目标 ${id}`,
  freed_bytes: 0,
  success: true,
  detail: "详情",
  // 默认给 0：表示「旧数据」，渲染时回退到 target/detail。
  // 本地化那条路径由专门的用例覆盖。
  item_count: 0,
  ok_count: 0,
  fail_count: 0,
  reason_code: "",
  // 旧数据没有诚实口径字段（迁移前默认 0，reclaimed 无从测量）。
  deleted_bytes: 0,
  trashed_bytes: 0,
  reclaimed_bytes: null,
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

  it("用结构化计数渲染本地化文本，而不是后端拼好的中文", async () => {
    // 英文界面此前整页显示中文（「3 项缓存」「成功 2 项，失败 1 项」）——
    // 历史是这个产品「可追溯」的核心页面，不该只有中文。
    mocks.getHistory.mockResolvedValue([
      {
        ...row(1, "cache"),
        item_count: 3,
        ok_count: 2,
        fail_count: 1,
        reason_code: "delete_failed",
      },
    ]);
    render(() => <HistoryView active />);
    // 本文件的 i18n mock 固定走 zh 词典，所以断言的是 zh 词条；
    // 关键是它**来自结构化计数**（3 项 / 2 成功 1 失败），而不是后端
    // 拼好的 `target`（"目标 1"）与 `detail`（"详情"）。
    await waitFor(() =>
      expect(document.body.textContent).toContain("3 项缓存"),
    );
    const text = document.body.textContent ?? "";
    expect(text).toContain("成功 2 项，失败 1 项");
    expect(text).not.toContain("目标 1");
    expect(text).not.toContain("详情");
  });

  it("计数为 1 时用单数变体（英文的 1 cache items 是语病）", async () => {
    mocks.getHistory.mockResolvedValue([
      { ...row(1, "cache"), item_count: 1, ok_count: 1, fail_count: 0, reason_code: "" },
    ]);
    render(() => <HistoryView active />);
    await waitFor(() =>
      expect(document.body.textContent).toContain("1 项缓存"),
    );
    // 复数模板是 "{count} 项缓存"，单数变体是 "1 项缓存" —— 两者在这里
    // 文案相同，所以再断言一次「没有走到复数模板」靠的是英文词条；
    // zh 下这条主要是防止单数分支抛错/取不到 key。
    expect(document.body.textContent).not.toContain("history.target");
  });

  it("旧数据（计数为 0）回退到后端原文，不猜", async () => {
    mocks.getHistory.mockResolvedValue([row(7, "cache")]);
    render(() => <HistoryView active />);
    await waitFor(() =>
      expect(document.body.textContent).toContain("目标 7"),
    );
  });

  it("卸载新纪录显示移入废纸篓，不显示为释放", async () => {
    // 卸载只是把文件移进废纸篓，空间**尚未释放**。历史页若照旧显示绿色的
    // 「+1.3 KB」，等于把「待清空的废纸篓」谎报成「已释放」。
    mocks.getHistory.mockResolvedValue([
      {
        ...row(1, "uninstall"),
        freed_bytes: 1300,
        item_count: 1,
        ok_count: 1,
        fail_count: 0,
        reason_code: "",
        deleted_bytes: 0,
        trashed_bytes: 1300,
        reclaimed_bytes: null,
      },
    ]);
    render(() => <HistoryView active />);
    await waitFor(() =>
      expect(document.body.textContent).toContain("废纸篓"),
    );
    const text = document.body.textContent ?? "";
    expect(text).toContain("1.3 KB");
    // 关键：不得出现绿色的「+」释放口径。
    expect(text).not.toContain("+1.3");
  });

  it("旧数据行仍显示 freed_bytes，回退行为与升级前一致", async () => {
    // item_count === 0 是迁移前的旧行，没有结构化口径，保持原来的 +freed_bytes。
    mocks.getHistory.mockResolvedValue([
      { ...row(1, "uninstall"), freed_bytes: 1300 },
    ]);
    render(() => <HistoryView active />);
    await waitFor(() =>
      expect(document.body.textContent).toContain("+1.3 KB"),
    );
  });

  it("缓存新行按实测/未测量分口径：实测显示 +，未测量只说已删除", async () => {
    // 未测量（reclaimed_bytes == null）绝不冒充实释放：不出现「+」，
    // 只报「已删除 X（未测量）」。
    mocks.getHistory.mockResolvedValue([
      {
        ...row(1, "cache"),
        item_count: 2,
        ok_count: 2,
        fail_count: 0,
        deleted_bytes: 2048,
        trashed_bytes: 0,
        reclaimed_bytes: null,
      },
    ]);
    render(() => <HistoryView active />);
    await waitFor(() =>
      expect(document.body.textContent).toContain("已删除 2.0 KB（未测量）"),
    );
    expect(document.body.textContent).not.toContain("+2.0");
  });

  it("缓存新行实测到回收量时显示绿色 +", async () => {
    mocks.getHistory.mockResolvedValue([
      {
        ...row(1, "cache"),
        item_count: 1,
        ok_count: 1,
        fail_count: 0,
        deleted_bytes: 1300,
        trashed_bytes: 0,
        reclaimed_bytes: 1300,
      },
    ]);
    render(() => <HistoryView active />);
    await waitFor(() =>
      expect(document.body.textContent).toContain("+1.3 KB"),
    );
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
