/* ============================================================
   主题：light / dark / system 三态 × 5 套配色（accent）
   ------------------------------------------------------------
   为什么用 localStorage 而不是直接读 SQLite 设置？
   因为 index.html 里的「防白闪」内联脚本必须在 React 挂载之前
   就决定 <html> 的 class —— 那一刻拿不到后端设置。
   所以 localStorage 是「首屏权威」，
   真正的用户设置后续由 SQLite 落盘，启动时反向同步过来。
   ============================================================ */

import type { MessageKey } from "./i18n";

export type ThemeMode = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

/**
 * 配色主题。
 *
 * ⚠️ 与 `ThemeMode` **正交**，不是第三种模式：
 * mode 管明暗（画布亮还是暗），accent 管色相（画布是什么调子）。
 * 两者叠加才是最终配色：`.dark[data-accent="violet"]`。
 * 写成 `dark-violet` 这类单值会退化成一维列表，
 * 「深色 × 5 种色调」得靠 10 个类名硬凑，改一处漏三处。
 *
 * 新增一档：CSS 里加一个 `[data-accent="xxx"]` 块（明暗各一份），
 * 这里加一个联合类型的成员，再往 `ACCENTS` 里加一行。**别的地方不用动。**
 */
export type Accent = "indigo" | "azure" | "violet" | "clay" | "teal";

export const THEME_STORAGE_KEY = "reqly.theme";
export const ACCENT_STORAGE_KEY = "reqly.accent";

export interface AccentDef {
  /** 色板预览色 = 该 accent 浅色模式的 `--brand`，两处必须一致 */
  swatch: string;
  /**
   * i18n 表的键。
   *
   * 收成 `MessageKey` 而不是 `string`：写成 string 的话，
   * 把 `labelKey` 拼错了（比如少个前缀）编译期一点错都不报，
   * 界面上那个色板的 tooltip 就直接显示 key 原文 ——
   * 而色板按钮没有文字，tooltip 是它唯一的可读标签。
   */
  labelKey: MessageKey;
}

/**
 * 配色清单。
 *
 * 渲染顺序另存 `ACCENT_ORDER`：UI 不该依赖对象键序 ——
 * `Object.keys(ACCENTS)` 此刻恰好等于声明顺序，但那是「恰好」，
 * 谁调一下字段顺序，色板就跟着变了。显式列一遍更省心。
 */
export const ACCENTS: Readonly<Record<Accent, AccentDef>> = {
  indigo: { swatch: "#184695", labelKey: "accent.indigo" },
  azure: { swatch: "#186195", labelKey: "accent.azure" },
  violet: { swatch: "#3e1895", labelKey: "accent.violet" },
  clay: { swatch: "#953a18", labelKey: "accent.clay" },
  teal: { swatch: "#176f5b", labelKey: "accent.teal" },
};

export const ACCENT_ORDER: readonly Accent[] = [
  "indigo",
  "azure",
  "violet",
  "clay",
  "teal",
];

/** 未设置时的默认配色 */
export const DEFAULT_ACCENT: Accent = "indigo";

const MODES: readonly ThemeMode[] = ["light", "dark", "system"];

function isThemeMode(v: unknown): v is ThemeMode {
  return typeof v === "string" && (MODES as readonly string[]).includes(v);
}

/**
 * 合法配色。
 *
 * ⚠️ 用 `Object.prototype.hasOwnProperty.call` 而不是 `Object.hasOwn`：
 * 后者要 ES2022 的 lib，而本项目的 tsconfig 还没抬到那里。
 * 为一个判断去改 tsconfig 的 target/lib是**全项目决策** ——
 * 抬高它可能连带改变别的类型检查行为，那属于另一件事，不该藏在一个补丁里。
 *
 * 也不能用 `v in ACCENTS`：`in` 会认到原型链上的成员
 * （`"toString" in ACCENTS` 为 true）。而 accent 的值直接来自 localStorage，
 * 一个畸形字符串就能把 `data-accent="toString"` 打到 DOM 上 ——
 * 它不匹配任何 CSS 块，界面静默退回默认色，**不报任何错**。
 */
function isAccent(v: unknown): v is Accent {
  return typeof v === "string" && Object.prototype.hasOwnProperty.call(ACCENTS, v);
}

/** 读取持久化的配色；非法值一律回退默认 */
export function getStoredAccent(): Accent {
  if (typeof localStorage === "undefined") return DEFAULT_ACCENT;
  const raw = localStorage.getItem(ACCENT_STORAGE_KEY);
  return isAccent(raw) ? raw : DEFAULT_ACCENT;
}

/** 系统当前偏好 */
export function getSystemTheme(): ResolvedTheme {
  if (typeof window === "undefined" || !window.matchMedia) return "light";
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

/** 读取持久化的模式，非法值一律回退到 system */
export function getStoredTheme(): ThemeMode {
  if (typeof localStorage === "undefined") return "system";
  const raw = localStorage.getItem(THEME_STORAGE_KEY);
  return isThemeMode(raw) ? raw : "system";
}

/** 把 mode 解析成实际要渲染的主题 */
export function resolveTheme(mode: ThemeMode): ResolvedTheme {
  return mode === "system" ? getSystemTheme() : mode;
}

/** 当前实际渲染的主题（含 system 解析） */
export function getResolvedTheme(): ResolvedTheme {
  return resolveTheme(getStoredTheme());
}

// ---------------------------------------------------------------
// 订阅：让 React 组件能通过 useSyncExternalStore 拿到最新值
// ---------------------------------------------------------------

type Listener = () => void;
const listeners = new Set<Listener>();

function emit(): void {
  for (const l of listeners) l();
}

export function subscribeTheme(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

// ---------------------------------------------------------------
// 应用到 DOM
// ---------------------------------------------------------------

let systemWatcherBound = false;

/**
 * 把配色写到 <html> 上：`data-accent` 属性。
 *
 * **每次 applyTheme 都要重打一遍**，不能只在启动时设一次 ——
 * 因为 `data-accent` 与 `.dark` 是两条独立的选择器分支，
 * 谁也不覆盖谁；但如果只有启动路径设它，那么 `syncThemeFromSettings`
 * 纠正过一次 DOM 之后…… 其实那也还是设过的。真正会丢的情况是
 * 首屏防白闪脚本只打了 accent 没打 class（或者反过来）。
 *
 * 无论如何，把「两个维度在同一个函数里落盘」是唯一不需要推理的写法：
 * 读 DOM 状态的人只需要看一个函数，不必回忆谁负责哪一半。
 */
function applyAccentToDom(accent: Accent): void {
  document.documentElement.dataset.accent = accent;
}

/**
 * 把主题写到 <html> 上。
 * 加 .dark 即可，颜色由 theme.css 的变量接管；同时打 data-accent 定色相。
 * 也写入 color-scheme，让原生控件（滚动条、input）也跟着变。
 *
 * @param animate 是否允许 150ms 颜色过渡。首屏/系统变化时不要，
 *                只有用户手动切换时才要 —— 否则开屏会看到闪一下。
 */
export function applyTheme(mode: ThemeMode, animate = false): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const resolved = resolveTheme(mode);

  if (animate) {
    root.classList.add("theme-transition");
    window.setTimeout(() => root.classList.remove("theme-transition"), 200);
  }

  root.classList.toggle("dark", resolved === "dark");
  root.style.colorScheme = resolved;
  root.dataset.themeMode = mode;
  applyAccentToDom(getStoredAccent());
}

/**
 * 手动切换配色：写盘 + 应用 + 通知订阅者。
 *
 * ⚠️ 走的是 applyTheme 而不是 applyAccentToDom：切配色必须**重新落一次
 * mode**，因为过渡类（`theme-transition`）只在 applyTheme 里挂，
 * 只改 data-accent 的话界面是硬切，颜色跳变没有 150ms 的过渡。
 */
export function setAccent(accent: Accent, animate = true): void {
  if (typeof localStorage !== "undefined") {
    localStorage.setItem(ACCENT_STORAGE_KEY, accent);
  }
  applyTheme(getStoredTheme(), animate);
  emit();
}

/** 手动切换主题：写盘 + 应用 + 通知订阅者 */
export function setTheme(mode: ThemeMode, animate = true): void {
  if (typeof localStorage !== "undefined") {
    localStorage.setItem(THEME_STORAGE_KEY, mode);
  }
  applyTheme(mode, animate);
  emit();
}

/** 在 light → dark 之间来回切（给快捷键用的） */
export function toggleTheme(): ThemeMode {
  const next: ThemeMode = getResolvedTheme() === "dark" ? "light" : "dark";
  setTheme(next);
  return next;
}

/**
 * 监听系统主题变化。仅当 mode === 'system' 时才真正动 DOM。
 * 幂等：重复调用不会重复绑定。
 */
export function watchSystemTheme(): void {
  if (systemWatcherBound || typeof window === "undefined") return;
  if (!window.matchMedia) return;
  systemWatcherBound = true;

  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  const onChange = () => {
    if (getStoredTheme() !== "system") return;
    applyTheme("system", false);
    emit();
  };
  mq.addEventListener("change", onChange);
}

/**
 * 用后端（SQLite 设置）里的值覆盖 localStorage。
 * 应用启动、设置读取完成后调用一次即可。
 * @returns 是否有变化
 */
export function syncThemeFromSettings(mode: ThemeMode): boolean {
  if (getStoredTheme() === mode) return false;
  setTheme(mode, false);
  return true;
}

/**
 * 用后端设置里的配色覆盖 localStorage。
 *
 * ⚠️ 老库里**没有** `accent` 这个键（加 accent 之前只有 `theme`）。
 * 那种情况下 `syncAccentFromSettings(null)` 不该把用户当前选的配色抹掉 ——
 * 所以调用方要先判 null 再调，这里只处理「值确实存在」的情形。
 *
 * @returns 是否有变化
 */
export function syncAccentFromSettings(accent: Accent): boolean {
  if (getStoredAccent() === accent) return false;
  setAccent(accent, false);
  return true;
}
