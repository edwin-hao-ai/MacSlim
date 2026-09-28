import { describe, expect, it, vi } from "vitest";
import {
  CATEGORY_LABELS,
  categoryLabel,
  fmtRelativeTime,
  isCategoryReclaimable,
  type Translate,
} from "./format";
import { en } from "@/i18n/en";
import { zhCN } from "@/i18n/zh-CN";

/**
 * `format.ts` 的 i18n 门禁。
 *
 * 这个文件原来有两处「英文用户必然看到中文」的问题：
 * 1. `fmtRelativeTime` 全程硬编码中文（「刚刚」/「N 分钟前」…）
 * 2. 超过 7 天走 `d.toLocaleDateString("zh-CN")` —— 界面切成英文后，
 *    日期仍然是 `2026/9/1` 这种中文格式
 *
 * 两条都必须由测试钉住，因为它们都不会在编译期或类型检查期暴露。
 *
 * 斜线网格（`isCategoryReclaimable`）的**判定**在这里测；**样式本体**
 * （纯 CSS / 深浅两套 / 不含分类名单）由 `scripts/tests/test_reclaimable_hatch.py`
 * 静态门禁守 —— TS 侧读不到未经 Tailwind 处理的 `styles.css` 原文
 * （`?raw` 拿到空串，断言会静默空转成全过）。
 */

type Dict = Record<string, unknown>;

function lookup(dict: Dict) {
  return (key: string): string => {
    let cur: unknown = dict;
    for (const part of key.split(".")) {
      if (typeof cur !== "object" || cur === null) return key;
      cur = (cur as Record<string, unknown>)[part];
    }
    return typeof cur === "string" ? cur : key;
  };
}

const zhT = lookup(zhCN as Dict);
const enT = lookup(en as Dict);

const at = (iso: string) => vi.useFakeTimers({ now: new Date(iso).getTime() });

const NOW = "2026-09-28T12:00:00Z";
const ago = (ms: number) => new Date(new Date(NOW).getTime() - ms).toISOString();
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

describe("fmtRelativeTime", () => {
  it("四档相对时间在中英两侧都不含 CJK", () => {
    at(NOW);
    try {
      const cases = [
        [ago(30_000), ["timeFormat.justNow", "timeFormat.justNow"]],
        [ago(5 * MIN), ["timeFormat.minutesAgo", "timeFormat.minutesAgo"]],
        [ago(3 * HOUR), ["timeFormat.hoursAgo", "timeFormat.hoursAgo"]],
        [ago(2 * DAY), ["timeFormat.daysAgo", "timeFormat.daysAgo"]],
      ] as const;
      for (const [iso, [zhKey, enKey]] of cases) {
        expect(fmtRelativeTime(iso, zhT as Translate, "zh-CN")).toBe(
          zhT(zhKey),
        );
        expect(fmtRelativeTime(iso, enT as Translate, "en")).toBe(enT(enKey));
      }
    } finally {
      vi.useRealTimers();
    }
  });

  it("英文侧的每一档输出都真的没有中文", () => {
    at(NOW);
    try {
      const outputs = [
        ago(30_000),
        ago(5 * MIN),
        ago(3 * HOUR),
        ago(2 * DAY),
      ].map((iso) => fmtRelativeTime(iso, enT as Translate, "en"));
      for (const output of outputs) {
        expect(output).not.toMatch(
          /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u,
        );
      }
      // 也不能退化成裸 key：查不到译文说明词典漏了条目
      for (const output of outputs) {
        expect(output.startsWith("timeFormat.")).toBe(false);
      }
    } finally {
      vi.useRealTimers();
    }
  });

  /**
   * 超过 7 天必须按**当前界面语言**格式化日期。
   *
   * 这条是硬编码 `zh-CN` 的直接反证：locale 传 `en` 就必须得到英文日期，
   * 传 `zh-CN` 才得到中文日期。
   */
  it("超过 7 天时日期格式跟着界面语言走", () => {
    at(NOW);
    try {
      const old = ago(30 * DAY);
      const asEn = fmtRelativeTime(old, enT as Translate, "en");
      const asZh = fmtRelativeTime(old, zhT as Translate, "zh-CN");
      expect(asEn).toBe(new Date(old).toLocaleDateString("en"));
      expect(asZh).toBe(new Date(old).toLocaleDateString("zh-CN"));
      expect(asEn).not.toBe(asZh);
    } finally {
      vi.useRealTimers();
    }
  });

  it("边界值落在与改造前相同的档位上", () => {
    at(NOW);
    try {
      expect(fmtRelativeTime(ago(MIN - 1), zhT as Translate, "zh-CN")).toBe(
        zhT("timeFormat.justNow"),
      );
      expect(fmtRelativeTime(ago(MIN), zhT as Translate, "zh-CN")).toBe(
        zhT("timeFormat.minutesAgo"),
      );
      expect(fmtRelativeTime(ago(HOUR - 1), zhT as Translate, "zh-CN")).toBe(
        zhT("timeFormat.minutesAgo"),
      );
      expect(fmtRelativeTime(ago(HOUR), zhT as Translate, "zh-CN")).toBe(
        zhT("timeFormat.hoursAgo"),
      );
      expect(fmtRelativeTime(ago(DAY - 1), zhT as Translate, "zh-CN")).toBe(
        zhT("timeFormat.hoursAgo"),
      );
      expect(fmtRelativeTime(ago(DAY), zhT as Translate, "zh-CN")).toBe(
        zhT("timeFormat.daysAgo"),
      );
      expect(fmtRelativeTime(ago(7 * DAY - 1), zhT as Translate, "zh-CN")).toBe(
        zhT("timeFormat.daysAgo"),
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it("源码里不再出现硬编码的 zh-CN 日期 locale", async () => {
    const source = await import("./format.ts?raw").then((m) => m.default);
    expect(source).not.toContain('toLocaleDateString("zh-CN")');
    expect(source).not.toContain("分钟前");
    expect(source).not.toContain("小时前");
  });
});

describe("categoryLabel", () => {
  it("品牌类分类两种语言写法相同，直接内联", () => {
    for (const key of ["npm", "docker", "xcode", "go"]) {
      expect(categoryLabel(key, enT as Translate)).toBe(CATEGORY_LABELS[key]);
      expect(categoryLabel(key, zhT as Translate)).toBe(CATEGORY_LABELS[key]);
    }
  });

  it("系统类分类查词典，英文侧不出现中文", () => {
    expect(categoryLabel("system", zhT as Translate)).toBe("系统");
    expect(categoryLabel("system", enT as Translate)).toBe("System");
  });

  it("未知分类原样回显（不吞掉后端给的新分类）", () => {
    expect(categoryLabel("brand-new-cache", enT as Translate)).toBe(
      "brand-new-cache",
    );
  });
});

describe("isCategoryReclaimable（斜线网格的判定）", () => {
  it("全组 safe / low 才算可清理", () => {
    expect(isCategoryReclaimable([{ safety: "safe" }])).toBe(true);
    expect(
      isCategoryReclaimable([
        { safety: "safe" },
        { safety: "low" },
      ]),
    ).toBe(true);
  });

  it("混进一项 medium 就不标（宁可少标不可多标）", () => {
    expect(
      isCategoryReclaimable([
        { safety: "safe" },
        { safety: "medium" },
      ]),
    ).toBe(false);
    expect(isCategoryReclaimable([{ safety: "medium" }])).toBe(false);
  });

  it("空组不算可清理（没有可标的空间）", () => {
    expect(isCategoryReclaimable([])).toBe(false);
  });

  it("不看 default_select：默认不勾选也可能确实可清（废纸篓就是这种）", () => {
    // `default_select` 表达「默认勾选 / 重新获取成本 < 5 分钟」，
    // 斜线纹理表达「这里确实是能清的空间」，两者语义不同。
    expect(
      isCategoryReclaimable([{ safety: "safe" }]),
    ).toBe(true);
  });
});
