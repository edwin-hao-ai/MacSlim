import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// 假词典的 `t` 把 key 与参数原样拼出来，于是断言可以直接看「渲染时用的是哪枚
// key、带的是哪些参数」—— 中文译文是否存在由 i18n 门禁单独守。
vi.mock("@/i18n", async () => {
  const { fakeI18n } = await import("@/i18n/fake-i18n");
  return {
    useI18n: () =>
      fakeI18n({
        t: (key: string, params?: Record<string, string | number>) =>
          params
            ? `${key}:${Object.values(params)
                .map((value) => String(value))
                .join(",")}`
            : key,
      }),
  };
});

import OperationConfirm, {
  IRREVERSIBLE_THRESHOLD_BYTES,
  ProtectedForceConfirm,
} from "@/components/OperationConfirm";

const prepared = (overrides: Record<string, unknown> = {}) => ({
  operation_id: "op-1",
  kind: "cache",
  expires_at_ms: Date.now() + 600_000,
  item_count: 3,
  estimated_bytes: 2_048,
  summary_key: "opSummary.cache",
  summary_params: [
    ["count", "3"],
    ["size", "2.0 KB"],
  ] as [string, string][],
  ...overrides,
});

describe("OperationConfirm", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date("2026-09-26T00:00:00Z"));
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("renders the backend summary, estimate, item count and validity window", () => {
    render(() => (
      <OperationConfirm
        prepared={prepared()}
        title="确认清理"
        confirmLabel="确认执行"
        onConfirm={() => undefined}
        onCancel={() => undefined}
      />
    ));

    // 摘要来自后端的 key + 参数，不是后端拼好的句子
    expect(screen.getByText("opSummary.cache:3,2.0 KB")).toBeTruthy();
    expect(screen.getByText("opConfirm.items:3")).toBeTruthy();
    expect(screen.getByText("opConfirm.estimated:2.0 KB")).toBeTruthy();
    expect(
      screen.getByTestId("operation-confirm-validity").textContent,
    ).toBe("opConfirm.validity:10");
  });

  it("computes the remaining validity in minutes from expires_at_ms", () => {
    render(() => (
      <OperationConfirm
        prepared={prepared({ expires_at_ms: Date.now() + 7 * 60_000 })}
        title="确认清理"
        confirmLabel="确认执行"
        onConfirm={() => undefined}
        onCancel={() => undefined}
      />
    ));

    expect(screen.getByTestId("operation-confirm-validity").textContent).toBe(
      "opConfirm.validity:7",
    );
  });

  it("shows a zero validity window instead of a negative one", () => {
    render(() => (
      <OperationConfirm
        prepared={prepared({ expires_at_ms: Date.now() - 60_000 })}
        title="确认清理"
        confirmLabel="确认执行"
        onConfirm={() => undefined}
        onCancel={() => undefined}
      />
    ));

    expect(screen.getByTestId("operation-confirm-validity").textContent).toBe(
      "opConfirm.validity:0",
    );
  });

  it("does not claim irreversibility for a small estimate", () => {
    render(() => (
      <OperationConfirm
        prepared={prepared({ estimated_bytes: IRREVERSIBLE_THRESHOLD_BYTES - 1 })}
        title="确认清理"
        confirmLabel="确认执行"
        onConfirm={() => undefined}
        onCancel={() => undefined}
      />
    ));

    expect(screen.queryByText("opConfirm.irreversible")).toBeNull();
    expect(screen.queryByText("opConfirm.largeWarning")).toBeNull();
  });

  it("switches to the irreversible warning above 10GB of backend estimate", () => {
    render(() => (
      <OperationConfirm
        prepared={prepared({
          estimated_bytes: IRREVERSIBLE_THRESHOLD_BYTES + 1,
        })}
        title="确认清理"
        confirmLabel="确认执行"
        onConfirm={() => undefined}
        onCancel={() => undefined}
      />
    ));

    expect(screen.getByText("opConfirm.irreversible")).toBeTruthy();
    expect(screen.getByText("opConfirm.largeWarning")).toBeTruthy();
  });

  it("honours an explicit irreversible notice even for a small estimate", () => {
    render(() => (
      <OperationConfirm
        prepared={prepared({ estimated_bytes: 0 })}
        title="确认清理"
        confirmLabel="确认执行"
        notice="docker.pruneIrreversible"
        onConfirm={() => undefined}
        onCancel={() => undefined}
      />
    ));

    expect(screen.getByText("docker.pruneIrreversible")).toBeTruthy();
  });

  it("only executes after the user presses the confirm button", () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(() => (
      <OperationConfirm
        prepared={prepared()}
        title="确认清理"
        confirmLabel="确认执行"
        onConfirm={onConfirm}
        onCancel={onCancel}
      />
    ));

    expect(onConfirm).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText("确认执行"));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it("abandons the operation when the user cancels", () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(() => (
      <OperationConfirm
        prepared={prepared()}
        title="确认清理"
        confirmLabel="确认执行"
        onConfirm={onConfirm}
        onCancel={onCancel}
      />
    ));

    fireEvent.click(screen.getByText("common.cancel"));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });
});

describe("ProtectedForceConfirm", () => {
  const rows = [
    {
      name: "Chrome",
      pid: 800,
      protected_reason_key: "process.protect.multiProcessComponent",
      protected_reason_params: [],
    },
    { name: "Slack", pid: 801, protected_reason_key: null },
  ];

  afterEach(() => {
    cleanup();
  });

  it("lists every protected row with its reason and waits for confirmation", () => {
    const onConfirm = vi.fn();
    render(() => (
      <ProtectedForceConfirm
        title="process.confirmProtectedTitle"
        message="process.confirmProtectedMessage"
        confirmLabel="process.forceTerminate"
        cancelLabel="common.cancel"
        rows={rows}
        onConfirm={onConfirm}
        onCancel={() => undefined}
      />
    ));

    expect(screen.getByText("Chrome")).toBeTruthy();
    expect(
      screen.getByText("process.protect.multiProcessComponent"),
    ).toBeTruthy();
    expect(screen.getByText("Slack")).toBeTruthy();
    expect(onConfirm).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("process.forceTerminate"));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });
});
