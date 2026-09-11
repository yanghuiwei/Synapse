import { create } from "zustand";

/** 左侧栏当前展示的板块 —— 与 ActivityRail 上的图标一一对应 */
export type SidebarSection =
  | "collections"
  | "history"
  | "environments"
  | "loadtest"
  | "trash"
  | "settings";

export interface UiState {
  section: SidebarSection;
  /** 侧栏是否折叠（折叠后只剩 ActivityRail） */
  sidebarCollapsed: boolean;
  /** 集合树里当前选中的请求 id */
  activeRequestId: number | null;

  setSection: (section: SidebarSection) => void;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  setActiveRequestId: (id: number | null) => void;
}

/**
 * 纯 UI 状态。业务数据（集合、历史、环境）走各自的 store，
 * 由后端命令驱动 —— 这里只放「跟界面长相有关」的东西。
 */
export const useUiStore = create<UiState>((set) => ({
  section: "collections",
  sidebarCollapsed: false,
  activeRequestId: null,

  setSection: (section) => set({ section }),
  toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  setSidebarCollapsed: (sidebarCollapsed) => set({ sidebarCollapsed }),
  setActiveRequestId: (activeRequestId) => set({ activeRequestId }),
}));
