import { createSignal } from "solid-js";

/**
 * 构建形态：Developer ID 全功能版 vs Mac App Store 版。
 *
 * 与 `src-tauri/src/flavor.rs` 一一对应。**后端是唯一真相源** —— 这里不做任何
 * 猜测（比如「看有没有 updater 权限」），只读后端在编译期定死的值。
 *
 * 为什么前端必须知道：MAS 版跑在 App Sandbox 里，终止不了别的进程。让用户点
 * 一个注定失败的按钮、再弹一个「权限不足」是比直接藏掉更糟的体验 —— 用户会
 * 以为是系统设置问题。宁可能力不出现，也不要出现一个坏掉的入口。
 */

/** 与 `src-tauri/src/flavor.rs` 的 `Flavor::as_str()` 逐字对应。 */
export type Flavor = "developer_id" | "mas";

/**
 * 当前二进制的形态。
 *
 * 默认按 Developer ID 处理（最保守的一侧：全功能）。真实值由后端
 * `flavor::CURRENT` 在**启动时**经 `get_build_flavor` 下发，见下方
 * `initFlavor()`。
 */
// 用 signal 而不是普通变量。
//
// 普通 `let current` 配普通赋值，Solid **追踪不到**：组件在 `initFlavor()`
// 之前渲染过一次，之后就永远停在 developer_id —— 表现是「侧栏里 MAS
// 该藏的入口还在」。踩过：能力门禁已经写好、`initFlavor()` 也确实调了，
// 但界面没变。signal 让依赖它的表达式自动重算。
const [current, setCurrent] = createSignal<Flavor>("developer_id");

export const getFlavor = (): Flavor => current();

/** 能否终止其他进程。MAS 版恒为 false。 */
export const canTerminateProcesses = (): boolean => current() !== "mas";

// ============================================================================
// 能力表 —— 「界面不许露出用不了的入口」这条规则的唯一真相源
// ============================================================================
//
// ## 为什么需要它
//
// MAS 版跑在沙箱里，有一批能力真的做不到。此前这些入口**照常显示**，
// 点进去是空页面、或点了直接报错：
//
// - 「应用程序」页按 .app 聚合进程，而聚合依赖的进程枚举在沙箱里拿不到
// - 「检查更新」在 MAS 走 App Store 更新，updater 插件整块没注册
//
// 这不只是体验问题，是**审核问题**：App Store 指南 2.1（App Completeness）
// 里「导航到空页面、按钮点了报错」是最典型的不完整形态；指南 4.0 把
// 「功能太少」列为下架原因第一位。
//
// 更糟的是**假数据**：MAS 版曾经显示「没有发现可优化的进程，系统运行良好」，
// 而那台机器上有 186 个进程 —— 只是枚举被沙箱拦了。对审核说是误导，
// 对用户是骗人。
//
// ## 铁律
//
// 1. **默认 false。** 新增能力必须显式写进某一张表，否则不显示 ——
//    漏写的后果是「坏入口漏出来」，而漏出来的是审核风险。
// 2. 门禁测试要覆盖两侧：MAS 藏起来的，完整版必须**一个不少**。
//    只测 MAS 会让「门禁把主产品削了」这种错发生得悄无声息。

/** 一项产品能力。名字用业务语义，不用「某某命令」。 */
export type Capability =
  /** 只读进程监控（列出进程、看 CPU/内存） */
  | "processMonitor"
  /** 终止其他进程 */
  | "terminateProcess"
  /** 按 .app 聚合「运行中的应用」 */
  | "appGrouping"
  /** 扫描并清理缓存 */
  | "cacheClean"
  /** 应用体积 / 架构分析 */
  | "appSizeAnalysis"
  /** 应用内检查更新 */
  | "inAppUpdate"
  /**
   * 需要用户逐目录授权才能读用户数据。
   *
   * 单独一项而不是复用 cacheClean：两者在 MAS 版**同时为真**，
   * 但语义不同 —— 一个说「这页有东西可看」，另一个说「看到它要先授权」。
   * 合成一项会让「有能力但没授权」和「有能力且已授权」这两种状态无法区分，
   * 而后者不该再显示授权卡片。
   */
  | "folderGrant"
  /**
   * Docker 镜像/容器/卷的清理。
   *
   * 必须单独一项而不是并进 cacheClean：Docker 清理的每一步都是
   * `docker system df` / `docker image prune` 这类**外部 CLI**，沙箱里
   * 跑不通。后端 `cache_scanner.rs` 早就在 `can_exec_external_tools()`
   * 下不产出 docker 分类了，前端若不跟着藏，就会出现一块永远空的
   * 卡片 + 一个点了没反应的「一键清理」—— 比空页面更糟。
   */
  | "dockerCleanup";

const DEVELOPER_ID: readonly Capability[] = [
  "processMonitor",
  "terminateProcess",
  "appGrouping",
  "cacheClean",
  "appSizeAnalysis",
  "inAppUpdate",
  "dockerCleanup",
];

// MAS 版刻意比完整版少的：
// - terminateProcess：沙箱不能给别的进程发信号，没有任何 entitlement 能放行
// - inAppUpdate：MAS 由 App Store 负责更新，updater 插件整块不注册
// - appGrouping：这一页依赖的按 .app 聚合还没接上（进程监控本身是好的），
//   放出来就是一个空页面
// - dockerCleanup：要 exec `docker` CLI，沙箱里拿不到 inventory
const MAS: readonly Capability[] = [
  "processMonitor",
  "cacheClean",
  "appSizeAnalysis",
  "folderGrant",
];

const CAPABILITIES: Record<Flavor, readonly Capability[]> = {
  developer_id: DEVELOPER_ID,
  mas: MAS,
};

/** 某项能力在当前形态下是否可用。未知能力一律 false。 */
export const can = (capability: Capability | string): boolean =>
  CAPABILITIES[current()].includes(capability as Capability);

/** MAS 版支持哪些能力（仅供测试与诊断用）。 */
export const masCan = (capability: string): boolean =>
  MAS.includes(capability as Capability);

/**
 * 某个形态支持哪些能力。
 *
 * 单独暴露而不是让调用方读运行期的 `can()`：侧栏要能按参数回答
 * 「如果当前形态是 X 会怎样」，测试才问得了这个问题。
 */
export const capabilitiesOf = (flavor: Flavor): readonly Capability[] =>
  CAPABILITIES[flavor];

/** 当前形态支持的能力（随 `initFlavor()` 自动变化）。 */
export const visibleCapabilities = (): readonly Capability[] =>
  CAPABILITIES[current()];

/**
 * 能否用系统 shell / 外部 CLI。MAS 版恒为 false。
 *
 * 目前前端没有直接依赖它 —— 后端那些调用点自己会降级（`docker::is_available`
 * 先 `which` 再 exec，任一步失败都返回 false）。留着是因为一旦前端新增了
 * 「顺便帮你跑个 npm --version」这类能力，这里就是它的判断点。
 */
export const canExecExternalTools = (): boolean => current() !== "mas";

/**
 * 拉取后端形态。失败时保持默认的 `developer_id`。
 *
 * 失败**不能**改成 `mas`：把能力藏起来是安全侧的保守默认，但反过来 ——
 * 误报成 mas 会让全功能版用户凭空丢掉进程管理。宁可多显示一个坏入口，
 * 也不能少给功能。
 */
export const initFlavor = async (): Promise<Flavor> => {
  try {
    const { getBuildFlavor } = await import("./tauri");
    const wire = await getBuildFlavor();
    if (wire === "mas" || wire === "developer_id") {
      setCurrent(wire);
    }
  } catch {
    // 拿不到就按全功能处理，见上面的注释
  }
  return current();
};
