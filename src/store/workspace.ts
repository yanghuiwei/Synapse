import { create } from "zustand";

import * as ipc from "@/lib/ipc";
import type { ApiRequest, Folder, Workspace } from "@/lib/types";

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
}

/**
 * 集合树数据。
 *
 * 为什么平铺而不是组好树：树形结构在 UI 里还要按「展开 / 折叠 / 筛选关键字 /
 * 拖拽顺序」重算，后端或 store 提前组好只会让每次筛选都重建一遍对象。
 * 平铺 + 一个 `children(parentId)` 选择器是渲染时最省的形状。
 *
 * 为什么 folders / requests 分成两个数组而不是嵌进树节点：
 * 移动接口（P2 的「移动到其他分类」）只需要改一个 `folderId`，
 * 嵌套结构则要在两棵树之间搬节点。
 */
export const useWorkspaceStore = create<WorkspaceState>((set) => ({
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
}));

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
