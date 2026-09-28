import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * MAS 形态下「终止」入口的收口。
 *
 * 单独一个文件，因为要 `vi.mock` 掉 flavor 模块 —— `flavor.ts` 是**单例**
 * （内部存当前形态），在同一个测试文件里没法既跑 developer_id 又跑 mas。
 * flavor 模块自身的状态机在 `src/lib/flavor.test.ts` 里测。
 */

// vi.mock 的工厂会被提升到 import 之前，不能直接闭包引用外层的普通变量，
// 否则报 "Cannot access 'x' before initialization"。必须走 hoisted。
const mocks = vi.hoisted(() => ({
  listAllProcesses: vi.fn(),
  prepareOperation: vi.fn(),
  executeOperation: vi.fn(),
}));

vi.mock("@/lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/tauri")>();
  return {
    ...actual,
    listAllProcesses: mocks.listAllProcesses,
    prepareOperation: mocks.prepareOperation,
    executeOperation: mocks.executeOperation,
  };
});

// 同样是 hoisted：`vi.mock("@/lib/flavor")` 的工厂闭包引用了它。
const flavorState = vi.hoisted(() => ({ canTerminate: true }));

vi.mock("@/lib/flavor", () => ({
  // 组件只读这个能力判断，不读 flavor 本身
  canTerminateProcesses: () => flavorState.canTerminate,
}));

vi.mock("@/i18n", () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));

import ProcessView from "@/views/ProcessView";

const ROW = {
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

const envelope = () => ({
  snapshot_id: "snap-proc",
  expires_at_ms: 1_700_000_000_000,
  value: [ROW],
});

describe("ProcessView 的构建形态收口", () => {
  beforeEach(() => {
    flavorState.canTerminate = true;
    mocks.listAllProcesses.mockReset();
    mocks.prepareOperation.mockReset();
    mocks.executeOperation.mockReset();
    mocks.listAllProcesses.mockResolvedValue(envelope());
  });
  afterEach(cleanup);

  it("Developer ID 形态：渲染终止按钮", async () => {
    render(() => <ProcessView />);
    await screen.findAllByRole("checkbox");
    expect(screen.getByRole("button", { name: "process.terminateSelected" })).toBeTruthy();
  });

  it("MAS 形态：不渲染终止按钮，改为一句说明", async () => {
    // App Sandbox 下沙箱进程不能给其他进程发信号，且无 entitlement 可放行。
    // 留一个点了必定失败的按钮、再弹「权限不足」，用户会误以为是系统设置问题。
    flavorState.canTerminate = false;
    render(() => <ProcessView />);
    await screen.findAllByRole("checkbox");
    expect(
      screen.queryByRole("button", { name: "process.terminateSelected" }),
    ).toBeNull();
    expect(screen.getByText("process.masTerminateUnsupported")).toBeTruthy();
  });

  it("MAS 形态：进程列表本身照常渲染（只有终止能力没了）", async () => {
    // 关键：不能因为砍掉终止就把整页藏了 —— 内存/CPU 占用查看在沙箱里是
    // 可用的（走的是只读的进程快照，不是发信号）。
    flavorState.canTerminate = false;
    render(() => <ProcessView />);
    const boxes = await screen.findAllByRole("checkbox");
    expect(boxes.length).toBeGreaterThan(0);
    expect(screen.getByText("process.protectedHint")).toBeTruthy();
  });
});
