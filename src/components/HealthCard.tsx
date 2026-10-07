import { Component, Show } from "solid-js";
import RingProgress from "./RingProgress";
import type { SystemHealth } from "@/lib/tauri";
import { useI18n } from "@/i18n";

type Props = { health: SystemHealth | null };

/**
 * 由三项读数决定右上角那句状态。
 *
 * 原来这里是无条件渲染绿点 + 「正常运行」—— 只要 `health` 非空就这么显示。
 * 于是磁盘 95% 满、内存 92% 的时候，第一屏最大的那句结论仍然是「正常运行」。
 * 对一个以「告诉你空间和资源去哪了」为卖点的工具，这是最不该出错的一句话。
 *
 * 阈值是产品判断，不是系统标准：
 * - 磁盘 85% 就提示：这正是本产品的目标场景，等到 95% 才说话就太晚了
 * - 内存/CPU 给到 90% 才算「压力较高」，避免日常波动频繁告警
 */
function healthLevel(health: SystemHealth): "normal" | "warning" | "critical" {
  const { cpu_percent: cpu, memory_percent: mem, disk_percent: disk } = health;
  if (cpu >= 90 || mem >= 90 || disk >= 95) return "critical";
  if (cpu >= 70 || mem >= 80 || disk >= 85) return "warning";
  return "normal";
}

const LEVEL_STYLE = {
  normal: { dot: "bg-success-500", key: "health.normal" },
  warning: { dot: "bg-warning-500", key: "health.warning" },
  critical: { dot: "bg-danger-500", key: "health.critical" },
} as const;

const HealthCard: Component<Props> = (props) => {
  const { t } = useI18n();
  const level = () => (props.health ? healthLevel(props.health) : null);
  return (
    <div class="card p-6 animate-fade-in">
      <div class="flex items-center justify-between mb-5">
        <div>
          <h2 class="text-base font-semibold">{t("health.title")}</h2>
          <p class="text-xs text-zinc-500 mt-0.5">{t("health.subtitle")}</p>
        </div>
        <Show
          when={level()}
          fallback={
            <span class="text-xs text-zinc-400">{t("health.reading")}</span>
          }
        >
          {(value) => (
            <span class="inline-flex items-center gap-1.5 text-xs text-zinc-500">
              <span
                class={`w-1.5 h-1.5 rounded-full ${LEVEL_STYLE[value()].dot} animate-pulse`}
              />
              {t(LEVEL_STYLE[value()].key)}
            </span>
          )}
        </Show>
      </div>

      <div class="grid grid-cols-3 gap-4">
        <Metric
          label={t("health.cpu")}
          value={props.health?.cpu_percent ?? 0}
          sub={props.health ? `${props.health.cpu_percent.toFixed(1)}%` : "—"}
        />
        <Metric
          label={t("health.memory")}
          value={props.health?.memory_percent ?? 0}
          sub={
            props.health
              ? `${fmtMb(props.health.memory_used_mb)} / ${fmtMb(
                  props.health.memory_total_mb,
                )}`
              : "—"
          }
        />
        <Metric
          label={t("health.disk")}
          value={props.health?.disk_percent ?? 0}
          sub={
            props.health
              ? `${props.health.disk_used_gb.toFixed(0)}GB / ${props.health.disk_total_gb.toFixed(0)}GB`
              : "—"
          }
        />
      </div>
    </div>
  );
};

const Metric: Component<{ label: string; value: number; sub: string }> = (
  props,
) => (
  <div class="flex flex-col items-center gap-2">
    <RingProgress value={props.value} />
    <div class="text-center">
      <div class="text-sm font-medium">{props.label}</div>
      <div class="text-xs text-zinc-500 tabular-nums">{props.sub}</div>
    </div>
  </div>
);

function fmtMb(mb: number): string {
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)}GB`;
  return `${Math.round(mb)}MB`;
}

export default HealthCard;
