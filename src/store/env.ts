import { create } from "zustand";

import * as ipc from "@/lib/ipc";
import type { EnvVar, Environment } from "@/lib/types";
import { useWorkspaceStore } from "@/store/workspace";

export type LoadStatus = "idle" | "loading" | "ready" | "error";

export interface EnvState {
  status: LoadStatus;
  error: ipc.IpcError | null;
  /** 当前工作区的环境列表（**不含**回收站里的） */
  environments: Environment[];
  /**
   * 当前激活的环境。`null` = 不使用环境。
   *
   * 真源是数据库的 `environment.is_active` 列 —— 这里只是它的缓存。
   * 任何「切换环境」的动作都必须走后端命令，然后整块重拉，
   * **不要**在前端自己 set 一份乐观值：两处状态一旦分叉，
   * 发送路径读的是后端那一份，界面上却显示另一份。
   */
  active: Environment | null;

  /**
   * 拉一次列表 + 激活项。
   *
   * 内部自己去问工作区 id（这一版只有一个工作区，前端不必把它塞进每个调用）。
   * `workspaceId` 为空时直接返回 —— 首挂时 `workspace.load()` 还在路上，
   * 这里必须能优雅地什么都不做，等调用方的 effect 再跑一次。
   */
  load: () => Promise<void>;

  create: (name: string) => Promise<Environment | null>;
  rename: (id: number, name: string) => Promise<Environment | null>;
  /** `id = null` 表示「不使用环境」 */
  setActive: (id: number | null) => Promise<boolean>;
  /** 软删除（进回收站） */
  remove: (id: number) => Promise<boolean>;

  /** 某个环境的变量表。整表替换语义 —— 不在这里缓存，编辑对话框自己持有 */
  vars: (environmentId: number) => Promise<EnvVar[]>;
  replaceVars: (environmentId: number, vars: EnvVar[]) => Promise<EnvVar[] | null>;
}

export const useEnvStore = create<EnvState>((set, get) => ({
  status: "idle",
  error: null,
  environments: [],
  active: null,

  load: async () => {
    const workspaceId = currentWorkspaceId();
    if (workspaceId === null) return;

    set({ status: "loading", error: null });
    try {
      // 两次查询并发：它们互不依赖，串行只是白等一个往返
      const [environments, active] = await Promise.all([
        ipc.envs.list(workspaceId),
        ipc.envs.active(workspaceId),
      ]);
      set({ status: "ready", error: null, environments, active });
    } catch (e) {
      // 失败时保留已有列表：锁库/断连重试失败时，残留的旧列表比一片空白有用
      set({ status: "error", error: ipc.toIpcError(e) });
    }
  },

  create: async (name) => {
    const workspaceId = currentWorkspaceId();
    if (workspaceId === null) return fail(null);

    try {
      const created = await ipc.envs.create(workspaceId, name);
      await get().load();
      return created;
    } catch (e) {
      return fail(e);
    }
  },

  rename: async (id, name) => {
    try {
      const renamed = await ipc.envs.rename(id, name);
      await get().load();
      return renamed;
    } catch (e) {
      return fail(e);
    }
  },

  setActive: async (id) => {
    const workspaceId = currentWorkspaceId();
    if (workspaceId === null) return false;

    try {
      await ipc.envs.setActive(workspaceId, id);
      // 重拉而不是就地改 `isActive` 标志：后端的 `set_active` 是一个事务里
      // 「先全置 0 再置 1」，哪一行真的变成了 1 只有库知道
      await get().load();
      return true;
    } catch (e) {
      fail(e);
      return false;
    }
  },

  remove: async (id) => {
    try {
      await ipc.envs.remove(id);
      await get().load();
      return true;
    } catch (e) {
      fail(e);
      return false;
    }
  },

  vars: (environmentId) => ipc.envs.vars(environmentId),

  replaceVars: async (environmentId, vars) => {
    try {
      return await ipc.envs.replaceVars(environmentId, vars);
    } catch (e) {
      return fail(e);
    }
  },
}));

/**
 * 失败处理：记控制台 + 回拉真实状态 + 把错误码留给界面。
 *
 * 回拉这一步不能省：写操作失败时前端的列表还停在「操作前」的样子，
 * 而库里可能已经变了一半（比如后端返回了错误但事务已提交）。
 */
function fail(raw: unknown): null {
  const error = ipc.toIpcError(raw);
  console.warn("[reqly] 环境操作失败：", error.debug);
  void useEnvStore.getState().load();
  useEnvStore.setState({ error });
  return null;
}

function currentWorkspaceId(): number | null {
  return useWorkspaceStore.getState().workspace?.id ?? null;
}
