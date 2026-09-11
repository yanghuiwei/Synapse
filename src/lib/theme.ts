/* ============================================================
   主题：light / dark / system 三态
   ------------------------------------------------------------
   为什么用 localStorage 而不是直接读 SQLite 设置？
   因为 index.html 里的「防白闪」内联脚本必须在 React 挂载之前
   就决定 <html> 的 class —— 那一刻拿不到后端设置。
   所以 localStorage 是「首屏权威」，
   真正的用户设置后续由 SQLite 落盘，启动时反向同步过来。
   ============================================================ */

export type ThemeMode = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

/** localStorage 键名：index.html 内联脚本也用这个字面量，改这里要同步改那边 */
export const THEME_STORAGE_KEY = "reqly.theme";

const MODES: readonly ThemeMode[] = ["light", "dark", "system"];

function isThemeMode(v: unknown): v is ThemeMode {
  return typeof v === "string" && (MODES as readonly string[]).includes(v);
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
 * 把主题写到 <html> 上。
 * 加 .dark 即可，颜色由 theme.css 的变量接管。
 * 同时写入 color-scheme，让原生控件（滚动条、input）也跟着变。
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
