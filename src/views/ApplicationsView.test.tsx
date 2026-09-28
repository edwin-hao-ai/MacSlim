import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  listApplications: vi.fn(),
  prepareOperation: vi.fn(),
  executeOperation: vi.fn(),
}));

vi.mock("@/lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/tauri")>();
  return {
    ...actual,
    listApplications: mocks.listApplications,
    prepareOperation: mocks.prepareOperation,
    executeOperation: mocks.executeOperation,
  };
});

vi.mock("@/i18n", async () => {
  const { fakeI18n, passthroughT } = await import("@/i18n/fake-i18n");
  return { useI18n: () => fakeI18n({ t: passthroughT }) };
});

import ApplicationsView from "@/views/ApplicationsView";

type AppRow = {
  selection_key: string;
  bundle_path: string;
  name: string;
  bundle_id: string;
  icon_base64: null;
  main_pid: number;
  all_pids: number[];
  children: {
    selection_key: string;
    pid: number;
    parent_pid: number | null;
    name: string;
    exe: string;
    start_time: number;
    memory_mb: number;
    cpu_percent: number;
    ports: number[];
    is_main: boolean;
    depth: number;
    protected: boolean;
    protected_reason_key: string | null;
    protected_reason_params: [string, string][];
    whitelisted: boolean;
  }[];
  memory_mb: number;
  cpu_percent: number;
  uptime_secs: number;
  ports: number[];
  is_system: boolean;
  protected_process_count: number;
  whitelisted_process_count: number;
};

const plainApp: AppRow = {
  selection_key: "key-notes",
  bundle_path: "/Applications/Notes.app",
  name: "Notes",
  bundle_id: "com.apple.Notes",
  icon_base64: null,
  main_pid: 700,
  all_pids: [700],
  children: [
    {
      selection_key: "key-notes-child",
      pid: 700,
      parent_pid: null,
      name: "Notes",
      exe: "/Applications/Notes.app/Contents/MacOS/Notes",
      start_time: 10,
      memory_mb: 40,
      cpu_percent: 1,
      ports: [],
      is_main: true,
      depth: 0,
      protected: false,
      protected_reason_key: null,
      protected_reason_params: [],
      whitelisted: false,
    },
  ],
  memory_mb: 40,
  cpu_percent: 1,
  uptime_secs: 300,
  ports: [],
  is_system: false,
  protected_process_count: 0,
  whitelisted_process_count: 0,
};

const protectedApp: AppRow = {
  ...plainApp,
  selection_key: "key-chrome",
  bundle_path: "/Applications/Chrome.app",
  name: "Chrome",
  bundle_id: "com.google.Chrome",
  main_pid: 800,
  all_pids: [800, 801],
  protected_process_count: 2,
  whitelisted_process_count: 0,
};

const whitelistApp: AppRow = {
  ...plainApp,
  selection_key: "key-guarded",
  bundle_path: "/Applications/Guarded.app",
  name: "Guarded",
  bundle_id: "com.example.Guarded",
  main_pid: 900,
  all_pids: [900],
  protected_process_count: 1,
  whitelisted_process_count: 1,
};

const envelope = {
  snapshot_id: "snap-apps",
  expires_at_ms: 1_700_000_000_000,
  value: [plainApp, protectedApp, whitelistApp],
};

const prepared = {
  operation_id: "op-app-1",
  kind: "app_terminate",
  expires_at_ms: 1_700_000_600_000,
  item_count: 1,
  estimated_bytes: 0,
  summary_key: "opSummary.appGracefulQuit",
  summary_params: [
    ["apps", "1"],
    ["processes", "1"],
  ],
};

const terminateReport = {
  kind: "app_terminate",
  value: { killed: [700], failed: [], details: [] },
};

const gracefulReport = (quitError: string | null = null) => ({
  kind: "app_graceful_quit",
  value: [
    {
      app_name: "Notes",
      bundle_id: "com.apple.Notes",
      quit_error: quitError,
    },
  ],
});

const intervalHarness = () => {
  const handlers: Array<() => void> = [];
  const setSpy = vi
    .spyOn(window, "setInterval")
    .mockImplementation(((handler: () => void) => {
      handlers.push(handler);
      return handlers.length;
    }) as unknown as typeof window.setInterval);
  const clearSpy = vi
    .spyOn(window, "clearInterval")
    .mockImplementation(
      (() => undefined) as unknown as typeof window.clearInterval,
    );
  return { handlers, setSpy, clearSpy };
};

const confirmDialog = () => screen.findByTestId("operation-confirm-validity");
const confirmRun = async () =>
  fireEvent.click(await screen.findByRole("button", { name: "opConfirm.confirm" }));

const quitButtons = () =>
  screen.getAllByRole("button", { name: "app.quit" });
const forceButtons = () =>
  screen.getAllByRole("button", { name: "app.forceQuit" });

describe("ApplicationsView broker flow", () => {
  beforeEach(() => {
    mocks.listApplications.mockReset();
    mocks.prepareOperation.mockReset();
    mocks.executeOperation.mockReset();
    mocks.listApplications.mockResolvedValue(envelope);
    mocks.prepareOperation.mockResolvedValue(prepared);
    mocks.executeOperation.mockResolvedValue(terminateReport);
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("quits a plain app through the dedicated graceful operation", async () => {
    mocks.prepareOperation.mockResolvedValue({
      ...prepared,
      operation_id: "op-quit-1",
      kind: "app_graceful_quit",
    });
    mocks.executeOperation.mockResolvedValue(gracefulReport());

    render(() => <ApplicationsView />);
    await screen.findByText("Notes");
    fireEvent.click(quitButtons()[0]);
    await confirmDialog();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
    await confirmRun();

    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(mocks.prepareOperation).toHaveBeenCalledWith({
      type: "app_graceful_quit",
      snapshot_id: "snap-apps",
      app_keys: ["key-notes"],
    });
    expect(mocks.executeOperation).toHaveBeenCalledWith("op-quit-1");
    expect(mocks.listApplications).toHaveBeenCalledTimes(2);
  });

  it("never sends a graceful mode through the tree-termination request", async () => {
    mocks.prepareOperation.mockResolvedValue({
      ...prepared,
      operation_id: "op-quit-2",
      kind: "app_graceful_quit",
    });
    mocks.executeOperation.mockResolvedValue(gracefulReport());

    render(() => <ApplicationsView />);
    await screen.findByText("Notes");
    fireEvent.click(quitButtons()[0]);

    await confirmDialog();
    await waitFor(() => expect(mocks.prepareOperation).toHaveBeenCalled());
    const [request] = mocks.prepareOperation.mock.calls[0] as [
      Record<string, unknown>,
    ];
    expect(request.type).toBe("app_graceful_quit");
    expect("mode" in request).toBe(false);
  });

  it("quits a protected app gracefully without a confirmation dialog", async () => {
    mocks.prepareOperation.mockResolvedValue({
      ...prepared,
      operation_id: "op-quit-3",
      kind: "app_graceful_quit",
    });
    mocks.executeOperation.mockResolvedValue(gracefulReport());

    render(() => <ApplicationsView />);
    await screen.findByText("Chrome");
    fireEvent.click(quitButtons()[1]);
    await confirmDialog();
    expect(screen.queryByText("app.confirmForceTitle")).toBeNull();
    await confirmRun();

    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(mocks.prepareOperation).toHaveBeenCalledWith({
      type: "app_graceful_quit",
      snapshot_id: "snap-apps",
      app_keys: ["key-chrome"],
    });
    expect(screen.queryByText("app.confirmForceTitle")).toBeNull();
  });

  it("keeps graceful quit available for a protected app", async () => {
    render(() => <ApplicationsView />);
    await screen.findByText("Chrome");
    expect(quitButtons()[1].hasAttribute("disabled")).toBe(false);
    expect(quitButtons()[1].getAttribute("title")).toBe("app.quitTitle");
  });

  it("reports an app that sent no exit signal", async () => {
    mocks.prepareOperation.mockResolvedValue({
      ...prepared,
      operation_id: "op-quit-4",
      kind: "app_graceful_quit",
    });
    mocks.executeOperation.mockResolvedValue(
      gracefulReport("应用没有响应退出信号"),
    );

    render(() => <ApplicationsView />);
    await screen.findByText("Notes");
    fireEvent.click(quitButtons()[0]);
    await confirmRun();

    await screen.findByText("app.quitFailed");
  });

  it("surfaces an identity change from the graceful quit path", async () => {
    mocks.prepareOperation.mockRejectedValue({
      code: "app_bundle_id_changed",
      message: "应用 Notes 的 bundle ID 已变化，请重新扫描",
      params: [["app", "Notes"]],
    });
    render(() => <ApplicationsView />);
    await screen.findByText("Notes");
    fireEvent.click(quitButtons()[0]);

    await screen.findByText("opError.stale");
    expect(screen.queryByText("opConfirm.confirm")).toBeNull();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
  });

  it("surfaces an AppleScript failure from the graceful quit path", async () => {
    mocks.prepareOperation.mockResolvedValue({
      ...prepared,
      operation_id: "op-quit-5",
      kind: "app_graceful_quit",
    });
    mocks.executeOperation.mockResolvedValue(gracefulReport("osascript 退出码 1"));

    render(() => <ApplicationsView />);
    await screen.findByText("Notes");
    fireEvent.click(quitButtons()[0]);
    await confirmRun();

    await screen.findByText("app.quitFailed");
  });

  it("keeps bundle paths, pids and child names off the wire", async () => {
    mocks.prepareOperation.mockResolvedValue({
      ...prepared,
      operation_id: "op-quit-7",
      kind: "app_graceful_quit",
    });
    mocks.executeOperation.mockResolvedValue(gracefulReport());

    render(() => <ApplicationsView />);
    await screen.findByText("Notes");
    fireEvent.click(quitButtons()[0]);

    await confirmDialog();
    await waitFor(() => expect(mocks.prepareOperation).toHaveBeenCalled());
    const [request] = mocks.prepareOperation.mock.calls[0] as [
      Record<string, unknown>,
    ];
    expect(Object.keys(request).sort()).toEqual([
      "app_keys",
      "snapshot_id",
      "type",
    ]);
    const payload = JSON.stringify(request);
    expect(payload).not.toContain("/Applications/Notes.app");
    expect(payload).not.toContain("700");
    expect(payload).not.toContain("osascript");
  });

  it("blocks both actions for an app with whitelisted processes", async () => {
    render(() => <ApplicationsView />);
    await screen.findByText("Guarded");
    const quit = quitButtons();
    const force = forceButtons();
    expect(quit[2].hasAttribute("disabled")).toBe(true);
    expect(quit[2].getAttribute("title")).toBe("app.whitelistLocked");
    expect(force[2].hasAttribute("disabled")).toBe(true);
  });

  it("asks for confirmation before forcing a protected app", async () => {
    render(() => <ApplicationsView />);
    await screen.findByText("Chrome");
    fireEvent.click(forceButtons()[1]);

    await screen.findByText("app.confirmForceTitle");
    expect(mocks.prepareOperation).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("app.confirmForce"));
    await confirmDialog();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
    await confirmRun();
    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(mocks.prepareOperation).toHaveBeenCalledWith({
      type: "app_terminate",
      snapshot_id: "snap-apps",
      app_keys: ["key-chrome"],
      mode: "force",
    });
  });

  it("shows the backend summary and validity before quitting an app", async () => {
    render(() => <ApplicationsView />);
    await screen.findByText("Notes");
    fireEvent.click(quitButtons()[0]);

    expect(await screen.findByText("opSummary.appGracefulQuit")).toBeTruthy();
    expect(screen.getByText("opConfirm.items")).toBeTruthy();
    expect(screen.getByText("opConfirm.estimated")).toBeTruthy();
  });

  it("never quits an app when the user cancels the confirmation", async () => {
    render(() => <ApplicationsView />);
    await screen.findByText("Notes");
    fireEvent.click(quitButtons()[0]);
    await confirmDialog();

    fireEvent.click(screen.getByRole("button", { name: "common.cancel" }));

    expect(mocks.executeOperation).not.toHaveBeenCalled();
    expect(screen.queryByText("opConfirm.confirm")).toBeNull();
  });

  it("suspends polling while the force-quit dialog is open", async () => {
    const { handlers, clearSpy } = intervalHarness();
    render(() => <ApplicationsView />);
    const poll = [...handlers];
    expect(poll).toHaveLength(1);

    poll.forEach((handler) => handler());
    await waitFor(() =>
      expect(mocks.listApplications).toHaveBeenCalledTimes(2),
    );

    fireEvent.click(forceButtons()[1]);
    await screen.findByText("app.confirmForceTitle");
    expect(clearSpy).toHaveBeenCalled();

    poll.forEach((handler) => handler());
    expect(mocks.listApplications).toHaveBeenCalledTimes(2);
  });

  it("reports a stale snapshot and reloads without executing", async () => {
    mocks.prepareOperation.mockRejectedValue({
      code: "operation_snapshot_stale",
      message: "操作所属快照已失效",
      params: [],
    });
    render(() => <ApplicationsView />);
    await screen.findByText("Notes");
    fireEvent.click(quitButtons()[0]);

    await screen.findByText("opError.stale");
    expect(mocks.executeOperation).not.toHaveBeenCalled();
    await waitFor(() => expect(mocks.listApplications).toHaveBeenCalledTimes(2));
  });

  it("unwraps the tagged app_graceful_quit result", async () => {
    mocks.prepareOperation.mockResolvedValue({
      ...prepared,
      operation_id: "op-quit-6",
      kind: "app_graceful_quit",
    });
    mocks.executeOperation.mockResolvedValue(gracefulReport());

    render(() => <ApplicationsView />);
    await screen.findByText("Notes");
    fireEvent.click(quitButtons()[0]);
    await confirmRun();

    await screen.findByText("app.quitSuccess");
  });

  it("ignores a graceful result tagged as another operation kind", async () => {
    mocks.executeOperation.mockResolvedValue({
      kind: "cache",
      value: { reports: [], total_freed_bytes: 0, success_count: 0, fail_count: 0 },
    });
    render(() => <ApplicationsView />);
    await screen.findByText("Notes");
    fireEvent.click(quitButtons()[0]);
    await confirmRun();

    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(screen.queryByText("app.quitSuccess")).toBeNull();
    expect(screen.queryByText("app.quitFailed")).toBeNull();
  });
});
