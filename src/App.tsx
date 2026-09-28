import { Component, createSignal, onCleanup, onMount } from "solid-js";
import Sidebar, { type ViewId } from "@/components/Sidebar";
import ScanView from "@/views/ScanView";
import CacheView from "@/views/CacheView";
import HistoryView from "@/views/HistoryView";
import SettingsView from "@/views/SettingsView";
import ProcessView from "@/views/ProcessView";
import ApplicationsView from "@/views/ApplicationsView";
import UninstallerView from "@/views/UninstallerView";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import AppShell from "@/components/shell/AppShell";
import { useI18n } from "@/i18n";
import { initFlavor } from "@/lib/flavor";

/** 用 CSS display 切换的 tab 面板，组件始终挂载不丢状态 */
const TabPanel: Component<{ id: ViewId; active: ViewId; children: any }> = (props) => (
  <div
    class="h-full"
    style={{ display: props.active === props.id ? "block" : "none" }}
  >
    {props.children}
  </div>
);

const App: Component = () => {
  const { t } = useI18n();
  const [view, setView] = createSignal<ViewId>("scan");

  let unlistenTray: UnlistenFn | undefined;
  let disposed = false;

  const stopTrayListener = (unlisten: UnlistenFn | undefined) => {
    if (!unlisten) return;
    void Promise.resolve()
      .then(() => unlisten())
      .catch((error: unknown) => {
        console.error("取消托盘扫描监听失败", error);
      });
  };

  onMount(async () => {
    // 先定构建形态，再做别的。视图会读 `canTerminateProcesses()` 来决定
    // 「终止」入口要不要出现，所以它必须早于任何视图渲染确定下来。
    await initFlavor();
    try {
      const registered = await listen<void>("tray:scan", () => {
        setView("scan");
      });
      if (disposed) {
        stopTrayListener(registered);
        return;
      }
      unlistenTray = registered;
    } catch (error: unknown) {
      console.error("注册托盘扫描监听失败", error);
    }
  });

  onCleanup(() => {
    disposed = true;
    stopTrayListener(unlistenTray);
    unlistenTray = undefined;
  });

  return (
    <AppShell
      sidebar={<Sidebar current={view()} onChange={setView} />}
      toolbar={
        <h1 class="pointer-events-none text-sm font-medium text-zinc-500">
          {t(`nav.${view()}`)}
        </h1>
      }
    >
      <TabPanel id="scan" active={view()}><ScanView /></TabPanel>
      <TabPanel id="process" active={view()}><ProcessView /></TabPanel>
      <TabPanel id="applications" active={view()}><ApplicationsView /></TabPanel>
      <TabPanel id="cache" active={view()}><CacheView /></TabPanel>
      <TabPanel id="uninstaller" active={view()}><UninstallerView /></TabPanel>
      <TabPanel id="history" active={view()}><HistoryView /></TabPanel>
      <TabPanel id="settings" active={view()}><SettingsView /></TabPanel>
    </AppShell>
  );
};

export default App;
