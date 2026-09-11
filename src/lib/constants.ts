/**
 * 全局常量。
 *
 * TODO(P1): APP_VERSION 目前与 package.json / tauri.conf.json 手工对齐。
 *  接入 Tauri 之后改用 `getVersion()`（@tauri-apps/api/app），
 *  以 tauri.conf.json 为唯一来源，消掉这处重复。
 */
export const APP_VERSION = "0.1.0";

/** 布局持久化键前缀（SplitPane 用），与 localStorage 里其他键区分开 */
export const LAYOUT_STORAGE_PREFIX = "reqly.layout.";

export interface ShortcutDef {
  ctrlKey?: boolean;
  shiftKey?: boolean;
  /** 用 event.code 而不是 event.key，避免中文输入法/键盘布局差异 */
  code: string;
}

/** 全局快捷键 */
export const SHORTCUTS = {
  /** 在 light / dark 之间切换 */
  toggleTheme: { ctrlKey: true, shiftKey: true, code: "KeyT" },
  /** 新建请求 */
  newRequest: { ctrlKey: true, code: "KeyN" },
  /** 聚焦搜索框 */
  focusSearch: { ctrlKey: true, code: "KeyK" },
  /** 发送当前请求 */
  send: { ctrlKey: true, code: "Enter" },
} as const satisfies Record<string, ShortcutDef>;

export type ShortcutName = keyof typeof SHORTCUTS;

/** 判断一次键盘事件是否命中某个快捷键（四个修饰键都必须精确匹配） */
export function matchShortcut(
  event: KeyboardEvent,
  name: ShortcutName,
): boolean {
  const s: ShortcutDef = SHORTCUTS[name];
  return (
    event.ctrlKey === (s.ctrlKey ?? false) &&
    event.shiftKey === (s.shiftKey ?? false) &&
    !event.altKey &&
    !event.metaKey &&
    event.code === s.code
  );
}
