/* ============================================================
   极简 i18n —— 只做中文，但把口子留好
   ------------------------------------------------------------
   为什么不用 react-i18next？
   这一版只有一种语言，引入整套 i18n 框架（复数规则、命名空间、
   语言探测、后端加载）属于纯负担。这里约 100 行搞定，
   以后加语言只需往 DICTIONARIES 里加一个文件。

   为什么 t() 是同步的？
   文案全部打包进 bundle（几百条，几十 KB），没有异步加载，
   所以不存在「首屏闪 key」的问题。
   ============================================================ */

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { zhCN, type MessageKey } from "./zh-CN";

/** 目前只有中文；加语言时这里加一项，并补一个同名 .ts 字典 */
export const DICTIONARIES = {
  "zh-CN": zhCN,
} as const;

export type Locale = keyof typeof DICTIONARIES;
export const DEFAULT_LOCALE: Locale = "zh-CN";
export const LOCALE_STORAGE_KEY = "reqly.locale";

/** 各语言在 UI 上的自称 */
export const LOCALE_LABELS: Record<Locale, string> = {
  "zh-CN": "简体中文",
};

export type TranslateParams = Record<string, string | number>;

/** 把 {name} 换成实际值；缺失的键保持原样，方便一眼看出漏了哪个参数 */
export function interpolate(
  template: string,
  params?: TranslateParams,
): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (raw, name: string) => {
    const v = params[name];
    return v === undefined ? raw : String(v);
  });
}

function isLocale(v: unknown): v is Locale {
  return typeof v === "string" && v in DICTIONARIES;
}

export function getStoredLocale(): Locale {
  if (typeof localStorage === "undefined") return DEFAULT_LOCALE;
  const raw = localStorage.getItem(LOCALE_STORAGE_KEY);
  return isLocale(raw) ? raw : DEFAULT_LOCALE;
}

export type TranslateFn = (key: MessageKey, params?: TranslateParams) => string;

export interface I18nContextValue {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: TranslateFn;
}

const I18nContext = createContext<I18nContextValue | null>(null);

export interface I18nProviderProps {
  children: ReactNode;
  /** 指定初始语言（比如从后端设置里读到的值），不传则读 localStorage */
  locale?: Locale;
}

export function I18nProvider({ children, locale: initial }: I18nProviderProps) {
  const [locale, setLocaleState] = useState<Locale>(
    () => initial ?? getStoredLocale(),
  );

  // 外部（设置页 / 后端设置加载完成）改了语言时同步进来
  useEffect(() => {
    if (initial && initial !== locale) setLocaleState(initial);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initial]);

  const setLocale = useCallback((next: Locale) => {
    setLocaleState(next);
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(LOCALE_STORAGE_KEY, next);
    }
    if (typeof document !== "undefined") {
      document.documentElement.lang = next;
    }
  }, []);

  const t = useCallback<TranslateFn>(
    (key, params) => {
      const dict = DICTIONARIES[locale] ?? zhCN;
      // 找不到就退回中文；再找不到就把 key 原样显示（开发期一眼可见）
      const template =
        (dict as Record<string, string | undefined>)[key] ??
        (zhCN as Record<string, string | undefined>)[key] ??
        key;
      return interpolate(template, params);
    },
    [locale],
  );

  const value = useMemo<I18nContextValue>(
    () => ({ locale, setLocale, t }),
    [locale, setLocale, t],
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

/** 取翻译函数与语言状态。必须在 <I18nProvider> 内使用 */
export function useI18n(): I18nContextValue {
  const ctx = useContext(I18nContext);
  if (!ctx) {
    throw new Error("useI18n() 必须在 <I18nProvider> 内使用");
  }
  return ctx;
}

/** 只要 t 时的快捷方式 */
export function useT(): TranslateFn {
  return useI18n().t;
}

export { zhCN };
export type { MessageKey };
