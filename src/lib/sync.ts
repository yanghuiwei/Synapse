/* ============================================================
   设置同步 —— localStorage ⇄ SQLite
   ------------------------------------------------------------
   管两个设置项：主题明暗（theme）与配色（accent）。
   两者**分开存两个键**，不合成一个字符串：
   合成 `"violet-dark"` 的写法看起来省一个键，实际把两个正交维度
   重新耦合了 —— 以后想「换配色但保持跟随系统」就得加
   `"violet-system"`、`"violet-light"`… 组合开始爆炸，
   而拆成两个键，两个维度各写各的，永远是 5×3 种可能。
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
  ACCENTS,
  getStoredAccent,
  getStoredTheme,
  subscribeTheme,
  syncAccentFromSettings,
  syncThemeFromSettings,
  type Accent,
  type ThemeMode,
} from "./theme";

/**
 * 设置键名。
 *
 * ⚠️ 与 `src-tauri/src/commands/settings.rs` 的 `keys` 模块同级；
 *    那个模块只做文档化，不做白名单，所以这里是唯一的拼写来源。
 *
 * ⚠️ **「当前环境」不在这里**，尽管这里曾经有过一个 `activeEnvironmentId`。
 *    它的唯一真源是 `environment.is_active` 这一列（P4 收敛，见
 *    `src-tauri/src/commands/env.rs` 顶部的说明）。两处都表达同一件事
 *    的结果是「界面在这一处改、发送路径读那一处」，然后出现
 *    「切换了环境但请求还是老变量」这种从代码上看不出因果的 bug。
 */
export const SETTING_KEYS = {
  theme: "theme",
  accent: "accent",
  locale: "locale",
  trashRetentionDays: "trashRetentionDays",
} as const;

export interface BootSettings {
  /** 全部设置项，key → value */
  values: Map<string, string>;
  /** 解析出来的主题；库里没有或值非法时为 null */
  theme: ThemeMode | null;
  /** 解析出来的配色；**老库里没有这个键**，此时为 null */
  accent: Accent | null;
}

function toThemeMode(raw: string | undefined): ThemeMode | null {
  return raw === "light" || raw === "dark" || raw === "system" ? raw : null;
}

/**
 * 合法配色。
 *
 * ⚠️ 不能复用 `theme.ts` 里的 `isAccent` —— 那个没导出。
 * 这里重新判一次而不是导出去，是因为两边的**失败语义不同**：
 * theme.ts 读到非法值要回退默认值继续跑，这里读到非法值必须报 null
 * （「库里存了个认不出的配色」不等于「用户没选过配色」，
 * 后者该保留 localStorage 里的值，前者该忽略库里的值）。
 *
 * 用 `hasOwnProperty.call` 而非 `Object.hasOwn`：见 theme.ts 里同名函数的
 * 注释 —— 本项目 tsconfig 的 lib 还没抬到 ES2022。
 */
function toAccent(raw: string | undefined): Accent | null {
  return raw !== undefined && Object.prototype.hasOwnProperty.call(ACCENTS, raw)
    ? (raw as Accent)
    : null;
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
    return {
      values,
      theme: toThemeMode(values.get(SETTING_KEYS.theme)),
      accent: toAccent(values.get(SETTING_KEYS.accent)),
    };
  } catch (e) {
    console.warn(
      "[reqly] 读取设置失败，回退到 localStorage：",
      ipc.toIpcError(e).debug,
    );
    return { values: new Map(), theme: null, accent: null };
  }
}

/**
 * 订阅主题变化并写回 SQLite。返回取消订阅的函数。
 *
 * @param initialMode 启动时的已知模式
 * @param initialAccent 启动时的已知配色。老库里没有 accent 键时传 undefined
 *
 * 「变更驱动」的订阅必须先把当前值记下来，否则启动后的第一次 emit
 * （来自系统主题变化）会被当成用户改动而多写一次库。
 *
 * ⚠️ mode 与 accent **各比各的**：两个维度可能只动了一个
 * （切配色不改明暗、反之亦然）。写成 `if (mode变了 || accent变了) 然后两个都写`，
 * 会在只切配色时把 theme 也重写一遍—— 值没变的 IPC 往返，
 * 白跑一趟还多一个竞态窗口。
 */
export function startThemePersistence(
  initialMode: ThemeMode = getStoredTheme(),
  initialAccent?: Accent,
): () => void {
  let lastMode: ThemeMode = initialMode;
  let lastAccent: Accent | undefined = initialAccent;

  return subscribeTheme(() => {
    const nowMode = getStoredTheme();
    const nowAccent = getStoredAccent();

    if (nowMode !== lastMode) {
      lastMode = nowMode;
      // 不 await：写库失败也只在控制台记一笔，绝不阻塞 UI 或弹错
      void ipc.settings.set(SETTING_KEYS.theme, nowMode).catch((e) => {
        console.warn("[reqly] 主题写入失败：", ipc.toIpcError(e).debug);
      });
    }

    if (nowAccent !== lastAccent) {
      lastAccent = nowAccent;
      void ipc.settings.set(SETTING_KEYS.accent, nowAccent).catch((e) => {
        console.warn("[reqly] 配色写入失败：", ipc.toIpcError(e).debug);
      });
    }
  });
}

/**
 * 启动序列里跟设置有关的部分：读库 → 纠偏 localStorage/DOM → 开始持久化。
 *
 * @returns 清理函数（React effect 里直接 return 它）
 */
export async function bootstrapSettings(): Promise<() => void> {
  const { theme, accent } = await loadSettings();
  if (theme) syncThemeFromSettings(theme);
  // accent 为 null = 老库（加配色之前建的）或值非法，两种情况都**不动** localStorage。
  // 当成「没这个键、用默认值」去覆盖，会把用户浏览器里存的配色抹成默认——
  // 而老库用户「根本还没选过配色」，被抹了也无从察觉。
  if (accent) syncAccentFromSettings(accent);
  return startThemePersistence(theme ?? getStoredTheme(), accent ?? undefined);
}
