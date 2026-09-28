import { type Component, Show } from "solid-js";
import { useI18n } from "@/i18n";
import { fmtBytes } from "@/lib/format";

type Props = {
  current: string | null;
  doneCount: number;
  total: number | null;
  foundBytes: number;
};

const ScanStageProgress: Component<Props> = (props) => {
  const { t, tStage } = useI18n();
  const percent = () =>
    props.total && props.total > 0
      ? Math.min(100, Math.round((props.doneCount / props.total) * 100))
      : 0;

  return (
    <div class="space-y-2" data-testid="scan-stage-progress">
      <div class="flex items-center gap-2 text-xs text-zinc-500">
        <span data-testid="scan-stage-name">
          <Show when={props.current} fallback={t("scanProgress.working")}>
            {/* 固定阶段是 `scanStage.*` 的 key（查词典）；残留扫描那条事件装的是
                应用名，`tStage` 原样返回。两种载荷共用同一个字段。 */}
            {t("scanProgress.current", { stage: tStage(props.current) })}
          </Show>
        </span>
        <span class="ml-auto" data-testid="scan-stage-count">
          <Show when={props.total !== null} fallback={props.doneCount}>
            {t("scanProgress.count", {
              done: props.doneCount,
              total: props.total ?? 0,
            })}
          </Show>
        </span>
      </div>
      <Show when={props.total !== null}>
        <div
          data-testid="scan-stage-bar"
          class="h-1 rounded-full bg-black/5 dark:bg-white/5 overflow-hidden"
        >
          <div
            class="h-full bg-brand-500 transition-[width] duration-300"
            style={{ width: `${percent()}%` }}
          />
        </div>
      </Show>
      <div class="text-xs text-zinc-400 tabular-nums" data-testid="scan-stage-bytes">
        {t("scanProgress.found", { size: fmtBytes(props.foundBytes) })}
      </div>
    </div>
  );
};

export default ScanStageProgress;
