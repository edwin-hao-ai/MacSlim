/**
 * 测试用的假 i18n 上下文工厂。
 *
 * 为什么需要它：`@/i18n` 的 `useI18n()` 现在返回的不只是 `t`，还有
 * `tText`（渲染后端下发的 key + 参数）、`tStage`（扫描阶段名）和
 * `effectiveLocale`。每个测试文件各自手写一份 mock 会立刻漂移 —— 谁漏了
 * 一个字段，谁的组件就在测试里抛 `xxx is not a function`，而报错点离
 * 真正的原因十万八千里。
 *
 * `t` 的行为**由各测试文件自己给**（有的要原样返回 key，有的要插值后返回
 * `key:值`，因为它们各自有精确文本断言）；本模块只补齐其余字段的合理默认。
 */

/** 参数二元组 → `t()` 认的对象。与 `paramsToRecord` 同形，但这里是测试替身。 */
export function paramsToRecordFake(
  params: ReadonlyArray<readonly [string, string]>,
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [name, value] of params) out[name] = value;
  return out;
}

export type FakeTranslate = (
  key: string,
  params?: Record<string, string | number>,
) => string;

const STAGE_PREFIX = "scanStage.";

/**
 * 组装一个可以冒充 `useI18n()` 返回值的对象。
 *
 * @param t 调用方自己的翻译函数（多数测试是「原样返回 key」）
 * @param locale `effectiveLocale()` 的返回值，喂给 `toLocaleDateString`
 */
export function fakeI18n(options: {
  t: FakeTranslate;
  tStage?: (stage: string) => string;
  locale?: "zh-CN" | "en";
}) {
  const t = options.t;
  return {
    t,
    /**
     * 渲染后端下发的「key + 参数」二元组。
     *
     * 空参数数组按「无参数」处理：真实现里 `paramsToRecord([])` 得到 `{}`，
     * `interpolate` 对它不做任何替换，所以输出就是模板本身。测试替身必须
     * 复刻这个行为，否则 `tText(key, [])` 会渲染成 `key:` 这种多一个冒号
     * 的字符串，让断言莫名其妙地找不到元素。
     */
    tText: (key: string, params?: ReadonlyArray<readonly [string, string]> | null) => {
      const record = params ? paramsToRecordFake(params) : {};
      return t(key, Object.keys(record).length > 0 ? record : undefined);
    },
    /**
     * 扫描阶段名：固定阶段是 `scanStage.*` 的 key（查词典）；残留扫描那条
     * 事件装的是应用名，原样返回。
     */
    tStage: (stage: string | null | undefined) => {
      const value = stage ?? "";
      if (options.tStage) return options.tStage(value);
      return value.startsWith(STAGE_PREFIX) ? t(value) : value;
    },
    locale: () => options.locale ?? "zh-CN",
    effectiveLocale: () => options.locale ?? "zh-CN",
    setLocale: () => undefined,
  };
}

/** `useI18n` 的最常见形态：原样返回 key。 */
export function passthroughT(key: string): string {
  return key;
}
