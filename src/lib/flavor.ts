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
let current: Flavor = "developer_id";

export const getFlavor = (): Flavor => current;

/** 能否终止其他进程。MAS 版恒为 false。 */
export const canTerminateProcesses = (): boolean => current !== "mas";

/**
 * 能否用系统 shell / 外部 CLI。MAS 版恒为 false。
 *
 * 目前前端没有直接依赖它 —— 后端那些调用点自己会降级（`docker::is_available`
 * 先 `which` 再 exec，任一步失败都返回 false）。留着是因为一旦前端新增了
 * 「顺便帮你跑个 npm --version」这类能力，这里就是它的判断点。
 */
export const canExecExternalTools = (): boolean => current !== "mas";

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
      current = wire;
    }
  } catch {
    // 拿不到就按全功能处理，见上面的注释
  }
  return current;
};
