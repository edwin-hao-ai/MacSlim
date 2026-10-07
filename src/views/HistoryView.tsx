import { Component, createEffect, createSignal, For, Show } from "solid-js";
import { getHistory, type HistoryEntry } from "@/lib/tauri";
import { fmtBytes, fmtRelativeTime } from "@/lib/format";
import { CheckCircle2, XCircle, Cpu, HardDrive, Loader2, LogOut, Trash2, PowerOff, Zap } from "lucide-solid";
import { useI18n } from "@/i18n";

/**
 * `active` 由 App 的当前视图传下来。
 *
 * 为什么必须重新加载：`TabPanel` 只是把非当前页 `display: none`，**所有视图
 * 从启动起就常驻挂载**。原来这里只在 `onMount` 读一次历史 —— 于是「清理成功
 * → 切到历史记录」看到的是启动那一刻的空列表，用户会以为刚才那一下什么都没
 * 发生。而「每步操作有记录、可追溯」正是这个产品对用户的核心承诺。
 *
 * 依赖 `props.active` 变真时重读，也顺带覆盖了别的入口（卸载、终止进程）
 * 写入的历史。
 */
const HistoryView: Component<{ active?: boolean }> = (props) => {
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

  createEffect(() => {
    if (props.active) void load();
  });

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

  /**
   * 本地化渲染 target / detail。
   *
   * 后端现在同时下发「拼好的中文」（`target`/`detail`）与结构化计数
   * （`item_count`/`ok_count`/`fail_count`/`reason_code`）。这里优先用结构化的
   * 那份拼出当前语言 —— 历史页此前在英文界面里整页显示中文（「3 项缓存」
   * 「成功 2 项，失败 1 项，释放 1.2 GB」），而「每步操作有记录、可追溯」
   * 正是这个产品对用户的核心承诺，英文用户不该看到一个中文的历史页。
   *
   * `item_count === 0` 表示这条是旧数据（新列默认 0），此时回退到原始文本，
   * 不做猜测。
   */
  const targetText = (e: HistoryEntry) => {
    if (e.item_count === 0) return e.target;
    // 计数为 1 时用单数变体：英文的 "1 cache items" 是明显语病，而历史页
    // 恰恰经常只有一条记录 —— 商店截图里就是这个形态。
    const suffix = e.item_count === 1 ? "_one" : "";
    const key = `history.target.${e.operation}${suffix}`;
    const text = t(key, { count: String(e.item_count) });
    if (text !== key) return text;
    // 没有单数变体就退回复数模板
    const fallbackKey = `history.target.${e.operation}`;
    const fallback = t(fallbackKey, { count: String(e.item_count) });
    return fallback === fallbackKey ? e.target : fallback;
  };

  const detailText = (e: HistoryEntry) => {
    if (e.item_count === 0) return e.detail;
    const key = `history.detail.${e.operation}`;
    const text = t(key, {
      ok: String(e.ok_count),
      fail: String(e.fail_count),
      count: String(e.item_count),
    });
    const base = text === key ? e.detail : text;
    if (!e.reason_code) return base;
    const reasonKey = `error.${e.reason_code}`;
    const reason = t(reasonKey);
    return reason === reasonKey ? base : `${base}；${reason}`;
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
                          {targetText(e)}
                        </span>
                        <Show when={e.success}>
                          <CheckCircle2
                            size={12}
                            class="text-success-500 flex-shrink-0"
                          />
                        </Show>
                      </div>
                      <div class="text-xs text-zinc-500 mt-0.5">
                        {opLabel(e.operation)} · {detailText(e)}
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
