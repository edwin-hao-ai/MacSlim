import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  listen: vi.fn(),
  scanAll: vi.fn(),
  prepareOperation: vi.fn(),
  executeOperation: vi.fn(),
  addWhitelist: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

vi.mock("@/lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/tauri")>();
  return {
    ...actual,
    scanAll: mocks.scanAll,
    prepareOperation: mocks.prepareOperation,
    executeOperation: mocks.executeOperation,
    addWhitelist: mocks.addWhitelist,
  };
});

vi.mock("@/components/HealthCard", () => ({ default: () => <div /> }));
vi.mock("@/components/ProcessList", () => ({
  default: (props: {
    processes: {
      selection_key: string;
      label?: string;
      protected: boolean;
      protected_reason_key: string | null;
      whitelisted: boolean;
      default_select: boolean;
    }[];
    selected: Set<string>;
    onToggle: (key: string) => void;
  }) => (
    <div>
      {props.processes.map((process) => (
        <div>
          <span>{process.selection_key}</span>
          <span data-testid={`checked-${process.selection_key}`}>
            {props.selected.has(process.selection_key) ? "on" : "off"}
          </span>
          <span data-testid={`protected-${process.selection_key}`}>
            {process.protected ? (process.protected_reason_key ?? "protected") : "free"}
          </span>
          <button
            type="button"
            onClick={() => props.onToggle(process.selection_key)}
          >
            toggle-{process.selection_key}
          </button>
        </div>
      ))}
    </div>
  ),
}));
vi.mock("@/components/Welcome", () => ({ default: () => <div /> }));
vi.mock("@/components/CleanupFlash", () => ({ default: () => <div /> }));
vi.mock("@/lib/cleanFeedback", () => ({
  playCleanSuccessSound: vi.fn(),
}));
vi.mock("@/i18n", async () => {
  const { fakeI18n, passthroughT } = await import("@/i18n/fake-i18n");
  return { useI18n: () => fakeI18n({ t: passthroughT }) };
});

import ScanView from "@/views/ScanView";

type OverviewProcess = {
  selection_key: string;
  pid: number;
  name: string;
  exe: string;
  start_time: number;
  cpu_percent: number;
  memory_mb: number;
  kind: string;
  risk: string;
  default_select: boolean;
  reason_key: string;
  reason_params: [string, string][];
  ports: number[];
  icon_base64: null;
  protected: boolean;
  protected_reason_key: string | null;
  protected_reason_params: [string, string][];
  whitelisted: boolean;
};

const plainProcess: OverviewProcess = {
  selection_key: "key-a",
  pid: 11,
  name: "a",
  exe: "/a",
  start_time: 1,
  cpu_percent: 1,
  memory_mb: 1,
  kind: "zombie",
  risk: "safe",
  default_select: true,
  reason_key: "process.reason.highCpu",
  reason_params: [],
  ports: [],
  icon_base64: null,
  protected: false,
  protected_reason_key: null,
  protected_reason_params: [],
  whitelisted: false,
};

const protectedProcess: OverviewProcess = {
  ...plainProcess,
  selection_key: "key-protected",
  pid: 12,
  name: "Chrome",
  exe: "/Applications/Chrome.app/Contents/MacOS/Chrome",
  default_select: true,
  protected: true,
  protected_reason_key: "process.protect.parentOfOthers",
  protected_reason_params: [],
};

const whitelistedProcess: OverviewProcess = {
  ...plainProcess,
  selection_key: "key-whitelisted",
  pid: 13,
  name: "Guarded",
  default_select: true,
  protected: true,
  whitelisted: true,
  protected_reason_key: "process.protect.whitelisted",
  protected_reason_params: [],
};

const overviewEnvelope = (processes: OverviewProcess[] = [plainProcess]) => ({
  snapshot_id: "snap-overview",
  expires_at_ms: 1_700_000_000_000,
  value: { health: {}, processes, scanned_at_ms: 1 },
});

describe("ScanView event lifecycle", () => {
  beforeEach(() => {
    localStorage.clear();
    localStorage.setItem("macslim.welcome.seen", "true");
    mocks.listen.mockReset();
    mocks.scanAll.mockReset();
    mocks.prepareOperation.mockReset();
    mocks.executeOperation.mockReset();
    mocks.addWhitelist.mockReset();
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("unlistens both Tauri listeners on cleanup", async () => {
    const healthUnlisten = vi.fn().mockResolvedValue(undefined);
    const optimizeUnlisten = vi.fn().mockResolvedValue(undefined);
    mocks.listen
      .mockResolvedValueOnce(healthUnlisten)
      .mockResolvedValueOnce(optimizeUnlisten);
    const view = render(() => <ScanView />);

    await waitFor(() => expect(mocks.listen).toHaveBeenCalledTimes(2));
    view.unmount();

    await waitFor(() => {
      expect(healthUnlisten).toHaveBeenCalledTimes(1);
      expect(optimizeUnlisten).toHaveBeenCalledTimes(1);
    });
  });

  it("catches rejected unlisten promises", async () => {
    const healthUnlisten = vi.fn().mockRejectedValue(new Error("health unlisten failed"));
    const optimizeUnlisten = vi.fn().mockRejectedValue(new Error("optimize unlisten failed"));
    mocks.listen
      .mockResolvedValueOnce(healthUnlisten)
      .mockResolvedValueOnce(optimizeUnlisten);
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const view = render(() => <ScanView />);

    await waitFor(() => expect(mocks.listen).toHaveBeenCalledTimes(2));
    view.unmount();

    await waitFor(() => expect(errorSpy).toHaveBeenCalledTimes(2));
  });
});

describe("ScanView broker flow", () => {
  beforeEach(() => {
    localStorage.clear();
    localStorage.setItem("macslim.welcome.seen", "true");
    mocks.listen.mockReset();
    mocks.listen.mockResolvedValue(() => Promise.resolve());
    mocks.scanAll.mockReset();
    mocks.prepareOperation.mockReset();
    mocks.executeOperation.mockReset();
    mocks.addWhitelist.mockReset();
    mocks.scanAll.mockResolvedValue(overviewEnvelope());
    mocks.prepareOperation.mockResolvedValue({
      operation_id: "op-overview-1",
      kind: "process",
      expires_at_ms: 1,
      item_count: 1,
      estimated_bytes: 0,
      summary_key: "opSummary.processGraceful",
      summary_params: [["count", "1"]],
    });
    mocks.executeOperation.mockResolvedValue({
      kind: "process",
      value: { killed: [11], failed: [], details: [] },
    });
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("optimises the default selection through prepare, confirm and execute", async () => {
    render(() => <ScanView />);
    await screen.findByText("key-a");
    fireEvent.click(screen.getByRole("button", { name: /scan\.oneClick/ }));

    await screen.findByTestId("operation-confirm-validity");
    expect(mocks.executeOperation).not.toHaveBeenCalled();
    expect(screen.getByText("opSummary.processGraceful")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "opConfirm.confirm" }));
    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(mocks.prepareOperation).toHaveBeenCalledWith({
      type: "process",
      snapshot_id: "snap-overview",
      process_keys: ["key-a"],
      mode: "graceful",
    });
    expect(mocks.executeOperation).toHaveBeenCalledWith("op-overview-1");
  });

  it("never auto-selects a protected or whitelisted process", async () => {
    mocks.scanAll.mockResolvedValue(
      overviewEnvelope([plainProcess, protectedProcess, whitelistedProcess]),
    );
    render(() => <ScanView />);
    await screen.findByText("key-protected");

    expect(screen.getByTestId("checked-key-a").textContent).toBe("on");
    expect(screen.getByTestId("checked-key-protected").textContent).toBe("off");
    expect(screen.getByTestId("checked-key-whitelisted").textContent).toBe("off");
    expect(screen.getByTestId("protected-key-protected").textContent).toBe(
      "process.protect.parentOfOthers",
    );
  });

  it("asks for a force confirmation instead of a failing graceful batch", async () => {
    mocks.scanAll.mockResolvedValue(overviewEnvelope([protectedProcess]));
    render(() => <ScanView />);
    await screen.findByText("key-protected");
    fireEvent.click(screen.getByRole("button", { name: "toggle-key-protected" }));
    fireEvent.click(screen.getByRole("button", { name: /scan\.oneClick/ }));

    await screen.findByText("scan.confirmProtectedTitle");
    expect(mocks.prepareOperation).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("scan.forceTerminate"));
    await screen.findByTestId("operation-confirm-validity");
    expect(mocks.prepareOperation).toHaveBeenCalledWith({
      type: "process",
      snapshot_id: "snap-overview",
      process_keys: ["key-protected"],
      mode: "force",
    });
    expect(mocks.executeOperation).not.toHaveBeenCalled();
  });

  it("abandons the optimisation when the user cancels the confirmation", async () => {
    render(() => <ScanView />);
    await screen.findByText("key-a");
    fireEvent.click(screen.getByRole("button", { name: /scan\.oneClick/ }));
    await screen.findByTestId("operation-confirm-validity");

    fireEvent.click(screen.getByRole("button", { name: "common.cancel" }));

    expect(mocks.executeOperation).not.toHaveBeenCalled();
    expect(screen.queryByText("opConfirm.confirm")).toBeNull();
  });

  it("shows the irreversible warning for a process batch over 10GB", async () => {
    mocks.prepareOperation.mockResolvedValue({
      operation_id: "op-big",
      kind: "process",
      expires_at_ms: 1_700_000_600_000,
      item_count: 1,
      estimated_bytes: 11 * 1024 * 1024 * 1024,
      summary_key: "opSummary.processGraceful",
      summary_params: [["count", "1"]],
    });
    render(() => <ScanView />);
    await screen.findByText("key-a");
    fireEvent.click(screen.getByRole("button", { name: /scan\.oneClick/ }));

    expect(await screen.findByText("opConfirm.irreversible")).toBeTruthy();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
  });

  it("keeps raw pids off the wire and reports a stale overview snapshot", async () => {
    mocks.prepareOperation.mockRejectedValue({
      code: "operation_snapshot_stale",
      message: "操作所属快照已失效",
      params: [],
    });
    render(() => <ScanView />);
    await screen.findByText("key-a");
    fireEvent.click(screen.getByRole("button", { name: /scan\.oneClick/ }));

    await screen.findByText("opError.stale");
    expect(screen.queryByText("opConfirm.confirm")).toBeNull();
    const [request] = mocks.prepareOperation.mock.calls[0] as [
      Record<string, unknown>,
    ];
    expect(JSON.stringify(request)).not.toContain("11");
    expect(mocks.executeOperation).not.toHaveBeenCalled();
  });
});
