import { createSignal, For, onMount, Show } from "solid-js";
import { FolderOpen, ShieldCheck, Trash2 } from "lucide-solid";
import { useI18n } from "@/i18n";
import { grantFolderAccess, listFolderAccess, revokeFolderAccess } from "@/lib/tauri";
import type { FolderTarget } from "@/lib/ipcTypes";

/**
 * 文件夹访问授权卡片（App Store 版）。
 *
 * ## 为什么必须有它，而不是一句「读不到你的缓存」
 *
 * 实测（2026-09-30，MAS 包真机，见 docs/mas-capability-matrix.md）：
 * App Sandbox 把 `$HOME` 重定向到应用自己的 container，真实 home 下的
 * `~/Library/Caches` 的 `read_dir` 直接 EPERM。而 Apple 文档明说
 * App Store 应用**即使拿到完全磁盘访问权限**，沙箱仍强制执行自己的
 * 文件限制 —— 所以「让用户去系统设置开 FDA」这条路是无效的。
 *
 * 唯一合规的入口是用户在标准文件选择框里亲手选定一个目录，应用用
 * security-scoped bookmark 把它持久化。这是竞品在用的做法（PureSpace）。
 *
 * ## 为什么不是弹窗
 *
 * 弹窗轰炸是这个品类最招人烦的做法。所以它是**一张常驻卡片**：
 * 已授权的目录显示为一行可撤销，没授权的才显示「授权」按钮。
 * 不拦路、不重复索权。
 *
 * ## 为什么放在缓存页而不是设置页
 *
 * 授权的目的是为了清理，放到设置页等于让用户自己推理「这个开关和上面
 * 那个 0 B 有什么关系」。放在产生需求的地方，因果链是连着的。
 */
export default function FolderAccessCard(props: {
  /** 授权成功后触发，由调用方重跑扫描 */
  onChanged?: () => void;
}) {
  const { t } = useI18n();
  const [targets, setTargets] = createSignal<FolderTarget[]>([]);
  const [busy, setBusy] = createSignal<string | null>(null);
  const [failed, setFailed] = createSignal<string | null>(null);

  const refresh = async () => {
    try {
      setTargets(await listFolderAccess());
    } catch {
      // 探不到就当作「一个都没授权」：那是最保守的一侧 —— 显示引导，
      // 而不是显示「已授权」然后扫出 0 让用户以为应用坏了。
      setTargets([]);
    }
  };

  onMount(() => void refresh());

  const grant = async (target: FolderTarget) => {
    setBusy(target.key);
    setFailed(null);
    try {
      // 文案 key 用**后端给的** reasonKey，不要从 target.key 拼。
      // target.key 是 snake_case（`user_caches`，也是落盘时的稳定标识），
      // 而 i18n 字典里的 key 是 camelCase（`userCaches`）。从前者拼后者
      // 会得到 `access.target.user_caches` —— 字典里没有，于是界面直接
      // 把 key 当文案显示出来。实测截图里就是这么发现的。
      const granted = await grantFolderAccess(target.key, t(target.reasonKey));
      if (granted) {
        await refresh();
        props.onChanged?.();
      }
    } catch {
      setFailed(target.key);
    } finally {
      setBusy(null);
    }
  };

  const revoke = async (target: FolderTarget) => {
    setBusy(target.key);
    try {
      await revokeFolderAccess(target.key);
      await refresh();
      props.onChanged?.();
    } catch {
      setFailed(target.key);
    } finally {
      setBusy(null);
    }
  };

  const grantedCount = () => targets().filter((t) => t.granted).length;

  return (
    <div class="card p-4" data-testid="folder-access-card">
      <div class="flex items-start gap-3">
        <div class="flex-shrink-0 mt-0.5">
          <ShieldCheck
            size={16}
            class={grantedCount() > 0 ? "text-success-600" : "text-amber-500"}
          />
        </div>
        <div class="flex-1 min-w-0">
          <div class="text-sm font-medium">{t("access.title")}</div>
          <div class="text-xs text-zinc-500 mt-0.5">{t("access.subtitle")}</div>

          <div class="mt-3 flex flex-col gap-1.5">
            <For each={targets()}>
              {(target) => (
                <div
                  class="flex items-center gap-2 rounded-lg px-2 py-1.5"
                  classList={{
                    "bg-black/[0.03] dark:bg-white/[0.04]": target.granted,
                  }}
                  data-testid={`folder-target-${target.key}`}
                >
                  <div class="flex-1 min-w-0">
                    <div class="text-xs font-medium">
                      {t(target.reasonKey)}
                    </div>
                    <div class="text-[11px] text-zinc-500 truncate">
                      {target.granted
                        ? (target.grantedPath ?? target.relativePath)
                        : `~/${target.relativePath}`}
                    </div>
                  </div>

                  <Show
                    when={target.granted}
                    fallback={
                      <button
                        type="button"
                        class="btn-ghost gap-1 text-xs"
                        disabled={busy() === target.key}
                        onClick={() => void grant(target)}
                      >
                        <FolderOpen size={12} />
                        {busy() === target.key
                          ? t("access.granting")
                          : t("access.grant")}
                      </button>
                    }
                  >
                    <button
                      type="button"
                      class="btn-ghost gap-1 text-xs text-zinc-500"
                      disabled={busy() === target.key}
                      onClick={() => void revoke(target)}
                      title={t("access.revoke")}
                    >
                      <Trash2 size={12} />
                      {t("access.revoke")}
                    </button>
                  </Show>
                </div>
              )}
            </For>
          </div>

          <Show when={failed()}>
            <div class="mt-2 text-xs text-danger-600" role="alert">
              {t("opError.unknown", { error: failed()! })}
            </div>
          </Show>

          <Show when={grantedCount() > 0}>
            <div class="mt-2 text-[11px] text-zinc-500">
              {t("access.grantedHint")}
            </div>
          </Show>
        </div>
      </div>
    </div>
  );
}