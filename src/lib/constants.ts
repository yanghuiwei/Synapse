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
  /**
   * 打开命令面板。
   *
   * P8 之前这个键是「聚焦侧栏搜索框」—— 那个搜索框只能搜到**当前板块**里的东西，
   * 而且想「跳到某个接口」得先切回集合板块。命令面板把「跳转任意请求」与
   * 「执行任意命令」合成一个入口（`design.md` §6.3）。
   */
  commandPalette: { ctrlKey: true, code: "KeyK" },
  /** 发送当前请求 */
  send: { ctrlKey: true, code: "Enter" },
  /** 切到「集合」。与 Ctrl+2 / Ctrl+3 一起覆盖最常用的三个板块 */
  sectionCollections: { ctrlKey: true, code: "Digit1" },
  sectionHistory: { ctrlKey: true, code: "Digit2" },
  sectionTrash: { ctrlKey: true, code: "Digit3" },
} as const satisfies Record<string, ShortcutDef>;

/**
 * 数字键 → 侧栏板块。
 *
 * 用 `event.code`（`Digit1`）而不是 `event.key`（"1"）：中文输入法下
 * `event.key` 在部分布局里会变成别的字符，而 `code` 反映的是**物理按键**，
 * 与键盘布局无关。
 */
export const SECTION_SHORTCUTS = [
  { name: "sectionCollections", section: "collections" },
  { name: "sectionHistory", section: "history" },
  { name: "sectionTrash", section: "trash" },
] as const;

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

/**
 * 把快捷键渲染成人看的写法（命令面板右侧那列提示）。
 *
 * 刻意**不**在提示里手写 `"Ctrl+K"` 字面量：那样改了 `SHORTCUTS` 之后，
 * 面板上会继续显示旧键，而且不报任何错。
 */
export function formatShortcut(name: ShortcutName): string {
  const s: ShortcutDef = SHORTCUTS[name];
  const parts: string[] = [];
  if (s.ctrlKey) parts.push("Ctrl");
  if (s.shiftKey) parts.push("Shift");
  // `code` 是物理键名（`KeyK` / `Digit1` / `Enter`），抹掉前缀就是键帽上的字
  parts.push(s.code.replace(/^Key|^Digit/, ""));
  return parts.join("+");
}
