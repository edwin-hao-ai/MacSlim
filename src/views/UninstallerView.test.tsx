import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type StageUpdate = {
  stage: string;
  state: "running" | "done";
  item_count: number;
  found_bytes: number;
};

const mocks = vi.hoisted(() => ({
  scanInstalledApps: vi.fn(),
  scanAppResiduesBatch: vi.fn(),
  prepareOperation: vi.fn(),
  executeOperation: vi.fn(),
  checkAppRunning: vi.fn(),
  onResidueScanProgress: vi.fn(),
  // 后端逐应用事件的手动触发入口，测试靠它驱动真实进度
  residueHandlers: [] as Array<(u: StageUpdate) => void>,
  unlisten: vi.fn(),
}));

vi.mock("@/lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/tauri")>();
  return {
    ...actual,
    scanInstalledApps: mocks.scanInstalledApps,
    scanAppResiduesBatch: mocks.scanAppResiduesBatch,
    prepareOperation: mocks.prepareOperation,
    executeOperation: mocks.executeOperation,
    checkAppRunning: mocks.checkAppRunning,
    onResidueScanProgress: mocks.onResidueScanProgress,
  };
});

vi.mock("@/i18n", async () => {
  // scanProgress 是带占位符文案族（{stage}/{done}/{total}/{size}），
  // 必须插值才能断言真实阶段名与计数；其余 key 保持原样，
  // 这样既有的精确文本断言一个字符都不用改。
  const { fakeI18n } = await import("@/i18n/fake-i18n");
  return {
    useI18n: () =>
      fakeI18n({
        t: (key: string, p?: Record<string, string | number>) =>
          p && key.startsWith("scanProgress.")
            ? `${key}:${JSON.stringify(p)}`
            : key,
      }),
  };
});

import UninstallerView from "@/views/UninstallerView";

type InstalledApp = {
  selection_key: string;
  bundle_path: string;
  name: string;
  bundle_id: string;
  icon_base64: null;
  bundle_size_bytes: number;
  is_system: boolean;
  is_running: boolean;
  estimated_residue_bytes: number;
};

const notesApp: InstalledApp = {
  selection_key: "key-notes",
  bundle_path: "/Applications/Notes.app",
  name: "Notes",
  bundle_id: "com.apple.Notes",
  icon_base64: null,
  bundle_size_bytes: 1_000,
  is_system: false,
  is_running: false,
  estimated_residue_bytes: 0,
};

const demoApp: InstalledApp = {
  selection_key: "key-demo",
  bundle_path: "/Applications/Demo.app",
  name: "Demo",
  bundle_id: "com.example.Demo",
  icon_base64: null,
  bundle_size_bytes: 2_000,
  is_system: false,
  is_running: true,
  estimated_residue_bytes: 0,
};

const appsEnvelope = {
  snapshot_id: "snap-installed",
  expires_at_ms: 1_700_000_000_000,
  value: [notesApp, demoApp],
};

const ALL_GROUPS = [
    {
      app_key: "key-notes",
      residue: {
        bundle_id: "com.apple.Notes",
        app_name: "Notes",
        items: [
          {
            selection_key: "res-key-1",
            path: "/Users/tester/Library/Containers/com.apple.Notes",
            category: "Containers",
            size_bytes: 300,
            is_dev_tool: false,
            selected: true,
          },
        ],
        total_bytes: 300,
        scan_complete: true,
      },
    },
    {
      app_key: "key-demo",
      residue: {
        bundle_id: "com.example.Demo",
        app_name: "Demo",
        items: [
          {
            selection_key: "res-key-2",
            path: "/Users/tester/Library/Preferences/com.example.Demo.plist",
            category: "Preferences",
            size_bytes: 400,
            is_dev_tool: false,
            selected: true,
          },
        ],
        total_bytes: 400,
        scan_complete: true,
      },
    },
  ];

const residueEnvelope = (appKeys: string[]) => ({
  snapshot_id: "snap-residue",
  expires_at_ms: 1_700_000_000_000,
  value: ALL_GROUPS.filter((group) => appKeys.includes(group.app_key)),
});

const prepared = {
  operation_id: "op-uninstall-1",
  kind: "uninstall",
  expires_at_ms: 1_700_000_600_000,
  item_count: 2,
  estimated_bytes: 1_700,
  summary_key: "opSummary.uninstall",
  summary_params: [
    ["apps", "1"],
    ["residues", "1"],
    ["size", "128 B"],
  ],
};

const uninstallResult = (quitError: string | null = null) => ({
  kind: "uninstall",
  value: [
    {
      app_name: "Notes",
      bundle_id: "com.apple.Notes",
      total_freed_bytes: 1_300,
      moved_count: 2,
      failed_count: 0,
      details: [],
      quit_error: quitError,
    },
  ],
});

const confirmDialog = () => screen.findByTestId("operation-confirm-validity");
const confirmRun = async () =>
  fireEvent.click(await screen.findByRole("button", { name: "opConfirm.confirm" }));

const appCheckbox = async (index: number) => {
  let target: HTMLInputElement | undefined;
  await waitFor(() => {
    const boxes = screen.getAllByRole("checkbox");
    expect(boxes.length).toBeGreaterThanOrEqual(3);
    target = boxes[boxes.length - 2 + index] as HTMLInputElement;
  });
  return target!;
};

const enterResiduePhase = async (opts?: { holdScan?: boolean }) => {
  fireEvent.click(await appCheckbox(0));
  fireEvent.click(
    screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
  );
  // holdScan：让调用方把断言落在「残留扫描中」这一段，扫描本身由调用方控制何时结束
  if (opts?.holdScan) return;
  await screen.findByText("Notes");
};

describe("UninstallerView broker flow", () => {
  beforeEach(() => {
    mocks.scanInstalledApps.mockReset();
    mocks.scanAppResiduesBatch.mockReset();
    mocks.prepareOperation.mockReset();
    mocks.executeOperation.mockReset();
    mocks.checkAppRunning.mockReset();
    mocks.onResidueScanProgress.mockReset();
    mocks.residueHandlers.length = 0;
    mocks.unlisten.mockReset();
    mocks.scanInstalledApps.mockResolvedValue(appsEnvelope);
    mocks.scanAppResiduesBatch.mockImplementation((_id: string, keys: string[]) =>
      Promise.resolve(residueEnvelope(keys)),
    );
    mocks.prepareOperation.mockResolvedValue(prepared);
    mocks.executeOperation.mockResolvedValue(uninstallResult());
    mocks.checkAppRunning.mockResolvedValue(false);
    // 默认注册一个立即返回的监听；个别测试会覆盖成「手动 resolve」来制造竞态
    mocks.onResidueScanProgress.mockImplementation(
      async (cb: (u: StageUpdate) => void) => {
        mocks.residueHandlers.push(cb);
        return mocks.unlisten;
      },
    );
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("registers the whole selected app batch in one residue scan", async () => {
    render(() => <UninstallerView />);
    fireEvent.click(await appCheckbox(0));
    fireEvent.click(await appCheckbox(1));
    fireEvent.click(
      screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
    );

    await waitFor(() => expect(mocks.scanAppResiduesBatch).toHaveBeenCalled());
    expect(mocks.scanAppResiduesBatch).toHaveBeenCalledTimes(1);
    expect(mocks.scanAppResiduesBatch).toHaveBeenCalledWith("snap-installed", [
      "key-notes",
      "key-demo",
    ]);
  });

  it("prepares one uninstall from both snapshots and executes it", async () => {
    render(() => <UninstallerView />);
    await enterResiduePhase();
    fireEvent.click(
      screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
    );
    await confirmRun();

    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(mocks.prepareOperation).toHaveBeenCalledWith({
      type: "uninstall",
      app_snapshot_id: "snap-installed",
      residue_snapshot_id: "snap-residue",
      app_keys: ["key-notes"],
      residue_keys: ["res-key-1"],
      quit_running: false,
    });
    expect(mocks.executeOperation).toHaveBeenCalledWith("op-uninstall-1");
  });

  it("keeps residue paths and bundle paths off the wire", async () => {
    render(() => <UninstallerView />);
    await enterResiduePhase();
    fireEvent.click(
      screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
    );
    await confirmDialog();

    await waitFor(() => expect(mocks.prepareOperation).toHaveBeenCalled());
    const [request] = mocks.prepareOperation.mock.calls[0] as [
      Record<string, unknown>,
    ];
    expect(Object.keys(request).sort()).toEqual([
      "app_keys",
      "app_snapshot_id",
      "quit_running",
      "residue_keys",
      "residue_snapshot_id",
      "type",
    ]);
    const payload = JSON.stringify(request);
    expect(payload).not.toContain("/Users/tester/Library");
    expect(payload).not.toContain("/Applications/Notes.app");
  });

  it("waits for the user before executing a prepared uninstall", async () => {
    render(() => <UninstallerView />);
    await enterResiduePhase();
    fireEvent.click(
      screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
    );

    await confirmDialog();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
    expect(screen.getByText("opSummary.uninstall")).toBeTruthy();
    expect(screen.getByText("opConfirm.items")).toBeTruthy();
    expect(screen.getByText("opConfirm.estimated")).toBeTruthy();
  });

  it("never uninstalls when the user cancels the confirmation", async () => {
    render(() => <UninstallerView />);
    await enterResiduePhase();
    fireEvent.click(
      screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
    );
    await confirmDialog();

    fireEvent.click(screen.getByRole("button", { name: "uninstaller.cancel" }));

    expect(mocks.executeOperation).not.toHaveBeenCalled();
    expect(screen.queryByText("opConfirm.confirm")).toBeNull();
  });

  it("gates the irreversible warning on the backend estimate above 10GB", async () => {
    mocks.prepareOperation.mockResolvedValue({
      ...prepared,
      estimated_bytes: 11 * 1024 * 1024 * 1024,
    });
    render(() => <UninstallerView />);
    await enterResiduePhase();
    fireEvent.click(
      screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
    );

    expect(await screen.findByText("opConfirm.irreversible")).toBeTruthy();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
  });

  it("never shows the irreversible warning below 10GB", async () => {
    render(() => <UninstallerView />);
    await enterResiduePhase();
    fireEvent.click(
      screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
    );

    await confirmDialog();
    expect(screen.queryByText("opConfirm.irreversible")).toBeNull();
  });

  it("forces quit_running for a running app", async () => {
    mocks.checkAppRunning.mockResolvedValue(true);
    render(() => <UninstallerView />);
    fireEvent.click(await appCheckbox(1));
    fireEvent.click(
      screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
    );
    await waitFor(() => expect(mocks.scanAppResiduesBatch).toHaveBeenCalled());
    fireEvent.click(
      screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
    );
    fireEvent.click(await screen.findByText("uninstaller.quitAndUninstall"));
    await confirmRun();

    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(mocks.prepareOperation).toHaveBeenCalledWith(
      expect.objectContaining({
        app_keys: ["key-demo"],
        residue_keys: ["res-key-2"],
        quit_running: true,
      }),
    );
  });

  it("renders the tagged uninstall reports", async () => {
    render(() => <UninstallerView />);
    await enterResiduePhase();
    fireEvent.click(
      screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
    );
    await confirmRun();

    await screen.findByText("uninstaller.complete");
  });

  it("shows that the app was removed even when quit failed", async () => {
    mocks.executeOperation.mockResolvedValue(
      uninstallResult("应用拒绝退出信号"),
    );
    render(() => <UninstallerView />);
    await enterResiduePhase();
    fireEvent.click(
      screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
    );
    await confirmRun();

    await screen.findByText("uninstaller.quitFailedButRemoved");
  });

  it("reports stale residue snapshots without executing", async () => {
    mocks.prepareOperation.mockRejectedValue({
      code: "snapshot_stale",
      message: "快照不存在或已失效",
      params: [],
    });
    render(() => <UninstallerView />);
    await enterResiduePhase();
    fireEvent.click(
      screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
    );
    await screen.findByText("opError.stale");
    expect(screen.queryByText("opConfirm.confirm")).toBeNull();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
  });

  it("re-scans installed apps after a finished uninstall", async () => {
    render(() => <UninstallerView />);
    await enterResiduePhase();
    fireEvent.click(
      screen.getByRole("button", { name: "uninstaller.uninstallSelected" }),
    );
    await confirmRun();
    await screen.findByText("uninstaller.complete");
    fireEvent.click(screen.getByText("common.back"));

    await waitFor(() => expect(mocks.scanInstalledApps).toHaveBeenCalledTimes(2));
  });

  it("残留扫描期间显示逐应用阶段与已完成计数", async () => {
    mocks.scanAppResiduesBatch.mockImplementation(() => new Promise(() => {}));
    render(() => <UninstallerView />);
    await enterResiduePhase({ holdScan: true });
    await waitFor(() => expect(mocks.residueHandlers.length).toBeGreaterThan(0));

    mocks.residueHandlers[0]({
      stage: "Google Chrome",
      state: "done",
      item_count: 3,
      found_bytes: 1024,
    });

    // 阶段名原样来自后端，不做任何映射
    expect(screen.getByTestId("scan-stage-name").textContent).toContain(
      "Google Chrome",
    );
    expect(screen.getByTestId("scan-stage-count").textContent).toContain("1");
    // total 接的是选中 app 数（此例 1 个），所以进度条必须在
    expect(screen.getByTestId("scan-stage-bar")).toBeTruthy();
    // fmtBytes(1024) === "1.0 KB"（KB 一位小数），与 App 其余位置同源
    expect(screen.getByTestId("scan-stage-bytes").textContent).toContain("1.0 KB");
  });

  it("残留扫描挂起时卸载组件也会调用 unlisten", async () => {
    mocks.scanAppResiduesBatch.mockImplementation(() => new Promise(() => {}));
    const { unmount } = render(() => <UninstallerView />);
    await enterResiduePhase({ holdScan: true });
    await waitFor(() => expect(mocks.residueHandlers.length).toBeGreaterThan(0));
    expect(mocks.unlisten).not.toHaveBeenCalled();

    unmount();

    expect(mocks.unlisten).toHaveBeenCalled();
  });
});
