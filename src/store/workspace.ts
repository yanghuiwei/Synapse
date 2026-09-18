import { create } from "zustand";

import * as ipc from "@/lib/ipc";
import type { ApiRequest, DeleteFolderResult, Folder, Workspace } from "@/lib/types";

export type LoadStatus = "idle" | "loading" | "ready" | "error";

export interface WorkspaceState {
  status: LoadStatus;
  /** 上一次加载失败的原因；成功时为 null */
  error: ipc.IpcError | null;

  /** 当前工作区。启动 `load()` 后必定有值 */
  workspace: Workspace | null;
  workspaces: Workspace[];

  /** 存活文件夹，**平铺**存着 —— 树在 UI 层按 parentId 现组 */
  folders: Folder[];
  /** 存活请求，同样平铺 */
  requests: ApiRequest[];

  load: () => Promise<void>;
  /** 重新拉文件夹与请求，但**不动** `status`（不闪加载态）。所有写操作之后都走它 */
  refresh: () => Promise<void>;
  /** 单行就地替换（自动保存用，省一次往返） */
  patchRequest: (request: ApiRequest) => void;

  // ---- 写操作 ----
  createFolder: (parentId: number | null, name: string) => Promise<Folder>;
  createRequest: (folderId: number | null, name: string) => Promise<ApiRequest>;
  renameFolder: (id: number, name: string) => Promise<void>;
  renameRequest: (id: number, name: string) => Promise<void>;
  duplicateRequest: (id: number, name: string) => Promise<ApiRequest>;
  moveFolder: (id: number, parentId: number | null) => Promise<void>;
  moveRequest: (id: number, folderId: number | null) => Promise<void>;
  deleteFolder: (id: number) => Promise<DeleteFolderResult>;
  deleteRequest: (id: number) => Promise<void>;
  /** 从回收站还原。会连带还原整棵子树，所以直接整块刷新而不是打补丁 */
  restoreFolder: (id: number) => Promise<void>;
  restoreRequest: (id: number) => Promise<void>;
}

/**
 * 集合树数据。
 *
 * 为什么平铺而不是组好树：树形结构在 UI 里还要按「展开 / 折叠 / 筛选关键字 /
 * 拖拽顺序」重算，后端或 store 提前组好只会让每次筛选都重建一遍对象。
 * 平铺 + 一个 `children(parentId)` 选择器是渲染时最省的形状。
 *
 * 为什么 folders / requests 分成两个数组而不是嵌进树节点：
 * 移动接口（「移动到其他分类」）只需要改一个 `folderId`，
 * 嵌套结构则要在两棵树之间搬节点。
 *
 * ## 写操作一律「先落库、再刷新」
 *
 * 所有写操作都**不做乐观更新**：本地 SQLite 的往返是亚毫秒级，
 * 而乐观更新要为每个操作手写一份「失败怎么回滚」，还要处理
 * 「级联删除在本地怎么算子树」这类只有后端才知道答案的问题。
 * 代价是每次操作多一次 `folders` + `requests` 查询 —— 在本地库上可以忽略。
 *
 * 唯一例外是 `patchRequest`：自动保存每 700ms 就会调一次，
 * 为它重拉两个列表不划算，而且它只改一行、不涉及树结构。
 */
export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  status: "idle",
  error: null,
  workspace: null,
  workspaces: [],
  folders: [],
  requests: [],

  load: async () => {
    set({ status: "loading", error: null });
    try {
      // 先拿工作区（并行），再按它的 id 拉树 —— 后者依赖前者，不能并成一批
      const [workspaces, active] = await Promise.all([
        ipc.workspaces.list(),
        ipc.workspaces.active(),
      ]);

      const [folders, requests] = await Promise.all([
        ipc.folders.list(active.id),
        ipc.requests.list(active.id),
      ]);

      set({
        status: "ready",
        error: null,
        workspace: active,
        workspaces,
        folders,
        requests,
      });
    } catch (e) {
      // 失败时**不清空**已有数据：断网/锁库后重新加载失败，
      // 界面上残留的旧列表比一片空白更有用。
      set({ status: "error", error: ipc.toIpcError(e) });
    }
  },

  refresh: async () => {
    const workspace = get().workspace;
    if (!workspace) return;
    try {
      const [folders, requests] = await Promise.all([
        ipc.folders.list(workspace.id),
        ipc.requests.list(workspace.id),
      ]);
      set({ folders, requests, status: "ready", error: null });
    } catch (e) {
      set({ error: ipc.toIpcError(e) });
    }
  },

  patchRequest: (request) =>
    set((s) => ({
      requests: s.requests.some((r) => r.id === request.id)
        ? s.requests.map((r) => (r.id === request.id ? request : r))
        : s.requests,
    })),

  // ---------------------------------------------------------------
  // 写操作
  // ---------------------------------------------------------------

  createFolder: async (parentId, name) => {
    const workspace = requireWorkspace(get());
    const folder = await ipc.folders.create(workspace.id, parentId, name);
    await get().refresh();
    return folder;
  },

  createRequest: async (folderId, name) => {
    const workspace = requireWorkspace(get());
    const request = await ipc.requests.create(workspace.id, folderId, name);
    await get().refresh();
    return request;
  },

  renameFolder: async (id, name) => {
    await ipc.folders.rename(id, name);
    await get().refresh();
  },

  renameRequest: async (id, name) => {
    await ipc.requests.rename(id, name);
    await get().refresh();
  },

  duplicateRequest: async (id, name) => {
    const copy = await ipc.requests.duplicate(id, name);
    await get().refresh();
    return copy;
  },

  moveFolder: async (id, parentId) => {
    await ipc.folders.move(id, parentId);
    await get().refresh();
  },

  moveRequest: async (id, folderId) => {
    await ipc.requests.move(id, folderId);
    await get().refresh();
  },

  deleteFolder: async (id) => {
    const result = await ipc.folders.remove(id);
    await get().refresh();
    return result;
  },

  deleteRequest: async (id) => {
    await ipc.requests.remove(id);
    await get().refresh();
  },

  restoreFolder: async (id) => {
    await ipc.folders.restore(id);
    await get().refresh();
  },

  restoreRequest: async (id) => {
    await ipc.requests.restore(id);
    await get().refresh();
  },
}));

/** 写操作都需要工作区 id；拿不到说明启动序列还没跑完 */
function requireWorkspace(state: WorkspaceState): Workspace {
  if (!state.workspace) {
    throw new ipc.IpcError({
      code: "E_WORKSPACE_MISSING",
      debug: "a write command ran before the workspace finished loading",
    });
  }
  return state.workspace;
}

/** 某个父节点下的直接子文件夹，按 sortOrder 排好（已按 id 兜底稳定） */
export function childFolders(folders: Folder[], parentId: number | null): Folder[] {
  return folders.filter((f) => f.parentId === parentId);
}

/** 某个文件夹下的请求；`null` 表示「未归档」那批 */
export function requestsInFolder(
  requests: ApiRequest[],
  folderId: number | null,
): ApiRequest[] {
  return requests.filter((r) => r.folderId === folderId);
}

/**
 * 某文件夹及其全部子孙的 id 集合（**含自身**）。
 *
 * 拖拽时要在 `dragstart` 就把它算出来 —— 「不能移到自己的子孙下」
 * 这条规则必须在这时判完，等到 `drop` 才判的话，用户会看到
 * 非法目标在整个拖拽过程中一直显示投放高亮，最后关头才被拒。
 */
export function folderSubtree(
  folders: Folder[],
  rootId: number,
): Set<number> {
  const byParent = new Map<number | null, Folder[]>();
  for (const f of folders) {
    const list = byParent.get(f.parentId);
    if (list) list.push(f);
    else byParent.set(f.parentId, [f]);
  }

  const out = new Set<number>([rootId]);
  // 广度优先 + `Set` 去重：万一库里已经有环（导入来的脏数据），
  // 这里不会无限循环 —— 与后端 `UNION` 而非 `UNION ALL` 是同一个理由
  const queue = [rootId];
  while (queue.length > 0) {
    const id = queue.shift() as number;
    for (const child of byParent.get(id) ?? []) {
      if (out.has(child.id)) continue;
      out.add(child.id);
      queue.push(child.id);
    }
  }
  return out;
}
