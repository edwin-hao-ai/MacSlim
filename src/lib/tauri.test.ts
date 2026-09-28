import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: unknown) => invoke(command, args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (
    event: string,
    handler: (e: { event: string; id: number; payload: unknown }) => void,
  ) => listen(event, handler),
}));

import * as tauri from "@/lib/tauri";
import type { CacheItem, StageUpdate } from "@/lib/tauri";
import {
  addWhitelist,
  appGracefulQuitOperation,
  appTerminateOperation,
  cacheOperation,
  checkAppRunning,
  dockerOperation,
  executeOperation,
  getHistory,
  getWhitelist,
  getSystemHealth,
  listAllProcesses,
  listApplications,
  onCacheScanProgress,
  onResidueScanProgress,
  prepareOperation,
  processOperation,
  removeWhitelist,
  scanAll,
  scanAppResiduesBatch,
  scanCache,
  scanInstalledApps,
  uninstallOperation,
} from "@/lib/tauri";

type CacheItemHasNoCommand = "command" extends keyof CacheItem ? never : true;
const cacheItemHasNoCommandField: CacheItemHasNoCommand = true;

/** `as const` 会把空数组推成 readonly，这里显式借用 CacheItem 的可写参数类型 */
const noParams: CacheItem["label_params"] = [];

const backendCacheItem = {
  selection_key: "key-npm",
  id: "npm",
  category: "npm",
  label_key: "cache.item.npmCache",
  label_params: noParams,
  description_key: "cache.desc.npmCache",
  description_params: noParams,
  path: "/Users/tester/.npm",
  size_bytes: 100,
  safety: "safe",
  default_select: true,
  recover_hint: "重新下载",
} as const;

const snapshot = <T>(value: T, id = "snap-1") => ({
  snapshot_id: id,
  expires_at_ms: 1_700_000_000_000,
  value,
});

describe("tauri api layer", () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it("scans return the snapshot envelope untouched", async () => {
    const envelopes: Record<string, unknown> = {
      scan_all: snapshot({ health: {}, processes: [], scanned_at_ms: 1 }),
      list_all_processes: snapshot([]),
      list_applications: snapshot([]),
      scan_cache: snapshot({ items: [], total_bytes: 0, scanned_at_ms: 1 }),
      docker_inventory: snapshot({ daemon_running: false }),
      scan_installed_apps: snapshot([]),
    };
    invoke.mockImplementation((command: string) => Promise.resolve(envelopes[command]));

    await expect(scanAll()).resolves.toEqual(envelopes.scan_all);
    await expect(listAllProcesses()).resolves.toEqual(envelopes.list_all_processes);
    await expect(listApplications()).resolves.toEqual(envelopes.list_applications);
    await expect(scanCache()).resolves.toEqual(envelopes.scan_cache);
    await expect(tauri.dockerInventory()).resolves.toEqual(
      envelopes.docker_inventory,
    );
    await expect(scanInstalledApps()).resolves.toEqual(
      envelopes.scan_installed_apps,
    );

    expect(invoke.mock.calls.map(([command]) => command)).toEqual([
      "scan_all",
      "list_all_processes",
      "list_applications",
      "scan_cache",
      "docker_inventory",
      "scan_installed_apps",
    ]);
  });

  it("registers residue for a whole batch of app keys in one call", async () => {
    const envelope = snapshot([{ app_key: "k1", residue: { items: [] } }]);
    invoke.mockResolvedValue(envelope);

    await expect(scanAppResiduesBatch("snap-9", ["k1", "k2"])).resolves.toEqual(
      envelope,
    );
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("scan_app_residues_batch", {
      appSnapshotId: "snap-9",
      appKeys: ["k1", "k2"],
    });
  });

  it("prepare_operation carries the tagged request as a single field", async () => {
    const prepared = {
      operation_id: "op-1",
      kind: "cache",
      expires_at_ms: 1,
      item_count: 2,
      estimated_bytes: 8,
      summary: "准备清理 2 项缓存",
    };
    invoke.mockResolvedValue(prepared);
    const request = cacheOperation("snap-1", ["k1", "k2"]);

    await expect(prepareOperation(request)).resolves.toEqual(prepared);
    expect(invoke).toHaveBeenCalledWith("prepare_operation", { request });
  });

  it("execute_operation only sends the operation id", async () => {
    const outcome = { kind: "cache", value: { reports: [] } };
    invoke.mockResolvedValue(outcome);

    await expect(executeOperation("op-1")).resolves.toEqual(outcome);
    expect(invoke).toHaveBeenCalledWith("execute_operation", {
      operationId: "op-1",
    });
  });

  it("keeps read-only and whitelist commands", async () => {
    invoke.mockResolvedValue(true);
    await getSystemHealth();
    await tauri.dockerAvailable();
    await checkAppRunning("/Applications/Safari.app");
    await getHistory(30);
    await getWhitelist();
    await addWhitelist("process", "Chrome", "note");
    await removeWhitelist(7);

    expect(invoke.mock.calls).toEqual([
      ["get_system_health", undefined],
      ["docker_available", undefined],
      ["check_app_running", { bundlePath: "/Applications/Safari.app" }],
      ["get_history", { limit: 30 }],
      ["get_whitelist", undefined],
      ["add_whitelist", { kind: "process", value: "Chrome", note: "note" }],
      ["remove_whitelist", { id: 7 }],
    ]);
  });

  it("drops every raw destructive wrapper from the module surface", () => {
    const surface = Object.keys(tauri).filter((name) => name !== "default");
    const forbidden = [
      "killProcesses",
      "cleanCache",
      "quitApplication",
      "forceQuitApplication",
      "uninstallApps",
      "quitAndUninstall",
      "scanAppResidues",
      "dockerRemoveImage",
      "dockerRemoveContainer",
      "dockerRemoveVolume",
      "dockerPruneAll",
    ];
    for (const name of forbidden) {
      expect(surface).not.toContain(name);
    }
  });

  it("exposes prepare/execute as the only destructive entry points", () => {
    const destructive = Object.keys(tauri).filter((name) =>
      /Operation$|^prepare|^execute/.test(name),
    );
    expect(destructive.sort()).toEqual([
      "appGracefulQuitOperation",
      "appTerminateOperation",
      "cacheOperation",
      "dockerOperation",
      "executeOperation",
      "prepareOperation",
      "processOperation",
      "uninstallOperation",
    ]);
  });
});

describe("scan progress event contract", () => {
  /** Tauri 传给 listen 回调的原始信封。payload 之前必须被拆出来。 */
  type EventEnvelope = { event: string; id: number; payload: StageUpdate };

  /** 取出第 n 次 listen 调用里注册的那个回调，避开未类型化的 mock calls。 */
  const handlerAt = (call: number) =>
    listen.mock.calls[call][1] as (e: EventEnvelope) => void;

  const stageUpdate: StageUpdate = {
    stage: "npm",
    state: "running",
    item_count: 3,
    found_bytes: 2048,
  };

  beforeEach(() => {
    listen.mockReset();
  });

  it("subscribes to the backend event names verbatim", async () => {
    listen.mockResolvedValue(vi.fn());

    await onCacheScanProgress(vi.fn());
    await onResidueScanProgress(vi.fn());

    // 逐字相等：改名 / 加后缀 / 换分隔符都会红。Rust 侧 lib_progress_tests.rs 钉的是同一对字符串。
    expect(listen.mock.calls.map(([name]) => name)).toEqual([
      "cache-scan-progress",
      "residue-scan-progress",
    ]);
  });

  it("registers exactly one handler per subscription", async () => {
    listen.mockResolvedValue(vi.fn());

    await onCacheScanProgress(vi.fn());

    expect(listen).toHaveBeenCalledTimes(1);
    expect(listen.mock.calls[0]).toHaveLength(2);
    expect(typeof handlerAt(0)).toBe("function");
  });

  it("hands the backend unlisten function back by reference", async () => {
    const unlisten = vi.fn();
    listen.mockResolvedValue(unlisten);

    // toBe 是引用相等：包一层新函数（return async () => {}）或吞掉返回值都会红。
    await expect(onCacheScanProgress(vi.fn())).resolves.toBe(unlisten);
    await expect(onResidueScanProgress(vi.fn())).resolves.toBe(unlisten);
    expect(unlisten).not.toHaveBeenCalled();
  });

  it("forwards event.payload to the subscriber untouched", async () => {
    listen.mockResolvedValue(vi.fn());
    const received: StageUpdate[] = [];

    await onCacheScanProgress((u) => received.push(u));
    handlerAt(0)({ event: "cache-scan-progress", id: 7, payload: stageUpdate });

    // 引用相等：cb(e) 会得到信封，cb(e.payload.data) 会得到 undefined，两者都红。
    expect(received).toHaveLength(1);
    expect(received[0]).toBe(stageUpdate);
  });

  it("unwraps the payload on the residue channel too", async () => {
    listen.mockResolvedValue(vi.fn());
    const received: StageUpdate[] = [];

    await onResidueScanProgress((u) => received.push(u));
    handlerAt(0)({
      event: "residue-scan-progress",
      id: 9,
      payload: { ...stageUpdate, stage: "app-scan" },
    });

    expect(received).toHaveLength(1);
    expect(received[0]).toEqual({ ...stageUpdate, stage: "app-scan" });
  });
});

describe("typed operation requests", () => {
  it("builds only key-based payloads", () => {
    const requests = [
      cacheOperation("s1", ["k1"]),
      processOperation("s2", ["k2"], "graceful"),
      appTerminateOperation("s3", ["k3"], "force"),
      appGracefulQuitOperation("s7", ["k7"]),
      uninstallOperation("s4", "s5", ["k4"], ["k5"], true),
      dockerOperation("s6", "prune", []),
    ];
    expect(requests).toEqual([
      {
        type: "cache",
        snapshot_id: "s1",
        item_keys: ["k1"],
      },
      {
        type: "process",
        snapshot_id: "s2",
        process_keys: ["k2"],
        mode: "graceful",
      },
      {
        type: "app_terminate",
        snapshot_id: "s3",
        app_keys: ["k3"],
        mode: "force",
      },
      {
        type: "app_graceful_quit",
        snapshot_id: "s7",
        app_keys: ["k7"],
      },
      {
        type: "uninstall",
        app_snapshot_id: "s4",
        residue_snapshot_id: "s5",
        app_keys: ["k4"],
        residue_keys: ["k5"],
        quit_running: true,
      },
      {
        type: "docker",
        snapshot_id: "s6",
        action: "prune",
        target_keys: [],
      },
    ]);
  });

  it("never carries raw target fields on the wire", () => {
    const forbidden = new Set([
      "path",
      "paths",
      "pid",
      "pids",
      "name",
      "names",
      "command",
      "args",
      "bundle_path",
      "bundlePath",
      "residue_paths",
      "items",
      "targets",
    ]);
    const requests = [
      cacheOperation("s1", ["k1"]),
      processOperation("s2", ["k2"], "force"),
      appTerminateOperation("s3", ["k3"], "force"),
      appGracefulQuitOperation("s7", ["k7"]),
      uninstallOperation("s4", "s5", ["k4"], ["k5"], false),
      dockerOperation("s6", "remove_image", ["k6"]),
    ];
    for (const request of requests) {
      for (const field of Object.keys(request)) {
        expect(forbidden.has(field)).toBe(false);
      }
    }
  });
});

describe("cache item view contract", () => {
  it("pins the cache item to the backend field set with no command field", () => {
    expect(cacheItemHasNoCommandField).toBe(true);
    const fields = Object.keys(backendCacheItem).sort();
    expect(fields).toEqual([
      "category",
      "default_select",
      "description_key",
      "description_params",
      "id",
      "label_key",
      "label_params",
      "path",
      "recover_hint",
      "safety",
      "selection_key",
      "size_bytes",
    ]);
    expect("command" in backendCacheItem).toBe(false);
  });

  it("keeps path as a display-only field", () => {
    const item: CacheItem = { ...backendCacheItem, category: "npm" };
    expect(item.path).toBe("/Users/tester/.npm");
    expect(JSON.stringify(item)).not.toContain("command");
  });

  it("carries display text as i18n keys, never as ready-made copy", () => {
    // 可读文案只存在于前端词典：后端下发的两个 key 位不得夹带译文
    expect(backendCacheItem.label_key).toMatch(/^cache\.item\.[A-Za-z0-9.]+$/);
    expect(backendCacheItem.description_key).toMatch(/^cache\.desc\.[A-Za-z0-9.]+$/);
    expect(backendCacheItem.label_key).not.toContain("NPM");
  });
});

describe("operation error classification", () => {
  // 分类的**行为等价性**（旧中文子串匹配 ↔ 新 code 匹配）由
  // `src/i18n/error-i18n.test.ts` 用一张覆盖全部旧文案样本的对照表守住。
  // 这里只钉住「返回值形状」与「非后端来源」这两件形状性的事。

  it("returns the code, message and params alongside the kind", () => {
    expect(
      tauri.classifyOperationError({
        code: "process_gone",
        message: "进程已不存在（PID 11），请重新扫描",
        params: [["pid", "11"]],
      }),
    ).toEqual({
      kind: "stale",
      code: "process_gone",
      message: "进程已不存在（PID 11），请重新扫描",
      params: [["pid", "11"]],
    });
  });

  it("treats a backend error without params as param-less", () => {
    const info = tauri.classifyOperationError({
      code: "snapshot_stale",
      message: "快照不存在或已失效",
    });
    expect(info.params).toEqual([]);
    expect(info.kind).toBe("stale");
  });

  it("falls back to a generic failure for non-backend throwables", () => {
    expect(tauri.classifyOperationError(new Error("Docker 未运行"))).toEqual({
      kind: "failed",
      code: "internal",
      message: "Docker 未运行",
      params: [],
    });
  });
});
