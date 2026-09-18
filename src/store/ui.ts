import { create } from "zustand";

/** 左侧栏当前展示的板块 —— 与 ActivityRail 上的图标一一对应 */
export type SidebarSection =
  | "collections"
  | "history"
  | "environments"
  | "loadtest"
  | "trash"
  | "settings";

/** 侧栏顶部的「新建」按钮与集合树的右键菜单共用同一个对话框 */
export interface PendingCreate {
  kind: "folder" | "request";
  /** `null` = 建在根级 */
  parentId: number | null;
}

/**
 * 环境板块要弹的对话框。
 *
 * 与 [`PendingCreate`] 同一个理由：触发它的有两处 —— 侧栏顶部的「新建环境」
 * 按钮（住在 `Sidebar` 的标题区）和环境列表里的行（住在 `EnvManager`）。
 * 两棵子树之间隔着一层，提状态要么穿透 props、要么把列表整个包一层。
 * 共享字段更直接。
 */
export type EnvDialogTarget =
  /** 新建（还不存在 id） */
  | { kind: "new" }
  /** 编辑某个已存在的环境 */
  | { kind: "edit"; id: number };

export interface UiState {
  section: SidebarSection;
  /** 侧栏是否折叠（折叠后只剩 ActivityRail） */
  sidebarCollapsed: boolean;
  /** 集合树里当前选中的请求 id */
  activeRequestId: number | null;

  /**
   * 待创建的节点。
   *
   * 放在这里而不是集合树的局部 state：触发它的一共有两处 ——
   * 侧栏顶部的「新建」按钮和树的右键菜单。放 local state 就得把
   * 事件从 Sidebar 穿透到 CollectionTree，或者反过来把树 state 提上去，
   * 两条路都比一个共享字段麻烦。
   */
  pendingCreate: PendingCreate | null;

  /**
   * 集合树的筛选关键字。
   *
   * 放在 store 而不是 `CollectionTree` 的局部 state：输入框住在
   * `Sidebar` 的标题区，树住在下面的滚动区 —— 两棵子树之间隔着一层，
   * 提状态要么穿透 props、要么把树整个包一层。共享字段更直接。
   */
  filter: string;

  /** 环境板块的对话框；`null` = 不弹 */
  envDialog: EnvDialogTarget | null;

  /**
   * 导入对话框是否打开。
   *
   * 与 `envDialog` 同一个理由：触发它的按钮住在 `Sidebar` 的标题区，
   * 而对话框本体挂在板块内容区（这样它不会随板块切换被卸载）。
   */
  importOpen: boolean;

  /**
   * 命令面板是否打开。
   *
   * 放在 store 而不是 `App` 的局部 state：打开它的入口有两处 ——
   * 全局快捷键（`App` 的 keydown）与命令面板自己执行完命令后的关闭动作，
   * 而面板本体挂在 `App` 的子树里。放 store 比一路传 props 直接。
   */
  paletteOpen: boolean;

  /**
   * 「保存到分类」对话框是否打开。
   *
   * 与 `importOpen` 同一个理由：对话框本体只能挂一处（挂在 `RequestPane`，
   * 这样它跟着当前请求走），而打开它的入口在请求标签栏上。
   * （007 之前授权页那条死路提示旁边还有第二个入口，那个页面已经没了。）
   */
  saveDraftOpen: boolean;

  setSection: (section: SidebarSection) => void;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  setActiveRequestId: (id: number | null) => void;
  setFilter: (filter: string) => void;
  openCreateDialog: (kind: PendingCreate["kind"], parentId: number | null) => void;
  closeCreateDialog: () => void;
  openEnvDialog: (target: EnvDialogTarget) => void;
  closeEnvDialog: () => void;
  openImport: () => void;
  closeImport: () => void;
  openPalette: () => void;
  closePalette: () => void;
  togglePalette: () => void;
  openSaveDraft: () => void;
  closeSaveDraft: () => void;
}

/**
 * 纯 UI 状态。业务数据（集合、历史、环境）走各自的 store，
 * 由后端命令驱动 —— 这里只放「跟界面长相有关」的东西。
 */
export const useUiStore = create<UiState>((set) => ({
  section: "collections",
  sidebarCollapsed: false,
  activeRequestId: null,
  pendingCreate: null,
  filter: "",
  envDialog: null,
  importOpen: false,
  saveDraftOpen: false,
  paletteOpen: false,

  setSection: (section) => set({ section }),
  toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  setSidebarCollapsed: (sidebarCollapsed) => set({ sidebarCollapsed }),
  setActiveRequestId: (activeRequestId) => set({ activeRequestId }),
  setFilter: (filter) => set({ filter }),
  openCreateDialog: (kind, parentId) => set({ pendingCreate: { kind, parentId } }),
  closeCreateDialog: () => set({ pendingCreate: null }),
  openEnvDialog: (envDialog) => set({ envDialog }),
  closeEnvDialog: () => set({ envDialog: null }),
  openImport: () => set({ importOpen: true }),
  closeImport: () => set({ importOpen: false }),
  openPalette: () => set({ paletteOpen: true }),
  closePalette: () => set({ paletteOpen: false }),
  togglePalette: () => set((s) => ({ paletteOpen: !s.paletteOpen })),
  openSaveDraft: () => set({ saveDraftOpen: true }),
  closeSaveDraft: () => set({ saveDraftOpen: false }),
}));
