import { Component, createSignal, For, onMount, Show } from "solid-js";
import { getHistory, type HistoryEntry } from "@/lib/tauri";
import { fmtBytes, fmtRelativeTime } from "@/lib/format";
import { CheckCircle2, XCircle, Cpu, HardDrive, Loader2, LogOut, Trash2, PowerOff, Zap } from "lucide-solid";
import { useI18n } from "@/i18n";

const HistoryView: Component = () => {
  const { t, effectiveLocale } = useI18n();
  const [entries, setEntries] = createSignal<HistoryEntry[]>([]);
  const [loading, setLoading] = createSignal(false);

  const load = async () => {
    setLoading(true);
    try {
      const h = await getHistory(300);
      setEntries(h);
    } finally {
      setLoading(false);
    }
  };

  onMount(load);

  const opLabel = (op: string) => {
    switch (op) {
      case "process":
        return t("history.opProcess");
      case "app_terminate":
        return t("history.opAppTerminate");
      case "app_graceful_quit":
        return t("history.opAppGracefulQuit");
      case "uninstall":
        return t("history.opUninstall");
      case "docker":
        return t("history.opDocker");
      case "cache":
        return t("history.opCache");
      default:
        return op;
    }
  };

  const opTone = (op: string) => {
    switch (op) {
      case "process":
        return {
          wrap: "bg-brand-500/10",
          icon: <Cpu size={16} class="text-brand-600" />,
        };
      case "app_graceful_quit":
        return {
          wrap: "bg-success-500/10",
          icon: <LogOut size={16} class="text-success-600" />,
        };
      case "app_terminate":
        return {
          wrap: "bg-zinc-500/10",
          icon: <PowerOff size={16} class="text-zinc-600" />,
        };
      case "uninstall":
        return {
          wrap: "bg-warning-500/10",
          icon: <Trash2 size={16} class="text-warning-600" />,
        };
      case "docker":
        return {
          wrap: "bg-danger-500/10",
          icon: <Zap size={16} class="text-danger-600" />,
        };
      default:
        return {
          wrap: "bg-success-500/10",
          icon: <HardDrive size={16} class="text-success-600" />,
        };
    }
  };

  return (
    <div class="h-full overflow-y-auto">
    <div class="flex flex-col gap-5 p-6">
      <div class="card p-6">
        <h2 class="text-base font-semibold">{t("history.title")}</h2>
        <p class="text-xs text-zinc-500 mt-0.5">{t("history.subtitle")}</p>
      </div>

      <Show
        when={!loading()}
        fallback={
          <div class="text-center py-12 text-sm text-zinc-500 flex items-center justify-center gap-2">
            <Loader2 size={14} class="animate-spin" />
            {t("common.loading")}
          </div>
        }
      >
        <Show
          when={entries().length > 0}
          fallback={
            <div class="card p-12 text-center text-sm text-zinc-500">
              {t("history.empty")}
            </div>
          }
        >
          <div class="card p-0 overflow-hidden">
            <ul class="divide-y divide-black/5 dark:divide-white/5">
              <For each={entries()}>
                {(e) => (
                  <li class="flex items-start gap-3 p-4 hover:bg-black/[0.02] dark:hover:bg-white/[0.02] transition-colors">
                    <div class="w-9 h-9 rounded-lg flex items-center justify-center flex-shrink-0 mt-0.5">
                      <Show
                        when={e.success}
                        fallback={
                          <div class="w-9 h-9 rounded-lg bg-danger-500/10 flex items-center justify-center">
                            <XCircle size={16} class="text-danger-500" />
                          </div>
                        }
                      >
                        <div class={`w-9 h-9 rounded-lg flex items-center justify-center ${opTone(e.operation).wrap}`}>
                          {opTone(e.operation).icon}
                        </div>
                      </Show>
                    </div>
                    <div class="flex-1 min-w-0">
                      <div class="flex items-center gap-2">
                        <span class="font-medium text-sm truncate">
                          {e.target}
                        </span>
                        <Show when={e.success}>
                          <CheckCircle2
                            size={12}
                            class="text-success-500 flex-shrink-0"
                          />
                        </Show>
                      </div>
                      <div class="text-xs text-zinc-500 mt-0.5">
                        {opLabel(e.operation)} · {e.detail}
                      </div>
                    </div>
                    <div class="text-right text-xs text-zinc-500 tabular-nums flex-shrink-0">
                      <Show when={e.freed_bytes > 0}>
                        <div class="font-medium text-success-600">
                          +{fmtBytes(e.freed_bytes)}
                        </div>
                      </Show>
                      <div>
                        {fmtRelativeTime(e.timestamp, t, effectiveLocale())}
                      </div>
                    </div>
                  </li>
                )}
              </For>
            </ul>
          </div>
        </Show>
      </Show>
    </div>
    </div>
  );
};

export default HistoryView;
