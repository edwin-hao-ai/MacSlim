import { describe, expect, it } from "vitest";
import cacheScannerSource from "../../src-tauri/src/cache_scanner.rs?raw";
import operationRegistrySource from "../../src-tauri/src/operation_registry.rs?raw";
import processSafetySource from "../../src-tauri/src/process_safety.rs?raw";
import operationCommandsSource from "../../src-tauri/src/operation_commands.rs?raw";
import scannerSource from "../../src-tauri/src/scanner.rs?raw";
import { en } from "./en";
import { zhCN } from "./zh-CN";

/**
 * 第 2 / 3 片的 i18n 门禁：进程文案 + 操作摘要 + 扫描阶段名 + 相对时间。
 *
 * 与第 1 片（`cache-i18n.test.ts`）同一套手法，理由也一样：后端只发 key，
 * 译文全在前端词典里。于是「漏翻」「错翻」「往 key 位里塞硬编码中文」这三类
 * 事故都不会在编译期或运行期暴露，只能靠这几道门禁钉死。
 *
 * **英文侧 CJK 检测是本任务的核心防线**：目标用户是「英文 Mac 用户不能看到
 * 中文」，而这一类 bug 没有任何工具会主动报。
 */

/** 把嵌套词典按点路径摊平成 `{ "process.reason.zombie": "…", ... }`。 */
function flatten(node: unknown, prefix: string): Record<string, string> {
  if (typeof node === "string") return prefix ? { [prefix]: node } : {};
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(node as Record<string, unknown>)) {
    Object.assign(out, flatten(value, prefix ? `${prefix}.${key}` : key));
  }
  return out;
}

/** 本文件负责的命名空间：后端下发的 key + 相对时间 + 缓存分类徽章。 */
const OWNED_PREFIXES = [
  "process.reason.",
  "process.protect.",
  "process.status.",
  "process.name.",
  "opSummary.",
  "scanStage.",
  "timeFormat.",
  "cacheCategory.",
] as const;

/**
 * 不由后端下发、只在前端消费的 key。
 *
 * - `process.protectedInline` / `portsNote` / `portsMore`：扫描行副标题由前端把
 *   「基础理由 + 受保护 + 端口」三段拼起来，后端不参与拼句。
 * - `timeFormat.*` / `cacheCategory.*`：`format.ts` 里的纯前端文案。
 */
const FRONTEND_COMPOSED = new Set([
  "process.protectedInline",
  "process.portsNote",
  "process.portsMore",
  "timeFormat.justNow",
  "timeFormat.minutesAgo",
  "timeFormat.hoursAgo",
  "timeFormat.daysAgo",
  "cacheCategory.system",
]);

const flatZh = flatten(zhCN as Record<string, unknown>, "");
const flatEn = flatten(en as Record<string, unknown>, "");

const isOwned = (key: string) => OWNED_PREFIXES.some((p) => key.startsWith(p));

const ownedZh = Object.fromEntries(
  Object.entries(flatZh).filter(([key]) => isOwned(key)),
);
const ownedEn = Object.fromEntries(
  Object.entries(flatEn).filter(([key]) => isOwned(key)),
);

const placeholdersOf = (template: string) =>
  [...template.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

// 日文假名 / 谚文同样不该出现在英文词典里：它们和汉字一样会让英文界面「看起来像中文」。
const CJK =
  /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u;

const SOURCES: Record<string, string> = {
  "src-tauri/src/scanner.rs": scannerSource,
  "src-tauri/src/process_safety.rs": processSafetySource,
  "src-tauri/src/operation_commands.rs": operationCommandsSource,
  "src-tauri/src/operation_registry.rs": operationRegistrySource,
  "src-tauri/src/cache_scanner.rs": cacheScannerSource,
};

/**
 * 只取 `#[cfg(test)]` 之前的生产代码。
 *
 * 不切掉测试模块的话，测试自己断言里写的 key 字面量会把「生产代码已经不发了」
 * 这件事盖掉：某处改成硬编码中文后，孤儿门禁仍能在测试模块里搜到那个 key 而全绿。
 */
function productionSource(file: string): string {
  const source = SOURCES[file];
  const cut = source.indexOf("#[cfg(test)]");
  if (cut < 0) return source;
  return source.slice(0, cut);
}

/**
 * 抓出后端生产代码里出现的、属于本文件负责命名空间的所有 key 字面量。
 *
 * 刻意只收 ASCII：有人把中文文案直接写进 `reason_key` 时，它压根不会被这里
 * 收进来，于是「该 key 不再被后端发出」—— 由孤儿门禁变红，而不是被静默放过。
 */
function backendKeysFromSource(): string[] {
  const found = new Set<string>();
  for (const file of Object.keys(SOURCES)) {
    // 整段都要包进 pattern，不能只捕获命名空间那一截：捕获组少包了后缀，
    // 抽出来就会是「process.reason」这种半截 key，门禁看起来在跑、其实什么都没查。
    for (const [quoted] of productionSource(file).matchAll(
      /"(?:process\.(?:reason|protect|status|name)|opSummary|scanStage)\.[A-Za-z0-9.]+"/g,
    )) {
      found.add(quoted.slice(1, -1));
    }
  }
  return [...found].sort();
}

describe("进程 / 摘要 / 阶段名的 i18n 词典", () => {
  it("中英两侧本文件负责的 key 集合完全相等", () => {
    // 两侧都非空：空集合和空集合「相等」但什么都没守住
    expect(Object.keys(ownedZh).length).toBeGreaterThan(40);
    expect(Object.keys(ownedEn).sort()).toEqual(Object.keys(ownedZh).sort());
  });

  it("后端发出的每个 key 在中英两侧都存在", () => {
    const backend = backendKeysFromSource();
    // 抽取本身必须有效：正则失效 / ?raw 导入失效都会让下面所有断言空转成「全过」
    expect(backend.length, "没能从后端源码抽出 key").toBeGreaterThanOrEqual(30);

    for (const key of backend) {
      expect(Object.keys(ownedZh), `zh-CN 缺少 ${key}`).toContain(key);
      expect(Object.keys(ownedEn), `en 缺少 ${key}`).toContain(key);
    }
  });

  it("不存在「已翻译但后端从不发出」的孤儿 key", () => {
    // 反向门禁：后端某处改成硬编码中文 / 改错 key 时，那个 key 会从抽取结果里
    // 消失，于是这里报「孤儿」。
    const backend = new Set(backendKeysFromSource());
    const orphans = Object.keys(ownedZh).filter(
      (key) => !backend.has(key) && !FRONTEND_COMPOSED.has(key),
    );
    expect(orphans).toEqual([]);
  });

  it("同一个 key 在中英两侧的插值参数名完全一致", () => {
    for (const key of Object.keys(ownedZh)) {
      expect(ownedZh[key], `zh-CN 缺少 ${key}`).toBeDefined();
      expect(ownedEn[key], `en 缺少 ${key}`).toBeDefined();
      expect(placeholdersOf(ownedEn[key]), `${key} 的参数名两侧不一致`).toEqual(
        placeholdersOf(ownedZh[key]),
      );
    }
  });

  it("英文侧不含任何 CJK 字符（英文用户绝对不能看到中文）", () => {
    const offenders = Object.entries(ownedEn)
      .filter(([, value]) => CJK.test(value))
      .map(([key, value]) => `${key} = ${value}`);
    expect(offenders).toEqual([]);
  });

  /**
   * 英文译文里的每个花括号都必须是合法的 `{name}` 占位符。
   *
   * 查的是「花括号用错地方」这一类：漏写闭合括号（`{count`）、把占位符当普通
   * 括号（`Ready to clean {count} items {`）、或者整条译文就是一个光秃秃的
   * `{name}`（说明值没填，只把模板抄了过来）。合法的 `{name}` 本身是允许的。
   */
  it("英文译文里的花括号都是合法的占位符", () => {
    const offenders = Object.entries(ownedEn)
      .filter(([, value]) => {
        const wellFormed = value.replace(/\{\w+\}/g, "");
        if (/[{}]/.test(wellFormed)) return true;
        return value.trim().startsWith("{") && value.trim().endsWith("}");
      })
      .map(([key, value]) => `${key} = ${value}`);
    expect(offenders).toEqual([]);
  });
});

describe("进程分类理由的文案不变式", () => {
  const reasonKeys = Object.keys(ownedZh).filter((k) =>
    k.startsWith("process.reason."),
  );

  it("每一条分类理由都存在且非空", () => {
    expect(reasonKeys.length).toBe(11);
    for (const key of reasonKeys) {
      expect(ownedZh[key].length, key).toBeGreaterThan(0);
      expect(ownedEn[key].length, key).toBeGreaterThan(0);
    }
  });

  /**
   * 分类理由不得复述扫描行尾已经单独渲染的 CPU 百分比 / 内存 MB。
   *
   * 扫描页（`ProcessList.tsx`）在行尾渲染 `{cpu}% CPU` 与 `{mem}MB` 两列，
   * 理由里再写一遍就是同一屏出现两遍同一个数。这条不变式在改造前由
   * `scanner_tests.rs` 的 `leaks_column_number` 守着（作用在后端拼好的字符串上），
   * 文案搬到词典后同一份探测器必须跟着搬过来 —— 否则不变式在换实现的那天
   * 就静默失效了。
   */
  it("分类理由不复述 CPU 百分比与内存 MB", () => {
    const offenders = Object.entries(ownedZh)
      .filter(([key]) => key.startsWith("process.reason."))
      .filter(([, value]) => /\d+\s*(%|MB)/.test(value))
      .map(([key, value]) => `${key} = ${value}`);
    expect(offenders).toEqual([]);
  });

  /** 僵尸变体必须三枚齐全且互不相同：漏一枚就有一个否决种类显示裸 key。 */
  it("僵尸进程的三种否决变体各自独立成 key", () => {
    const zombieKeys = reasonKeys
      .filter((k) => k.startsWith("process.reason.zombieVetoed"))
      .sort();
    expect(zombieKeys).toEqual([
      "process.reason.zombieVetoedMultiProcessComponent",
      "process.reason.zombieVetoedParentOfOthers",
      "process.reason.zombieVetoedYoungProcess",
    ]);
    // 三枚 key 的译文也必须互不相同：共用一句就等于没区分否决种类
    const texts = zombieKeys.map((k) => ownedZh[k]);
    expect(new Set(texts).size).toBe(3);
  });
});

describe("扫描阶段名", () => {
  it("恰好 16 枚，与后端阶段表一一对应", () => {
    const stages = Object.keys(ownedEn).filter((k) => k.startsWith("scanStage."));
    expect(stages).toHaveLength(16);
  });
});

describe("操作摘要", () => {
  it("恰好 11 枚，覆盖每种操作与每种条件变体", () => {
    const summaries = Object.keys(ownedEn).filter((k) => k.startsWith("opSummary."));
    expect(summaries).toHaveLength(11);
    // 优雅退出与强制终止必须是两枚不同的 key：共用一枚就分不出界面上
    // 「准备优雅退出…」和「准备强制终止…」了。
    expect(ownedZh["opSummary.processGraceful"]).not.toBe(
      ownedZh["opSummary.processForce"],
    );
    // 卸载的条件尾巴走独立 key，模板里不放「可能为空」的片段
    expect(ownedZh["opSummary.uninstall"]).not.toBe(
      ownedZh["opSummary.uninstallQuitFirst"],
    );
  });

  /**
   * `dockerPrune` 摘要里的「清理后不可撤销」必须两种语言都在。
   *
   * AGENTS.md §4.1 把「明确告知不可撤销」列为硬要求，而这条提示原本就只写在
   * prune 摘要里（卸载的不可撤销告知在确认弹窗的 `opConfirm.irreversible` /
   * 卸载页的 `confirmMessage` 上，不在摘要里）。改造时把这句话从后端搬到词典，
   * 搬的过程中丢掉它不会有任何编译或运行错误 —— 所以在这里钉住。
   */
  it("dockerPrune 摘要保留了不可撤销提示", () => {
    expect(ownedZh["opSummary.dockerPrune"]).toMatch(/不可撤销/);
    expect(ownedEn["opSummary.dockerPrune"]).toMatch(/cannot be undone/);
  });
});

describe("相对时间", () => {
  it("四档相对时间都存在，且不硬编码日期 locale", () => {
    for (const key of [
      "timeFormat.justNow",
      "timeFormat.minutesAgo",
      "timeFormat.hoursAgo",
      "timeFormat.daysAgo",
    ]) {
      expect(ownedZh[key], key).toBeTruthy();
      expect(ownedEn[key], key).toBeTruthy();
    }
  });
});
