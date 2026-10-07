import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/i18n", () => ({
  useI18n: () => ({ t: (key: string) => key }),
}));

import HealthCard from "@/components/HealthCard";
import type { SystemHealth } from "@/lib/tauri";

const health = (
  cpu_percent: number,
  memory_percent: number,
  disk_percent: number,
): SystemHealth => ({
  cpu_percent,
  memory_used_mb: 8_000,
  memory_total_mb: 16_000,
  memory_percent,
  disk_used_gb: 400,
  disk_total_gb: 500,
  disk_percent,
});

const renderLevel = (h: SystemHealth | null) => {
  // 一个用例里会渲染多次（比较不同读数），必须先清理上一次的 DOM，
  // 否则 screen 会同时看到两个状态文案。
  cleanup();
  render(() => <HealthCard health={h} />);
  return screen.getByText(/^health\.(normal|warning|critical)$/).textContent;
};

describe("HealthCard 状态判定", () => {
  afterEach(cleanup);

  it("三项都宽松时报正常", () => {
    expect(renderLevel(health(10, 40, 50))).toBe("health.normal");
  });

  it("磁盘 87% 不再说「正常」", () => {
    // 这条是本次修复的核心：健康卡原来只要拿到读数就无条件渲染绿点 +
    // 「正常运行」，磁盘 95% 满时第一屏最大的结论仍是「正常运行」。
    // 对一个以「告诉你空间去哪了」为卖点的工具，这是最不该错的一句话。
    expect(renderLevel(health(10, 40, 87))).toBe("health.warning");
  });

  it("任一项越过高位报压力较高", () => {
    expect(renderLevel(health(95, 40, 50))).toBe("health.critical");
    expect(renderLevel(health(10, 92, 50))).toBe("health.critical");
    expect(renderLevel(health(10, 40, 96))).toBe("health.critical");
  });

  it("CPU 与内存的高位阈值比磁盘宽", () => {
    // 磁盘 85% 就提示（本产品的目标场景），而 CPU/内存要到 90% 才算压力，
    // 避免日常波动频繁告警。
    expect(renderLevel(health(80, 40, 50))).toBe("health.warning");
    expect(renderLevel(health(10, 82, 50))).toBe("health.warning");
    expect(renderLevel(health(10, 40, 50))).toBe("health.normal");
  });

  it("没有读数时不编造状态", () => {
    render(() => <HealthCard health={null} />);
    expect(screen.getByText("health.reading")).toBeTruthy();
    expect(screen.queryByText("health.normal")).toBeNull();
  });
});
