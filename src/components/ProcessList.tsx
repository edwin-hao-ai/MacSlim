import { Component, For, Show } from "solid-js";
import type { ProcessInfo } from "@/lib/tauri";
import { ShieldAlert, ShieldCheck } from "lucide-solid";
import { useI18n } from "@/i18n";
import { canTerminateProcesses } from "@/lib/flavor";

type Props = {
  processes: ProcessInfo[];
  selected: Set<string>;
  onToggle: (selectionKey: string) => void;
  onWhitelist?: (name: string) => void;
};

const ProcessList: Component<Props> = (props) => {
  const { t, tText } = useI18n();

  /**
   * 扫描行副标题 = 基础分类理由 + 「受保护（原因）」+「端口 N」。
   *
   * 这三段过去是后端用 `format!` 拼成一整串中文发过来的。现在后端只发
   * 「基础理由的 key + 参数」「受保护原因的 key」和 `ports`，句子在这里拼：
   * 每一段各自可翻译，英文语序才立得住。
   */
  const reasonLine = (p: ProcessInfo) => {
    const parts = [tText(p.reason_key, p.reason_params)];
    if (p.protected && p.protected_reason_key) {
      parts.push(
        t("process.protectedInline", {
          reason: tText(p.protected_reason_key, p.protected_reason_params),
        }),
      );
    }
    if (p.ports.length > 0) {
      const preview =
        p.ports.length <= 3
          ? p.ports.join("/")
          : `${p.ports.slice(0, 3).join("/")} ${t("process.portsMore", {
              count: p.ports.length,
            })}`;
      parts.push(t("process.portsNote", { ports: preview }));
    }
    return parts.join(" · ");
  };

  const riskBadge = (risk: ProcessInfo["risk"]) => {
    switch (risk) {
      case "safe":
        return (
          <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-success-500/15 text-success-600">
            {t("risk.safe")}
          </span>
        );
      case "low":
        return (
          <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-warning-500/15 text-warning-600">
            {t("risk.low")}
          </span>
        );
      case "dev":
        return (
          <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-brand-500/15 text-brand-600">
            {t("risk.dev")}
          </span>
        );
      default:
        return null;
    }
  };

  return (
    <div class="card p-4 animate-fade-in">
      <div class="flex items-center justify-between mb-3 px-1">
        <h3 class="text-sm font-semibold">{t("scan.processListTitle")}</h3>
        <span class="text-xs text-zinc-500">
          {t("scan.itemsCount", { count: props.processes.length })}
        </span>
      </div>

      <Show
        when={props.processes.length > 0}
        fallback={
          <div class="text-center py-12 text-sm text-zinc-500">
            {t("scan.noProcesses")}
          </div>
        }
      >
        <ul class="divide-y divide-black/5 dark:divide-white/5 max-h-[320px] overflow-y-auto">
          <For each={props.processes}>
            {(p) => (
              <li
                class="flex items-center gap-3 py-2 px-1 hover:bg-black/[0.02] dark:hover:bg-white/[0.02] rounded-lg transition-colors"
                classList={{ "opacity-70": p.protected }}
              >
                {/* 复选框只在**真的能终止**时才渲染。
                    它的唯一用途是勾选后点「终止」；MAS 版终止不了任何进程，
                    留着一排点下去毫无反应的框，就是「显示了但用不了」——
                    而这在 App Store 的列表截图里尤其明显：用户看到可勾选，
                    就以为能杀进程。 */}
                <Show when={canTerminateProcesses()}>
                  <input
                    type="checkbox"
                    checked={props.selected.has(p.selection_key)}
                    disabled={p.whitelisted}
                    onChange={() => props.onToggle(p.selection_key)}
                    class="w-4 h-4 rounded accent-brand-500 disabled:opacity-40"
                    title={
                      p.whitelisted
                        ? t("process.whitelistLocked")
                        : p.protected
                          ? t("process.protectedCheckboxTitle")
                          : undefined
                    }
                  />
                </Show>
                <Show when={p.protected}>
                  <span
                    title={
                      (p.protected_reason_key
                        ? tText(p.protected_reason_key, p.protected_reason_params)
                        : null) ?? t("process.protectedDefault")
                    }
                  >
                    <ShieldAlert
                      size={13}
                      class="text-warning-500 flex-shrink-0"
                    />
                  </span>
                </Show>
                <Show when={p.icon_base64}>
                  <img
                    src={`data:image/png;base64,${p.icon_base64}`}
                    alt=""
                    class="w-6 h-6 rounded flex-shrink-0"
                  />
                </Show>
                <div class="min-w-0 flex-1">
                  <div class="flex items-center gap-2 flex-wrap">
                    <span class="truncate font-medium text-sm">{p.name}</span>
                    {riskBadge(p.risk)}
                    <span class="text-[10px] text-zinc-400">
                      {t(`kind.${p.kind}`)}
                    </span>
                    <Show when={p.ports.length > 0}>
                      <span class="px-1.5 py-0.5 rounded-md text-[10px] font-mono font-medium bg-brand-500/10 text-brand-700 dark:text-brand-300">
                        :{p.ports.slice(0, 3).join(",")}
                        {p.ports.length > 3 && `+${p.ports.length - 3}`}
                      </span>
                    </Show>
                  </div>
                  <div class="text-[11px] text-zinc-500 font-mono truncate">
                    PID {p.pid} · {reasonLine(p)}
                  </div>
                  <Show when={p.protected && p.protected_reason_key}>
                    <div class="text-[10px] text-warning-600 dark:text-warning-400 truncate">
                      {t("scan.protectedHint")}{" "}
                      {tText(p.protected_reason_key!, p.protected_reason_params)}
                    </div>
                  </Show>
                </div>
                <div class="text-right tabular-nums text-xs text-zinc-500 min-w-[80px]">
                  <div>{p.cpu_percent.toFixed(1)}% CPU</div>
                  <div>{Math.round(p.memory_mb)}MB</div>
                </div>
                <Show when={props.onWhitelist}>
                  <button
                    type="button"
                    title={t("scan.whitelistTooltip")}
                    onClick={() => props.onWhitelist?.(p.name)}
                    class="p-1.5 rounded-lg text-zinc-400 hover:text-brand-600 hover:bg-brand-500/10 transition-colors"
                  >
                    <ShieldCheck size={14} />
                  </button>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </div>
  );
};

export default ProcessList;
