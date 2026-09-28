import { Component, createMemo, For, Show } from "solid-js";
import type { I18nParams, PreparedOperation } from "@/lib/tauri";
import { fmtBytes } from "@/lib/format";
import { AlertTriangle, ShieldAlert } from "lucide-solid";
import { useI18n } from "@/i18n";

/**
 * AGENTS.md §4.5：超过 10GB 的清理必须二次确认。
 * 门槛只认后端 `PreparedOperation.estimated_bytes`（权威估算），
 * 客户端的字节数相加只用于展示，不能作为授权依据。
 */
export const IRREVERSIBLE_THRESHOLD_BYTES = 10 * 1024 * 1024 * 1024;

type OperationConfirmProps = {
  prepared: PreparedOperation;
  title: string;
  confirmLabel: string;
  cancelLabel?: string;
  notice?: string;
  irreversible?: boolean;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
};

const remainingMinutes = (expiresAtMs: number): number =>
  Math.max(0, Math.ceil((expiresAtMs - Date.now()) / 60_000));

const OperationConfirm: Component<OperationConfirmProps> = (props) => {
  const { t, tText } = useI18n();
  const minutes = createMemo(() => remainingMinutes(props.prepared.expires_at_ms));
  const irreversible = createMemo(
    () =>
      props.irreversible === true ||
      props.prepared.estimated_bytes > IRREVERSIBLE_THRESHOLD_BYTES,
  );

  return (
    <div
      class="fixed inset-0 bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center p-6 animate-fade-in"
      onClick={props.onCancel}
    >
      <div
        class="card p-6 max-w-md w-full animate-slide-up"
        onClick={(event) => event.stopPropagation()}
      >
        <div class="flex items-start gap-3">
          <div class="w-10 h-10 rounded-xl bg-warning-500/15 flex items-center justify-center flex-shrink-0">
            <AlertTriangle size={20} class="text-warning-600" />
          </div>
          <div class="flex-1 min-w-0">
            <h3 class="font-semibold">{props.title}</h3>
            <p class="text-sm text-zinc-600 dark:text-zinc-300 mt-1">
              {/* 摘要由后端发 key + 参数，译文在这里查词典。 */}
              {tText(props.prepared.summary_key, props.prepared.summary_params)}
            </p>
            <div class="mt-3 space-y-1 text-xs text-zinc-500">
              <div>{t("opConfirm.items", { count: props.prepared.item_count })}</div>
              <div>
                {t("opConfirm.estimated", {
                  size: fmtBytes(props.prepared.estimated_bytes),
                })}
              </div>
              <div data-testid="operation-confirm-validity">
                {t("opConfirm.validity", { minutes: minutes() })}
              </div>
            </div>
            <Show when={props.notice}>
              <p class="mt-2 text-xs text-warning-600 dark:text-warning-400">
                {props.notice}
              </p>
            </Show>
            <Show when={irreversible()}>
              <div class="mt-3 rounded-lg bg-danger-500/10 border border-danger-500/25 p-2.5 space-y-1">
                <p class="text-xs text-danger-700 dark:text-danger-400">
                  {t("opConfirm.irreversible")}
                </p>
                <Show when={props.prepared.estimated_bytes > IRREVERSIBLE_THRESHOLD_BYTES}>
                  <p class="text-xs text-danger-700 dark:text-danger-400">
                    {t("opConfirm.largeWarning")}
                  </p>
                </Show>
              </div>
            </Show>
          </div>
        </div>
        <div class="flex justify-end gap-2 mt-5">
          <button type="button" class="btn-ghost" onClick={props.onCancel}>
            {props.cancelLabel ?? t("common.cancel")}
          </button>
          <button
            type="button"
            class="inline-flex items-center justify-center rounded-xl px-5 py-2.5 font-medium bg-danger-500 hover:bg-danger-400 text-white shadow-sm transition-all disabled:opacity-60"
            disabled={props.busy === true}
            onClick={props.onConfirm}
          >
            {props.confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
};

/**
 * 受保护进程在确认弹窗里的一行。
 *
 * `protected_reason_key` + `params` 而不是一整串文案：这是 `ProcessInfo` 的
 * 结构化子集，所以扫描页与进程页喂进来的都是同一份后端数据。
 */
export type ProtectedRow = {
  name: string;
  pid: number;
  protected_reason_key?: string | null;
  protected_reason_params?: I18nParams | null;
};

type ProtectedForceConfirmProps = {
  title: string;
  message: string;
  confirmLabel: string;
  cancelLabel: string;
  rows: ProtectedRow[];
  onConfirm: () => void;
  onCancel: () => void;
};

export const ProtectedForceConfirm: Component<ProtectedForceConfirmProps> = (props) => {
  const { tText } = useI18n();
  return (
  <div
    class="fixed inset-0 bg-black/40 backdrop-blur-sm z-50 flex items-center justify-center p-6 animate-fade-in"
    onClick={props.onCancel}
  >
    <div
      class="card p-6 max-w-md w-full animate-slide-up"
      onClick={(event) => event.stopPropagation()}
    >
      <div class="flex items-start gap-3">
        <div class="w-10 h-10 rounded-xl bg-warning-500/15 flex items-center justify-center flex-shrink-0">
          <AlertTriangle size={20} class="text-warning-600" />
        </div>
        <div class="flex-1">
          <h3 class="font-semibold">{props.title}</h3>
          <p class="text-sm text-zinc-500 mt-1">{props.message}</p>
          <ul class="mt-3 space-y-1.5 max-h-[200px] overflow-y-auto">
            <For each={props.rows}>
              {(row) => (
                <li class="text-xs flex items-start gap-2">
                  <ShieldAlert
                    size={11}
                    class="text-warning-500 flex-shrink-0 mt-0.5"
                  />
                  <div class="min-w-0">
                    <div class="font-medium truncate">
                      {row.name}
                      <span class="text-zinc-400 ml-1">(PID {row.pid})</span>
                    </div>
                    <Show when={row.protected_reason_key}>
                      <div class="text-[10px] text-zinc-500 truncate">
                        {tText(row.protected_reason_key!, row.protected_reason_params)}
                      </div>
                    </Show>
                  </div>
                </li>
              )}
            </For>
          </ul>
        </div>
      </div>
      <div class="flex justify-end gap-2 mt-5">
        <button type="button" class="btn-ghost" onClick={props.onCancel}>
          {props.cancelLabel}
        </button>
        <button
          type="button"
          class="inline-flex items-center justify-center rounded-xl px-5 py-2.5 font-medium bg-danger-500 hover:bg-danger-400 text-white shadow-sm transition-all"
          onClick={props.onConfirm}
        >
          {props.confirmLabel}
        </button>
      </div>
    </div>
  </div>
  );
};

export default OperationConfirm;
