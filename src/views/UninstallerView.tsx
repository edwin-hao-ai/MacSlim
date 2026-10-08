import {
  Component,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import {
  checkAppRunning,
  classifyOperationError,
  errorText,
  executeOperation,
  onResidueScanProgress,
  prepareOperation,
  scanAppResiduesBatch,
  scanInstalledApps,
  uninstallOperation,
  type InstalledApp,
  type PreparedOperation,
  type ResidueAppGroup,
  type ResidueItem,
  type StageUpdate,
  type UninstallReport,
  type UnlistenFn,
} from "@/lib/tauri";
import { fmtBytes } from "@/lib/format";
import OperationConfirm from "@/components/OperationConfirm";
import ScanStageProgress from "@/components/ScanStageProgress";
import { can } from "@/lib/flavor";
import { useI18n } from "@/i18n";
import {
  Search,
  X,
  Loader2,
  Package,
  Trash2,
  AlertTriangle,
  CheckCircle2,
  XCircle,
  ChevronDown,
  ChevronRight,
} from "lucide-solid";

type Phase =
  | "list"
  | "residue"
  | "running"
  | "uninstalling"
  | "done";

const UninstallerView: Component = () => {
  const { t, tText } = useI18n();

  const [apps, setApps] = createSignal<InstalledApp[]>([]);
  const [appSnapshotId, setAppSnapshotId] = createSignal<string | null>(null);
  const [scanning, setScanning] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [hideSystem, setHideSystem] = createSignal(true);
  const [selectedApps, setSelectedApps] = createSignal(new Set<string>());

  const [phase, setPhase] = createSignal<Phase>("list");
  const [residueGroups, setResidueGroups] = createSignal<ResidueAppGroup[]>([]);
  const [residueSnapshotId, setResidueSnapshotId] = createSignal<string | null>(
    null,
  );
  const [residueLoading, setResidueLoading] = createSignal(false);
  const [residueSelection, setResidueSelection] = createSignal(
    new Map<string, boolean>(),
  );
  const [expandedCategories, setExpandedCategories] = createSignal(
    new Set<string>(),
  );

  const [reports, setReports] = createSignal<UninstallReport[]>([]);
  const [runningApp, setRunningApp] = createSignal<InstalledApp | null>(null);
  const [prepared, setPrepared] = createSignal<PreparedOperation | null>(null);
  const [forceQuitTimer, setForceQuitTimer] = createSignal(0);
  const [error, setError] = createSignal<string | null>(null);
  const [stageCurrent, setStageCurrent] = createSignal<string | null>(null);
  const [residueDone, setResidueDone] = createSignal(0);
  const [residueFound, setResidueFound] = createSignal(0);

  // onResidueScanProgress 是异步的：组件可能在它 resolve 之前就卸载。
  // disposed 让「注册」和「解绑」两个动作在任何顺序下都不漏对方。
  // 句柄用集合而不是单个变量：enterResiduePhase 可能被重复触发，
  // 单槽会让后一次覆盖前一次，导致前一个监听永久泄漏。
  let disposed = false;
  const stageListeners = new Set<UnlistenFn>();

  /** 取出并调用一个解绑句柄；已经释放过就什么都不做（保证恰好一次）。 */
  const releaseStageListener = (unlisten: UnlistenFn) => {
    if (!stageListeners.delete(unlisten)) return;
    unlisten();
  };

  const releaseAllStageListeners = () => {
    for (const unlisten of [...stageListeners]) releaseStageListener(unlisten);
  };

  onCleanup(() => {
    disposed = true;
    releaseAllStageListeners();
  });

  const loadApps = async () => {
    setScanning(true);
    try {
      const snapshot = await scanInstalledApps();
      setApps(snapshot.value);
      setAppSnapshotId(snapshot.snapshot_id);
      setSelectedApps(new Set<string>());
    } catch (e) {
      setError(t("opError.failed", {
        error: errorText(classifyOperationError(e), tText),
      }));
    } finally {
      setScanning(false);
    }
  };

  onMount(() => {
    void loadApps();
  });

  const filtered = createMemo(() => {
    const q = query().trim().toLowerCase();
    return apps().filter((a) => {
      if (hideSystem() && a.is_system) return false;
      if (q) {
        return a.name.toLowerCase().includes(q) || a.bundle_id.toLowerCase().includes(q);
      }
      return true;
    });
  });

  const toggleApp = (key: string) => {
    const next = new Set(selectedApps());
    next.has(key) ? next.delete(key) : next.add(key);
    setSelectedApps(next);
  };

  const selectedAppList = createMemo(() =>
    apps().filter((a) => selectedApps().has(a.selection_key)),
  );

  const estimatedFreeBytes = createMemo(() =>
    selectedAppList().reduce(
      (s, a) => s + a.bundle_size_bytes + a.estimated_residue_bytes,
      0,
    ),
  );

  const onStageUpdate = (u: StageUpdate) => {
    setStageCurrent(u.stage);
    setResidueDone((n) => n + 1);
    setResidueFound((n) => n + u.found_bytes);
  };

  const enterResiduePhase = async () => {
    const currentAppSnapshot = appSnapshotId();
    const keys = Array.from(selectedApps());
    if (!currentAppSnapshot || keys.length === 0) return;
    setPhase("residue");
    setResidueLoading(true);
    setResidueGroups([]);
    setError(null);
    setStageCurrent(null);
    setResidueDone(0);
    setResidueFound(0);
    let unlisten: UnlistenFn | undefined;
    try {
      const registered = await onResidueScanProgress(onStageUpdate);
      if (disposed) {
        // 组件已卸载：立刻解绑，并且不再发起一次没人看的扫描
        registered();
        return;
      }
      unlisten = registered;
      stageListeners.add(unlisten);
      const snapshot = await scanAppResiduesBatch(currentAppSnapshot, keys);
      setResidueGroups(snapshot.value);
      setResidueSnapshotId(snapshot.snapshot_id);
      const sel = new Map<string, boolean>();
      for (const group of snapshot.value) {
        for (const item of group.residue.items) {
          sel.set(item.selection_key, item.selected);
        }
      }
      setResidueSelection(sel);
    } catch (e) {
      setError(t("opError.failed", {
        error: errorText(classifyOperationError(e), tText),
      }));
      setPhase("list");
    } finally {
      if (unlisten) releaseStageListener(unlisten);
      setResidueLoading(false);
    }
  };

  const selectedResidueBytes = createMemo(() => {
    const sel = residueSelection();
    let total = 0;
    for (const group of residueGroups()) {
      for (const item of group.residue.items) {
        if (sel.get(item.selection_key)) total += item.size_bytes;
      }
    }
    return total;
  });

  const totalUninstallBytes = createMemo(
    () =>
      selectedAppList().reduce((s, a) => s + a.bundle_size_bytes, 0) +
      selectedResidueBytes(),
  );

  const selectedResidueKeys = createMemo(() => {
    const sel = residueSelection();
    const keys: string[] = [];
    for (const group of residueGroups()) {
      for (const item of group.residue.items) {
        if (sel.get(item.selection_key)) keys.push(item.selection_key);
      }
    }
    return keys;
  });

  const toggleResidue = (key: string) => {
    const next = new Map(residueSelection());
    next.set(key, !next.get(key));
    setResidueSelection(next);
  };

  const toggleAllResidues = (selectAll: boolean) => {
    const next = new Map<string, boolean>();
    for (const group of residueGroups()) {
      for (const item of group.residue.items) {
        next.set(item.selection_key, selectAll);
      }
    }
    setResidueSelection(next);
  };

  const toggleCategory = (key: string) => {
    const next = new Set(expandedCategories());
    next.has(key) ? next.delete(key) : next.add(key);
    setExpandedCategories(next);
  };

  const startUninstall = async () => {
    setError(null);
    for (const app of selectedAppList()) {
      if (!app.is_running) continue;
      try {
        if (await checkAppRunning(app.bundle_path)) {
          setRunningApp(app);
          setPhase("running");
          setForceQuitTimer(5);
          const interval = window.setInterval(() => {
            setForceQuitTimer((v) => {
              if (v <= 1) {
                clearInterval(interval);
                return 0;
              }
              return v - 1;
            });
          }, 1000);
          return;
        }
      } catch {
        /* the backend rejects running apps that are not running */
      }
    }
    await requestUninstall(false);
  };

  const requestUninstall = async (quitRunning: boolean) => {
    const appSnapshot = appSnapshotId();
    const residueSnapshot = residueSnapshotId();
    if (!appSnapshot || !residueSnapshot) return;
    setError(null);
    try {
      // prepare 只生成不可逆计划：真正的删除必须等用户在确认弹窗里点头。
      setPrepared(
        await prepareOperation(
          uninstallOperation(
            appSnapshot,
            residueSnapshot,
            Array.from(selectedApps()),
            selectedResidueKeys(),
            quitRunning,
          ),
        ),
      );
    } catch (e) {
      const info = classifyOperationError(e);
      setError(t(`opError.${info.kind}`, { error: errorText(info, tText) }));
      setPhase("residue");
    }
  };

  const executePrepared = async () => {
    const operation = prepared();
    if (!operation) return;
    setPrepared(null);
    setPhase("uninstalling");
    setError(null);
    try {
      const outcome = await executeOperation(operation.operation_id);
      if (outcome.kind !== "uninstall") {
        setPhase("done");
        return;
      }
      setReports(outcome.value);
      setPhase("done");
    } catch (e) {
      const info = classifyOperationError(e);
      setError(t(`opError.${info.kind}`, { error: errorText(info, tText) }));
      setPhase("residue");
    }
  };

  const totalTrashed = createMemo(() =>
    reports().reduce((s, r) => s + r.trashed_bytes, 0),
  );
  const totalMoved = createMemo(() =>
    reports().reduce((s, r) => s + r.moved_count, 0),
  );
  const totalFailed = createMemo(() =>
    reports().reduce((s, r) => s + r.failed_count, 0),
  );
  const quitFailures = createMemo(() =>
    reports()
      .filter((r) => r.quit_error)
      .map((r) => ({
        app: r.app_name,
        error: r.quit_error ? errorText(r.quit_error, tText) : "",
      })),
  );

  const resetToList = () => {
    setPhase("list");
    setSelectedApps(new Set<string>());
    setResidueGroups([]);
    setResidueSnapshotId(null);
    setResidueSelection(new Map<string, boolean>());
    setReports([]);
    setRunningApp(null);
    setPrepared(null);
    setError(null);
    void loadApps();
  };

  const AppListView = () => (
    <div class="flex flex-col h-full">
      <Show when={!can("appUninstall")}>
        {/* App Store 版收起了「卸载」动作：沙箱里 exec 不了 osascript，
            也写不了 /Applications 与 ~/Library/Application Support。
            体积分析是只读的、照常可用，所以这一页保留，只说明少掉的是什么
            —— 与进程页对「终止」的处理同一套口径。 */}
        <div class="mx-6 mt-4 rounded-lg bg-brand-500/8 border border-brand-500/20 px-4 py-3 text-xs text-zinc-600 dark:text-zinc-300 leading-relaxed">
          {t("uninstaller.sandboxNotice")}
        </div>
      </Show>
      <div class="px-6 py-4 border-b border-black/5 dark:border-white/5 flex items-center gap-4">
        <label class="inline-flex items-center gap-2 text-xs text-zinc-600 dark:text-zinc-300 cursor-pointer">
          <input
            type="checkbox"
            checked={hideSystem()}
            onChange={(e) => setHideSystem(e.currentTarget.checked)}
            class="accent-brand-500"
          />
          {t("uninstaller.hideSystem")}
        </label>
        <div class="relative flex-1 max-w-[320px]">
          <Search size={14} class="absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400" />
          <input
            type="text"
            placeholder={t("uninstaller.search")}
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            class="w-full pl-8 pr-8 py-1.5 rounded-lg text-sm bg-black/5 dark:bg-white/5 border border-transparent focus:border-brand-500/50 focus:bg-white dark:focus:bg-zinc-800 outline-none"
          />
          <Show when={query()}>
            <button type="button" onClick={() => setQuery("")} class="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-400 hover:text-zinc-600">
              <X size={12} />
            </button>
          </Show>
        </div>
        <Show when={scanning()}>
          <div class="flex items-center gap-2 text-xs text-zinc-500">
            <Loader2 size={14} class="animate-spin" />
            {t("uninstaller.scanning")}
          </div>
        </Show>
      </div>

      <div class="flex-1 overflow-y-auto p-4 space-y-2">
        <Show when={!scanning() && filtered().length === 0}>
          <div class="text-center py-20 text-sm text-zinc-500">{t("uninstaller.noApps")}</div>
        </Show>
        <Show when={scanning() && apps().length === 0}>
          <div class="space-y-3">
            <For each={[1, 2, 3, 4, 5]}>
              {() => (
                <div class="card p-4 animate-pulse">
                  <div class="flex items-center gap-3">
                    <div class="w-10 h-10 rounded-xl bg-zinc-200 dark:bg-zinc-700" />
                    <div class="flex-1 space-y-2">
                      <div class="h-4 w-32 bg-zinc-200 dark:bg-zinc-700 rounded" />
                      <div class="h-3 w-48 bg-zinc-100 dark:bg-zinc-800 rounded" />
                    </div>
                    <div class="h-4 w-16 bg-zinc-200 dark:bg-zinc-700 rounded" />
                  </div>
                </div>
              )}
            </For>
          </div>
        </Show>
        <For each={filtered()}>
          {(app) => {
            const isSelected = () => selectedApps().has(app.selection_key);
            return (
              <div
                class="card p-4 transition-all duration-200 hover:scale-[1.01] hover:shadow-md cursor-pointer"
                classList={{ "ring-2 ring-brand-500/50": isSelected() }}
                onClick={() =>
                  can("appUninstall") && !app.is_system && toggleApp(app.selection_key)
                }
              >
                <div class="flex items-center gap-3">
                  <Show when={can("appUninstall") && !app.is_system}>
                    <input
                      type="checkbox"
                      checked={isSelected()}
                      onChange={() => toggleApp(app.selection_key)}
                      onClick={(e) => e.stopPropagation()}
                      class="w-4 h-4 accent-brand-500 flex-shrink-0"
                    />
                  </Show>
                  <Show when={app.icon_base64} fallback={
                    <div class="w-10 h-10 rounded-xl bg-gradient-to-br from-brand-400 to-brand-600 flex items-center justify-center flex-shrink-0 text-white">
                      <Package size={18} />
                    </div>
                  }>
                    <img
                      src={`data:image/png;base64,${app.icon_base64}`}
                      alt={app.name}
                      class="w-10 h-10 rounded-xl flex-shrink-0"
                    />
                  </Show>
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2">
                      <span class="font-semibold text-sm truncate">{app.name}</span>
                      <Show when={app.is_system}>
                        <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-zinc-500/15 text-zinc-600 dark:text-zinc-400">
                          {t("uninstaller.systemApp")}
                        </span>
                      </Show>
                      <Show when={app.is_running}>
                        <span class="w-2 h-2 rounded-full bg-success-500 flex-shrink-0" title={t("uninstaller.appRunning")} />
                      </Show>
                    </div>
                    <div class="text-[10px] text-zinc-400 truncate font-mono">{app.bundle_id || app.bundle_path}</div>
                  </div>
                  <div class="text-right flex-shrink-0">
                    <div class="text-sm font-semibold tabular-nums">{fmtBytes(app.bundle_size_bytes)}</div>
                    <div class="text-[10px] text-zinc-500">{t("uninstaller.appSize")}</div>
                  </div>
                </div>
              </div>
            );
          }}
        </For>
      </div>

      <Show when={can("appUninstall") && selectedApps().size > 0}>
        <div class="px-6 py-3 border-t border-black/5 dark:border-white/5 flex items-center gap-4 bg-white/50 dark:bg-zinc-900/50 backdrop-blur-sm">
          <span class="text-sm text-zinc-600 dark:text-zinc-300">
            {t("uninstaller.selectedCount", { count: selectedApps().size })}
          </span>
          <span class="text-sm text-zinc-500">
            {t("uninstaller.estimatedFree", { size: fmtBytes(estimatedFreeBytes()) })}
          </span>
          <button
            type="button"
            class="btn-primary gap-2 ml-auto"
            onClick={() => void enterResiduePhase()}
          >
            <Trash2 size={16} />
            {t("uninstaller.uninstallSelected")}
          </button>
        </div>
      </Show>
    </div>
  );

  const ResidueView = () => {
    const groupedResidues = createMemo(() =>
      residueGroups().map((group) => {
        const groups = new Map<string, ResidueItem[]>();
        for (const item of group.residue.items) {
          const cat = item.is_dev_tool
            ? t("uninstaller.devToolData")
            : item.category;
          if (!groups.has(cat)) groups.set(cat, []);
          groups.get(cat)!.push(item);
        }
        return { group, groups: Array.from(groups.entries()) };
      }),
    );

    const allSelected = createMemo(() => {
      const sel = residueSelection();
      for (const group of residueGroups()) {
        for (const item of group.residue.items) {
          if (!sel.get(item.selection_key)) return false;
        }
      }
      return residueGroups().some((group) => group.residue.items.length > 0);
    });

    return (
      <div class="flex flex-col h-full">
        <div class="px-6 py-4 border-b border-black/5 dark:border-white/5 flex items-center gap-4">
          <button type="button" class="btn-ghost text-xs" onClick={() => setPhase("list")}>
            {t("common.back")}
          </button>
          <span class="text-sm font-medium">
            {t("uninstaller.selectedCount", { count: selectedApps().size })}
          </span>
          <button
            type="button"
            class="ml-auto text-xs text-brand-600 hover:underline"
            onClick={() => toggleAllResidues(!allSelected())}
          >
            {allSelected() ? t("uninstaller.deselectAll") : t("uninstaller.selectAll")}
          </button>
        </div>

        <div class="flex-1 overflow-y-auto p-4 space-y-4">
          <Show when={residueLoading()}>
            <div class="py-8">
              <ScanStageProgress
                current={stageCurrent()}
                doneCount={residueDone()}
                total={selectedAppList().length}
                foundBytes={residueFound()}
              />
            </div>
          </Show>
          <For each={groupedResidues()}>
            {({ group, groups }) => (
              <div class="card p-4 space-y-3">
                <div class="flex items-center gap-2">
                  <span class="font-semibold text-sm">{group.residue.app_name}</span>
                  <span class="text-xs text-zinc-500">
                    {fmtBytes(group.residue.total_bytes)}
                  </span>
                  <Show when={!group.residue.scan_complete}>
                    <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-warning-500/15 text-warning-600">
                      {t("uninstaller.residueIncomplete")}
                    </span>
                  </Show>
                </div>
                <For each={groups}>
                  {([category, items]) => {
                    const catKey = () =>
                      `${group.residue.bundle_id}:${category}`;
                    const isOpen = () =>
                      expandedCategories().has(catKey()) || groups.length <= 3;
                    return (
                      <div>
                        <button
                          type="button"
                          class="flex items-center gap-2 text-xs text-zinc-600 dark:text-zinc-400 hover:text-zinc-800 dark:hover:text-zinc-200 w-full"
                          onClick={() => toggleCategory(catKey())}
                        >
                          <Show when={isOpen()} fallback={<ChevronRight size={12} />}>
                            <ChevronDown size={12} />
                          </Show>
                          <span class="font-medium">{category}</span>
                          <span class="text-zinc-400">
                            {items.length} ·{" "}
                            {fmtBytes(items.reduce((s, i) => s + i.size_bytes, 0))}
                          </span>
                        </button>
                        <Show when={isOpen()}>
                          <ul class="mt-1 space-y-0.5 ml-5">
                            <For each={items}>
                              {(item) => (
                                <li class="flex items-center gap-2 py-1 text-xs hover:bg-black/[0.02] dark:hover:bg-white/[0.02] rounded px-1">
                                  <input
                                    type="checkbox"
                                    checked={residueSelection().get(item.selection_key) ?? false}
                                    onChange={() => toggleResidue(item.selection_key)}
                                    class="w-3.5 h-3.5 accent-brand-500"
                                  />
                                  <span class="truncate flex-1 font-mono text-zinc-500">{item.path}</span>
                                  <span class="tabular-nums text-zinc-600 dark:text-zinc-400 flex-shrink-0">
                                    {fmtBytes(item.size_bytes)}
                                  </span>
                                </li>
                              )}
                            </For>
                          </ul>
                        </Show>
                      </div>
                    );
                  }}
                </For>
              </div>
            )}
          </For>
        </div>

        <Show when={error()}>
          <p class="px-6 py-2 text-xs text-danger-600 dark:text-danger-400" role="alert">
            {error()}
          </p>
        </Show>

        <div class="px-6 py-3 border-t border-black/5 dark:border-white/5 flex items-center gap-4 bg-white/50 dark:bg-zinc-900/50 backdrop-blur-sm">
          <span class="text-sm text-zinc-600 dark:text-zinc-300">
            {t("uninstaller.totalSize")}: {fmtBytes(totalUninstallBytes())}
          </span>
          <button
            type="button"
            class="btn-primary gap-2 ml-auto"
            onClick={() => void startUninstall()}
          >
            <Trash2 size={16} />
            {t("uninstaller.uninstallSelected")}
          </button>
        </div>
      </div>
    );
  };

  const RunningDialog = () => (
    <div class="fixed inset-0 bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center p-6 animate-fade-in" onClick={() => setPhase("residue")}>
      <div class="card p-6 max-w-md w-full animate-slide-up" onClick={(e) => e.stopPropagation()}>
        <div class="flex items-start gap-3">
          <div class="w-10 h-10 rounded-xl bg-warning-500/15 flex items-center justify-center flex-shrink-0">
            <AlertTriangle size={20} class="text-warning-600" />
          </div>
          <div class="flex-1">
            <h3 class="font-semibold">{t("uninstaller.appRunning")}</h3>
            <p class="text-sm text-zinc-500 mt-1">
              {runningApp()?.name} {t("uninstaller.appRunning").toLowerCase()}
            </p>
          </div>
        </div>
        <div class="flex justify-end gap-2 mt-5">
          <button type="button" class="btn-ghost" onClick={() => setPhase("residue")}>
            {t("uninstaller.cancel")}
          </button>
          <Show when={forceQuitTimer() <= 0} fallback={
            <button
              type="button"
              class="inline-flex items-center justify-center rounded-xl px-5 py-2.5 font-medium bg-brand-500 hover:bg-brand-400 text-white shadow-sm transition-all"
              onClick={() => void requestUninstall(true)}
            >
              {t("uninstaller.quitAndUninstall")}
            </button>
          }>
            <button
              type="button"
              class="inline-flex items-center justify-center rounded-xl px-5 py-2.5 font-medium bg-danger-500 hover:bg-danger-400 text-white shadow-sm transition-all"
              onClick={() => void requestUninstall(true)}
            >
              {t("uninstaller.forceQuitAndUninstall")}
            </button>
          </Show>
        </div>
      </div>
    </div>
  );

  const UninstallingView = () => (
    <div class="flex flex-col items-center justify-center h-full gap-4">
      <Loader2 size={32} class="animate-spin text-brand-500" />
      <span class="text-sm text-zinc-600 dark:text-zinc-300">{t("uninstaller.uninstalling")}</span>
    </div>
  );

  const DoneView = () => (
    <div class="flex flex-col items-center justify-center h-full gap-6 p-6">
      <div class="w-16 h-16 rounded-full bg-success-500/15 flex items-center justify-center">
        <CheckCircle2 size={32} class="text-success-600" />
      </div>
      <h2 class="text-lg font-semibold">{t("uninstaller.complete")}</h2>
      <div class="text-3xl font-bold tabular-nums text-success-600">
        {fmtBytes(totalTrashed())}
      </div>
      <div class="text-sm text-zinc-500 space-y-1 text-center">
        <div>{t("result.trashedPending", { size: fmtBytes(totalTrashed()) })}</div>
        <div>{t("uninstaller.cleanedFiles", { count: totalMoved() })}</div>
        <Show when={totalFailed() > 0}>
          <div class="text-warning-600">{t("uninstaller.failedFiles", { count: totalFailed() })}</div>
        </Show>
        <For each={quitFailures()}>
          {(failure) => (
            <div class="text-warning-600">
              {t("uninstaller.quitFailedButRemoved", {
                app: failure.app,
                error: failure.error,
              })}
            </div>
          )}
        </For>
      </div>
      <Show when={error()}>
        <p class="text-xs text-danger-600 dark:text-danger-400 text-center" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={totalFailed() > 0}>
        <div class="card p-4 w-full max-w-lg border-danger-500/20">
          <div class="flex items-center gap-2 mb-2">
            <XCircle size={16} class="text-danger-500" />
            <span class="font-medium text-sm">{t("uninstaller.failedFiles", { count: totalFailed() })}</span>
          </div>
          <ul class="text-xs space-y-1 max-h-40 overflow-y-auto">
            <For each={reports().flatMap((r) => r.details.filter((d) => !d.success))}>
              {(detail) => (
                <li class="flex gap-2">
                  <span class="truncate font-mono text-zinc-500 flex-1">{detail.path}</span>
                  <span class="text-danger-500 flex-shrink-0">
                    {detail.error ? errorText(detail.error, tText) : ""}
                  </span>
                </li>
              )}
            </For>
          </ul>
        </div>
      </Show>
      <button type="button" class="btn-primary mt-4" onClick={resetToList}>
        {t("common.back")}
      </button>
    </div>
  );

  return (
    <div class="h-full relative">
      <Show when={phase() === "list"}>
        <AppListView />
      </Show>
      <Show when={phase() === "residue"}>
        <ResidueView />
      </Show>
      <Show when={phase() === "running"}>
        <ResidueView />
        <RunningDialog />
      </Show>
      <Show when={phase() === "uninstalling"}>
        <UninstallingView />
      </Show>
      <Show when={phase() === "done"}>
        <DoneView />
      </Show>

      <Show when={prepared()}>
        {(operation) => (
          <OperationConfirm
            prepared={operation()}
            title={t("opConfirm.title")}
            confirmLabel={t("opConfirm.confirm")}
            cancelLabel={t("uninstaller.cancel")}
            onConfirm={() => void executePrepared()}
            onCancel={() => setPrepared(null)}
          />
        )}
      </Show>
    </div>
  );
};

export default UninstallerView;
