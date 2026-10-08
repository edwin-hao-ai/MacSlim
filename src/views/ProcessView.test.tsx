import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  listAllProcesses: vi.fn(),
  prepareOperation: vi.fn(),
  executeOperation: vi.fn(),
  addWhitelist: vi.fn(),
}));

vi.mock("@/lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/tauri")>();
  return {
    ...actual,
    listAllProcesses: mocks.listAllProcesses,
    prepareOperation: mocks.prepareOperation,
    executeOperation: mocks.executeOperation,
    addWhitelist: mocks.addWhitelist,
  };
});

vi.mock("@/i18n", async () => {
  const { fakeI18n, passthroughT } = await import("@/i18n/fake-i18n");
  return { useI18n: () => fakeI18n({ t: passthroughT }) };
});

import ProcessView from "@/views/ProcessView";

type Row = {
  selection_key: string;
  pid: number;
  parent_pid: number | null;
  name: string;
  full_name: string;
  exe: string;
  start_time: number;
  cpu_percent: number;
  memory_mb: number;
  uptime_secs: number;
  name_key: string | null;
  status_key: string;
  ports: number[];
  icon_base64: null;
  protected: boolean;
  protected_reason_key: string | null;
  protected_reason_params: [string, string][];
  whitelisted: boolean;
};

const normalRow: Row = {
  selection_key: "key-normal",
  pid: 4242,
  parent_pid: null,
  name: "sleepy",
  full_name: "sleepy",
  exe: "/usr/bin/sleepy",
  start_time: 100,
  cpu_percent: 1,
  memory_mb: 20,
  uptime_secs: 900,
  name_key: null,
  status_key: "process.status.run",
  ports: [],
  icon_base64: null,
  protected: false,
  protected_reason_key: null,
  protected_reason_params: [],
  whitelisted: false,
};

const protectedRow: Row = {
  selection_key: "key-protected",
  pid: 5151,
  parent_pid: null,
  name: "Chrome",
  full_name: "Chrome",
  exe: "/Applications/Chrome.app/Contents/MacOS/Chrome",
  start_time: 200,
  cpu_percent: 5,
  memory_mb: 10,
  uptime_secs: 1200,
  name_key: null,
  status_key: "process.status.run",
  ports: [9222],
  icon_base64: null,
  protected: true,
  protected_reason_key: "process.protect.parentOfOthers",
  protected_reason_params: [],
  whitelisted: false,
};

const whitelistRow: Row = {
  selection_key: "key-whitelisted",
  pid: 6161,
  parent_pid: null,
  name: "Guarded",
  full_name: "Guarded",
  exe: "/Applications/Guarded.app/Contents/MacOS/Guarded",
  start_time: 300,
  cpu_percent: 2,
  memory_mb: 5,
  uptime_secs: 5000,
  name_key: null,
  status_key: "process.status.run",
  ports: [],
  icon_base64: null,
  protected: true,
  protected_reason_key: "process.protect.whitelisted",
  protected_reason_params: [],
  whitelisted: true,
};

/** 真的 Dock：原始名就是 `Dock`，受保护且在白名单里，勾选框被锁。 */
const realDockRow: Row = {
  selection_key: "key-dock",
  pid: 7001,
  parent_pid: null,
  name: "Dock",
  full_name: "Dock",
  exe: "/System/Library/CoreServices/Dock.app/Contents/MacOS/Dock",
  start_time: 400,
  cpu_percent: 0.4,
  memory_mb: 40,
  uptime_secs: 9000,
  name_key: null,
  status_key: "process.status.run",
  ports: [],
  icon_base64: null,
  protected: true,
  protected_reason_key: "process.protect.whitelisted",
  protected_reason_params: [],
  whitelisted: true,
};

/** 撞名行：展示名同样落成 `Dock`，但原始名带 bundle id 后缀，且不受保护。 */
const dockExtraRow: Row = {
  selection_key: "key-dock-extra",
  pid: 7002,
  parent_pid: 7001,
  name: "Dock",
  full_name: "com.apple.dock.extra",
  exe: "/usr/libexec/com.apple.dock.extra",
  start_time: 400,
  cpu_percent: 0.2,
  memory_mb: 8,
  uptime_secs: 9000,
  name_key: null,
  status_key: "process.status.run",
  ports: [],
  icon_base64: null,
  protected: false,
  protected_reason_key: null,
  protected_reason_params: [],
  whitelisted: false,
};

const envelope = (rows: Row[]) => ({
  snapshot_id: "snap-proc",
  expires_at_ms: 1_700_000_000_000,
  value: rows,
});

/**
 * 行的 6 个格子（索引固定：0 名称 / 1 CPU / 2 内存 / 3 运行 / 4 PID / 5 操作）。
 * jsdom 没有布局引擎，行高对齐只能靠「结构 + 类名」锁，见 I-1 用例。
 */
const rowCell = (index: number, row = 0): HTMLElement => {
  const list = screen.getAllByTestId("process-row");
  return list[row].children[index] as HTMLElement;
};

/** 名称格里真正渲染出来的副标题行（名称行 / 原始名 / 受保护原因 / exe）。 */
const nameCellRows = (row = 0): string[] =>
  Array.from(within(rowCell(0, row)).getByTestId("process-row-name").children).map(
    (el) => (el as HTMLElement).textContent ?? "",
  );

const prepared = {
  operation_id: "op-proc-1",
  kind: "process",
  expires_at_ms: 1_700_000_600_000,
  item_count: 1,
  estimated_bytes: 0,
  summary_key: "opSummary.processGraceful",
  summary_params: [["count", "1"]],
};

const killReport = {
  kind: "process",
  value: { killed: [4242], failed: [], details: [] },
};

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

const selectRow = async (index: number) => {
  const boxes = await screen.findAllByRole("checkbox");
  fireEvent.click(boxes[index]);
};

const terminate = () =>
  fireEvent.click(screen.getByRole("button", { name: "process.terminateSelected" }));

const confirmDialog = () => screen.findByTestId("operation-confirm-validity");
const confirmRun = async () =>
  fireEvent.click(await screen.findByRole("button", { name: "opConfirm.confirm" }));

describe("ProcessView broker flow", () => {
  beforeEach(() => {
    mocks.listAllProcesses.mockReset();
    mocks.prepareOperation.mockReset();
    mocks.executeOperation.mockReset();
    mocks.addWhitelist.mockReset();
    mocks.listAllProcesses.mockResolvedValue(
      envelope([normalRow, protectedRow, whitelistRow]),
    );
    mocks.prepareOperation.mockResolvedValue(prepared);
    mocks.executeOperation.mockResolvedValue(killReport);
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("enables the terminate action only after a key-based selection", async () => {
    render(() => <ProcessView />);
    const cta = await screen.findByRole("button", {
      name: "process.terminateSelected",
    });
    expect(cta.hasAttribute("disabled")).toBe(true);
    await selectRow(0);
    await waitFor(() =>
      expect(cta.hasAttribute("disabled")).toBe(false),
    );
  });

  it("prepares graceful for unprotected rows and refreshes after execute", async () => {
    render(() => <ProcessView />);
    await selectRow(0);
    terminate();

    await confirmDialog();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
    await confirmRun();
    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(mocks.prepareOperation).toHaveBeenCalledWith({
      type: "process",
      snapshot_id: "snap-proc",
      process_keys: ["key-normal"],
      mode: "graceful",
    });
    expect(mocks.executeOperation).toHaveBeenCalledWith("op-proc-1");
    expect(mocks.listAllProcesses).toHaveBeenCalledTimes(2);
  });

  it("keeps raw pid, exe and protection flags off the wire", async () => {
    render(() => <ProcessView />);
    await selectRow(0);
    terminate();

    await confirmDialog();
    await waitFor(() => expect(mocks.prepareOperation).toHaveBeenCalled());
    const [request] = mocks.prepareOperation.mock.calls[0] as [
      Record<string, unknown>,
    ];
    expect(Object.keys(request).sort()).toEqual([
      "mode",
      "process_keys",
      "snapshot_id",
      "type",
    ]);
    const payload = JSON.stringify(request);
    expect(payload).not.toContain("4242");
    expect(payload).not.toContain("/usr/bin/sleepy");
  });

  it("requires force mode for protected rows", async () => {
    render(() => <ProcessView />);
    await selectRow(1);
    terminate();

    await screen.findByText("process.confirmProtectedTitle");
    expect(mocks.prepareOperation).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("process.forceTerminate"));
    await confirmDialog();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
    await confirmRun();
    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(mocks.prepareOperation).toHaveBeenCalledWith({
      type: "process",
      snapshot_id: "snap-proc",
      process_keys: ["key-protected"],
      mode: "force",
    });
  });

  it("shows the backend summary and validity before terminating", async () => {
    render(() => <ProcessView />);
    await selectRow(0);
    terminate();

    expect(await screen.findByText("opSummary.processGraceful")).toBeTruthy();
    expect(screen.getByText("opConfirm.items")).toBeTruthy();
    expect(screen.getByText("opConfirm.estimated")).toBeTruthy();
    expect(screen.getByTestId("operation-confirm-validity")).toBeTruthy();
  });

  it("never terminates when the user cancels the prepared confirmation", async () => {
    render(() => <ProcessView />);
    await selectRow(0);
    terminate();
    await confirmDialog();

    fireEvent.click(screen.getByRole("button", { name: "common.cancel" }));

    expect(mocks.executeOperation).not.toHaveBeenCalled();
    expect(screen.queryByText("opConfirm.confirm")).toBeNull();
  });

  it("keeps polling suspended while the prepared confirmation is open", async () => {
    const { handlers } = intervalHarness();
    render(() => <ProcessView />);
    const poll = [...handlers];
    await selectRow(0);
    terminate();
    await confirmDialog();

    poll.forEach((handler) => handler());

    expect(mocks.listAllProcesses).toHaveBeenCalledTimes(1);
  });

  it("blocks whitelisted rows from being selected at all", async () => {
    render(() => <ProcessView />);
    const boxes = await screen.findAllByRole("checkbox");
    expect((boxes[2] as HTMLInputElement).disabled).toBe(true);
  });

  it("keeps polling suspended while rows are selected", async () => {
    const { handlers } = intervalHarness();
    render(() => <ProcessView />);
    const poll = [...handlers];
    await selectRow(0);
    expect(mocks.prepareOperation).not.toHaveBeenCalled();

    poll.forEach((handler) => handler());
    expect(mocks.listAllProcesses).toHaveBeenCalledTimes(1);
  });

  it("suspends polling while the confirmation dialog is open", async () => {
    const { handlers, clearSpy } = intervalHarness();
    render(() => <ProcessView />);
    const poll = [...handlers];
    expect(poll).toHaveLength(1);

    poll.forEach((handler) => handler());
    await waitFor(() =>
      expect(mocks.listAllProcesses).toHaveBeenCalledTimes(2),
    );

    await selectRow(1);
    terminate();
    await screen.findByText("process.confirmProtectedTitle");
    expect(clearSpy).toHaveBeenCalled();

    poll.forEach((handler) => handler());
    expect(mocks.listAllProcesses).toHaveBeenCalledTimes(2);
  });

  it("reports stale operations without replaying them", async () => {
    mocks.prepareOperation.mockRejectedValue({
      code: "selection_missing",
      message: "选择项不存在",
      params: [],
    });
    render(() => <ProcessView />);
    await selectRow(0);
    terminate();

    await screen.findByText("opError.stale");
    expect(screen.queryByText("opConfirm.confirm")).toBeNull();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
    await waitFor(() => expect(mocks.listAllProcesses).toHaveBeenCalledTimes(2));
  });

  it("shows the force-only hint when the backend refuses graceful", async () => {
    mocks.prepareOperation.mockRejectedValue({
      code: "protected_force_only",
      message: "受保护进程只能强制终止",
      params: [],
    });
    render(() => <ProcessView />);
    await selectRow(0);
    terminate();

    await screen.findByText("opError.forceOnly");
  });

  it("unwraps the tagged process result", async () => {
    render(() => <ProcessView />);
    await selectRow(0);
    terminate();
    await confirmRun();

    await screen.findByText("process.killSuccess");
  });

  it("keeps a selection made while a background refresh is still in flight", async () => {
    const { handlers } = intervalHarness();
    let release: (value: unknown) => void = () => {};
    mocks.listAllProcesses.mockReset();
    mocks.listAllProcesses.mockResolvedValueOnce(
      envelope([normalRow, protectedRow, whitelistRow]),
    );
    mocks.listAllProcesses.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );

    render(() => <ProcessView />);
    const cta = await screen.findByRole("button", {
      name: "process.terminateSelected",
    });
    [...handlers].forEach((handler) => handler());

    await selectRow(0);
    await waitFor(() => expect(cta.hasAttribute("disabled")).toBe(false));

    release(envelope([normalRow, protectedRow, whitelistRow]));
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(cta.hasAttribute("disabled")).toBe(false);
    const boxes = screen.getAllByRole("checkbox") as HTMLInputElement[];
    expect(boxes[0].checked).toBe(true);
    expect(mocks.prepareOperation).not.toHaveBeenCalled();
  });

  it("freezes the live list while the pointer rests on it", async () => {
    const { handlers } = intervalHarness();
    render(() => <ProcessView />);
    const poll = [...handlers];
    await screen.findAllByRole("checkbox");

    fireEvent.mouseEnter(screen.getByTestId("process-list"));
    poll.forEach((handler) => handler());
    expect(mocks.listAllProcesses).toHaveBeenCalledTimes(1);

    fireEvent.mouseLeave(screen.getByTestId("process-list"));
    poll.forEach((handler) => handler());
    await waitFor(() =>
      expect(mocks.listAllProcesses).toHaveBeenCalledTimes(2),
    );
  });

  it("clears a stale error when the user rescans from the toolbar", async () => {
    mocks.prepareOperation.mockRejectedValue({
      code: "snapshot_stale",
      message: "快照不存在或已失效",
      params: [],
    });
    render(() => <ProcessView />);
    await selectRow(0);
    terminate();
    await screen.findByText("opError.stale");

    fireEvent.click(screen.getByTestId("process-refresh"));

    await waitFor(() => expect(screen.queryByText("opError.stale")).toBeNull());
  });

  it("ignores a result tagged as another operation kind", async () => {
    mocks.executeOperation.mockResolvedValue({
      kind: "cache",
      value: { reports: [], deleted_bytes: 0, reclaimed_bytes: null, success_count: 0, fail_count: 0 },
    });
    render(() => <ProcessView />);
    await selectRow(0);
    terminate();
    await confirmRun();

    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(screen.queryByText("process.killSuccess")).toBeNull();
  });

  it("长名称提供完整值 tooltip，不让用户只看到截断结果", async () => {
    render(() => <ProcessView />);
    await screen.findAllByRole("checkbox");
    const name = screen.getByText("sleepy");
    expect(name.getAttribute("title")).toBe("sleepy");
  });

  it("展示名与原始名不同时 tooltip 使用原始名", async () => {
    mocks.listAllProcesses.mockResolvedValue(
      envelope([
        {
          ...normalRow,
          name: "WebKit 网页内容",
          full_name: "com.apple.WebKit.WebContent",
          // sysinfo 0.33.1 macOS：process.name 恒等于 exe 的 basename
          // （macos/process.rs:550-559 从同一个 KERN_PROCARGS2 路径取两者），
          // 所以 raw_name 与 exe 的组合必须是「同一个路径」，不能沿用 normalRow 的 /usr/bin/sleepy。
          exe: "/System/Library/Frameworks/WebKit.framework/Versions/A/XPCServices/com.apple.WebKit.WebContent.xpc/Contents/MacOS/com.apple.WebKit.WebContent",
        },
      ]),
    );
    render(() => <ProcessView />);
    await screen.findAllByRole("checkbox");
    const name = screen.getByText("WebKit 网页内容");
    expect(name.getAttribute("title")).toBe("com.apple.WebKit.WebContent");
  });

  it("同名不同保护状态的两行靠原始名副标题消歧", async () => {
    mocks.listAllProcesses.mockResolvedValue(envelope([realDockRow, dockExtraRow]));
    render(() => <ProcessView />);
    const boxes = await screen.findAllByRole("checkbox");

    expect(screen.getAllByText("Dock")).toHaveLength(2);
    expect(screen.getByText("com.apple.dock.extra")).toBeTruthy();
    expect((boxes[0] as HTMLInputElement).disabled).toBe(true);
    expect((boxes[1] as HTMLInputElement).disabled).toBe(false);
  });

  it("展示名与原始名相同时不额外渲染副标题行", async () => {
    render(() => <ProcessView />);
    await screen.findAllByRole("checkbox");
    // normalRow：name === full_name、非受保护、有 exe → 只应有「名称行 + exe 行」两行。
    // 数子元素个数而不是 getAllByText 长度：后者挡不住一个 text-transparent / sr-only
    // / 空内容的副标题元素。
    expect(nameCellRows()).toEqual(["sleepy", "/usr/bin/sleepy"]);
  });

  it("非受保护行同时显示原始名与 exe 路径", async () => {
    mocks.listAllProcesses.mockResolvedValue(envelope([dockExtraRow]));
    render(() => <ProcessView />);
    await screen.findAllByRole("checkbox");
    // 三行：名称行 + 原始名副标题 + exe 路径。副标题不能把 exe 挤掉。
    expect(nameCellRows()).toEqual([
      "Dock",
      "com.apple.dock.extra",
      "/usr/libexec/com.apple.dock.extra",
    ]);
  });

  it("原始名副标题不挤掉受保护原因", async () => {
    mocks.listAllProcesses.mockResolvedValue(
      envelope([
        {
          ...whitelistRow,
          name: "Google Chrome",
          full_name: "Google Chrome Helper (Renderer)",
          exe: "/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/A/Helpers/Google Chrome Helper (Renderer)",
        },
      ]),
    );
    render(() => <ProcessView />);
    await screen.findAllByRole("checkbox");

    expect(screen.getByText("Google Chrome Helper (Renderer)")).toBeTruthy();
    expect(screen.getByText("process.protect.whitelisted")).toBeTruthy();
  });

  it("展示出来的原始名也能被搜到", async () => {
    mocks.listAllProcesses.mockResolvedValue(envelope([realDockRow, dockExtraRow]));
    render(() => <ProcessView />);
    await screen.findAllByRole("checkbox");

    fireEvent.input(screen.getByPlaceholderText("process.searchPlaceholder"), {
      target: { value: "com.apple.dock.extra" },
    });

    await waitFor(() => expect(screen.getAllByText("Dock")).toHaveLength(1));
    expect(screen.getByText("com.apple.dock.extra")).toBeTruthy();
  });

  it("加入白名单用原始名而不是展示名", async () => {
    mocks.listAllProcesses.mockResolvedValue(envelope([dockExtraRow]));
    render(() => <ProcessView />);
    await screen.findAllByRole("checkbox");

    fireEvent.click(screen.getByTitle("scan.whitelistTooltip"));

    await waitFor(() => expect(mocks.addWhitelist).toHaveBeenCalled());
    // 后端 is_whitelisted 按 proc.name()（原始名）比对（scanner.rs:289）。
    // 传展示名 "Dock" 会写出一条下次扫描永远匹配不上的死条目。
    expect(mocks.addWhitelist).toHaveBeenCalledWith(
      "process",
      "com.apple.dock.extra",
      "process view add",
    );
  });

  it("所有格子锚在首行基线，2 行与 3 行高的行不会错开", async () => {
    mocks.listAllProcesses.mockResolvedValue(envelope([normalRow, dockExtraRow]));
    render(() => <ProcessView />);
    await screen.findAllByRole("checkbox");

    // 前置：这份 fixture 里同时存在内容 2 行（46.29px）与 3 行（60.57px）的行，
    // 所以才需要首行锚定。用 toContain 而不是按下标断言，刻意不依赖排序顺序。
    const lineCounts = [0, 1].map((i) => nameCellRows(i).length);
    expect(lineCounts).toContain(2);
    expect(lineCounts).toContain(3);

    const rows = screen.getAllByTestId("process-row");
    for (const row of [0, 1]) {
      expect(rows[row].className).toContain("items-start");
      // 行容器 .text-sm 把 line-height 设成 var(--text-sm--line-height) = 1.42857
      // （覆盖 preflight 的 html{line-height:1.5}），首行 14×1.42857 = 20.00px。
      // 数字格靠 pt-[2px] 补偏置：text-xs 行高 12×1.33333 = 16.00px → 需 2.00px（精确）；
      // text-[11px] 继承 1.42857 → 15.71px → 需 2.14px，pt-[2px] 残差 0.14px（亚像素，不可见）。
      for (let i = 1; i < 6; i += 1) {
        expect(rowCell(i, row).className).toContain("pt-[2px]");
      }
    }
  });

  it("前导控件固定 20px 锚在首行，不随名称格行数变高", async () => {
    mocks.listAllProcesses.mockResolvedValue(envelope([normalRow, dockExtraRow]));
    render(() => <ProcessView />);
    await screen.findAllByRole("checkbox");

    for (const row of [0, 1]) {
      // 前导格第 0 个子元素是「缩进 + 展开/收起」容器。它是 16px（w-4 占位）还是
      // 20px，取决于有没有展开按钮与图标；不定高时格子高度随子元素浮动，
      // 居中的复选框就会跟着漂。h-5 把它钉成与首行等高的 20px。
      const lead = rowCell(0, row).children[0] as HTMLElement;
      expect(lead.className).toContain("h-5");
      expect(lead.className).toContain("self-start");
      // 前导格自身不再需要 pt-[2px]：内容已被 h-5 归一到 20px，
      // items-center 在 20px 盒子里自动产出精确的 2px 内缩。
      expect(rowCell(0, row).className).not.toContain("pt-[2px]");
    }
  });
});
