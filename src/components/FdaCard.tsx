import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { Check, ShieldAlert } from "lucide-solid";
import { useI18n } from "@/i18n";
import { getFdaStatus, openFullDiskAccessSettings, type FdaStatus } from "@/lib/tauri";

/**
 * 完全磁盘访问权限（FDA）引导卡片。
 *
 * ## 为什么必须有
 *
 * App Sandbox 下即使带了 `files.all` entitlement，**用户没在系统设置里实际授权时
 * 敏感路径一律读不到**。实测（2026-09-29，MAS 包真机）：缓存页显示
 * 「没有发现可清理的缓存」，而同一台机器的完整版扫出 13.99 GB；进程列表为 0。
 *
 * 这不是我们独有的限制 —— CleanMyMac 官方文档说明它的 App Store 版要读硬盘
 * SMART 同样需要用户授予 FDA。
 *
 * ## 为什么放在设置页而不是启动弹窗
 *
 * 弹窗轰炸是这个品类最招人烦的做法（用户想清理缓存，先被拦下来要授权）。
 * 所以这里是**一张常驻卡片**：已授权就显示绿色对勾走人；没授权才显示引导，
 * 点了按钮直接跳系统设置。**不拦路，不重复索权。**
 */
export default function FdaCard() {
  const { t } = useI18n();
  const [status, setStatus] = createSignal<FdaStatus | null>(null);
  const [openFailed, setOpenFailed] = createSignal(false);
  let timer: number | undefined;

  const refresh = async () => {
    try {
      setStatus(await getFdaStatus());
    } catch {
      // 探不到就当作「可能缺权限」：引导卡片是安全侧的保守默认，
      // 多显示一张卡片的代价远小于让用户对着空列表猜。
      setStatus({
        userCache: false,
        systemDirs: false,
        homeRedirected: false,
        flavor: "developer_id",
      });
    }
  };

  onMount(() => {
    void refresh();
    // 授权要用户在系统设置里手动开，回来时状态才变。窗口重新获得焦点时
    // 重查一次，比让用户手动刷新或重启 app 自然。
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    // 兜底轮询：某些 macOS 版本下焦点事件不触发
    timer = window.setInterval(() => void refresh(), 5000);
    onCleanup(() => {
      window.removeEventListener("focus", onFocus);
      if (timer !== undefined) window.clearInterval(timer);
    });
  });

  const granted = () => {
    const s = status();
    return s !== null && s.userCache && s.systemDirs && !s.homeRedirected;
  };

  /**
   * 沙箱形态：$HOME 被换到了应用自己的 container，授权无效。
   *
   * 这时**不能**给「打开系统设置」按钮 —— 它会让用户去系统设置里折腾
   * 半天，发现还是 0，然后认定这个 App 骗人。换成说清边界 + 引流完整版。
   */
  const sandboxed = () => {
    const s = status();
    return s !== null && s.homeRedirected;
  };

  const reasonKey = () => {
    const s = status();
    if (!s) return "settings.fda.needBoth";
    if (s.homeRedirected) return "settings.fda.sandboxed";
    if (!s.userCache && !s.systemDirs) return "settings.fda.needBoth";
    if (!s.userCache) return "settings.fda.needUserCache";
    if (!s.systemDirs) return "settings.fda.needSystem";
    return "settings.fda.granted";
  };

  const open = async () => {
    setOpenFailed(false);
    if (!(await openFullDiskAccessSettings())) setOpenFailed(true);
  };

  return (
    <div class="card" data-testid="fda-card">
      <div class="flex items-start gap-3">
        <div class="flex-shrink-0 mt-0.5">
          <Show
            when={granted()}
            fallback={<ShieldAlert size={16} class="text-amber-500" />}
          >
            <Check size={16} class="text-success-600" />
          </Show>
        </div>
        <div class="flex-1 min-w-0">
          <div class="text-sm font-medium">{t("settings.fda.title")}</div>
          <div
            class="text-xs mt-0.5"
            classList={{
              "text-zinc-500": granted(),
              "text-amber-600 dark:text-amber-400": !granted(),
            }}
            role="status"
          >
            {t(reasonKey())}
          </div>

          <Show when={sandboxed() && !granted()}>
            <div class="mt-3">
              <a
                class="btn-primary inline-block"
                href="https://vgoapp.com"
                target="_blank"
                rel="noreferrer noopener"
              >
                {t("settings.fda.getFullVersion")}
              </a>
            </div>
          </Show>

          <Show when={!granted() && !sandboxed()}>
            <div class="mt-3 flex flex-wrap items-center gap-2">
              <button
                type="button"
                class="btn-primary"
                onClick={() => void open()}
              >
                {t("settings.fda.openSettings")}
              </button>
              <span class="text-[11px] text-zinc-500">
                {openFailed()
                  ? t("settings.fda.openFailed")
                  : t("settings.fda.manualHint")}
              </span>
            </div>
          </Show>
        </div>
      </div>
    </div>
  );
}
