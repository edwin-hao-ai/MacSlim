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
  addWhitelist,
  classifyOperationError,
  errorText,
  executeOperation,
  listAllProcesses,
  prepareOperation,
  processOperation,
  type PreparedOperation,
  type ProcessKillDetail,
  type ProcessMode,
  type ProcessRow,
} from "@/lib/tauri";
import OperationConfirm, {
  ProtectedForceConfirm,
  type ProtectedRow,
} from "@/components/OperationConfirm";
import {
  Loader2,
  Network,
  RefreshCw,
  Search,
  ShieldCheck,
  ShieldAlert,
  X,
  ArrowDown,
  ArrowUp,
  ChevronRight,
  ChevronDown,
  ListTree,
  List,
} from "lucide-solid";
import { useI18n } from "@/i18n";
import { canTerminateProcesses } from "@/lib/flavor";

type SortKey = "memory" | "cpu" | "name" | "pid" | "uptime";
type ViewMode = "tree" | "flat";

type TreeNode = {
  row: ProcessRow;
  children: TreeNode[];
  depth: number;
};

function fmtUptime(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h`;
  return `${Math.floor(secs / 86400)}d`;
}

function buildTree(rows: ProcessRow[]): TreeNode[] {
  const byPid = new Map<number, TreeNode>();
  const all = rows.map((r) => ({ row: r, children: [], depth: 0 }) as TreeNode);
  for (const n of all) byPid.set(n.row.pid, n);

  const roots: TreeNode[] = [];
  for (const n of all) {
    const ppid = n.row.parent_pid;
    if (ppid !== null && byPid.has(ppid)) {
      const parent = byPid.get(ppid)!;
      parent.children.push(n);
    } else {
      roots.push(n);
    }
  }

  const sortChildren = (node: TreeNode) => {
    node.children.sort((a, b) => b.row.memory_mb - a.row.memory_mb);
    node.children.forEach(sortChildren);
  };
  roots.forEach(sortChildren);
  roots.sort((a, b) => b.row.memory_mb - a.row.memory_mb);

  return roots;
}

function flattenTree(
  roots: TreeNode[],
  collapsed: Set<number>,
  sortKey: SortKey,
  sortDir: "desc" | "asc",
): TreeNode[] {
  const dir = sortDir === "desc" ? -1 : 1;
  const cmp = (a: TreeNode, b: TreeNode) => {
    switch (sortKey) {
      case "memory":
        return dir * (a.row.memory_mb - b.row.memory_mb);
      case "cpu":
        return dir * (a.row.cpu_percent - b.row.cpu_percent);
      case "name":
        return dir * a.row.name.localeCompare(b.row.name);
      case "pid":
        return dir * (a.row.pid - b.row.pid);
      case "uptime":
        return dir * (a.row.uptime_secs - b.row.uptime_secs);
    }
  };
  const sortedRoots = [...roots].sort(cmp);
  const out: TreeNode[] = [];
  const visit = (node: TreeNode, depth: number) => {
    node.depth = depth;
    out.push(node);
    if (collapsed.has(node.row.pid)) return;
    const kids = [...node.children].sort(cmp);
    for (const c of kids) visit(c, depth + 1);
  };
  for (const r of sortedRoots) visit(r, 0);
  return out;
}

const ProcessView: Component = () => {
  const { t, tText } = useI18n();
  const [rows, setRows] = createSignal<ProcessRow[]>([]);
  const [snapshotId, setSnapshotId] = createSignal<string | null>(null);
  const [loading, setLoading] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [portsOnly, setPortsOnly] = createSignal(false);
  const [viewMode, setViewMode] = createSignal<ViewMode>("tree");
  const [collapsed, setCollapsed] = createSignal(new Set<number>());
  const [sortKey, setSortKey] = createSignal<SortKey>("memory");
  const [sortDir, setSortDir] = createSignal<"desc" | "asc">("desc");
  const [selected, setSelected] = createSignal(new Set<string>());
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [killDetails, setKillDetails] = createSignal<ProcessKillDetail[] | null>(null);
  const [prepared, setPrepared] = createSignal<PreparedOperation | null>(null);
  const [hoveringList, setHoveringList] = createSignal(false);
  const [confirmProtected, setConfirmProtected] = createSignal<
    null | { keys: string[]; mode: ProcessMode; protected: ProtectedRow[] }
  >(null);

  const frozen = createMemo(
    () =>
      confirmProtected() !== null ||
      prepared() !== null ||
      busy() ||
      hoveringList() ||
      selected().size > 0,
  );

  let loadSeq = 0;
  const load = async (force = false) => {
    if (frozen() && !force) return;
    const seq = ++loadSeq;
    setLoading(true);
    try {
      const snapshot = await listAllProcesses();
      if (seq !== loadSeq) return;
      if (!force && frozen()) return;
      setRows(snapshot.value);
      setSnapshotId(snapshot.snapshot_id);
      setSelected(new Set<string>());
    } catch (e) {
      if (seq !== loadSeq) return;
      setError(t("opError.failed", {
        error: errorText(classifyOperationError(e), tText),
      }));
    } finally {
      if (seq === loadSeq) setLoading(false);
    }
  };

  let timer: number | undefined;
  onMount(() => {
    void load();
  });
  createEffect(() => {
    if (frozen()) {
      if (timer !== undefined) {
        clearInterval(timer);
        timer = undefined;
      }
      return;
    }
    timer = window.setInterval(() => void load(), 5000);
  });
  onCleanup(() => {
    if (timer !== undefined) clearInterval(timer);
  });

  const filteredRows = createMemo(() => {
    const q = query().trim().toLowerCase();
    let list = rows();
    if (portsOnly()) list = list.filter((r) => r.ports.length > 0);
    if (q) {
      // 刻意不搜 full_name：行内副标题把它展示给用户了，看着像「搜索不认它」的洞。
      // 但 sysinfo 0.33.1 在 macOS 上从同一个 KERN_PROCARGS2 路径同时取出
      // process.exe 与 process.name = basename(exe)（macos/process.rs:550-559），
      // 所以 full_name 恒等于 exe 的 basename，下面的 exe 匹配已经把它完全覆盖。
      // 依赖的是这条实现细节：若将来 sysinfo 改用 proc_pidpath 取 exe、
      // 或 name 不再从 exe 推导，这里必须补 r.full_name 匹配。
      list = list.filter(
        (r) =>
          r.name.toLowerCase().includes(q) ||
          String(r.pid).includes(q) ||
          r.exe.toLowerCase().includes(q) ||
          r.ports.some((p) => String(p).includes(q)),
      );
    }
    return list;
  });

  const visibleNodes = createMemo((): TreeNode[] => {
    const list = filteredRows();
    if (viewMode() === "flat") {
      const dir = sortDir() === "desc" ? -1 : 1;
      const sorted = [...list].sort((a, b) => {
        switch (sortKey()) {
          case "memory":
            return dir * (a.memory_mb - b.memory_mb);
          case "cpu":
            return dir * (a.cpu_percent - b.cpu_percent);
          case "name":
            return dir * a.name.localeCompare(b.name);
          case "pid":
            return dir * (a.pid - b.pid);
          case "uptime":
            return dir * (a.uptime_secs - b.uptime_secs);
        }
      });
      return sorted.map((r) => ({ row: r, children: [], depth: 0 }));
    }
    const tree = buildTree(list);
    return flattenTree(tree, collapsed(), sortKey(), sortDir());
  });

  const hasChildren = (pid: number): boolean =>
    rows().some((r) => r.parent_pid === pid);

  const toggleCollapse = (pid: number) => {
    const next = new Set(collapsed());
    next.has(pid) ? next.delete(pid) : next.add(pid);
    setCollapsed(next);
  };

  const toggleSelect = (key: string) => {
    const next = new Set(selected());
    next.has(key) ? next.delete(key) : next.add(key);
    setSelected(next);
  };

  const onHeaderClick = (key: SortKey) => {
    if (sortKey() === key) {
      setSortDir(sortDir() === "desc" ? "asc" : "desc");
    } else {
      setSortKey(key);
      setSortDir(key === "name" || key === "pid" ? "asc" : "desc");
    }
  };

  const terminateSelected = async () => {
    const keys = Array.from(selected());
    if (keys.length === 0) return;
    const protectedRows = rows().filter(
      (r) => keys.includes(r.selection_key) && r.protected,
    );
    if (protectedRows.length > 0) {
      setConfirmProtected({ keys, mode: "force", protected: protectedRows });
      return;
    }
    await requestOperation(keys, "graceful");
  };

  const requestOperation = async (keys: string[], mode: ProcessMode) => {
    const current = snapshotId();
    if (!current || keys.length === 0) return;
    setMessage(null);
    setError(null);
    setKillDetails(null);
    setSelected(new Set<string>());
    try {
      setPrepared(
        await prepareOperation(processOperation(current, keys, mode)),
      );
    } catch (e) {
      const info = classifyOperationError(e);
      setError(t(`opError.${info.kind}`, { error: errorText(info, tText) }));
      await load(true);
    }
  };

  const executePrepared = async () => {
    const operation = prepared();
    if (!operation) return;
    setPrepared(null);
    setBusy(true);
    setMessage(null);
    setError(null);
    setKillDetails(null);
    try {
      const outcome = await executeOperation(operation.operation_id);
      if (outcome.kind !== "process") return;
      setMessage(
        t("process.killSuccess", { count: outcome.value.killed.length }) +
          (outcome.value.failed.length > 0
            ? t("process.killPartial", { failed: outcome.value.failed.length })
            : ""),
      );
      if (outcome.value.failed.length > 0) {
        setKillDetails(outcome.value.details.filter((d) => !d.success));
      }
    } catch (e) {
      const info = classifyOperationError(e);
      setError(t(`opError.${info.kind}`, { error: errorText(info, tText) }));
    } finally {
      setBusy(false);
      await load(true);
    }
  };

  const whitelist = async (name: string) => {
    await addWhitelist("process", name, "process view add");
    setMessage(t("scan.whitelistAdded", { name }));
    await load(true);
  };

  const totalMem = createMemo(() => rows().reduce((s, r) => s + r.memory_mb, 0));
  const totalCPU = createMemo(() => rows().reduce((s, r) => s + r.cpu_percent, 0));

  const SortHeader: Component<{ k: SortKey; label: string; class?: string }> = (
    p,
  ) => (
    <button
      type="button"
      onClick={() => onHeaderClick(p.k)}
      class={`flex items-center gap-1 hover:text-zinc-900 dark:hover:text-zinc-100 transition-colors ${p.class ?? ""}`}
    >
      <span>{p.label}</span>
      <Show when={sortKey() === p.k}>
        {sortDir() === "desc" ? <ArrowDown size={10} /> : <ArrowUp size={10} />}
      </Show>
    </button>
  );

  return (
    <div class="flex flex-col h-full">
      <div class="px-6 py-4 border-b border-black/5 dark:border-white/5 flex items-center gap-3 flex-wrap">
        <div class="flex gap-6">
          <div>
            <div class="text-xs text-zinc-500">{t("process.totalRows")}</div>
            <div class="text-lg font-semibold tabular-nums">{rows().length}</div>
          </div>
          <div>
            <div class="text-xs text-zinc-500">{t("process.totalMemory")}</div>
            <div class="text-lg font-semibold tabular-nums">
              {(totalMem() / 1024).toFixed(1)}
              <span class="text-xs text-zinc-500 ml-1">GB</span>
            </div>
          </div>
          <div>
            <div class="text-xs text-zinc-500">{t("process.totalCpu")}</div>
            <div class="text-lg font-semibold tabular-nums">
              {totalCPU().toFixed(1)}
              <span class="text-xs text-zinc-500 ml-1">%</span>
            </div>
          </div>
          <div>
            <div class="text-xs text-zinc-500">{t("process.selectedRows")}</div>
            <div class="text-lg font-semibold tabular-nums text-brand-600">
              {selected().size}
            </div>
          </div>
        </div>

        <div class="flex-1" />

        <div class="inline-flex bg-black/5 dark:bg-white/5 rounded-lg p-1 gap-0.5">
          <button
            type="button"
            onClick={() => setViewMode("tree")}
            class={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-medium transition-colors ${
              viewMode() === "tree"
                ? "bg-white dark:bg-zinc-700 shadow-sm"
                : "text-zinc-500 hover:text-zinc-700"
            }`}
            title={t("process.viewTreeTitle")}
          >
            <ListTree size={12} />
            {t("process.viewTree")}
          </button>
          <button
            type="button"
            onClick={() => setViewMode("flat")}
            class={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-medium transition-colors ${
              viewMode() === "flat"
                ? "bg-white dark:bg-zinc-700 shadow-sm"
                : "text-zinc-500 hover:text-zinc-700"
            }`}
            title={t("process.viewFlatTitle")}
          >
            <List size={12} />
            {t("process.viewFlat")}
          </button>
        </div>

        <button
          type="button"
          onClick={() => setPortsOnly(!portsOnly())}
          title={
            portsOnly() ? t("process.portsOnlyOff") : t("process.portsOnlyTitle")
          }
          class={`inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors ${
            portsOnly()
              ? "bg-brand-500 text-white"
              : "bg-black/5 dark:bg-white/5 text-zinc-600 dark:text-zinc-300 hover:bg-black/10 dark:hover:bg-white/10"
          }`}
        >
          <Network size={12} />
          {t("process.portsOnlyOn")}
        </button>

        <div class="relative">
          <Search size={14} class="absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400" />
          <input
            type="text"
            placeholder={t("process.searchPlaceholder")}
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            class="pl-8 pr-8 py-1.5 rounded-lg text-sm bg-black/5 dark:bg-white/5 border border-transparent focus:border-brand-500/50 focus:bg-white dark:focus:bg-zinc-800 outline-none w-[220px]"
          />
          <Show when={query()}>
            <button
              type="button"
              onClick={() => setQuery("")}
              class="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-400 hover:text-zinc-600"
            >
              <X size={12} />
            </button>
          </Show>
        </div>

        <button
          type="button"
          data-testid="process-refresh"
          class="btn-ghost gap-1.5"
          disabled={loading()}
          onClick={() => {
            setError(null);
            void load(true);
          }}
        >
          <Show when={!loading()} fallback={<Loader2 size={12} class="animate-spin" />}>
            <RefreshCw size={12} />
          </Show>
        </button>
      </div>

      <div class="px-6 py-2 grid grid-cols-[1fr_72px_72px_64px_56px_96px] gap-2 text-[11px] font-medium text-zinc-500 border-b border-black/5 dark:border-white/5">
        <SortHeader k="name" label={t("process.columnName")} />
        <SortHeader k="cpu" label="CPU" class="justify-end" />
        <SortHeader k="memory" label={t("process.totalMemory")} class="justify-end" />
        <SortHeader k="uptime" label={t("process.columnUptime")} class="justify-end" />
        <SortHeader k="pid" label={t("process.columnPid")} class="justify-end" />
        <div class="text-right">{t("process.columnAction")}</div>
      </div>

      <div
        data-testid="process-list"
        class="flex-1 overflow-y-auto"
        onMouseEnter={() => setHoveringList(true)}
        onMouseLeave={() => setHoveringList(false)}
      >
        <Show
          when={visibleNodes().length > 0}
          fallback={
            <div class="py-20 text-center text-sm text-zinc-500">
              <Show when={!loading()}>
                {query()
                  ? t("process.noMatch", { query: query() })
                  : t("process.noProcesses")}
              </Show>
            </div>
          }
        >
          <For each={visibleNodes()}>
            {(node) => {
              const r = node.row;
              const isExpandable =
                viewMode() === "tree" && hasChildren(r.pid);
              const isCollapsed = collapsed().has(r.pid);
              return (
                <div
                  data-testid="process-row"
                  // items-start + 数字格 pt-[2px]：行高由内容决定（2 行 46.29px /
                  // 3 行 60.57px），若用 items-center，右侧数字列会按各自行高居中
                  // 而互相错开 7.14px。改成首行锚定后残差归零。
                  // 算式（Tailwind v4.3.3 实测编译值，勿凭 preflight 的 1.5 推算）：
                  //   行容器 .text-sm → line-height: var(--tw-leading, var(--text-sm
                  //   --line-height)) = calc(1.25/0.875) = 1.42857，覆盖 preflight 的
                  //   html { line-height: 1.5 }；首行 14 × 1.42857 = 20.00px。
                  //   .text-[10px] / .text-[11px] 只输出 font-size、不输出 line-height，
                  //   因此继承 1.42857：副标题行 10 × 1.42857 = 14.29px。
                  //   2 行内容 20.00 + 14.29 = 34.29px，3 行 48.57px，各加 py-1.5 12px。
                  // 数字格偏置：text-xs 行高 12 × 1.33333 = 16.00px → 需 2.00px（精确，
                  // 零残差）；text-[11px] 行高 15.71px → 需 2.14px，pt-[2px] 残差
                  // 0.14px（亚像素，不可见）。
                  // 结构由 ProcessView.test.tsx 的「所有格子锚在首行基线」用例锁住。
                  class="px-6 py-1.5 grid grid-cols-[1fr_72px_72px_64px_56px_96px] gap-2 items-start text-sm hover:bg-black/[0.02] dark:hover:bg-white/[0.02] cursor-default group"
                  classList={{ "opacity-60": r.protected }}
                >
                  <div class="min-w-0 flex items-center gap-2">
                    <div
                      // h-5 = 20px，与首行行高一致，把前导控件钉在首行基线上。
                      // 不加 h-5 时本格高度随子元素浮动（w-4 占位 16px / 展开按钮
                      // 16px / shield 行盒 20px / 图标 w-5 20px），居中的复选框
                      // 就会跟着行内是否出现 shield 与图标而漂。
                      // self-start 目前是冗余的（父格是 items-center，不拉伸），
                      // 留着是为了父格哪天改成 items-stretch 时这层仍锚在顶部。
                      // 本格刻意不加 pt-[2px]：内容已被 h-5 归一到 20px，
                      // items-center 在 20px 盒子里自动产出精确的 2px 内缩。
                      class="flex-shrink-0 flex h-5 items-center self-start"
                      style={{ "padding-left": `${node.depth * 16}px` }}
                    >
                      <Show
                        when={isExpandable}
                        fallback={<div class="w-4" />}
                      >
                        <button
                          type="button"
                          onClick={() => toggleCollapse(r.pid)}
                          class="p-0.5 rounded hover:bg-black/10 dark:hover:bg-white/10 text-zinc-500"
                          title={isCollapsed ? t("process.expand") : t("process.collapse")}
                        >
                          <Show when={isCollapsed} fallback={<ChevronDown size={12} />}>
                            <ChevronRight size={12} />
                          </Show>
                        </button>
                      </Show>
                    </div>

                    {/* 同一处门禁：这份复选框是「排序后的行」用的，
                        与 ProcessList 里那份是两套实现 —— 只改一处会漏。
                        留着它的后果和另一处一样：点下去不会有任何结果。 */}
                    <Show when={canTerminateProcesses()}>
                      <input
                        type="checkbox"
                        checked={selected().has(r.selection_key)}
                        disabled={r.whitelisted}
                        onChange={() => toggleSelect(r.selection_key)}
                        class="w-4 h-4 rounded accent-brand-500 flex-shrink-0 disabled:opacity-40"
                        title={
                          r.whitelisted
                            ? t("process.whitelistLocked")
                            : r.protected
                              ? t("process.protectedCheckboxTitle")
                              : undefined
                        }
                      />
                    </Show>
                    <Show when={r.protected}>
                      <span
                        title={
                          (r.protected_reason_key
                            ? tText(r.protected_reason_key, r.protected_reason_params)
                            : null) ?? t("process.protectedDefault")
                        }
                      >
                        <ShieldAlert
                          size={12}
                          class="text-warning-500 flex-shrink-0"
                        />
                      </span>
                    </Show>
                    <Show when={r.icon_base64}>
                      <img
                        src={`data:image/png;base64,${r.icon_base64}`}
                        alt=""
                        class="w-5 h-5 rounded flex-shrink-0"
                      />
                    </Show>
                    <div class="min-w-0 flex-1" data-testid="process-row-name">
                      <div class="flex items-center gap-2 min-w-0">
                        <span class="truncate font-medium" title={r.full_name || r.name}>
                          {r.name_key ? t(r.name_key) : r.name}
                        </span>
                        <Show when={r.ports.length > 0}>
                          <span
                            class="px-1.5 py-0.5 rounded-md text-[10px] font-mono font-semibold bg-brand-500/15 text-brand-700 dark:text-brand-300"
                            title={t("process.listeningPorts", {
                              ports: r.ports.join(", "),
                            })}
                          >
                            :
                            {portsOnly()
                              ? r.ports.join(", ")
                              : r.ports.length <= 2
                                ? r.ports.join(",")
                                : `${r.ports.slice(0, 2).join(",")}+${r.ports.length - 2}`}
                          </span>
                        </Show>
                        <Show
                          when={
                            r.status_key === "process.status.zombie" ||
                            r.status_key === "process.status.dead"
                          }
                        >
                          <span class="px-1.5 py-0.5 rounded-md text-[9px] font-medium bg-danger-500/15 text-danger-600">
                            {t(r.status_key)}
                          </span>
                        </Show>
                      </div>
                      <Show when={r.full_name && r.full_name !== r.name}>
                        <div
                          class="text-[10px] text-zinc-500 dark:text-zinc-400 font-mono truncate"
                          title={r.full_name}
                        >
                          {r.full_name}
                        </div>
                      </Show>
                      <Show when={r.protected && r.protected_reason_key}>
                        <div class="text-[10px] text-warning-600 dark:text-warning-400 truncate">
                          {tText(r.protected_reason_key!, r.protected_reason_params)}
                        </div>
                      </Show>
                      <Show when={!r.protected && r.exe}>
                        <div class="text-[10px] text-zinc-400 font-mono truncate">
                          {r.exe}
                        </div>
                      </Show>
                    </div>
                  </div>
                  <div class="text-right tabular-nums text-xs pt-[2px]">
                    <span
                      classList={{
                        "text-danger-600 font-semibold": r.cpu_percent > 50,
                        "text-warning-600":
                          r.cpu_percent > 20 && r.cpu_percent <= 50,
                        "text-zinc-600 dark:text-zinc-400": r.cpu_percent <= 20,
                      }}
                    >
                      {r.cpu_percent.toFixed(1)}%
                    </span>
                  </div>
                  <div class="text-right tabular-nums text-xs text-zinc-600 dark:text-zinc-400 pt-[2px]">
                    {r.memory_mb < 1024
                      ? `${r.memory_mb.toFixed(0)}M`
                      : `${(r.memory_mb / 1024).toFixed(1)}G`}
                  </div>
                  <div class="text-right tabular-nums text-[11px] text-zinc-500 pt-[2px]">
                    {fmtUptime(r.uptime_secs)}
                  </div>
                  <div class="text-right tabular-nums text-[11px] text-zinc-400 font-mono pt-[2px]">
                    {r.pid}
                  </div>
                  <div class="flex items-center justify-end gap-1 pt-[2px] opacity-0 group-hover:opacity-100 transition-opacity">
                    <button
                      type="button"
                      title={t("scan.whitelistTooltip")}
                      // 必须传原始名：后端 is_whitelisted 按 proc.name() 比对
                      // （scanner.rs:289），传展示名会写出永远匹配不上的死条目。
                      onClick={() => void whitelist(r.full_name || r.name)}
                      class="p-1 rounded-md text-zinc-400 hover:text-brand-600 hover:bg-brand-500/10"
                    >
                      <ShieldCheck size={13} />
                    </button>
                  </div>
                </div>
              );
            }}
          </For>
        </Show>
      </div>

      <Show when={killDetails() && killDetails()!.length > 0}>
        <div class="mx-6 mb-3 p-3 rounded-lg bg-warning-500/10 border border-warning-500/20 animate-slide-up">
          <div class="text-xs font-semibold text-warning-700 dark:text-warning-400 mb-1.5">
            {t("process.failedTitle")}
          </div>
          <ul class="text-[11px] space-y-1 text-zinc-600 dark:text-zinc-300">
            <For each={killDetails()}>
              {(d) => (
                <li class="flex gap-2">
                  <span class="font-medium min-w-[120px]">
                    {d.name} <span class="text-zinc-400">(PID {d.pid})</span>
                  </span>
                  <span class="text-zinc-500">{errorText(d.message, tText)}</span>
                </li>
              )}
            </For>
          </ul>
        </div>
      </Show>

      <div class="px-6 py-3 border-t border-black/5 dark:border-white/5 flex items-center gap-3">
        <Show
          when={canTerminateProcesses()}
          fallback={
            // MAS 版：App Sandbox 下沙箱进程不能给其他进程发信号，且没有
            // entitlement 能放行。与其留一个点了必定失败的按钮、再弹「权限不足」
            // （会被误解成系统设置问题），不如把入口换成一句说明。
            <p class="text-xs text-zinc-500 max-w-prose">
              {t("process.masTerminateUnsupported")}
            </p>
          }
        >
          <button
            type="button"
            class="btn-primary"
            disabled={selected().size === 0 || busy()}
            onClick={() => void terminateSelected()}
          >
            <Show when={!busy()} fallback={<Loader2 size={14} class="animate-spin" />}>
              {t("process.terminateSelected", { count: selected().size })}
            </Show>
          </button>
        </Show>
        {/* 「受保护进程」的提示只在真的有受保护行时才相关。
            MAS 版的只读列表里 protected 恒为 false（没有东西需要保护），
            所以这句提示是纯噪音 —— 而且它出现在列表底部，很容易被
            误读成一条错误信息。 */}
        <Show when={canTerminateProcesses()}>
          <span class="text-xs text-zinc-500">
            {t("process.protectedHint")}
          </span>
        </Show>
        <Show when={error()}>
          <span class="ml-auto text-xs text-danger-600 dark:text-danger-400" role="alert">
            {error()}
          </span>
        </Show>
        <Show when={message() && !killDetails()}>
          <span class="ml-auto text-xs text-zinc-500">{message()}</span>
        </Show>
      </div>

      <Show when={confirmProtected()}>
        {(d) => (
          <ProtectedForceConfirm
            title={t("process.confirmProtectedTitle")}
            message={t("process.confirmProtectedMessage")}
            confirmLabel={t("process.forceTerminate")}
            cancelLabel={t("common.cancel")}
            rows={d().protected}
            onConfirm={() => {
              const info = d();
              setConfirmProtected(null);
              void requestOperation(info.keys, info.mode);
            }}
            onCancel={() => setConfirmProtected(null)}
          />
        )}
      </Show>

      <Show when={prepared()}>
        {(operation) => (
          <OperationConfirm
            prepared={operation()}
            title={t("opConfirm.title")}
            confirmLabel={t("opConfirm.confirm")}
            busy={busy()}
            onConfirm={() => void executePrepared()}
            onCancel={() => setPrepared(null)}
          />
        )}
      </Show>
    </div>
  );
};

export default ProcessView;
