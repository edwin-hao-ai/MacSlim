import { Component, createSignal, For, onMount, Show } from "solid-js";
import {
  addWhitelist,
  getWhitelist,
  removeWhitelist,
  type WhitelistEntry,
} from "@/lib/tauri";
import { checkForUpdate, downloadAndInstall, type UpdateStatus } from "@/lib/updater";
import { can } from "@/lib/flavor";
import { fmtBytes, fmtRelativeTime } from "@/lib/format";
import FdaCard from "@/components/FdaCard";
import { Plus, Trash2, Check, Download, RefreshCw } from "lucide-solid";
import {
  disable as autostartDisable,
  enable as autostartEnable,
  isEnabled as autostartIsEnabled,
} from "@tauri-apps/plugin-autostart";
import {
  isPermissionGranted,
  requestPermission,
} from "@tauri-apps/plugin-notification";
import { useI18n, type LocaleCode } from "@/i18n";
import { getVersion } from "@tauri-apps/api/app";

const PREFS_KEY = "macslim.prefs.v1";
type Prefs = {
  notifyOnCleanComplete: boolean;
  cleanupSoundEnabled: boolean;
};
const defaultPrefs: Prefs = {
  notifyOnCleanComplete: true,
  cleanupSoundEnabled: true,
};

function loadPrefs(): Prefs {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (!raw) return defaultPrefs;
    return { ...defaultPrefs, ...JSON.parse(raw) };
  } catch {
    return defaultPrefs;
  }
}

function savePrefs(p: Prefs) {
  localStorage.setItem(PREFS_KEY, JSON.stringify(p));
}

const SettingsView: Component = () => {
  const { t, locale, setLocale, effectiveLocale } = useI18n();
  const [items, setItems] = createSignal<WhitelistEntry[]>([]);
  const [adding, setAdding] = createSignal(false);
  const [kind, setKind] = createSignal<"process" | "cache_path">("process");
  const [value, setValue] = createSignal("");
  const [note, setNote] = createSignal("");

  const [autostart, setAutostart] = createSignal(false);
  const [notifyGranted, setNotifyGranted] = createSignal(false);
  const [prefs, setPrefs] = createSignal<Prefs>(defaultPrefs);
  const [currentVersion, setCurrentVersion] = createSignal("0.1.0");
  const [updateStatus, setUpdateStatus] = createSignal<UpdateStatus>({
    state: "idle",
  });

  const load = async () => {
    setItems(await getWhitelist());
    try {
      setAutostart(await autostartIsEnabled());
    } catch {
      /* noop */
    }
    try {
      setNotifyGranted(await isPermissionGranted());
    } catch {
      /* noop */
    }
    try {
      setCurrentVersion(await getVersion());
    } catch {
      /* noop */
    }
    setPrefs(loadPrefs());
  };

  onMount(load);

  const toggleAutostart = async (next: boolean) => {
    try {
      if (next) await autostartEnable();
      else await autostartDisable();
      setAutostart(next);
    } catch (e) {
      console.error(e);
    }
  };

  const ensureNotifyPermission = async () => {
    if (notifyGranted()) return;
    const result = await requestPermission();
    setNotifyGranted(result === "granted");
  };

  const updatePref = (patch: Partial<Prefs>) => {
    const next = { ...prefs(), ...patch };
    setPrefs(next);
    savePrefs(next);
  };

  const submit = async () => {
    if (!value().trim()) return;
    await addWhitelist(kind(), value().trim(), note().trim());
    setValue("");
    setNote("");
    setAdding(false);
    await load();
  };

  const remove = async (id: number) => {
    await removeWhitelist(id);
    await load();
  };

  const runUpdateCheck = async () => {
    setUpdateStatus({ state: "checking" });
    const s = await checkForUpdate();
    setUpdateStatus(s);
  };

  const runUpdateInstall = async () => {
    const s = updateStatus();
    if (s.state !== "available") return;
    setUpdateStatus({ state: "downloading", downloaded: 0, total: null });
    try {
      await downloadAndInstall(s.update, (downloaded, total) => {
        setUpdateStatus({ state: "downloading", downloaded, total });
      });
      setUpdateStatus({ state: "ready" });
    } catch (e) {
      setUpdateStatus({ state: "error", message: String(e) });
    }
  };

  return (
    <div class="flex flex-col gap-5 p-6 h-full overflow-y-auto">
      {/* FDA 引导放在最上面：没授权时缓存清理与进程管理都是空的，
          让用户先看到「为什么是空的 + 怎么解决」再去点别的设置。
          已授权时它显示绿色对勾、直接走人，不占注意力。 */}
      <FdaCard />

      <div class="card p-6">
        <h2 class="text-base font-semibold">{t("settings.general")}</h2>
        <p class="text-xs text-zinc-500 mt-0.5">{t("settings.generalDesc")}</p>
      </div>

      <div class="card p-2">
        <ToggleRow
          label={t("settings.autostart")}
          desc={t("settings.autostartDesc")}
          checked={autostart()}
          onChange={toggleAutostart}
        />
        <ToggleRow
          label={t("settings.notifyClean")}
          desc={t("settings.notifyCleanDesc")}
          checked={prefs().notifyOnCleanComplete && notifyGranted()}
          disabled={!notifyGranted() && prefs().notifyOnCleanComplete}
          onChange={async (v) => {
            if (v) await ensureNotifyPermission();
            updatePref({ notifyOnCleanComplete: v });
          }}
        />
        <Show when={!notifyGranted() && prefs().notifyOnCleanComplete}>
          <div class="px-4 pb-3 -mt-1">
            <button
              type="button"
              class="text-xs text-brand-600 hover:underline"
              onClick={ensureNotifyPermission}
            >
              {t("settings.notifyRequestPrompt")}
            </button>
          </div>
        </Show>
        <ToggleRow
          label={t("settings.cleanupSound")}
          desc={t("settings.cleanupSoundDesc")}
          checked={prefs().cleanupSoundEnabled}
          onChange={(v) => updatePref({ cleanupSoundEnabled: v })}
        />

        {/* 语言 */}
        <div class="px-3 py-3 rounded-lg">
          <div class="flex items-start justify-between gap-4">
            <div>
              <div class="text-sm font-medium">{t("settings.language")}</div>
              <div class="text-xs text-zinc-500 mt-0.5">
                {t("settings.languageDesc")}
              </div>
            </div>
            <div class="flex items-center gap-1 bg-black/5 dark:bg-white/5 rounded-lg p-1">
              <For
                each={
                  [
                    ["auto", t("settings.languageAuto")],
                    ["zh-CN", t("settings.languageZh")],
                    ["en", t("settings.languageEn")],
                  ] as [LocaleCode, string][]
                }
              >
                {([val, label]) => (
                  <button
                    type="button"
                    onClick={() => setLocale(val)}
                    class={`px-3 py-1 rounded-md text-xs font-medium transition-colors ${
                      locale() === val
                        ? "bg-white dark:bg-zinc-700 shadow-sm text-zinc-900 dark:text-zinc-100"
                        : "text-zinc-500 hover:text-zinc-700 dark:hover:text-zinc-300"
                    }`}
                  >
                    {label}
                  </button>
                )}
              </For>
            </div>
          </div>
        </div>
      </div>

      {/*
        更新检查按能力门禁：MAS 由 App Store 负责更新，
        `tauri_plugin_updater` 在 MAS 构建里整块没注册
        （`lib.rs` 的 `#[cfg(not(feature = "mas"))]`），
        capability/mas.json 里的两条 updater 权限也一并去掉了。
        留着这个按钮，点了必然报错 —— 那是审核一眼能看出来的坏入口。
      */}
      <Show when={can("inAppUpdate")}>
      <div class="card p-3">
        <div class="flex items-start justify-between gap-4 px-2 pt-1">
          <div class="flex-1">
            <div class="text-sm font-medium">{t("settings.updates")}</div>
            <div class="text-xs text-zinc-500 mt-0.5">
              {t("settings.updatesDesc")}
            </div>
            <Show when={updateStatus().state === "uptodate"}>
              <div class="mt-2 flex items-center gap-1.5 text-xs text-success-600">
                <Check size={12} />
                {t("settings.updatesLatest", { version: currentVersion() })}
              </div>
            </Show>
            <Show when={updateStatus().state === "available"}>
              <div class="mt-2 text-xs text-brand-600">
                {t("settings.updatesAvailable", {
                  version:
                    (updateStatus() as Extract<
                      UpdateStatus,
                      { state: "available" }
                    >).update.version,
                })}
              </div>
            </Show>
            <Show when={updateStatus().state === "downloading"}>
              {(() => {
                const s = updateStatus() as Extract<
                  UpdateStatus,
                  { state: "downloading" }
                >;
                return (
                  <div class="mt-2 text-xs text-zinc-600 dark:text-zinc-400">
                    {t("settings.updatesDownloading")}{" "}
                    {fmtBytes(s.downloaded)}
                    {s.total ? ` / ${fmtBytes(s.total)}` : ""}
                  </div>
                );
              })()}
            </Show>
            <Show when={updateStatus().state === "error"}>
              <div class="mt-2 text-xs text-danger-500">
                {t("settings.updatesError", {
                  error:
                    (updateStatus() as Extract<UpdateStatus, { state: "error" }>)
                      .message,
                })}
              </div>
            </Show>
          </div>
          <div class="flex-shrink-0">
            <Show
              when={updateStatus().state === "available"}
              fallback={
                <button
                  type="button"
                  class="btn-ghost gap-1.5"
                  disabled={updateStatus().state === "checking"}
                  onClick={runUpdateCheck}
                >
                  <Show
                    when={updateStatus().state !== "checking"}
                    fallback={<RefreshCw size={12} class="animate-spin" />}
                  >
                    <RefreshCw size={12} />
                  </Show>
                  {updateStatus().state === "checking"
                    ? t("settings.updatesChecking")
                    : t("settings.updatesCheck")}
                </button>
              }
            >
              <button
                type="button"
                class="btn-primary gap-1.5"
                disabled={updateStatus().state === "downloading"}
                onClick={runUpdateInstall}
              >
                <Download size={12} />
                {t("settings.updatesDownload")}
              </button>
            </Show>
          </div>
        </div>
      </div>
      </Show>

      <div class="card p-4">
        <div class="flex items-center justify-between mb-3 px-1">
          <div>
            <div class="text-sm font-medium">{t("settings.whitelist")}</div>
            <div class="text-xs text-zinc-500 mt-0.5">
              {t("settings.whitelistCount", { count: items().length })}
            </div>
          </div>
          <button
            type="button"
            class="btn-ghost gap-1.5"
            onClick={() => setAdding(!adding())}
          >
            <Plus size={14} />
            {t("common.add")}
          </button>
        </div>

        <Show when={adding()}>
          <div class="p-3 mb-3 rounded-xl bg-black/5 dark:bg-white/5 space-y-2 animate-slide-up">
            <div class="flex gap-2">
              <select
                value={kind()}
                onChange={(e) =>
                  setKind(e.currentTarget.value as "process" | "cache_path")
                }
                class="rounded-lg px-2 py-1.5 text-sm bg-white dark:bg-zinc-800 border border-black/10 dark:border-white/10"
              >
                <option value="process">
                  {t("settings.whitelistKindProcess")}
                </option>
                <option value="cache_path">
                  {t("settings.whitelistKindPath")}
                </option>
              </select>
              <input
                type="text"
                placeholder={
                  kind() === "process"
                    ? t("settings.whitelistKindProcessPH")
                    : t("settings.whitelistKindPathPH")
                }
                value={value()}
                onInput={(e) => setValue(e.currentTarget.value)}
                class="flex-1 rounded-lg px-3 py-1.5 text-sm bg-white dark:bg-zinc-800 border border-black/10 dark:border-white/10"
              />
            </div>
            <input
              type="text"
              placeholder={t("settings.whitelistNotePH")}
              value={note()}
              onInput={(e) => setNote(e.currentTarget.value)}
              class="w-full rounded-lg px-3 py-1.5 text-sm bg-white dark:bg-zinc-800 border border-black/10 dark:border-white/10"
            />
            <div class="flex justify-end gap-2">
              <button
                type="button"
                class="btn-ghost"
                onClick={() => setAdding(false)}
              >
                {t("common.cancel")}
              </button>
              <button type="button" class="btn-primary" onClick={submit}>
                {t("common.add")}
              </button>
            </div>
          </div>
        </Show>

        <Show
          when={items().length > 0}
          fallback={
            <div class="text-center py-8 text-sm text-zinc-500">
              {t("settings.whitelistEmpty")}
            </div>
          }
        >
          <ul class="divide-y divide-black/5 dark:divide-white/5">
            <For each={items()}>
              {(w) => (
                <li class="flex items-center gap-3 py-2 px-1">
                  <span
                    class={`px-2 py-0.5 rounded-md text-[10px] font-semibold ${
                      w.kind === "process"
                        ? "bg-brand-500/15 text-brand-600"
                        : "bg-warning-500/15 text-warning-600"
                    }`}
                  >
                    {w.kind === "process"
                      ? t("settings.whitelistBadgeProcess")
                      : t("settings.whitelistBadgePath")}
                  </span>
                  <div class="flex-1 min-w-0">
                    <div class="text-sm font-medium truncate">{w.value}</div>
                    <Show when={w.note}>
                      <div class="text-xs text-zinc-500 truncate">{w.note}</div>
                    </Show>
                  </div>
                  <div class="text-[10px] text-zinc-400">
                    {fmtRelativeTime(w.added_at, t, effectiveLocale())}
                  </div>
                  <button
                    type="button"
                    class="p-1.5 rounded-lg hover:bg-danger-500/10 text-zinc-400 hover:text-danger-500 transition-colors"
                    onClick={() => remove(w.id)}
                  >
                    <Trash2 size={14} />
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </div>

      <div class="card p-6">
        <div class="text-sm font-medium mb-1">{t("settings.about")}</div>
        <div class="text-xs text-zinc-500 space-y-1">
          <div>{t("settings.aboutLine1", { version: currentVersion() })}</div>
          <div>{t("settings.aboutLine2")}</div>
          <div class="font-mono text-[10px] mt-2">{t("settings.aboutDb")}</div>
          <div class="font-mono text-[10px]">{t("settings.aboutCli")}</div>
        </div>
      </div>
    </div>
  );
};

const ToggleRow: Component<{
  label: string;
  desc?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (next: boolean) => void;
}> = (props) => (
  <label class="flex items-start justify-between gap-4 px-3 py-3 rounded-lg hover:bg-black/[0.02] dark:hover:bg-white/[0.02] cursor-pointer">
    <div class="flex-1 min-w-0">
      <div class="text-sm font-medium">{props.label}</div>
      <Show when={props.desc}>
        <div class="text-xs text-zinc-500 mt-0.5">{props.desc}</div>
      </Show>
    </div>
    <button
      type="button"
      role="switch"
      aria-checked={props.checked}
      onClick={() => !props.disabled && props.onChange(!props.checked)}
      class={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors ${
        props.checked ? "bg-brand-500" : "bg-zinc-300 dark:bg-zinc-700"
      } ${props.disabled ? "opacity-50 cursor-not-allowed" : ""}`}
    >
      <span
        class={`inline-block h-5 w-5 transform rounded-full bg-white shadow transition-transform ${
          props.checked ? "translate-x-5" : "translate-x-0.5"
        }`}
      />
    </button>
  </label>
);

export default SettingsView;
