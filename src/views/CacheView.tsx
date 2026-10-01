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
  cacheOperation,
  classifyOperationError,
  errorText,
  executeOperation,
  onCacheScanProgress,
  prepareOperation,
  scanCache,
  type CacheItem,
  type CacheSnapshotView,
  type CleanSummary,
  type Keyed,
  type PreparedOperation,
  type SnapshotResult,
  type StageUpdate,
  type UnlistenFn,
} from "@/lib/tauri";
import {
  CATEGORY_COLORS,
  categoryLabel,
  fmtBytes,
  fmtDuration,
  isCategoryReclaimable,
} from "@/lib/format";
import DockerSection from "@/components/DockerSection";
import {
  animateNumber,
  playCleanFailureSound,
  playCleanStartSound,
  playCleanSuccessSound,
} from "@/lib/cleanFeedback";
import CleanupFlash from "@/components/CleanupFlash";
import FolderAccessCard from "@/components/FolderAccessCard";
import { can } from "@/lib/flavor";
import OperationConfirm from "@/components/OperationConfirm";
import ScanStageProgress from "@/components/ScanStageProgress";
import { CheckCircle2, Loader2, RefreshCw, Sparkles, XCircle } from "lucide-solid";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { useI18n, getT, paramsToRecord } from "@/i18n";

function isNotifyEnabled(): boolean {
  try {
    const raw = localStorage.getItem("macslim.prefs.v1");
    if (!raw) return true;
    const p = JSON.parse(raw);
    return p?.notifyOnCleanComplete !== false;
  } catch {
    return true;
  }
}

async function notifyCleanComplete(bytes: number, count: number) {
  if (!isNotifyEnabled()) return;
  const t = getT();
  try {
    let granted = await isPermissionGranted();
    if (!granted) {
      granted = (await requestPermission()) === "granted";
    }
    if (!granted) return;
    sendNotification({
      title: t("cache.notifyTitle"),
      body: t("cache.notifyBody", { size: fmtBytes(bytes), count }),
    });
  } catch {
    /* noop */
  }
}

/**
 * 缓存扫描的阶段总数，只用于算进度条百分比。
 *
 * 真源是后端 `src-tauri/src/cache_scanner.rs` 里的 `EXPECTED_STAGES`
 * （测试 `stage_table_declares_exactly_the_sixteen_expected_stages` 钉死了
 * 「阶段表恰好 16 个扫描器、阶段名逐个一致」）。
 *
 * 后端增删阶段时**必须同步改这里**。不同步的后果：进度条按错误的分母算，
 * 永远到不了 100%（或提前跑满），但不会崩、不会报错，只是进度条失真。
 * 载荷的 4 个字段是安全约束，不允许为此加 `stage_total`，所以只能靠这道注释。
 */
const CACHE_STAGE_TOTAL = 16;

const CacheView: Component = () => {
  const { t, tText } = useI18n();
  const [snapshot, setSnapshot] = createSignal<SnapshotResult<CacheSnapshotView> | null>(null);
  const [scanning, setScanning] = createSignal(false);
  const [selected, setSelected] = createSignal(new Set<string>());
  const [cleaning, setCleaning] = createSignal(false);
  const [summary, setSummary] = createSignal<CleanSummary | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [cleanProgress, setCleanProgress] = createSignal(0);
  const [displayFreedBytes, setDisplayFreedBytes] = createSignal(0);
  const [showFlash, setShowFlash] = createSignal(false);
  const [pending, setPending] = createSignal<PreparedOperation | null>(null);
  // prepare 阶段（点「清理」→ 确认弹窗弹出）的 loading。
  //
  // 为什么要单独一个 signal：这一段实测要 14~21 秒（prepareOperation 要对每个
  // 选中项真实 dry-run 测一遍体积，15 项就是 15 次目录树遍历）。而 `cleaning()`
  // 只覆盖 execute 阶段。没它的话，用户点完「清理」要盯着一个**什么都不变**的
  // 界面等二十秒 —— 报告里被当成「点清理没反应」。
  const [preparing, setPreparing] = createSignal(false);
  const [stageCurrent, setStageCurrent] = createSignal<string | null>(null);
  const [stageDone, setStageDone] = createSignal(0);
  const [stageFound, setStageFound] = createSignal(0);

  let progressTimer: number | undefined;
  let cancelBytesAnimation: (() => void) | undefined;
  // onCacheScanProgress 是异步的：组件可能在它 resolve 之前就卸载。
  // disposed 让「注册」和「解绑」两个动作在任何顺序下都不漏对方。
  // 句柄用集合而不是单个变量：runScan 可能重入（prepareOperation 失败会重扫，
  // 连点两次清理按钮就会有两个 runScan 同时在飞），单槽会让后一次覆盖前一次。
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

  const onStageUpdate = (u: StageUpdate) => {
    if (u.state === "running") {
      setStageCurrent(u.stage);
      return;
    }
    setStageDone((n) => n + 1);
    setStageFound((n) => n + u.found_bytes);
  };

  /**
 * App Store 版有没有授权过目录。
 *
 * 只在这两个条件下关心：
 * - 沙箱形态（完整版不受限，谈授权是废话）
 * - 缓存清理能力在（否则这页本来就不该出现）
 */
const needsFolderGrant = () => can("folderGrant");

const runScan = async () => {
    setScanning(true);
    setSummary(null);
    setStageCurrent(null);
    setStageDone(0);
    setStageFound(0);
    let unlisten: UnlistenFn | undefined;
    try {
      const registered = await onCacheScanProgress(onStageUpdate);
      if (disposed) {
        // 组件已卸载：立刻解绑，并且不再发起一次没人看的扫描
        registered();
        return;
      }
      unlisten = registered;
      stageListeners.add(unlisten);
      const snapshotResult = await scanCache();
      setSnapshot(snapshotResult);
      setSelected(
        new Set(
          snapshotResult.value.items
            .filter((item) => item.default_select)
            .map((item) => item.selection_key),
        ),
      );
    } catch (e) {
      console.error(e);
    } finally {
      if (unlisten) releaseStageListener(unlisten);
      setScanning(false);
    }
  };

  onMount(runScan);
  onCleanup(() => {
    disposed = true;
    releaseAllStageListeners();
    if (progressTimer) window.clearInterval(progressTimer);
    cancelBytesAnimation?.();
  });

  const items = createMemo<Keyed<CacheItem>[]>(() => snapshot()?.value.items ?? []);

  const toggle = (key: string) => {
    const next = new Set(selected());
    next.has(key) ? next.delete(key) : next.add(key);
    setSelected(next);
  };

  const selectedBytes = createMemo(() =>
    items()
      .filter((item) => selected().has(item.selection_key))
      .reduce((sum, item) => sum + item.size_bytes, 0),
  );

  const selectedKeys = createMemo(() =>
    items()
      .filter((item) => selected().has(item.selection_key))
      .map((item) => item.selection_key),
  );

  const showError = (thrown: unknown) => {
    const info = classifyOperationError(thrown);
    setError(t(`opError.${info.kind}`, { error: errorText(info, tText) }));
  };

  const requestClean = async () => {
    const current = snapshot();
    const keys = selectedKeys();
    if (!current || keys.length === 0) return;
    setError(null);
    // 破坏性动作先停在 prepare：把后端算好的摘要与估算交给用户确认，
    // 用户点了确认才 execute（AGENTS.md §4.1 / §4.5）。
    setPreparing(true);
    try {
      setPending(
        await prepareOperation(cacheOperation(current.snapshot_id, keys)),
      );
    } catch (e) {
      showError(e);
      await runScan();
    } finally {
      setPreparing(false);
    }
  };

  const startProgress = () => {
    setCleaning(true);
    setSummary(null);
    setCleanProgress(0.08);
    setDisplayFreedBytes(0);
    void playCleanStartSound();
    if (progressTimer) window.clearInterval(progressTimer);
    progressTimer = window.setInterval(() => {
      setCleanProgress((prev) => Math.min(prev + (prev < 0.6 ? 0.06 : 0.025), 0.92));
    }, 180);
  };

  const confirmClean = async () => {
    const prepared = pending();
    if (!prepared) return;
    setPending(null);
    startProgress();
    try {
      const outcome = await executeOperation(prepared.operation_id);
      if (outcome.kind !== "cache") return;
      setSummary(outcome.value);
      setCleanProgress(1);
      await runScan();
      await notifyCleanComplete(
        outcome.value.total_freed_bytes,
        outcome.value.success_count,
      );
      if (outcome.value.success_count > 0) {
        setShowFlash(true);
        void playCleanSuccessSound();
      } else {
        void playCleanFailureSound();
      }
    } catch (e) {
      showError(e);
      void playCleanFailureSound();
      await runScan();
    } finally {
      if (progressTimer) {
        window.clearInterval(progressTimer);
        progressTimer = undefined;
      }
      setCleaning(false);
    }
  };

  createEffect(() => {
    const freed = summary()?.total_freed_bytes ?? 0;
    cancelBytesAnimation?.();
    cancelBytesAnimation = animateNumber(
      displayFreedBytes(),
      freed,
      summary() ? 1000 : 0,
      setDisplayFreedBytes,
    );
  });

  const grouped = createMemo(() => {
    const map = new Map<string, Keyed<CacheItem>[]>();
    for (const item of items()) {
      if (!map.has(item.category)) map.set(item.category, []);
      map.get(item.category)!.push(item);
    }
    // 「可清理空间」由**每项的 safety** 现场判定（`isCategoryReclaimable`），
    // 不维护一份分类名单 —— 名单一漂移就会把斜线纹理标到不该标的地方。
    return Array.from(map.entries()).map(([category, groupedItems]) => ({
      category,
      groupedItems,
      reclaimable: isCategoryReclaimable(groupedItems),
    }));
  });

  return (
    <div class="flex flex-col gap-5 p-6 h-full overflow-y-auto">
      {/*
        App Store 版把授权卡片放在这里，而不是设置页：
        授权的目的是为了清理，放到设置页等于让用户自己推理「这个开关和
        上面那个 0 B 有什么关系」。放在产生需求的地方，因果链是连着的。
      */}
      <Show when={can("folderGrant")}>
        <FolderAccessCard onChanged={() => void runScan()} />
      </Show>

      <CleanupFlash visible={showFlash()} onDone={() => setShowFlash(false)} />
      <div
        class="card p-6 animate-fade-in relative"
        classList={{
          "clean-hero clean-hero--active": cleaning() || !!summary(),
          "clean-hero--done": !!summary() && !cleaning(),
        }}
      >
        <div class="clean-hero__glow" />
        <Show when={cleaning() || summary()}>
          <div class="clean-hero__particles">
            <For each={Array.from({ length: 10 })}>
              {(_, index) => (
                <span
                  class="clean-hero__particle"
                  style={{
                    left: `${8 + index() * 9}%`,
                    "animation-delay": `${index() * 60}ms`,
                  }}
                />
              )}
            </For>
          </div>
        </Show>
        <div class="relative z-10 flex items-center justify-between gap-6">
          <div class="min-w-0">
            <h2 class="text-base font-semibold">{t("cache.title")}</h2>
            <p class="text-xs text-zinc-500 mt-0.5">{t("cache.subtitle")}</p>
            <Show when={cleaning()}>
              <div class="mt-4 max-w-[420px]">
                <div class="flex items-center justify-between text-[11px] uppercase tracking-[0.16em] text-brand-700 dark:text-brand-300">
                  <span>{t("cache.cleaningStage")}</span>
                  <span>{Math.round(cleanProgress() * 100)}%</span>
                </div>
                <div class="mt-2 h-2.5 rounded-full bg-brand-500/10 overflow-hidden">
                  <div
                    class="clean-progress-bar"
                    style={{ width: `${cleanProgress() * 100}%` }}
                  />
                </div>
                <div class="mt-3 text-sm text-zinc-600 dark:text-zinc-300">
                  {t("cache.cleaningLive", {
                    count: selected().size,
                    size: fmtBytes(selectedBytes()),
                  })}
                </div>
              </div>
            </Show>
            <Show when={summary() && !cleaning()}>
              <div class="mt-4">
                <div class="text-[11px] uppercase tracking-[0.16em] text-success-600 dark:text-success-400">
                  {t("cache.releaseLabel")}
                </div>
                <div class="mt-1 text-4xl font-bold tabular-nums text-success-600 clean-result-number">
                  {fmtBytes(displayFreedBytes())}
                </div>
                <div class="mt-2 text-sm text-zinc-600 dark:text-zinc-300">
                  {t("cache.cleanSuccessDetail", {
                    count: summary()!.success_count,
                    failed: summary()!.fail_count,
                  })}
                </div>
              </div>
            </Show>
            <Show when={error()}>
              <p class="mt-3 text-xs text-danger-600 dark:text-danger-400" role="alert">
                {error()}
              </p>
            </Show>
          </div>
          <Show
            when={!scanning()}
            fallback={
              <ScanStageProgress
                current={stageCurrent()}
                doneCount={stageDone()}
                total={CACHE_STAGE_TOTAL}
                foundBytes={stageFound()}
              />
            }
          >
            <div class="text-right shrink-0">
              <div
                class="text-3xl font-bold text-brand-600 tabular-nums transition-all duration-500"
                classList={{ "scale-[1.06]": cleaning() }}
              >
                {fmtBytes(snapshot()?.value.total_bytes ?? 0)}
              </div>
              <div class="text-xs text-zinc-500">{t("cache.freeable")}</div>
            </div>
          </Show>
        </div>
      </div>

      <Show when={summary()}>
        {(s) => (
          <div class="card p-5 animate-slide-up bg-success-500/5 border-success-500/20 clean-summary-card">
            <div class="flex items-center gap-3">
              <div class="w-10 h-10 rounded-full bg-success-500/15 flex items-center justify-center">
                <CheckCircle2 size={20} class="text-success-600" />
              </div>
              <div>
                <div class="font-semibold">
                  {t("cache.cleanSuccess", {
                    size: fmtBytes(s().total_freed_bytes),
                  })}
                </div>
                <div class="text-xs text-zinc-500">
                  {t("cache.successItems", { count: s().success_count })}
                  {s().fail_count > 0 &&
                    ` · ${t("cache.failItems", { count: s().fail_count })}`}
                </div>
              </div>
            </div>
          </div>
        )}
      </Show>

      <Show
        when={items().length > 0}
        fallback={
          <Show when={!scanning()}>
            <div class="card p-12 text-center text-sm text-zinc-500">
              {/* 0 B 在沙箱下的真实含义是「看不到」，不是「很干净」。
                  说成「你的 Mac 很干净」是把权限问题说成用户的好处 ——
                  用户会以为刚清过，而审核看到的是误导。 */}
              <Show
                when={needsFolderGrant()}
                fallback={t("cache.noItems")}
              >
                {t("cache.noAccess")}
              </Show>
            </div>
          </Show>
        }
      >
        <For each={grouped()}>
          {(group) => (
            <div class="card p-4 animate-fade-in">
              <div class="flex items-center gap-2 mb-3 px-1">
                <span
                  class={`px-2 py-0.5 rounded-md text-[11px] font-semibold ${CATEGORY_COLORS[group.category] ?? ""}`}
                  classList={{ "reclaimable-hatch": group.reclaimable }}
                  data-reclaimable={group.reclaimable ? "true" : "false"}
                >
                  {categoryLabel(group.category, t)}
                </span>
                <span class="text-xs text-zinc-500">
                  {t("cache.groupCount", {
                    count: group.groupedItems.length,
                    size: fmtBytes(
                      group.groupedItems.reduce(
                        (sum, item) => sum + item.size_bytes,
                        0,
                      ),
                    ),
                  })}
                </span>
              </div>
              <ul class="space-y-1">
                <For each={group.groupedItems}>
                  {(item) => (
                    <li class="flex items-start gap-3 p-2 rounded-lg hover:bg-black/[0.02] dark:hover:bg-white/[0.02]">
                      <input
                        type="checkbox"
                        checked={selected().has(item.selection_key)}
                        onChange={() => toggle(item.selection_key)}
                        class="mt-1 w-4 h-4 rounded accent-brand-500"
                      />
                      <div class="flex-1 min-w-0">
                        <div class="flex items-center gap-2">
                          <span class="font-medium text-sm">
                            {t(
                              item.label_key,
                              paramsToRecord(item.label_params),
                            )}
                          </span>
                          {item.safety === "safe" && (
                            <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-success-500/15 text-success-600">
                              {t("risk.safe")}
                            </span>
                          )}
                          {item.safety === "low" && (
                            <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-warning-500/15 text-warning-600">
                              {t("risk.low")}
                            </span>
                          )}
                          {item.safety === "medium" && (
                            <span class="px-1.5 py-0.5 rounded-md text-[10px] font-medium bg-danger-500/15 text-danger-600">
                              {t("risk.notice")}
                            </span>
                          )}
                        </div>
                        <div class="text-xs text-zinc-500 mt-0.5">
                          {t(
                            item.description_key,
                            paramsToRecord(item.description_params),
                          )}
                        </div>
                        <Show when={item.path}>
                          <div class="text-[10px] font-mono text-zinc-400 mt-0.5 truncate">
                            {item.path}
                          </div>
                        </Show>
                      </div>
                      <div class="text-right tabular-nums text-sm font-semibold min-w-[80px]">
                        {fmtBytes(item.size_bytes)}
                      </div>
                    </li>
                  )}
                </For>
              </ul>
            </div>
          )}
        </For>
      </Show>

      <Show when={can("dockerCleanup")}>
        <DockerSection />
      </Show>

      <div class="flex items-center gap-3 pb-4">
        <button
          type="button"
          class="btn-primary gap-2 min-w-[220px] clean-cta"
          classList={{
            "clean-cta--armed":
              !cleaning() && !preparing() && !scanning()
              && selected().size > 0 && selectedBytes() > 0,
            "clean-cta--busy": cleaning() || preparing(),
          }}
          disabled={
            cleaning() ||
            preparing() ||
            scanning() ||
            selected().size === 0 ||
            selectedBytes() === 0
          }
          onClick={() => void requestClean()}
        >
          <Show
            when={!cleaning() && !preparing()}
            fallback={<Loader2 size={16} class="animate-spin" />}
          >
            <Sparkles size={16} />
          </Show>
          <Show when={preparing()}>
            {t("cache.preparingCta")}
          </Show>
          <Show when={!preparing()}>
            {t("cache.cleanCta", {
              size: fmtBytes(selectedBytes()),
              count: selected().size,
            })}
          </Show>
        </button>
        <button
          type="button"
          class="btn-ghost gap-2 shrink-0"
          disabled={scanning() || cleaning()}
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

        {/* prepare 期间用说明文字换掉「不可撤销」提示，而不是在按钮旁边另加一条：
            这一行总宽只有 ~700px，额外插入文案会把「重新扫描」和提示都挤到换行
            （实测过一次，很难看）。两者都是同一位置的辅助说明，互斥显示即可。 */}
        <span class="ml-auto text-[11px] text-zinc-400 whitespace-nowrap">
          {preparing() ? t("cache.preparingHint") : t("common.notice_irreversible")}
        </span>
      </div>

      <Show when={pending()}>
        {(prepared) => (
          <OperationConfirm
            prepared={prepared()}
            title={t("opConfirm.title")}
            confirmLabel={t("opConfirm.confirm")}
            busy={cleaning()}
            onConfirm={() => void confirmClean()}
            onCancel={() => setPending(null)}
          />
        )}
      </Show>

      <Show when={summary() && summary()!.fail_count > 0}>
        <div class="card p-4 border-danger-500/20">
          <div class="flex items-center gap-2 mb-2">
            <XCircle size={16} class="text-danger-500" />
            <span class="font-medium text-sm">{t("cache.partialFail")}</span>
          </div>
          <ul class="text-xs space-y-1">
            <For each={summary()!.reports.filter((r) => !r.success)}>
              {(r) => (
                <li class="flex gap-2">
                  <span class="font-medium min-w-[140px]">
                    {t(r.label_key, paramsToRecord(r.label_params))}
                  </span>
                  <span class="text-zinc-500">
                    {r.error ? errorText(r.error, tText) : ""}
                  </span>
                  <span class="ml-auto text-zinc-400">
                    {fmtDuration(r.duration_ms)}
                  </span>
                </li>
              )}
            </For>
          </ul>
        </div>
      </Show>
    </div>
  );
};

export default CacheView;
