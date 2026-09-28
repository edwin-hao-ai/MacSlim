import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/i18n", async () => {
  const { fakeI18n } = await import("@/i18n/fake-i18n");
  return {
    useI18n: () =>
      fakeI18n({
        t: (k: string, p?: Record<string, string | number>) =>
          p ? `${k}:${JSON.stringify(p)}` : k,
      }),
  };
});

import ScanStageProgress from "@/components/ScanStageProgress";

describe("ScanStageProgress", () => {
  afterEach(cleanup);

  it("第一个 running 事件到达后立刻显示阶段名，不等待任何 done", () => {
    render(() => (
      <ScanStageProgress current="scanStage.xcodeCache" doneCount={0} total={16} foundBytes={0} />
    ));
    expect(screen.getByTestId("scan-stage-name").textContent).toContain(
      "scanStage.xcodeCache",
    );
    expect(screen.getByTestId("scan-stage-count").textContent).toContain("0");
    expect(screen.getByTestId("scan-stage-count").textContent).toContain("16");
  });

  /**
   * 载荷形状不变，但 `stage` 装的内容变成了 i18n key。
   *
   * 关键约束：`StageUpdate` 仍然只有 stage / state / item_count / found_bytes
   * 四个字段（Rust 侧 `lib_progress_tests.rs` 用字段名逐一精确比对钉死）。
   * 所以阶段名 i18n **不能**靠「给事件加一个 stage_label 字段」实现 ——
   * 只能让 `stage` 本身装 key，前端在展示时翻译。
   */
  it("阶段名是 scanStage.* 的 key，展示时翻译；不是 key 的载荷原样显示", () => {
    // 固定阶段：走词典
    const stageRender = render(() => (
      <ScanStageProgress current="scanStage.trash" doneCount={0} total={16} foundBytes={0} />
    ));
    expect(stageRender.getByTestId("scan-stage-name").textContent).toContain(
      "scanStage.trash",
    );
    stageRender.unmount();

    // 残留扫描那条事件装的是**应用名**（用户数据，不是 key），必须原样显示
    const appRender = render(() => (
      <ScanStageProgress current="Google Chrome" doneCount={0} total={16} foundBytes={0} />
    ));
    expect(appRender.getByTestId("scan-stage-name").textContent).toContain(
      "Google Chrome",
    );
    appRender.unmount();
  });

  it("已完成计数与累计体积都可见", () => {
    render(() => (
      <ScanStageProgress current={null} doneCount={7} total={16} foundBytes={27_400_000_000} />
    ));
    expect(screen.getByTestId("scan-stage-count").textContent).toContain("7");
    expect(screen.getByTestId("scan-stage-bytes").textContent).toBeTruthy();
  });

  it("体积复用共享的 fmtBytes，全 App 只有一种数字精度", () => {
    render(() => (
      <ScanStageProgress current={null} doneCount={0} total={null} foundBytes={27_400_000_000} />
    ));
    // fmtBytes(27_400_000_000) === "25.52 GB"（GB 两位小数）。
    // 一旦这里换回组件内联的低精度实现，会变成 "25.5 GB" 而红。
    expect(screen.getByTestId("scan-stage-bytes").textContent).toContain(
      "25.52 GB",
    );
  });

  it("小于 1KB 时 fmtBytes 与进度组件的展示一致", () => {
    render(() => (
      <ScanStageProgress current={null} doneCount={0} total={null} foundBytes={300} />
    ));
    expect(screen.getByTestId("scan-stage-bytes").textContent).toContain("300 B");
  });

  it("总阶段数未知时不渲染进度条", () => {
    const { container } = render(() => (
      <ScanStageProgress current="scanStage.npmCache" doneCount={3} total={null} foundBytes={0} />
    ));
    expect(container.querySelector('[data-testid="scan-stage-bar"]')).toBeNull();
  });
});
