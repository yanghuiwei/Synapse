import { useCallback, useSyncExternalStore } from "react";
import {
  DEFAULT_ACCENT,
  getResolvedTheme,
  getStoredAccent,
  getStoredTheme,
  setAccent,
  setTheme,
  subscribeTheme,
  toggleTheme,
  type Accent,
  type ResolvedTheme,
  type ThemeMode,
} from "./theme";

export interface UseThemeResult {
  /** 用户选择的模式（可能是 system） */
  mode: ThemeMode;
  /** 实际渲染出来的主题（system 已解析） */
  resolved: ResolvedTheme;
  /** 当前配色 */
  accent: Accent;
  setMode: (mode: ThemeMode) => void;
  setAccent: (accent: Accent) => void;
  toggle: () => void;
}

/**
 * 订阅主题状态。
 * getSnapshot 必须返回「稳定值」，否则 useSyncExternalStore 会死循环 ——
 * 所以这里返回的是字符串（基本类型），不是对象。
 *
 * accent 与 mode 共用 `subscribeTheme` 同一个订阅源：
 * 两者都是「改 <html> 上的某个属性」这一个动作（见 theme.ts 的 applyTheme），
 * 拆成两套订阅只会让人漏掉其中一套的退订 —— 漏退订的代价是
 * 组件卸载后闭包仍被持有，且每次 emit 都白跑一遍。
 */
export function useTheme(): UseThemeResult {
  const mode = useSyncExternalStore(subscribeTheme, getStoredTheme, () => "system" as ThemeMode);
  const resolved = useSyncExternalStore(
    subscribeTheme,
    getResolvedTheme,
    () => "light" as ResolvedTheme,
  );
  // 第三个参数是 server snapshot（本项目不 SSR，走不到），
  // 但仍要给合法值 —— 有 DOM 之前 getStoredAccent 拿不到 localStorage。
  // 写DEFAULT_ACCENT 而不是字面量：加配色时这里不该是需要改的地方
  const accent = useSyncExternalStore(
    subscribeTheme,
    getStoredAccent,
    () => DEFAULT_ACCENT,
  );

  const setMode = useCallback((next: ThemeMode) => setTheme(next), []);
  const chooseAccent = useCallback((next: Accent) => setAccent(next), []);
  const toggle = useCallback(() => {
    toggleTheme();
  }, []);

  return { mode, resolved, accent, setMode, setAccent: chooseAccent, toggle };
}