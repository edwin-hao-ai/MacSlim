import {
  Component,
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import {
  appGracefulQuitOperation,
  appTerminateOperation,
  classifyOperationError,
  errorText,
  executeOperation,
  listApplications,
  prepareOperation,
  type AppInfo,
  type PreparedOperation,
} from "@/lib/tauri";
import {
  Package,
  RefreshCw,
  Loader2,
  PowerOff,
  Zap,
  Search,
  X,
  ChevronRight,
  ChevronDown,
  ShieldAlert,
  ShieldCheck,
  AlertTriangle,
} from "lucide-solid";
import { fmtBytes } from "@/lib/format";
import OperationConfirm from "@/components/OperationConfirm";
import { useI18n } from "@/i18n";

function fmtUptime(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h`;
  return `${Math.floor(secs / 86400)}d`;
}

const ApplicationsView: Component = () => {
  const { t, tText } = useI18n();
  const [apps, setApps] = createSignal<AppInfo[]>([]);
  const [snapshotId, setSnapshotId] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [hideSystem, setHideSystem] = createSignal(true);
  const [expanded, setExpanded] = createSignal(new Set<string>());
  const [busy, setBusy] = createSignal<string | null>(null);
  const [message, setMessage] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [prepared, setPrepared] = createSignal<PreparedOperation | null>(null);
  const [quitTarget, setQuitTarget] = createSignal<AppInfo | null>(null);
  const [confirmForce, setConfirmForce] = createSignal<AppInfo | null>(null);

  const frozen = createMemo(
    () => confirmForce() !== null || prepared() !== null || busy() !== null,
  );

  let loadSeq = 0;
  const load = async (force = false) => {
    if (frozen() && !force) return;
    const seq = ++loadSeq;
    setLoading(true);
    try {
      const snapshot = await listApplications();
      if (seq !== loadSeq) return;
      if (!force && frozen()) return;
      setApps(snapshot.value);
      setSnapshotId(snapshot.snapshot_id);
    } catch (e) {
      if (seq !== loadSeq) return;
      setError(t("opError.failed", {
        error: errorText(classifyOperationError(e), tText),
      }));
    } finally {
      if (seq === loadSeq) setLoading(false);
    }
  };

  let timer: number | undefined;
  onMount(() => {
    void load();
  });
  createEffect(() => {
    if (frozen()) {
      if (timer !== undefined) {
        clearInterval(timer);
        timer = undefined;
      }
      return;
    }
    timer = window.setInterval(() => void load(), 5000);
  });
  onCleanup(() => {
    if (timer !== undefined) clearInterval(timer);
  });

  const filtered = createMemo(() => {
    const q = query().trim().toLowerCase();
    return apps().filter((a) => {
      if (hideSystem() && a.is_system) return false;
      if (q) {
        return (
          a.name.toLowerCase().includes(q) ||
          a.bundle_id.toLowerCase().includes(q) ||
          a.bundle_path.toLowerCase().includes(q)
        );
      }
      return true;
    });
  });

  const toggleExpand = (key: string) => {
    const next = new Set(expanded());
    next.has(key) ? next.delete(key) : next.add(key);
    setExpanded(next);
  };

  const canQuit = (app: AppInfo) => app.whitelisted_process_count === 0;

  const needsForceConfirm = (app: AppInfo) =>
    app.is_system || app.protected_process_count > 0;

  const quitTitle = (app: AppInfo) => {
    if (!canQuit(app)) return t("app.whitelistLocked");
    return t("app.quitTitle");
  };

  const executePrepared = async () => {
    const operation = prepared();
    if (!operation) return;
    setPrepared(null);
    setBusy(quitTarget()?.selection_key ?? null);
    setMessage(null);
    setError(null);
    try {
      const outcome = await executeOperation(operation.operation_id);
      if (outcome.kind === "app_graceful_quit") {
        const failed = outcome.value.filter((report) => report.quit_error !== null);
        if (failed.length > 0) {
          setError(
            failed
              .map((report) =>
                t("app.quitFailed", {
                  name: report.app_name,
                  error: report.quit_error
                    ? errorText(report.quit_error, tText)
                    : "",
                }),
              )
              .join("；"),
          );
        }
        setMessage(
          t("app.quitSuccess", {
            name: quitTarget()?.name ?? "",
            count: outcome.value.length - failed.length,
          }),
        );
        return;
      }
      if (outcome.kind !== "app_terminate") return;
      setMessage(
        t("app.forceQuitSuccess", {
          name: quitTarget()?.name ?? "",
          count: outcome.value.details.length,
        }),
      );
    } catch (e) {
      const info = classifyOperationError(e);
      setError(t(`opError.${info.kind}`, { error: errorText(info, tText) }));
    } finally {
      setPrepared(null);
      setBusy(null);
      await load(true);
    }
  };

  const quit = async (app: AppInfo) => {
    const current = snapshotId();
    if (!current || !canQuit(app) || app.is_system) return;
    setBusy(app.selection_key);
    setMessage(null);
    setError(null);
    try {
      setQuitTarget(app);
      setPrepared(
        await prepareOperation(
          appGracefulQuitOperation(current, [app.selection_key]),
        ),
      );
      return;
    } catch (e) {
      const info = classifyOperationError(e);
      setError(t(`opError.${info.kind}`, { error: errorText(info, tText) }));
      await load(true);
    } finally {
      setBusy(null);
    }
  };


  const forceQuitApp = async (app: AppInfo) => {
    const current = snapshotId();
    if (!current) return;
    setBusy(app.selection_key);
    setMessage(null);
    setError(null);
    try {
      setQuitTarget(app);
      setPrepared(
        await prepareOperation(
          appTerminateOperation(current, [app.selection_key], "force"),
        ),
      );
    } catch (e) {
      const info = classifyOperationError(e);
      setError(t(`opError.${info.kind}`, { error: errorText(info, tText) }));
      await load(true);
    } finally {
      setBusy(null);
    }
  };

  const forceQuit = (app: AppInfo) => {
    if (!canQuit(app) || app.is_system) return;
    if (needsForceConfirm(app)) {
      setConfirmForce(app);
      return;
    }
    void forceQuitApp(app);
  };

  const totalMem = createMemo(() =>
    filtered().reduce((s, a) => s + a.memory_mb, 0),
  );
  const totalCPU = createMemo(() =>
    filtered().reduce((s, a) => s + a.cpu_percent, 0),
  );

  return (
    <div class="flex flex-col h-full">
      <div class="px-6 py-4 border-b border-black/5 dark:border-white/5 flex items-center gap-4">
        <div class="flex-1 flex gap-6">
          <div>
            <div class="text-xs text-zinc-500">{t("app.appCount")}</div>
            <div class="text-lg font-semibold tabular-nums">
              {filtered().length}
            </div>
          </div>
          <div>
            <div class="text-xs text-zinc-500">{t("process.totalMemory")}</div>
            <div class="text-lg font-semibold tabular-nums">
              {(totalMem() / 1024).toFixed(1)}
              <span class="text-xs text-zinc-500 ml-1">GB</span>
            </div>
          </div>
          <div>
            <div class="text-xs text-zinc-500">{t("process.totalCpu")}</div>
            <div class="text-lg font-semibold tabular-nums">
              {totalCPU().toFixed(1)}
              <span class="text-xs text-zinc-500 ml-1">%</span>
            </div>
          </div>
        </div>

        <label class="inline-flex items-center gap-2 text-xs text-zinc-600 dark:text-zinc-300 cursor-pointer">
          <input
            type="checkbox"
            checked={hideSystem()}
            onChange={(e) => setHideSystem(e.currentTarget.checked)}
            class="accent-brand-500"
          />
          {t("app.hideSystem")}
        </label>

        <div class="relative">
          <Search size={14} class="absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400" />
          <input
            type="text"
            placeholder={t("app.searchPlaceholder")}
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            class="pl-8 pr-8 py-1.5 rounded-lg text-sm bg-black/5 dark:bg-white/5 border border-transparent focus:border-brand-500/50 focus:bg-white dark:focus:bg-zinc-800 outline-none w-[220px]"
          />
          <Show when={query()}>
            <button
              type="button"
              onClick={() => setQuery("")}
              class="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-400 hover:text-zinc-600"
            >
              <X size={12} />
            </button>
          </Show>
        </div>

        <button
          type="button"
          class="btn-ghost gap-1.5"
          disabled={loading()}
          onClick={() => void load(true)}
        >
          <Show when={!loading()} fallback={<Loader2 size={12} class="animate-spin" />}>
            <RefreshCw size={12} />
          </Show>
        </button>
      </div>

      <div class="flex-1 overflow-y-auto p-4 space-y-3">
        <For each={filtered()}>
          {(app) => {
            const isOpen = () => expanded().has(app.selection_key);
            const graceful = () => canQuit(app);
            const forceable = () => canQuit(app);
            return (
              <div class="card overflow-hidden">
                <div class="p-4">
                  <div class="flex items-center gap-3">
                    <button
                      type="button"
                      class="w-6 h-6 flex items-center justify-center text-zinc-400 hover:text-zinc-700"
                      onClick={() => toggleExpand(app.selection_key)}
                      title={isOpen() ? t("process.collapse") : t("process.expand")}
                    >
                      <Show
                        when={isOpen()}
                        fallback={<ChevronRight size={14} />}
                      >
                        <ChevronDown size={14} />
                      </Show>
                    </button>

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
                        <div class="font-semibold text-sm truncate">
                          {app.name}
                        </div>
                        <Show when={app.is_system}>
                          <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-zinc-500/15 text-zinc-600 dark:text-zinc-400">
                            {t("app.systemBadge")}
                          </span>
                        </Show>
                        <Show when={app.whitelisted_process_count > 0}>
                          <span class="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-brand-500/15 text-brand-600">
                            <ShieldCheck size={10} />
                            {t("app.whitelistBadge")}
                          </span>
                        </Show>
                        <Show when={app.protected_process_count > 0}>
                          <span class="inline-flex items-center gap-1 px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-warning-500/15 text-warning-600">
                            <ShieldAlert size={10} />
                            {t("app.cautionBadge")}
                          </span>
                        </Show>
                        <Show when={app.ports.length > 0}>
                          <span
                            class="px-1.5 py-0.5 rounded-md text-[10px] font-mono font-semibold bg-brand-500/15 text-brand-700 dark:text-brand-300"
                            title={t("app.listening", {
                              ports: app.ports.join(", "),
                            })}
                          >
                            :{app.ports.slice(0, 3).join(",")}
                            {app.ports.length > 3 && `+${app.ports.length - 3}`}
                          </span>
                        </Show>
                      </div>
                      <div class="text-[10px] text-zinc-400 truncate font-mono">
                        {app.bundle_id || app.bundle_path.split("/").pop()}
                      </div>
                    </div>

                    <div class="grid grid-cols-3 gap-3 text-center flex-shrink-0 mr-2">
                      <div>
                        <div class="text-[10px] text-zinc-500">
                          {t("process.totalMemory")}
                        </div>
                        <div class="text-sm font-semibold tabular-nums">
                          {fmtBytes(app.memory_mb * 1024 * 1024)}
                        </div>
                      </div>
                      <div>
                        <div class="text-[10px] text-zinc-500">CPU</div>
                        <div
                          class="text-sm font-semibold tabular-nums"
                          classList={{
                            "text-danger-600": app.cpu_percent > 50,
                            "text-warning-600":
                              app.cpu_percent > 20 && app.cpu_percent <= 50,
                          }}
                        >
                          {app.cpu_percent.toFixed(1)}%
                        </div>
                      </div>
                      <div>
                        <div class="text-[10px] text-zinc-500">
                          {t("app.childProcesses")}
                        </div>
                        <div class="text-sm font-semibold tabular-nums">
                          {app.all_pids.length}
                        </div>
                      </div>
                    </div>

                    <span class="text-[10px] text-zinc-400 flex-shrink-0">
                      {fmtUptime(app.uptime_secs)}
                    </span>

                    <div class="flex gap-1 flex-shrink-0">
                      <button
                        type="button"
                        class="btn-ghost !py-1.5 !px-2 !text-xs gap-1"
                        disabled={!graceful() || busy() === app.selection_key}
                        onClick={() => quit(app)}
                        title={quitTitle(app)}
                      >
                        <Show
                          when={busy() !== app.selection_key}
                          fallback={<Loader2 size={11} class="animate-spin" />}
                        >
                          <PowerOff size={11} />
                        </Show>
                        {t("app.quit")}
                      </button>
                      <button
                        type="button"
                        class="!py-1.5 !px-2 !text-xs gap-1 inline-flex items-center justify-center rounded-lg font-medium text-danger-600 hover:bg-danger-500/10 transition-colors"
                        disabled={!forceable() || busy() === app.selection_key}
                        onClick={() => forceQuit(app)}
                        title={t("app.forceQuitTitle")}
                      >
                        <Zap size={11} />
                        {t("app.forceQuit")}
                      </button>
                    </div>
                  </div>
                </div>

                <Show when={isOpen() && app.children.length > 0}>
                  <div class="border-t border-black/5 dark:border-white/5 bg-black/[0.02] dark:bg-white/[0.02]">
                    <div class="px-4 py-2 text-[10px] font-medium text-zinc-500 grid grid-cols-[1fr_72px_72px_56px_56px] gap-2">
                      <div>{t("app.childProcesses")}</div>
                      <div class="text-right">CPU</div>
                      <div class="text-right">{t("process.totalMemory")}</div>
                      <div class="text-right">{t("process.columnPid")}</div>
                      <div class="text-right">{t("process.columnAction")}</div>
                    </div>
                    <For each={app.children}>
                      {(child) => (
                        <div class="px-4 py-1.5 grid grid-cols-[1fr_72px_72px_56px_56px] gap-2 items-center text-xs hover:bg-black/[0.03] dark:hover:bg-white/[0.03] group">
                          <div class="min-w-0">
                            <div class="flex items-center gap-2">
                              <div
                                style={{
                                  "padding-left": `${child.depth * 14 + 4}px`,
                                }}
                                class="text-zinc-400 font-mono flex-shrink-0"
                              >
                                <Show when={child.depth > 0}>
                                  <span>└</span>
                                </Show>
                              </div>
                              <span
                                class="truncate"
                                classList={{
                                  "font-semibold": child.is_main,
                                  "text-zinc-700 dark:text-zinc-300":
                                    child.is_main,
                                }}
                              >
                                {child.name}
                              </span>
                              <Show when={child.is_main}>
                                <span class="px-1 py-0 rounded text-[9px] font-medium bg-brand-500/15 text-brand-600">
                                  {t("app.mainBadge")}
                                </span>
                              </Show>
                              <Show when={child.whitelisted}>
                                <span class="inline-flex items-center gap-1 px-1 py-0 rounded text-[9px] font-medium bg-brand-500/15 text-brand-600">
                                  <ShieldCheck size={9} />
                                  {t("app.whitelistBadge")}
                                </span>
                              </Show>
                              <Show when={child.protected && !child.whitelisted}>
                                <span class="inline-flex items-center gap-1 px-1 py-0 rounded text-[9px] font-medium bg-warning-500/15 text-warning-600">
                                  <ShieldAlert size={9} />
                                  {t("app.cautionBadge")}
                                </span>
                              </Show>
                              <Show when={child.ports.length > 0}>
                                <span class="px-1 py-0 rounded text-[9px] font-mono font-semibold bg-brand-500/10 text-brand-700 dark:text-brand-300">
                                  :{child.ports.join(",")}
                                </span>
                              </Show>
                            </div>
                            <Show when={child.protected_reason_key}>
                              <div class="text-[10px] text-warning-600 dark:text-warning-400 truncate mt-0.5">
                                {tText(
                                  child.protected_reason_key!,
                                  child.protected_reason_params,
                                )}
                              </div>
                            </Show>
                          </div>
                          <div class="text-right tabular-nums text-[11px]">
                            {child.cpu_percent.toFixed(1)}%
                          </div>
                          <div class="text-right tabular-nums text-[11px] text-zinc-600 dark:text-zinc-400">
                            {child.memory_mb < 1024
                              ? `${child.memory_mb.toFixed(0)}M`
                              : `${(child.memory_mb / 1024).toFixed(1)}G`}
                          </div>
                          <div class="text-right tabular-nums text-[10px] text-zinc-400 font-mono">
                            {child.pid}
                          </div>
                          <div class="text-right text-[10px] text-zinc-400">
                            {child.whitelisted
                              ? t("app.whitelistBadge")
                              : child.protected
                                ? t("app.cautionBadge")
                                : ""}
                          </div>
                        </div>
                      )}
                    </For>
                  </div>
                </Show>
              </div>
            );
          }}
        </For>

        <Show when={filtered().length === 0 && !loading()}>
          <div class="text-center py-20 text-sm text-zinc-500">
            {query() || hideSystem() ? t("app.noMatch") : t("app.noRunning")}
          </div>
        </Show>
      </div>

      <Show when={message() || error()}>
        <div
          class="px-6 py-2 border-t border-black/5 dark:border-white/5 text-xs"
          classList={{
            "text-zinc-600 dark:text-zinc-400": !error(),
            "text-danger-600 dark:text-danger-400": !!error(),
          }}
          role={error() ? "alert" : undefined}
        >
          {error() ?? message()}
        </div>
      </Show>

      <Show when={prepared()}>
        {(operation) => (
          <OperationConfirm
            prepared={operation()}
            title={t("opConfirm.title")}
            confirmLabel={t("opConfirm.confirm")}
            busy={busy() !== null}
            onConfirm={() => void executePrepared()}
            onCancel={() => setPrepared(null)}
          />
        )}
      </Show>

      <Show when={confirmForce()}>
        {(app) => (
          <div
            class="fixed inset-0 bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center p-6 animate-fade-in"
            onClick={() => setConfirmForce(null)}
          >
            <div
              class="card p-6 max-w-md w-full animate-slide-up"
              onClick={(e) => e.stopPropagation()}
            >
              <div class="flex items-start gap-3">
                <div class="w-10 h-10 rounded-xl bg-warning-500/15 flex items-center justify-center flex-shrink-0">
                  <AlertTriangle size={20} class="text-warning-600" />
                </div>
                <div class="flex-1">
                  <h3 class="font-semibold">
                    {t("app.confirmForceTitle", { name: app().name })}
                  </h3>
                  <p class="text-sm text-zinc-500 mt-1">
                    {t("app.confirmForceMessage")}
                  </p>
                  <div class="mt-3 text-xs text-zinc-500">
                    {t("app.confirmForceCounts", {
                      protected: app().protected_process_count,
                      whitelisted: app().whitelisted_process_count,
                    })}
                  </div>
                </div>
              </div>
              <div class="flex justify-end gap-2 mt-5">
                <button
                  type="button"
                  class="btn-ghost"
                  onClick={() => setConfirmForce(null)}
                >
                  {t("common.cancel")}
                </button>
                <button
                  type="button"
                  class="inline-flex items-center justify-center rounded-xl px-5 py-2.5 font-medium bg-danger-500 hover:bg-danger-400 text-white shadow-sm transition-all"
                  onClick={async () => {
                    const target = app();
                    setConfirmForce(null);
                    await forceQuitApp(target);
                  }}
                >
                  {t("app.confirmForce")}
                </button>
              </div>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
};

export default ApplicationsView;
