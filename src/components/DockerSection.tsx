import {
  Component,
  createMemo,
  createResource,
  createSignal,
  For,
  Show,
} from "solid-js";
import {
  classifyOperationError,
  errorText,
  dockerInventory,
  dockerOperation,
  executeOperation,
  prepareOperation,
  type DockerAction,
  type DockerExecutionReport,
  type DockerInventory,
  type PreparedOperation,
} from "@/lib/tauri";
import OperationConfirm from "@/components/OperationConfirm";
import { fmtBytes } from "@/lib/format";
import { useI18n } from "@/i18n";
import {
  Container,
  Database,
  HardDrive,
  Layers,
  Loader2,
  RefreshCw,
  Sparkles,
  Trash2,
} from "lucide-solid";

type Tab = "images" | "containers" | "volumes";

const DockerSection: Component = () => {
  const { t, tText } = useI18n();
  const [snapshot, { refetch }] = createResource(dockerInventory);
  const [tab, setTab] = createSignal<Tab>("images");
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<{
    prepared: PreparedOperation;
    action: DockerAction;
  } | null>(null);

  const inventory = createMemo<DockerInventory | null>(
    () => snapshot()?.value ?? null,
  );
  const snapshotId = () => snapshot()?.snapshot_id ?? null;

  const dangling = createMemo(
    () => inventory()?.images.filter((i) => i.dangling) ?? [],
  );
  const unusedVols = createMemo(
    () => inventory()?.volumes.filter((v) => !v.in_use) ?? [],
  );
  const stoppedContainers = createMemo(
    () => inventory()?.containers.filter((c) => !c.running) ?? [],
  );

  const requestOperation = async (
    action: DockerAction,
    targetKeys: string[],
  ) => {
    const current = snapshotId();
    if (!current) return;
    setMessage(null);
    setError(null);
    try {
      // prepare 只生成不可逆计划：删除/清理必须等用户在确认弹窗里点头。
      setPending({
        prepared: await prepareOperation(
          dockerOperation(current, action, targetKeys),
        ),
        action,
      });
    } catch (e) {
      const info = classifyOperationError(e);
      setError(t(`opError.${info.kind}`, { error: errorText(info, tText) }));
      await refetch();
    }
  };

  const reportOutcome = (action: DockerAction, outcome: { value: DockerExecutionReport }) => {
    if (outcome.value.failed.length > 0) {
      setError(
        t("docker.partialFail", {
          count: outcome.value.failed.length,
          error: outcome.value.failed
            .map(([name, reason]) => `${name}: ${reason}`)
            .join("；"),
        }),
      );
      return;
    }
    // 诚实口径：只有实测到回收量时才追加数字；`null`（未能测量）不显示。
    const reclaimed = outcome.value.reclaimed_bytes;
    const reclaimNote =
      reclaimed != null
        ? `\n${t("result.reclaimed", { size: fmtBytes(reclaimed) })}`
        : "";
    if (action === "prune") {
      setMessage(
        (outcome.value.output.split("\n").slice(-3).join("\n").trim() ||
          t("docker.pruneDone")) + reclaimNote,
      );
      return;
    }
    setMessage(
      t("docker.done", { count: outcome.value.succeeded.length }) + reclaimNote,
    );
  };

  const executePending = async () => {
    const current = pending();
    if (!current) return;
    setPending(null);
    setBusy(true);
    try {
      const outcome = await executeOperation(current.prepared.operation_id);
      if (outcome.kind !== "docker") return;
      reportOutcome(current.action, outcome);
    } catch (e) {
      const info = classifyOperationError(e);
      setError(t(`opError.${info.kind}`, { error: errorText(info, tText) }));
    } finally {
      setBusy(false);
      await refetch();
    }
  };

  const removeImg = (key: string) => requestOperation("remove_image", [key]);
  const removeContainer = (key: string) =>
    requestOperation("remove_container", [key]);
  const removeVol = (key: string) => requestOperation("remove_volume", [key]);
  const pruneAll = () => requestOperation("prune", []);

  return (
    <div class="card p-0 overflow-hidden animate-fade-in">
      <div class="px-6 py-5 border-b border-black/5 dark:border-white/5">
        <Show
          when={!snapshot.loading}
          fallback={
            <div class="flex items-center gap-2 text-sm text-zinc-500">
              <Loader2 size={14} class="animate-spin" />
              {t("docker.loading")}
            </div>
          }
        >
          <Show
            when={inventory()?.daemon_running}
            fallback={
              <div class="space-y-2">
                <div class="flex items-center justify-between gap-4">
                  <div>
                    <h3 class="text-base font-semibold">{t("docker.title")}</h3>
                    <p class="text-xs text-zinc-500 mt-1">{t("docker.subtitleOff")}</p>
                  </div>
                  <button type="button" class="btn-ghost gap-1.5" onClick={() => refetch()} disabled={snapshot.loading}>
                    <RefreshCw size={12} />
                    {t("docker.refresh")}
                  </button>
                </div>
                <div class="text-sm text-zinc-500">{t("docker.notRunning")}</div>
              </div>
            }
          >
            <div class="space-y-4">
              <div class="flex items-start justify-between gap-4">
                <div>
                  <h3 class="text-base font-semibold">{t("docker.title")}</h3>
                  <p class="text-xs text-zinc-500 mt-1">{t("docker.subtitleOn")}</p>
                </div>
                <div class="flex items-center gap-2">
                  <button
                    type="button"
                    class="btn-primary gap-1.5"
                    disabled={busy() || (inventory()?.reclaimable_bytes ?? 0) === 0}
                    onClick={() => void pruneAll()}
                  >
                    <Show when={!busy()} fallback={<Loader2 size={14} class="animate-spin" />}>
                      <Sparkles size={14} />
                    </Show>
                    {t("docker.pruneAll")}
                  </button>
                  <button type="button" class="btn-ghost gap-1.5" onClick={() => refetch()} disabled={snapshot.loading}>
                    <RefreshCw size={12} />
                  </button>
                </div>
              </div>
              <div class="grid grid-cols-2 lg:grid-cols-5 gap-3">
                <StatCard label={t("docker.reclaimable")} value={fmtBytes(inventory()?.reclaimable_bytes ?? 0)} />
                <StatCard label={t("docker.images")} value={`${inventory()?.images.length ?? 0}`} note={t("docker.dangling", { count: dangling().length })} />
                <StatCard label={t("docker.containers")} value={`${inventory()?.containers.length ?? 0}`} note={t("docker.stopped", { count: stoppedContainers().length })} />
                <StatCard label={t("docker.volumes")} value={`${inventory()?.volumes.length ?? 0}`} note={t("docker.unused", { count: unusedVols().length })} />
                <StatCard label={t("docker.buildCache")} value={fmtBytes(inventory()?.builder.total_bytes ?? 0)} />
              </div>
            </div>
          </Show>
        </Show>
      </div>

      <Show when={inventory()?.daemon_running}>
        <div class="px-6 pt-3 border-b border-black/5 dark:border-white/5">
          <div class="flex gap-1">
            <TabBtn active={tab() === "images"} onClick={() => setTab("images")} icon={Layers} label={t("docker.images")} count={inventory()?.images.length ?? 0} />
            <TabBtn active={tab() === "containers"} onClick={() => setTab("containers")} icon={Container} label={t("docker.containers")} count={inventory()?.containers.length ?? 0} />
            <TabBtn active={tab() === "volumes"} onClick={() => setTab("volumes")} icon={Database} label={t("docker.volumes")} count={inventory()?.volumes.length ?? 0} />
          </div>
        </div>

        <div class="max-h-[440px] overflow-y-auto">
          <Show when={tab() === "images"}>
            <ul class="divide-y divide-black/5 dark:divide-white/5">
              <For each={inventory()?.images ?? []}>
                {(img) => (
                  <li class="flex items-center gap-3 px-6 py-3 hover:bg-black/[0.02] dark:hover:bg-white/[0.02] group">
                    <HardDrive size={14} class={img.dangling ? "text-warning-500" : "text-zinc-400"} />
                    <div class="min-w-0 flex-1">
                      <div class="flex items-center gap-2">
                        <span class="font-medium text-sm truncate">
                          {img.dangling ? `<${t("docker.danglingBadge")}>` : `${img.repository}:${img.tag}`}
                        </span>
                        <Show when={img.dangling}>
                          <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-warning-500/15 text-warning-600">{t("docker.danglingBadge")}</span>
                        </Show>
                        <Show when={img.in_use}>
                          <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-brand-500/15 text-brand-600">{t("docker.inUseBadge")}</span>
                        </Show>
                      </div>
                      <div class="text-[10px] text-zinc-500 font-mono truncate">{img.id} · {img.created}</div>
                    </div>
                    <div class="text-sm tabular-nums text-zinc-600 dark:text-zinc-400">{fmtBytes(img.size_bytes)}</div>
                    <button type="button" class="p-1.5 rounded-lg text-zinc-400 hover:text-danger-500 hover:bg-danger-500/10 opacity-0 group-hover:opacity-100 transition" disabled={busy()} onClick={() => void removeImg(img.selection_key)} title={t("docker.deleteImage")}>
                      <Trash2 size={13} />
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>

          <Show when={tab() === "containers"}>
            <ul class="divide-y divide-black/5 dark:divide-white/5">
              <For each={inventory()?.containers ?? []}>
                {(c) => (
                  <li class="flex items-center gap-3 px-6 py-3 hover:bg-black/[0.02] dark:hover:bg-white/[0.02] group">
                    <Container size={14} class={c.running ? "text-success-500" : "text-zinc-400"} />
                    <div class="min-w-0 flex-1">
                      <div class="flex items-center gap-2">
                        <span class="font-medium text-sm truncate">{c.name}</span>
                        <Show when={c.running}>
                          <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-success-500/15 text-success-600">{t("docker.runningBadge")}</span>
                        </Show>
                        <Show when={!c.running}>
                          <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-zinc-500/15 text-zinc-500">{t("docker.stoppedBadge")}</span>
                        </Show>
                      </div>
                      <div class="text-[10px] text-zinc-500 truncate">{c.image} · {c.status}</div>
                    </div>
                    <div class="text-sm tabular-nums text-zinc-600 dark:text-zinc-400">{c.size_bytes > 0 ? fmtBytes(c.size_bytes) : "—"}</div>
                    <button type="button" class="p-1.5 rounded-lg text-zinc-400 hover:text-danger-500 hover:bg-danger-500/10 opacity-0 group-hover:opacity-100 transition" disabled={busy()} onClick={() => void removeContainer(c.selection_key)} title={c.running ? t("docker.deleteContainerForce") : t("docker.deleteContainer")}>
                      <Trash2 size={13} />
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>

          <Show when={tab() === "volumes"}>
            <ul class="divide-y divide-black/5 dark:divide-white/5">
              <For each={inventory()?.volumes ?? []}>
                {(v) => (
                  <li class="flex items-center gap-3 px-6 py-3 hover:bg-black/[0.02] dark:hover:bg-white/[0.02] group">
                    <Database size={14} class={v.in_use ? "text-brand-500" : "text-zinc-400"} />
                    <div class="min-w-0 flex-1">
                      <div class="flex items-center gap-2">
                        <span class="font-medium text-sm font-mono truncate">{v.name}</span>
                        <Show when={v.in_use}>
                          <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-brand-500/15 text-brand-600">{t("docker.inUseBadge")}</span>
                        </Show>
                      </div>
                      <div class="text-[10px] text-zinc-500">driver: {v.driver}</div>
                    </div>
                    <button type="button" class="p-1.5 rounded-lg text-zinc-400 hover:text-danger-500 hover:bg-danger-500/10 opacity-0 group-hover:opacity-100 transition" disabled={busy() || v.in_use} onClick={() => void removeVol(v.selection_key)} title={v.in_use ? t("docker.deleteVolumeDisabled") : t("docker.deleteVolume")}>
                      <Trash2 size={13} />
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </div>
      </Show>

      <Show when={pending()}>
        {(current) => (
          <OperationConfirm
            prepared={current().prepared}
            title={t("opConfirm.title")}
            confirmLabel={t("opConfirm.confirm")}
            notice={current().action === "prune" ? t("docker.pruneIrreversible") : undefined}
            irreversible={current().action === "prune"}
            busy={busy()}
            onConfirm={() => void executePending()}
            onCancel={() => setPending(null)}
          />
        )}
      </Show>

      <Show when={message() || error()}>
        <div
          class="px-6 py-2 border-t border-black/5 dark:border-white/5 text-xs whitespace-pre-wrap"
          classList={{
            "text-zinc-600 dark:text-zinc-400": !error(),
            "text-danger-600 dark:text-danger-400": !!error(),
          }}
          role={error() ? "alert" : undefined}
        >
          {error() ?? message()}
        </div>
      </Show>
    </div>
  );
};

const TabBtn: Component<{
  active: boolean;
  onClick: () => void;
  icon: Component<{ size?: number; class?: string }>;
  label: string;
  count: number;
}> = (p) => (
  <button
    type="button"
    onClick={p.onClick}
    class={`inline-flex items-center gap-1.5 px-3 py-1.5 text-sm font-medium rounded-t-lg border-b-2 -mb-[2px] transition-colors ${
      p.active
        ? "border-brand-500 text-zinc-900 dark:text-zinc-100"
        : "border-transparent text-zinc-500 hover:text-zinc-700"
    }`}
  >
    <p.icon size={14} />
    {p.label}
    <span class="text-xs text-zinc-400">({p.count})</span>
  </button>
);

const StatCard: Component<{ label: string; value: string; note?: string }> = (props) => (
  <div class="rounded-2xl bg-black/[0.03] dark:bg-white/[0.03] px-4 py-3">
    <div class="text-[11px] text-zinc-500">{props.label}</div>
    <div class="mt-1 text-lg font-semibold tabular-nums">{props.value}</div>
    <Show when={props.note}>
      <div class="mt-1 text-[10px] text-zinc-400">{props.note}</div>
    </Show>
  </div>
);

export default DockerSection;
