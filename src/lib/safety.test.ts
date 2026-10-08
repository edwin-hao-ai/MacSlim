import { describe, expect, it } from "vitest";
import { filterBySafety, safetyBucket } from "./safety";

const items = [
  { safety: "safe" as const },
  { safety: "low" as const },
  { safety: "medium" as const },
];

describe("safetyBucket", () => {
  it("把 safe 归为 safe，low/medium 归为 check", () => {
    expect(safetyBucket("safe")).toBe("safe");
    expect(safetyBucket("low")).toBe("check");
    expect(safetyBucket("medium")).toBe("check");
  });
});

describe("filterBySafety", () => {
  it("all 返回全部", () => {
    expect(filterBySafety(items, "all")).toHaveLength(3);
  });
  it("safe 只留 safe", () => {
    expect(filterBySafety(items, "safe")).toEqual([{ safety: "safe" }]);
  });
  it("check 只留 low/medium", () => {
    expect(filterBySafety(items, "check")).toEqual([
      { safety: "low" },
      { safety: "medium" },
    ]);
  });
});
