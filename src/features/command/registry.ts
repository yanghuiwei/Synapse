/* ============================================================
   命令面板的动作注册表
   ------------------------------------------------------------
   这里只放**纯数据 + 动作**，不放任何 React 相关的东西：
   面板负责翻译标题、模糊匹配、键盘导航，注册表只回答
   「有哪些命令」和「这条命令做什么」。

   为什么动作直接读 store 的 `getState()` 而不是走 hooks：
   注册表是模块级常量，在组件外层被求值（`PALETTE_COMMANDS` 一建就定型），
   那一刻没有组件实例。`getState()` 拿到的是同一个单例，
   执行时读到的永远是最新值。
   ============================================================ */

import {
  Clock,
  FolderPlus,
  Gauge,
  Globe,
  Import,
  Layers,
  Palette,
  PanelLeftOpen,
  Plus,
  Send,
  Settings,
  SquareX,
  SunMoon,
  Trash,
  Wrench,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

import type { ShortcutName } from "@/lib/constants";
import type { MessageKey } from "@/lib/i18n";
import { ACCENTS, ACCENT_ORDER, setAccent, toggleTheme, type Accent } from "@/lib/theme";
import { useRequestStore } from "@/store/request";
import { useUiStore, type SidebarSection } from "@/store/ui";

/**
 * 命令面板里每套配色的标题键。
 *
 * 为什么单独一张表，而不是 `ACCENTS[accent].labelKey`：
 * 色板上显示裸色名（「靛青」）是对的 —— 它就一个色块，旁边没有别的信息；
 * 命令面板里孤零零一个「靛青」，用户看不出这是要切配色还是改别的东西。
 * 同一个概念在两个语境下需要两种措辞，那就分开写，别在渲染时凑。
 *
 * ⚠️ 新增配色时这里也要加一行 —— 漏了的话 TS 会在
 * `Record<Accent, MessageKey>` 上报错（这是刻意的，见下）。
 */
const PALETTE_ACCENT_TITLES: Record<Accent, MessageKey> = {
  indigo: "palette.accentIndigo",
  azure: "palette.accentAzure",
  violet: "palette.accentViolet",
  clay: "palette.accentClay",
  teal: "palette.accentTeal",
};

/** 命令归组。只影响视觉分组，不影响排序 */
export type PaletteGroup = "go" | "action" | "app";

export interface PaletteCommand {
  id: string;
  titleKey: MessageKey;
  /**
   * 额外搜索词（英文名 / 别名）。
   *
   * 不显示，纯粹让「搜 `dark` 也能找到切换主题」成立 ——
   * 中文界面下用户照样会按英文键名去搜。
   */
  keywords?: readonly string[];
  group: PaletteGroup;
  icon: LucideIcon;
  /**
   * 右侧那列快捷键提示。
   *
   * ⚠️ 存的是**快捷键名**而不是 `"Ctrl+K"` 字符串：手写字符串的话，
   * `SHORTCUTS` 改了这里不会跟着变，也不报错。
   */
  shortcut?: ShortcutName;
  /**
   * 返回 `false` 时这条命令**根本不出现**在结果里。
   *
   * 比「列出来但点了没反应」好：例如没有正在跑的请求时，
   * 「取消当前请求」出现只会让人怀疑是不是坏了。
   */
  enabled?: () => boolean;
  run: () => void;
}

/**
 * 切板块，并保证侧栏可见。
 *
 * 侧栏折叠时切板块**什么都不会发生**（内容区在侧栏里），
 * 所以顺手展开 —— 与 `App.tsx` 里 Ctrl+1/2/3 的处理保持一致。
 */
export function goToSection(section: SidebarSection): void {
  const ui = useUiStore.getState();
  ui.setSection(section);
  if (ui.sidebarCollapsed) ui.setSidebarCollapsed(false);
}

/**
 * 打开「新建分类 / 新建请求」对话框。
 *
 * 对话框本体挂在集合树里（`pendingCreate` → `CreateDialog`），
 * 所以必须先切回集合板块 —— 否则在历史 / 回收站板块下按了毫无反应，
 * 而且不报错。
 */
export function openCreate(kind: "folder" | "request"): void {
  goToSection("collections");
  useUiStore.getState().openCreateDialog(kind, null);
}

/** 面板里列出的全部命令。顺序 = 空查询时的展示顺序 */
export const PALETTE_COMMANDS: readonly PaletteCommand[] = [
  // ---- 前往 ----
  {
    id: "go.collections",
    titleKey: "palette.goCollections",
    group: "go",
    icon: Layers,
    shortcut: "sectionCollections",
    run: () => goToSection("collections"),
  },
  {
    id: "go.history",
    titleKey: "palette.goHistory",
    group: "go",
    icon: Clock,
    shortcut: "sectionHistory",
    run: () => goToSection("history"),
  },
  {
    id: "go.environments",
    titleKey: "palette.goEnvironments",
    keywords: ["env", "变量"],
    group: "go",
    icon: Globe,
    run: () => goToSection("environments"),
  },
  {
    id: "go.loadtest",
    titleKey: "palette.goLoadtest",
    keywords: ["loadtest", "bench", "并发"],
    group: "go",
    icon: Gauge,
    run: () => goToSection("loadtest"),
  },
  {
    id: "go.tools",
    titleKey: "palette.goTools",
    keywords: ["tools", "jwt", "token", "decode", "工具", "解析"],
    group: "go",
    icon: Wrench,
    // 「前往工具」顺带把 Token 解析打开 —— 切到板块却不打开那个工具，
    // 中间会是一片空态，用户会以为这个功能没做
    run: () => {
      goToSection("tools");
      useUiStore.getState().setTokenToolOpen(true);
    },
  },
  {
    id: "go.trash",
    titleKey: "palette.goTrash",
    group: "go",
    icon: Trash,
    shortcut: "sectionTrash",
    run: () => goToSection("trash"),
  },
  {
    id: "go.settings",
    titleKey: "palette.goSettings",
    group: "go",
    icon: Settings,
    run: () => goToSection("settings"),
  },

  // ---- 命令 ----
  {
    id: "request.new",
    titleKey: "palette.newRequest",
    keywords: ["new", "create", "request"],
    group: "action",
    icon: Plus,
    shortcut: "newRequest",
    run: () => openCreate("request"),
  },
  {
    id: "folder.new",
    titleKey: "palette.newFolder",
    keywords: ["new", "create", "folder", "collection"],
    group: "action",
    icon: FolderPlus,
    run: () => openCreate("folder"),
  },
  {
    id: "request.send",
    titleKey: "palette.send",
    keywords: ["send", "run", "execute"],
    group: "action",
    icon: Send,
    shortcut: "send",
    // 地址还没填时发送没有意义，列出来只会让人怀疑点不动
    enabled: () => useRequestStore.getState().draft.url.trim() !== "",
    run: () => {
      void useRequestStore.getState().send();
    },
  },
  {
    id: "request.cancel",
    titleKey: "palette.cancel",
    keywords: ["cancel", "abort", "stop"],
    group: "action",
    icon: SquareX,
    enabled: () => useRequestStore.getState().runId !== null,
    run: () => {
      void useRequestStore.getState().cancel();
    },
  },
  {
    id: "data.import",
    titleKey: "palette.import",
    keywords: ["import", "postman", "apipost", "curl", "har"],
    group: "action",
    icon: Import,
    run: () => useUiStore.getState().openImport(),
  },

  // ---- 应用 ----
  {
    id: "app.toggleTheme",
    titleKey: "palette.toggleTheme",
    keywords: ["theme", "dark", "light", "外观"],
    group: "app",
    icon: SunMoon,
    shortcut: "toggleTheme",
    run: () => {
      toggleTheme();
    },
  },
  // ---- 配色 ----
  //
  // 五条命令而不是一条「切换配色」：配色是「选某一档」而不是「在两者间翻」，
  // 没有「下一个」这种东西。命令面板是键盘用户唯一能碰到配色的入口 ——
  // 色板按钮只能点，而 Ctrl+K 的持有者恰恰是最不想用鼠标的那批人。
  ...ACCENT_ORDER.map((accent) => ({
    id: `app.accent.${accent}`,
    // ⚠️ 不能直接用 `ACCENTS[accent].labelKey`（那是色板上的裸色名）。
    // 命令面板里孤零零一个「靛青」，用户看不出这是切配色还是改别的东西。
    titleKey: PALETTE_ACCENT_TITLES[accent],
    keywords: ["accent", "配色", "颜色", ACCENTS[accent].labelKey],
    group: "app" as const,
    icon: Palette,
    run: () => {
      setAccent(accent);
    },
  })),
  {
    id: "app.toggleSidebar",
    titleKey: "palette.toggleSidebar",
    keywords: ["sidebar", "panel", "侧边栏"],
    group: "app",
    icon: PanelLeftOpen,
    run: () => useUiStore.getState().toggleSidebar(),
  },
];
