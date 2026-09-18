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
  PanelLeftOpen,
  Plus,
  Send,
  Settings,
  SquareX,
  SunMoon,
  Trash,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

import type { ShortcutName } from "@/lib/constants";
import type { MessageKey } from "@/lib/i18n";
import { toggleTheme } from "@/lib/theme";
import { useRequestStore } from "@/store/request";
import { useUiStore, type SidebarSection } from "@/store/ui";

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
  {
    id: "app.toggleSidebar",
    titleKey: "palette.toggleSidebar",
    keywords: ["sidebar", "panel", "侧边栏"],
    group: "app",
    icon: PanelLeftOpen,
    run: () => useUiStore.getState().toggleSidebar(),
  },
];
