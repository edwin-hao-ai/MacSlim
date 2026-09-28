import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type StageUpdate = {
  stage: string;
  state: "running" | "done";
  item_count: number;
  found_bytes: number;
};

const mocks = vi.hoisted(() => ({
  scanCache: vi.fn(),
  prepareOperation: vi.fn(),
  executeOperation: vi.fn(),
  onCacheScanProgress: vi.fn(),
  // 后端每阶段事件的手动触发入口，测试靠它驱动真实进度
  stageHandlers: [] as Array<(u: StageUpdate) => void>,
  unlisten: vi.fn(),
}));

vi.mock("@/lib/tauri", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/tauri")>();
  return {
    ...actual,
    scanCache: mocks.scanCache,
    prepareOperation: mocks.prepareOperation,
    executeOperation: mocks.executeOperation,
    onCacheScanProgress: mocks.onCacheScanProgress,
  };
});

vi.mock("@/components/DockerSection", () => ({ default: () => <div /> }));
vi.mock("@/components/CleanupFlash", () => ({ default: () => <div /> }));
vi.mock("@/lib/cleanFeedback", () => ({
  animateNumber: () => () => undefined,
  playCleanStartSound: vi.fn(),
  playCleanSuccessSound: vi.fn(),
  playCleanFailureSound: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn().mockResolvedValue(false),
  requestPermission: vi.fn().mockResolvedValue("denied"),
  sendNotification: vi.fn(),
}));
vi.mock("@/i18n", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/i18n")>();
  const { fakeI18n } = await import("@/i18n/fake-i18n");
  return {
    // `paramsToRecord` 用真实现：CacheView 靠它把后端的 [名, 值] 转成 t() 的参数，
    // 抄一份实现等于把待测逻辑抄进测试。
    ...actual,
    // scanProgress 是带占位符文案族（{stage}/{done}/{total}/{size}），
    // cache.item./cache.desc. 是后端下发的缓存项文案 key，
    // 两族都必须插值才能断言真实参数；其余 key 保持原样，
    // 这样既有的精确文本断言一个字符都不用改。
    useI18n: () =>
      fakeI18n({
        t: (key: string, p?: Record<string, string | number>) => {
          if (!p) return key;
          if (key.startsWith("scanProgress.")) return `${key}:${JSON.stringify(p)}`;
          if (key.startsWith("cache.item.") || key.startsWith("cache.desc.")) {
            return Object.keys(p).length > 0
              ? `${key}:${JSON.stringify(p)}`
              : key;
          }
          return key;
        },
      }),
    getT: () => (key: string) => key,
  };
});

import CacheView from "@/views/CacheView";

type CacheItem = {
  selection_key: string;
  id: string;
  label_key: string;
  label_params: [string, string][];
  description_key: string;
  description_params: [string, string][];
  path: string | null;
  size_bytes: number;
  safety: "safe" | "low" | "medium";
  default_select: boolean;
  recover_hint: string;
  category: string;
};

const npmItem: CacheItem = {
  selection_key: "key-npm",
  id: "npm",
  label_key: "cache.item.npmCache",
  label_params: [],
  description_key: "cache.desc.npmCache",
  description_params: [],
  path: "/Users/tester/.npm",
  size_bytes: 100,
  safety: "safe",
  default_select: true,
  recover_hint: "重新下载",
  category: "npm",
};

/** 故意带插值参数：参数必须跟着 key 一起走到 `t()`，不能被丢掉 */
const xcodeItem: CacheItem = {
  selection_key: "key-xcode",
  id: "xcode",
  label_key: "cache.item.dockerStoppedContainers",
  label_params: [["count", "3"]],
  description_key: "cache.desc.staleNodeModules",
  description_params: [["path", "/Users/tester/Projects/app/node_modules"]],
  path: "/Users/tester/Library/Developer",
  size_bytes: 200,
  safety: "medium",
  default_select: false,
  recover_hint: "重新编译",
  category: "xcode",
};

const scanEnvelope = () => ({
  snapshot_id: "snap-cache",
  expires_at_ms: 1_700_000_000_000,
  value: {
    items: [npmItem, xcodeItem],
    total_bytes: 300,
    scanned_at_ms: 1,
  },
});

const prepared = {
  operation_id: "op-cache-1",
  kind: "cache",
  expires_at_ms: 1_700_000_600_000,
  item_count: 1,
  estimated_bytes: 100,
  summary_key: "opSummary.cache",
  summary_params: [
    ["count", "1"],
    ["size", "128 B"],
  ],
};

const confirmDialog = () => screen.findByTestId("operation-confirm-validity");
const confirmRun = async () => {
  fireEvent.click(await screen.findByRole("button", { name: "opConfirm.confirm" }));
};
const cancelConfirm = async () => {
  fireEvent.click(await screen.findByRole("button", { name: "common.cancel" }));
};

const resetScanMocks = () => {
  mocks.scanCache.mockReset();
  mocks.prepareOperation.mockReset();
  mocks.executeOperation.mockReset();
  mocks.onCacheScanProgress.mockReset();
  mocks.stageHandlers.length = 0;
  mocks.unlisten.mockReset();
  mocks.scanCache.mockResolvedValue(scanEnvelope());
  // 默认注册一个立即返回的监听；个别测试会覆盖成「手动 resolve」来制造竞态
  mocks.onCacheScanProgress.mockImplementation(
    async (cb: (u: StageUpdate) => void) => {
      mocks.stageHandlers.push(cb);
      return mocks.unlisten;
    },
  );
};

describe("斜线网格标记「可清理空间」", () => {
  beforeEach(resetScanMocks);

  afterEach(() => {
    cleanup();
  });

  it("只给「可清理」分类的色块加纹理，含注意级条目的分类不加", async () => {
    // 夹具里 npm 分组只有 safe 项、xcode 分组有一项 medium（`xcodeItem`），
    // 所以判定完全由 `CacheItem.safety` 驱动，不依赖任何分类名单。
    render(() => <CacheView />);

    const npmBadge = await screen.findByText("NPM");
    const xcodeBadge = await screen.findByText("Xcode");

    expect(npmBadge.className).toContain("reclaimable-hatch");
    expect(npmBadge.dataset.reclaimable).toBe("true");

    expect(xcodeBadge.className).not.toContain("reclaimable-hatch");
    expect(xcodeBadge.dataset.reclaimable).toBe("false");
  });

  it("同一分类里 medium 项消失后，色块自动恢复纹理（判定随数据变化）", async () => {
    const xcodeSafe: CacheItem = { ...xcodeItem, safety: "low" };
    mocks.scanCache.mockResolvedValue({
      snapshot_id: "snap-cache",
      expires_at_ms: 1_700_000_000_000,
      value: {
        items: [npmItem, xcodeSafe],
        total_bytes: 300,
        scanned_at_ms: 1,
      },
    });
    render(() => <CacheView />);

    const xcodeBadge = await screen.findByText("Xcode");
    expect(xcodeBadge.className).toContain("reclaimable-hatch");
  });
});

describe("CacheView broker flow", () => {
  beforeEach(resetScanMocks);

  afterEach(() => {
    cleanup();
  });

  it("selects default-safe items by opaque key", async () => {
    render(() => <CacheView />);
    await waitFor(() => expect(mocks.scanCache).toHaveBeenCalledTimes(1));
    const boxes = await screen.findAllByRole("checkbox");
    expect((boxes[0] as HTMLInputElement).checked).toBe(true);
    expect((boxes[1] as HTMLInputElement).checked).toBe(false);
  });

  it("renders backend i18n keys through t() with their params", async () => {
    render(() => <CacheView />);

    // 无参数的 key 原样交给 t()
    expect(await screen.findByText("cache.item.npmCache")).toBeTruthy();
    // 有参数的 key 必须把参数一起传下去，否则 {count}/{path} 会留在界面上
    expect(
      await screen.findByText('cache.item.dockerStoppedContainers:{"count":"3"}'),
    ).toBeTruthy();
    expect(
      await screen.findByText(
        'cache.desc.staleNodeModules:{"path":"/Users/tester/Projects/app/node_modules"}',
      ),
    ).toBeTruthy();
    // 界面不得出现未替换的占位符
    expect(document.body.textContent).not.toContain("{count}");
    expect(document.body.textContent).not.toContain("{path}");
  });

  it("runs scan -> prepare -> execute -> rescan in order", async () => {
    mocks.prepareOperation.mockResolvedValue(prepared);
    mocks.executeOperation.mockResolvedValue({
      kind: "cache",
      value: {
        reports: [
          {
            id: "npm",
            label_key: "cache.item.npmCache",
            label_params: [],
            success: true,
            freed_bytes: 100,
            duration_ms: 5,
            error: null,
          },
        ],
        total_freed_bytes: 100,
        success_count: 1,
        fail_count: 0,
      },
    });

    render(() => <CacheView />);
    await screen.findByText("cache.item.npmCache");
    screen.getByText("cache.cleanCta").click();
    await confirmRun();

    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(mocks.prepareOperation).toHaveBeenCalledWith({
      type: "cache",
      snapshot_id: "snap-cache",
      item_keys: ["key-npm"],
    });
    expect(mocks.executeOperation).toHaveBeenCalledWith("op-cache-1");
    const order = [
      mocks.scanCache.mock.invocationCallOrder[0],
      mocks.prepareOperation.mock.invocationCallOrder[0],
      mocks.executeOperation.mock.invocationCallOrder[0],
      mocks.scanCache.mock.invocationCallOrder[1],
    ];
    expect(order).toEqual([...order].sort((a, b) => a - b));
  });

  it("never puts raw path or command on the wire", async () => {
    mocks.prepareOperation.mockResolvedValue(prepared);
    mocks.executeOperation.mockResolvedValue({
      kind: "cache",
      value: {
        reports: [],
        total_freed_bytes: 0,
        success_count: 0,
        fail_count: 0,
      },
    });

    render(() => <CacheView />);
    await screen.findByText("cache.item.npmCache");

    fireEvent.click(screen.getByText("cache.cleanCta"));

    await waitFor(() => expect(mocks.prepareOperation).toHaveBeenCalled());
    const [request] = mocks.prepareOperation.mock.calls[0] as [
      Record<string, unknown>,
    ];
    expect(Object.keys(request).sort()).toEqual([
      "item_keys",
      "snapshot_id",
      "type",
    ]);
    expect(JSON.stringify(request)).not.toContain("/Users/tester");
  });

  it("never renders a raw shell command for a cache item", async () => {
    const leaky = {
      ...scanEnvelope(),
      value: {
        ...scanEnvelope().value,
        items: [{ ...npmItem, command: "npm cache clean --force" }],
      },
    };
    mocks.scanCache.mockResolvedValue(leaky);

    render(() => <CacheView />);
    await screen.findByText("cache.item.npmCache");
    expect(document.body.textContent).not.toContain("npm cache clean");
    expect(screen.queryByText(/^\$ /)).toBeNull();
  });

  it("still shows the cache path as a display-only field", async () => {
    render(() => <CacheView />);
    await screen.findByText("cache.item.npmCache");
    expect(screen.getByText("/Users/tester/.npm")).toBeTruthy();
  });

  it("shows the rescan hint and refreshes when the snapshot expired", async () => {
    mocks.prepareOperation.mockRejectedValue({
      code: "snapshot_stale",
      message: "快照不存在或已失效",
      params: [],
    });

    render(() => <CacheView />);
    await screen.findByText("cache.item.npmCache");
    screen.getByText("cache.cleanCta").click();

    await screen.findByText("opError.stale");
    expect(mocks.executeOperation).not.toHaveBeenCalled();
    expect(screen.queryByText("opConfirm.confirm")).toBeNull();
    await waitFor(() => expect(mocks.scanCache).toHaveBeenCalledTimes(2));
  });

  it("waits for the user before executing a prepared cleanup", async () => {
    mocks.prepareOperation.mockResolvedValue(prepared);
    render(() => <CacheView />);
    await screen.findByText("cache.item.npmCache");

    fireEvent.click(screen.getByText("cache.cleanCta"));

    await confirmDialog();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
  });

  it("shows the backend summary, estimate and item count before executing", async () => {
    mocks.prepareOperation.mockResolvedValue(prepared);
    render(() => <CacheView />);
    await screen.findByText("cache.item.npmCache");

    fireEvent.click(screen.getByText("cache.cleanCta"));

    expect(await screen.findByText("opSummary.cache")).toBeTruthy();
    expect(screen.getByText("opConfirm.items")).toBeTruthy();
    expect(screen.getByText("opConfirm.estimated")).toBeTruthy();
    expect(screen.getByTestId("operation-confirm-validity").textContent).toBe(
      "opConfirm.validity",
    );
  });

  it("never executes when the user cancels the confirmation", async () => {
    mocks.prepareOperation.mockResolvedValue(prepared);
    render(() => <CacheView />);
    await screen.findByText("cache.item.npmCache");

    fireEvent.click(screen.getByText("cache.cleanCta"));
    await confirmDialog();
    await cancelConfirm();

    expect(mocks.executeOperation).not.toHaveBeenCalled();
    expect(screen.queryByText("opConfirm.confirm")).toBeNull();
  });

  it("demands a second, irreversible confirmation above 10GB", async () => {
    const tenGb = 10 * 1024 * 1024 * 1024;
    mocks.prepareOperation.mockResolvedValue({
      ...prepared,
      estimated_bytes: tenGb + 1,
      summary_key: "opSummary.cache",
      summary_params: [
        ["count", "1"],
        ["size", "10.0 GB"],
      ],
    });
    render(() => <CacheView />);
    await screen.findByText("cache.item.npmCache");

    fireEvent.click(screen.getByText("cache.cleanCta"));

    expect(await screen.findByText("opConfirm.irreversible")).toBeTruthy();
    expect(screen.getByText("opConfirm.largeWarning")).toBeTruthy();
    expect(mocks.executeOperation).not.toHaveBeenCalled();
    await confirmRun();
    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
  });

  it("does not replay an operation that the backend already consumed", async () => {
    mocks.prepareOperation.mockResolvedValue(prepared);
    mocks.executeOperation.mockRejectedValue({
      code: "operation_used_or_expired",
      message: "操作已使用或已失效",
      params: [],
    });

    render(() => <CacheView />);
    await screen.findByText("cache.item.npmCache");
    screen.getByText("cache.cleanCta").click();
    await confirmRun();

    await screen.findByText("opError.stale");
    expect(mocks.prepareOperation).toHaveBeenCalledTimes(1);
    expect(mocks.executeOperation).toHaveBeenCalledTimes(1);
  });

  it("surfaces history write failures without retrying", async () => {
    mocks.prepareOperation.mockResolvedValue(prepared);
    mocks.executeOperation.mockRejectedValue({
      code: "history_write_failed",
      message:
        "操作已执行，但写入历史记录失败（缓存清理 · 1 项缓存）：历史数据库不可写，请到历史页核对",
      params: [],
    });

    render(() => <CacheView />);
    await screen.findByText("cache.item.npmCache");
    screen.getByText("cache.cleanCta").click();
    await confirmRun();

    await screen.findByText("opError.history");
    expect(mocks.prepareOperation).toHaveBeenCalledTimes(1);
  });

  it("ignores a result tagged as another operation kind", async () => {
    mocks.prepareOperation.mockResolvedValue(prepared);
    mocks.executeOperation.mockResolvedValue({
      kind: "docker",
      value: { action: "prune", succeeded: [], failed: [], output: "" },
    });

    render(() => <CacheView />);
    await screen.findByText("cache.item.npmCache");
    screen.getByText("cache.cleanCta").click();
    await confirmRun();

    await waitFor(() => expect(mocks.executeOperation).toHaveBeenCalled());
    expect(screen.queryByText("cache.cleanSuccess")).toBeNull();
  });
});

describe("CacheView scan stage progress", () => {
  beforeEach(resetScanMocks);

  afterEach(() => {
    cleanup();
  });

  /** 让 scanCache 一直挂起，从而稳定停留在「扫描中」分支 */
  const pendingScan = () => {
    let resolveScan: (value: ReturnType<typeof scanEnvelope>) => void = () => {};
    mocks.scanCache.mockImplementation(
      () => new Promise((resolve) => { resolveScan = resolve; }),
    );
    return () => resolveScan(scanEnvelope());
  };

  it("扫描期间显示真实阶段名与完成计数", async () => {
    pendingScan();
    render(() => <CacheView />);
    await waitFor(() => expect(mocks.stageHandlers.length).toBeGreaterThan(0));

    mocks.stageHandlers[0]({
      stage: "废纸篓",
      state: "running",
      item_count: 0,
      found_bytes: 0,
    });

    // 阶段名原样来自后端，不做任何映射
    expect(screen.getByTestId("scan-stage-name").textContent).toContain("废纸篓");
    // 纯 spinner 文案必须被真实进度取代
    expect(screen.queryByText("cache.scanning")).toBeNull();
    // total 接的是 16 个缓存阶段，所以进度条必须在
    expect(screen.getByTestId("scan-stage-bar")).toBeTruthy();
    expect(screen.getByTestId("scan-stage-count").textContent).toContain("16");
  });

  it("阶段 done 事件累加已完成计数与已发现体积", async () => {
    pendingScan();
    render(() => <CacheView />);
    await waitFor(() => expect(mocks.stageHandlers.length).toBeGreaterThan(0));

    mocks.stageHandlers[0]({
      stage: "npm 缓存",
      state: "done",
      item_count: 2,
      found_bytes: 300,
    });

    expect(screen.getByTestId("scan-stage-count").textContent).toContain("1");
    expect(screen.getByTestId("scan-stage-bytes").textContent).toContain("300 B");
  });

  it("扫描完成后卸载进度并渲染列表", async () => {
    const finishScan = pendingScan();
    render(() => <CacheView />);
    await waitFor(() => expect(mocks.stageHandlers.length).toBeGreaterThan(0));
    expect(screen.getByTestId("scan-stage-progress")).toBeTruthy();

    finishScan();

    await waitFor(() =>
      expect(screen.queryByTestId("scan-stage-progress")).toBeNull(),
    );
    expect(await screen.findByText("cache.item.npmCache")).toBeTruthy();
    // 扫描收尾也必须解绑，不能只靠卸载兜底
    expect(mocks.unlisten).toHaveBeenCalled();
  });

  it("组件卸载时调用 unlisten", async () => {
    pendingScan();
    const { unmount } = render(() => <CacheView />);
    await waitFor(() => expect(mocks.stageHandlers.length).toBeGreaterThan(0));
    // 扫描还挂着，unlisten 只可能来自卸载路径
    expect(mocks.unlisten).not.toHaveBeenCalled();

    unmount();

    expect(mocks.unlisten).toHaveBeenCalled();
  });

  it("监听注册期间卸载也不会泄漏监听", async () => {
    let resolveListen: (fn: () => void) => void = () => {};
    mocks.onCacheScanProgress.mockImplementation(
      () => new Promise((resolve) => { resolveListen = resolve; }),
    );
    const { unmount } = render(() => <CacheView />);
    unmount();

    // 监听在组件已卸载之后才注册成功，必须立刻自我解绑
    resolveListen(mocks.unlisten);

    await waitFor(() => expect(mocks.unlisten).toHaveBeenCalled());
    expect(mocks.scanCache).not.toHaveBeenCalled();
  });

  it("重入扫描时每个监听句柄都恰好解绑一次", async () => {
    // 每次注册返回一个可区分的新句柄，才能验证「两个都被解绑」且「同一个不被重复调用」
    const handles: Array<ReturnType<typeof vi.fn>> = [];
    mocks.onCacheScanProgress.mockImplementation(async () => {
      const handle = vi.fn();
      handles.push(handle);
      return handle;
    });
    // onMount 那次扫描正常收尾，之后每次扫描都挂起 → runScan 不会自己解绑
    mocks.scanCache.mockImplementationOnce(() =>
      Promise.resolve(scanEnvelope()),
    );
    const pendingScans: Array<(v: ReturnType<typeof scanEnvelope>) => void> = [];
    mocks.scanCache.mockImplementation(
      () => new Promise((resolve) => { pendingScans.push(resolve); }),
    );
    // 生产可达的重入路径：prepareOperation 挂起期间 cleaning / scanning 都是 false，
    // 清理按钮仍然可点 → 连点两次 → 两次 prepare 先后失败 → 两个 runScan 重叠
    const pendingPrepares: Array<(reason: string) => void> = [];
    mocks.prepareOperation.mockImplementation(
      () => new Promise((_resolve, reject) => { pendingPrepares.push(reject); }),
    );

    render(() => <CacheView />);
    await screen.findByText("cache.item.npmCache");
    // onMount 那次扫描已收尾：它自己的句柄被解绑恰好一次
    expect(handles).toHaveLength(1);
    expect(handles[0]).toHaveBeenCalledTimes(1);
    // 从这里起只看重入的那两个句柄
    handles.length = 0;

    fireEvent.click(screen.getByText("cache.cleanCta"));
    fireEvent.click(screen.getByText("cache.cleanCta"));
    expect(pendingPrepares.length).toBe(2);

    pendingPrepares[0]("快照不存在或已失效");
    await waitFor(() => expect(handles.length).toBe(1));
    pendingPrepares[1]("快照不存在或已失效");
    await waitFor(() => expect(handles.length).toBe(2));

    // 两次扫描都还挂着：一个句柄都不该被解绑
    expect(handles[0]).not.toHaveBeenCalled();
    expect(handles[1]).not.toHaveBeenCalled();

    pendingScans.forEach((resolve) => resolve(scanEnvelope()));

    // 各自解绑自己那一个：两个都被解绑，且各只被调一次
    await waitFor(() => {
      expect(handles[0]).toHaveBeenCalledTimes(1);
      expect(handles[1]).toHaveBeenCalledTimes(1);
    });
  });
});
