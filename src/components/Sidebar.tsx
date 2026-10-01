import { Component, For, createResource } from "solid-js";
import { getVersion } from "@tauri-apps/api/app";
import {
  Activity,
  Cpu,
  HardDrive,
  History as HistoryIcon,
  Package,
  Settings as SettingsIcon,
  Trash2,
} from "lucide-solid";
import { useI18n } from "@/i18n";
import SafeDragSurface from "@/components/shell/SafeDragSurface";
import { visibleNavItems, type ViewId } from "@/lib/navItems";
import { getFlavor } from "@/lib/flavor";

export type { ViewId } from "@/lib/navItems";

type Props = {
  current: ViewId;
  onChange: (id: ViewId) => void;
};

/**
 * 导航项 → 图标组件。
 *
 * 映射放在这里而不是数据层，是因为图标是**渲染**关注点；而「哪些页面该
 * 出现」是数据关注点，住在 `lib/navItems` 里 —— 那边不 import 图标库，
 * 所以那条规则可以被秒级测到（实测放在一起时，光 import 就超时 20 秒）。
 */
const ICONS = {
  activity: Activity,
  cpu: Cpu,
  package: Package,
  drive: HardDrive,
  trash: Trash2,
  history: HistoryIcon,
  settings: SettingsIcon,
} as const;

const items = (): { id: ViewId; icon: Component<{ size?: number }> }[] =>
  visibleNavItems(getFlavor()).map((spec) => ({
    id: spec.id,
    icon: ICONS[spec.icon],
  }));

const Sidebar: Component<Props> = (props) => {
  // 版本号从 Tauri 运行时读取，避免与 package.json 脱节
  const [version] = createResource(getVersion);
  const { t } = useI18n();
  return (
    <aside
      class="w-[200px] flex flex-col border-r border-black/5 dark:border-white/5 bg-[rgb(var(--bg-sidebar))/var(--bg-sidebar-alpha)]"
    >
      <SafeDragSurface
        data-testid="sidebar-drag-surface"
        class="flex flex-1 flex-col"
      >
        <div class="flex h-13 items-end px-5 pb-2">
          <div class="flex items-center gap-2">
            <div class="flex h-6 w-6 items-center justify-center rounded-lg bg-gradient-to-br from-brand-400 to-brand-600 shadow-sm">
              <Activity size={13} class="text-white" />
            </div>
            <span class="text-[15px] font-semibold tracking-tight">
              {t("common.appName")}
            </span>
          </div>
        </div>
        <nav class="px-2 py-2 flex flex-col gap-0.5">
          <For each={items()}>
            {(item) => (
              <button
                type="button"
                class="sidebar-item"
                data-active={props.current === item.id}
                onClick={() => props.onChange(item.id)}
              >
                <item.icon size={16} />
                <span>{t(`nav.${item.id}`)}</span>
              </button>
            )}
          </For>
        </nav>
        <div class="mt-auto p-3 text-[10px] text-zinc-400">v{version()}</div>
      </SafeDragSurface>
    </aside>
  );
};

export default Sidebar;
