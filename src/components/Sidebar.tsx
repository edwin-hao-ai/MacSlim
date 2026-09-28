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

export type ViewId =
  | "scan"
  | "process"
  | "applications"
  | "cache"
  | "uninstaller"
  | "history"
  | "settings";

type Props = {
  current: ViewId;
  onChange: (id: ViewId) => void;
};

const items: { id: ViewId; icon: Component<{ size?: number }> }[] = [
  { id: "scan", icon: Activity },
  { id: "process", icon: Cpu },
  { id: "applications", icon: Package },
  { id: "cache", icon: HardDrive },
  { id: "uninstaller", icon: Trash2 },
  { id: "history", icon: HistoryIcon },
  { id: "settings", icon: SettingsIcon },
];

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
          <For each={items}>
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
