import { describe, expect, it } from "vitest";
import cacheScannerSource from "../../src-tauri/src/cache_scanner.rs?raw";
import { en } from "./en";
import { zhCN } from "./zh-CN";

/**
 * 缓存页 i18n 门禁。
 *
 * 后端 `cache_scanner.rs` 只发 i18n key + 插值参数，译文全在前端词典里。
 * 于是「漏翻」「错翻」「往 key 位里塞硬编码中文」这三类事故都**不会**在编译期
 * 或运行期暴露，只能靠这几道门禁钉死。
 */

/** 把嵌套词典按点路径摊平成 `{ "cache.item.trash": "废纸篓", ... }`。 */
function flatten(node: unknown, prefix: string): Record<string, string> {
  if (typeof node === "string") return { [prefix]: node };
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(node as Record<string, unknown>)) {
    Object.assign(out, flatten(value, `${prefix}.${key}`));
  }
  return out;
}

const zhCache = flatten((zhCN as Record<string, unknown>).cache, "cache");
const enCache = flatten((en as Record<string, unknown>).cache, "cache");

/** 只取后端负责的那两个命名空间（chrome 文案由 CacheView 自己的 key 覆盖）。 */
const isBackendKey = (key: string) =>
  key.startsWith("cache.item.") || key.startsWith("cache.desc.");

const backendKeysOf = (dict: Record<string, string>) =>
  Object.keys(dict).filter(isBackendKey).sort();

const placeholdersOf = (template: string) =>
  [...template.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

// 日文假名 / 谚文同样不该出现在英文词典里：它们和汉字一样会让英文界面「看起来像中文」。
const CJK =
  /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u;

const CACHE_SCANNER_SOURCE_HINT = "src-tauri/src/cache_scanner.rs（?raw 导入）";

/**
 * 只取 `#[cfg(test)] mod tests` 之前的生产代码。
 *
 * 不切掉测试模块的话，测试自己断言里写的 key 字面量会把「生产代码已经不发了」
 * 这件事盖掉：某处改成硬编码中文后，孤儿门禁仍能在测试模块里搜到那个 key 而全绿。
 */
function productionSource(): string {
  const cut = cacheScannerSource.indexOf("#[cfg(test)]");
  if (cut < 0) {
    throw new Error(
      `${CACHE_SCANNER_SOURCE_HINT} 里找不到 #[cfg(test)] 边界，孤儿门禁会误判`,
    );
  }
  return cacheScannerSource.slice(0, cut);
}

/**
 * 抓出后端生产代码里出现的所有 `cache.item.*` / `cache.desc.*` 字面量。
 *
 * 刻意只收 ASCII：有人把中文文案直接写进 `label_key` 时，它压根不会被这里收进来，
 * 于是「该 key 不再被后端发出」——由下面的孤儿测试变红，而不是被静默放过。
 */
function backendKeysFromSource(): string[] {
  const found = productionSource().matchAll(
    /"(cache\.(?:item|desc)\.[A-Za-z0-9]+)"/g,
  );
  return [...new Set([...found].map((m) => m[1]))].sort();
}

describe("缓存页 i18n 词典", () => {
  it("中英两侧的 cache 命名空间 key 集合完全相等", () => {
    // 两侧都非空：空集合和空集合「相等」但什么都没守住
    expect(Object.keys(zhCache).length).toBeGreaterThan(30);
    expect(Object.keys(enCache).sort()).toEqual(Object.keys(zhCache).sort());
  });

  it("后端发出的每个缓存文案 key 在中英两侧都存在", () => {
    const backend = backendKeysFromSource();
    // 抽取本身必须有效：正则失效 / ?raw 导入失效都会让下面所有断言空转成「全过」
    expect(backend.length, `没能从 ${CACHE_SCANNER_SOURCE_HINT} 抽出 key`).toBeGreaterThan(30);

    for (const key of backend) {
      expect(Object.keys(zhCache), `zh-CN 缺少 ${key}`).toContain(key);
      expect(Object.keys(enCache), `en 缺少 ${key}`).toContain(key);
    }
  });

  it("不存在「已翻译但后端从不发出」的孤儿缓存文案 key", () => {
    // 反向门禁：后端某处改成硬编码中文 / 改错 key 时，
    // 那个 key 会从抽取结果里消失，于是这里报「孤儿」。
    const backend = new Set(backendKeysFromSource());
    const orphans = backendKeysOf(zhCache).filter((key) => !backend.has(key));
    expect(orphans).toEqual([]);
  });

  it("同一个 key 在中英两侧的插值参数名完全一致", () => {
    for (const key of backendKeysOf(zhCache)) {
      // 先把「某一侧根本没这个 key」报成清楚的断言，
      // 否则下面的 matchAll 会以 TypeError 崩掉，看不出是哪一侧缺了
      expect(zhCache[key], `zh-CN 缺少 ${key}`).toBeDefined();
      expect(enCache[key], `en 缺少 ${key}`).toBeDefined();
      expect(placeholdersOf(enCache[key]), `${key} 的参数名两侧不一致`).toEqual(
        placeholdersOf(zhCache[key]),
      );
    }
  });

  it("英文侧的 cache 命名空间不含任何 CJK 字符", () => {
    const offenders = Object.entries(enCache)
      .filter(([, value]) => CJK.test(value))
      .map(([key, value]) => `${key} = ${value}`);
    expect(offenders).toEqual([]);
  });
});
