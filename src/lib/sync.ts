/* ============================================================
   设置同步 —— localStorage ⇄ SQLite
   ------------------------------------------------------------
   分工（design.md §9.2）：
   * **localStorage 是首屏权威**。`index.html` 里的防白闪内联脚本要在
     React 挂载前决定 <html> 的 class，那一刻拿不到后端。
   * **SQLite 是真源**。它跟着数据目录一起备份/迁移，用户换机器也能带走。
   启动时把 SQLite 的值反向同步进 localStorage，之后用户每次改动再写回 SQLite。

   为什么不做「启动时阻塞等 SQLite」：读库要走 IPC + 建池，快的话几十毫秒，
   慢的话几百毫秒 —— 为了一个主题把首屏按住不划算。先按 localStorage 渲染，
   读到之后再纠偏；只有「localStorage 与库不一致」时才会看到一次跳变。
   ============================================================ */

import * as ipc from "./ipc";
import {
  getStoredTheme,
  subscribeTheme,
  syncThemeFromSettings,
  type ThemeMode,
} from "./theme";

/**
 * 设置键名。
 *
 * ⚠️ 与 `src-tauri/src/commands/settings.rs` 的 `keys` 模块同级；
 *    那个模块只做文档化，不做白名单，所以这里是唯一的拼写来源。
 */
export const SETTING_KEYS = {
  theme: "theme",
  locale: "locale",
  activeEnvironmentId: "activeEnvironmentId",
  trashRetentionDays: "trashRetentionDays",
} as const;

export interface BootSettings {
  /** 全部设置项，key → value */
  values: Map<string, string>;
  /** 解析出来的主题；库里没有或值非法时为 null */
  theme: ThemeMode | null;
}

function toThemeMode(raw: string | undefined): ThemeMode | null {
  return raw === "light" || raw === "dark" || raw === "system" ? raw : null;
}

/**
 * 拉一次全部设置。
 *
 * **失败不抛出**：设置读不到（浏览器里预览、后端未启动、库被锁）不该拦住应用启动，
 * 退化的结果只是「用 localStorage 里的值继续跑」。
 */
export async function loadSettings(): Promise<BootSettings> {
  try {
    const rows = await ipc.settings.all();
    const values = new Map(rows.map((r) => [r.key, r.value]));
    return { values, theme: toThemeMode(values.get(SETTING_KEYS.theme)) };
  } catch (e) {
    console.warn(
      "[reqly] 读取设置失败，回退到 localStorage：",
      ipc.toIpcError(e).debug,
    );
    return { values: new Map(), theme: null };
  }
}

/**
 * 订阅主题变化并写回 SQLite。返回取消订阅的函数。
 *
 * @param initial 启动时的已知值。订阅是「变更驱动」的，所以必须先把当前值
 *                记下来，否则启动后的第一次 emit（来自系统主题变化）会被
 *                当成用户改动而多写一次库。
 */
export function startThemePersistence(
  initial: ThemeMode = getStoredTheme(),
): () => void {
  let last: ThemeMode = initial;

  return subscribeTheme(() => {
    const now = getStoredTheme();
    if (now === last) return;
    last = now;

    // 不 await：写库失败也只在控制台记一笔，绝不阻塞 UI 或弹错
    void ipc.settings.set(SETTING_KEYS.theme, now).catch((e) => {
      console.warn("[reqly] 主题写入失败：", ipc.toIpcError(e).debug);
    });
  });
}

/**
 * 启动序列里跟设置有关的部分：读库 → 纠偏 localStorage/DOM → 开始持久化。
 *
 * @returns 清理函数（React effect 里直接 return 它）
 */
export async function bootstrapSettings(): Promise<() => void> {
  const { theme } = await loadSettings();
  if (theme) syncThemeFromSettings(theme);
  return startThemePersistence(theme ?? getStoredTheme());
}
