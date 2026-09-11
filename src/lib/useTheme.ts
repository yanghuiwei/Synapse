import { useCallback, useSyncExternalStore } from "react";
import {
  getResolvedTheme,
  getStoredTheme,
  setTheme,
  subscribeTheme,
  toggleTheme,
  type ResolvedTheme,
  type ThemeMode,
} from "./theme";

export interface UseThemeResult {
  /** 用户选择的模式（可能是 system） */
  mode: ThemeMode;
  /** 实际渲染出来的主题（system 已解析） */
  resolved: ResolvedTheme;
  setMode: (mode: ThemeMode) => void;
  toggle: () => void;
}

/**
 * 订阅主题状态。
 * getSnapshot 必须返回「稳定值」，否则 useSyncExternalStore 会死循环 ——
 * 所以这里返回的是字符串（基本类型），不是对象。
 */
export function useTheme(): UseThemeResult {
  const mode = useSyncExternalStore(subscribeTheme, getStoredTheme, () => "system" as ThemeMode);
  const resolved = useSyncExternalStore(
    subscribeTheme,
    getResolvedTheme,
    () => "light" as ResolvedTheme,
  );

  const setMode = useCallback((next: ThemeMode) => setTheme(next), []);
  const toggle = useCallback(() => {
    toggleTheme();
  }, []);

  return { mode, resolved, setMode, toggle };
}
