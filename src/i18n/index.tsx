import {
  createContext,
  createSignal,
  useContext,
  type Accessor,
  type JSX,
} from "solid-js";
import { zhCN, type Dict } from "./zh-CN";
import { en } from "./en";

export type LocaleCode = "zh-CN" | "en" | "auto";

const DICTS: Record<"zh-CN" | "en", Dict> = {
  "zh-CN": zhCN,
  en,
};

const STORAGE_KEY = "macslim.locale.v1";

/** 从系统语言推测默认语言。非中文一律 fallback 到英文。 */
function detectSystemLocale(): "zh-CN" | "en" {
  const lang = navigator.language || "zh-CN";
  return lang.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}

/** 解析 effective locale（把 auto 展开成实际语言）。 */
function resolve(locale: LocaleCode): "zh-CN" | "en" {
  if (locale === "auto") return detectSystemLocale();
  return locale;
}

function loadStored(): LocaleCode {
  try {
    const v = localStorage.getItem(STORAGE_KEY) as LocaleCode | null;
    if (v === "zh-CN" || v === "en" || v === "auto") return v;
  } catch {
    /* ignore */
  }
  return "auto";
}

/** 支持 {key} 占位符的简易插值。 */
export function interpolate(
  template: string,
  params?: Record<string, string | number>,
): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (_, k) =>
    params[k] !== undefined ? String(params[k]) : `{${k}}`,
  );
}

/** 用点路径从嵌套对象取字符串。找不到返回 key 本身。 */
function get(dict: Dict, path: string): string {
  const parts = path.split(".");
  let cur: unknown = dict;
  for (const p of parts) {
    if (typeof cur !== "object" || cur === null) return path;
    cur = (cur as Record<string, unknown>)[p];
  }
  return typeof cur === "string" ? cur : path;
}

/**
 * 后端下发的插值参数（`[名, 值]` 二元组数组，Rust `Vec<(String, String)>` 的
 * JSON 形状）转成 `t()` 认的记录。
 *
 * 后端必须用「key + 参数」而不是把文案拼好：文案只有前端词典知道。
 */
export function paramsToRecord(
  params: ReadonlyArray<readonly [string, string]>,
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [name, value] of params) out[name] = value;
  return out;
}

export type TextParams = ReadonlyArray<readonly [string, string]>;

type Translate = (key: string, params?: Record<string, string | number>) => string;

type I18nCtx = {
  t: (key: string, params?: Record<string, string | number>) => string;
  /** 渲染后端下发的「key + 参数」二元组。 */
  tText: (key: string, params?: TextParams | null | undefined) => string;
  /** 渲染扫描阶段名：固定阶段走 i18n key，残留扫描那条是应用名，原样显示。 */
  tStage: (stage: string | null | undefined) => string;
  locale: Accessor<LocaleCode>;
  effectiveLocale: Accessor<"zh-CN" | "en">;
  setLocale: (l: LocaleCode) => void;
};

/** 扫描阶段名的 i18n 命名空间。后端 `cache_scanner.rs` 的阶段表全部落在这里。 */
const STAGE_PREFIX = "scanStage.";

const Ctx = createContext<I18nCtx>();

export function I18nProvider(props: { children: JSX.Element }) {
  const [locale, setLocale] = createSignal<LocaleCode>(loadStored());

  const effective = () => resolve(locale());

  const t: Translate = (key, params) => {
    const dict = DICTS[effective()];
    return interpolate(get(dict, key), params);
  };

  const tText = (key: string, params?: TextParams | null) =>
    t(key, params ? paramsToRecord(params) : undefined);

  const tStage = (stage: string | null | undefined) => {
    const value = stage ?? "";
    return value.startsWith(STAGE_PREFIX) ? t(value) : value;
  };

  const save = (l: LocaleCode) => {
    setLocale(l);
    try {
      localStorage.setItem(STORAGE_KEY, l);
    } catch {
      /* ignore */
    }
  };

  return (
    <Ctx.Provider
      value={{
        t,
        tText,
        tStage,
        locale,
        effectiveLocale: effective,
        setLocale: save,
      }}
    >
      {props.children}
    </Ctx.Provider>
  );
}

/** 组件内使用的 hook。未在 Provider 内调用时抛异常。 */
export function useI18n(): I18nCtx {
  const c = useContext(Ctx);
  if (!c) {
    throw new Error("useI18n 必须在 <I18nProvider> 内使用");
  }
  return c;
}

/** 纯函数式访问（比如 notification 回调外用） */
export function getT(): (
  key: string,
  params?: Record<string, string | number>,
) => string {
  const effective = resolve(loadStored());
  return (key, params) => interpolate(get(DICTS[effective], key), params);
}
