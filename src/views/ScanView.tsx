import {
  Component,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
  Show,
} from "solid-js";
import HealthCard from "@/components/HealthCard";
import ProcessList from "@/components/ProcessList";
import Welcome from "@/components/Welcome";
import CleanupFlash from "@/components/CleanupFlash";
import { can } from "@/lib/flavor";
import OperationConfirm, {
  ProtectedForceConfirm,
  type ProtectedRow,
} from "@/components/OperationConfirm";
import {
  addWhitelist,
  classifyOperationError,
  errorText,
  executeOperation,
  prepareOperation,
  processOperation,
  scanAll,
  type PreparedOperation,
  type ProcessInfo,
  type ProcessMode,
  type ScanResult,
  type SnapshotResult,
  type SystemHealth,
} from "@/lib/tauri";
import { Sparkles, RefreshCw, Loader2 } from "lucide-solid";
import { playCleanSuccessSound } from "@/lib/cleanFeedback";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useI18n } from "@/i18n";

const WELCOME_SEEN_KEY = "macslim.welcome.seen";

const ScanView: Component = () => {
  const { t, tText } = useI18n();
  const [snapshot, setSnapshot] = createSignal<SnapshotResult<ScanResult> | null>(null);
  const [scanning, setScanning] = createSignal(false);
  const [selected, setSelected] = createSignal(new Set<string>());
  const [optimizing, setOptimizing] = createSignal(false);
  const [message, setMessage] = createSignal<string | null>(null);
  const [showWelcome, setShowWelcome] = createSignal(
    localStorage.getItem(WELCOME_SEEN_KEY) !== "true",
  );
  const [showFlash, setShowFlash] = createSignal(false);
  const [prepared, setPrepared] = createSignal<PreparedOperation | null>(null);
  const [confirmForce, setConfirmForce] = createSignal<null | {
    keys: string[];
    protected: ProtectedRow[];
  }>(null);

  const processes = createMemo(() => snapshot()?.value.processes ?? []);

  /** 默认选择只收「后端说可以默认选中且不受保护也不在白名单」的行。 */
  const defaultSelectedKeys = (rows: ProcessInfo[]): string[] =>
    rows
      .filter(
        (p) => p.default_select && !p.protected && !p.whitelisted,
      )
      .map((p) => p.selection_key);

  const runScan = async () => {
    setScanning(true);
    setMessage(null);
    try {
      const next = await scanAll();
      setSnapshot(next);
      setSelected(new Set(defaultSelectedKeys(next.value.processes)));
    } catch (e) {
      setMessage(t("scan.scanFailed", { error: String(e) }));
    } finally {
      setScanning(false);
    }
  };

  let unlistenHealth: UnlistenFn | undefined;
  let unlistenOptimize: UnlistenFn | undefined;
  let disposed = false;

  const stopListener = (unlisten: UnlistenFn | undefined, label: string) => {
    if (!unlisten) return;
    void Promise.resolve()
      .then(() => unlisten())
      .catch((error: unknown) => {
        console.error(`取消${label}监听失败`, error);
      });
  };

  onMount(async () => {
    try {
      if (!showWelcome()) {
        await runScan();
      }
      if (disposed) return;
      const healthUnlisten = await listen<SystemHealth>("health:update", (e) => {
        const current = snapshot();
        if (!current) return;
        setSnapshot({ ...current, value: { ...current.value, health: e.payload } });
      });
      if (disposed) {
        stopListener(healthUnlisten, "健康更新");
        return;
      }
      unlistenHealth = healthUnlisten;
      // 托盘「一键优化」菜单触发
      const optimizeUnlisten = await listen<void>("tray:optimize", async () => {
        await runScan();
        await optimize();
      });
      if (disposed) {
        stopListener(optimizeUnlisten, "一键优化");
        return;
      }
      unlistenOptimize = optimizeUnlisten;
    } catch (error: unknown) {
      console.error("注册扫描事件监听失败", error);
    }
  });
  onCleanup(() => {
    disposed = true;
    stopListener(unlistenHealth, "健康更新");
    stopListener(unlistenOptimize, "一键优化");
    unlistenHealth = undefined;
    unlistenOptimize = undefined;
  });

  const handleStart = async () => {
    localStorage.setItem(WELCOME_SEEN_KEY, "true");
    setShowWelcome(false);
    await runScan();
  };

  const toggle = (selectionKey: string) => {
    const next = new Set(selected());
    next.has(selectionKey) ? next.delete(selectionKey) : next.add(selectionKey);
    setSelected(next);
  };

  const requestOptimize = async (keys: string[], mode: ProcessMode) => {
    const current = snapshot();
    if (!current || keys.length === 0) return;
    setMessage(null);
    try {
      setPrepared(
        await prepareOperation(processOperation(current.snapshot_id, keys, mode)),
      );
    } catch (e) {
      await runScan();
      const info = classifyOperationError(e);
      setMessage(t(`opError.${info.kind}`, { error: errorText(info, tText) }));
    }
  };

  const optimize = async () => {
    const keys = Array.from(selected());
    if (keys.length === 0) return;
    // 受保护目标只能强制终止：先显式确认，绝不把整批默认选择塞进 graceful 然后整批失败。
    const protectedRows = processes().filter(
      (p) => keys.includes(p.selection_key) && p.protected,
    );
    if (protectedRows.length > 0) {
      setConfirmForce({ keys, protected: protectedRows });
      return;
    }
    await requestOptimize(keys, "graceful");
  };

  const executePrepared = async () => {
    const operation = prepared();
    if (!operation) return;
    setPrepared(null);
    setOptimizing(true);
    setMessage(null);
    try {
      const outcome = await executeOperation(operation.operation_id);
      if (outcome.kind !== "process") return;
      let msg = t("scan.killSuccess", { count: outcome.value.killed.length });
      if (outcome.value.failed.length > 0) {
        msg += t("scan.killPartial", { failed: outcome.value.failed.length });
        const failReasons = outcome.value.details
          .filter((d) => !d.success)
          .map((d) => `${d.name}: ${errorText(d.message, tText)}`)
          .join("；");
        if (failReasons) msg += ` —— ${failReasons}`;
      }
      setMessage(msg);
      if (outcome.value.killed.length > 0) {
        setShowFlash(true);
        void playCleanSuccessSound();
      }
      await runScan();
    } catch (e) {
      await runScan();
      const info = classifyOperationError(e);
      setMessage(t(`opError.${info.kind}`, { error: errorText(info, tText) }));
    } finally {
      setOptimizing(false);
    }
  };

  return (
    <Show when={!showWelcome()} fallback={<Welcome onStart={handleStart} />}>
      <div class="flex flex-col gap-5 p-6 h-full overflow-y-auto">
        <CleanupFlash visible={showFlash()} onDone={() => setShowFlash(false)} />
        <HealthCard health={snapshot()?.value.health ?? null} />

        {/*
          「可优化进程 + 一键优化」整块按能力门禁。

          MAS 版里 `scan_all` 枚举不到进程（sysinfo 走 proc_listallpids，
          被沙箱拦），于是这张表是空的，空表会显示
          「没有发现可优化的进程，系统运行良好」—— 那台机器上有 186 个进程，
          这句话是假的。留着它既是骗用户，也是审核眼里的「误导」。
          终止不了进程时，这一块没有任何诚实可展示的内容。
        */}
        <Show when={can("terminateProcess")}>
          <ProcessList
            processes={processes()}
            selected={selected()}
            onToggle={toggle}
            onWhitelist={async (name) => {
              await addWhitelist("process", name, "scan list add");
              setMessage(t("scan.whitelistAdded", { name }));
              await runScan();
            }}
          />

          <div class="flex items-center gap-3">
            <button
              type="button"
              class="btn-primary gap-2 min-w-[180px]"
              disabled={optimizing() || scanning() || selected().size === 0}
              onClick={optimize}
            >
              <Show
                when={!optimizing()}
                fallback={<Loader2 size={16} class="animate-spin" />}
              >
                <Sparkles size={16} />
              </Show>
              {t("scan.oneClick")}
              {selected().size > 0 ? ` (${selected().size})` : ""}
            </button>

            <Show when={message()}>
              <span class="text-xs text-zinc-500 animate-fade-in">
                {message()}
              </span>
            </Show>

            <span class="ml-auto text-[11px] text-zinc-400">
              {t("common.notice_irreversible")}
            </span>
          </div>
        </Show>

        <div class="flex items-center gap-3">
          <button
            type="button"
            class="btn-ghost gap-2"
            disabled={scanning()}
            onClick={runScan}
          >
            <Show
              when={!scanning()}
              fallback={<Loader2 size={16} class="animate-spin" />}
            >
              <RefreshCw size={16} />
            </Show>
            {t("common.rescan")}
          </button>

          <Show when={!can("terminateProcess") && message()}>
            <span class="text-xs text-zinc-500 animate-fade-in">
              {message()}
            </span>
          </Show>
        </div>

        <Show when={confirmForce()}>
          {(d) => (
            <ProtectedForceConfirm
              title={t("scan.confirmProtectedTitle")}
              message={t("scan.confirmProtectedMessage")}
              confirmLabel={t("scan.forceTerminate")}
              cancelLabel={t("common.cancel")}
              rows={d().protected}
              onConfirm={() => {
                const info = d();
                setConfirmForce(null);
                void requestOptimize(info.keys, "force");
              }}
              onCancel={() => setConfirmForce(null)}
            />
          )}
        </Show>

        <Show when={prepared()}>
          {(operation) => (
            <OperationConfirm
              prepared={operation()}
              title={t("opConfirm.title")}
              confirmLabel={t("opConfirm.confirm")}
              busy={optimizing()}
              onConfirm={() => void executePrepared()}
              onCancel={() => setPrepared(null)}
            />
          )}
        </Show>
      </div>
    </Show>
  );
};

export default ScanView;
