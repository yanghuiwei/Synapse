import { create } from "zustand";

import * as ipc from "@/lib/ipc";
import type { PurgeTrashResult, TrashContents } from "@/lib/types";
import { useHistoryStore } from "@/store/history";
import { useWorkspaceStore } from "@/store/workspace";

export type LoadStatus = "idle" | "loading" | "ready" | "error";

/** 空回收站。做成常量是为了让「清空之后」和「从没加载过」共用同一个引用 */
const EMPTY: TrashContents = { folders: [], requests: [], history: [] };

export interface TrashState {
  status: LoadStatus;
  error: ipc.IpcError | null;
  /** 三块内容一次拿回 —— 见 `TrashContents` 的说明 */
  contents: TrashContents;

  load: () => Promise<void>;

  /** 还原。分类会连带还原整棵子树（后端做），所以之后要整块刷新而不是打补丁 */
  restoreFolder: (id: number) => Promise<void>;
  restoreRequest: (id: number) => Promise<void>;
  restoreHistory: (id: number) => Promise<void>;

  /** 彻底删除。**不可撤销** */
  purgeFolder: (id: number) => Promise<void>;
  purgeRequest: (id: number) => Promise<void>;
  purgeHistory: (id: number) => Promise<void>;
  /**
   * 清空回收站。返回各类删了多少 —— 界面要用它做一句「已彻底删除 …」的反馈；
   * 返回 `null` 表示**失败了**（错误码在 `error` 里），此时界面不该报「删了 0 个」。
   */
  purgeAll: () => Promise<PurgeTrashResult | null>;
}

/**
 * 回收站。
 *
 * ## 为什么写操作之后要把工作区与历史一起重拉
 *
 * 「还原」这个动作的效果**不落在回收站里** —— 它是把东西从回收站搬回树上。
 * 只刷新回收站的话，用户会看到列表少了一行，但左侧集合树里什么都没出现，
 * 于是再点一次还原（此时已经不存在了）。
 *
 * 三次查询都在本地 SQLite 上，代价可以忽略；换来的是「一个动作，全界面一致」。
 *
 * ## 为什么不做乐观更新
 *
 * 与 `workspace` store 同一个理由：彻底删除要算整棵子树的规模，
 * 而那个算法只有后端 SQL 有。多一次往返换「计数永远对得上」，值。
 */
export const useTrashStore = create<TrashState>((set) => ({
  status: "idle",
  error: null,
  contents: EMPTY,

  load: async () => {
    const workspaceId = currentWorkspaceId();
    if (workspaceId === null) return;

    set({ status: "loading", error: null });
    try {
      const contents = await ipc.trash.list(workspaceId);
      set({ status: "ready", error: null, contents });
    } catch (e) {
      // 失败时保留已有内容：锁库/断连重试失败时，残留的旧列表比一片空白有用
      set({ status: "error", error: ipc.toIpcError(e) });
    }
  },

  restoreFolder: (id) => runMutation(() => ipc.folders.restore(id)),
  restoreRequest: (id) => runMutation(() => ipc.requests.restore(id)),
  restoreHistory: (id) => runMutation(() => ipc.history.restore(id)),

  purgeFolder: (id) => runMutation(() => ipc.folders.purge(id)),
  purgeRequest: (id) => runMutation(() => ipc.requests.purge(id)),
  purgeHistory: (id) => runMutation(() => ipc.history.purge(id)),

  purgeAll: async () => {
    const workspaceId = currentWorkspaceId();
    if (workspaceId === null) return null;

    try {
      const result = await ipc.trash.purge(workspaceId);
      // 清空之后回收站必然是空的，直接置空比再跑一趟 `list_trash` 更快，
      // 也不会出现「刚清完还是旧列表」的一帧
      set({ status: "ready", error: null, contents: EMPTY });
      await syncOthers();
      return result;
    } catch (e) {
      return fail(e);
    }
  },
}));

/**
 * 单个条目的还原 / 彻底删除。
 *
 * 失败**不能静默**：这些动作是用户明确点下去的，尤其「彻底删除」——
 * 什么都不说、列表也不动，用户只会以为按钮坏了然后反复点。
 * 所以失败时先把列表重新拉一遍（让它反映真实状态），再把错误码写进 `error`
 * 让界面显示一句提示。
 *
 * ⚠️ 顺序不能反：`load()` 成功时会把 `error` 清成 `null`，
 * 先写错误再刷新的话，提示会被那次刷新擦掉。
 */
async function runMutation(action: () => Promise<unknown>): Promise<void> {
  try {
    await action();
    await syncAfterChange();
    useTrashStore.setState({ error: null });
  } catch (e) {
    await fail(e);
  }
}

/** 出错处理：记控制台 + 回拉真实状态 + 把错误码留给界面 */
async function fail(raw: unknown): Promise<null> {
  const error = ipc.toIpcError(raw);
  console.warn("[reqly] 回收站操作失败：", error.debug);
  await useTrashStore.getState().load();
  useTrashStore.setState({ error, status: "ready" });
  return null;
}

/**
 * 一次写操作之后的统一收尾。
 *
 * 顺序有讲究：先刷新别处（工作区 / 历史），**最后**重拉回收站自身。
 * 反过来的话，用户会先看到回收站少了一行，隔一拍才看到集合树长出来，
 * 中间那一帧正是「东西去哪了」的困惑来源。
 */
async function syncAfterChange(): Promise<void> {
  await syncOthers();
  await useTrashStore.getState().load();
}

/** 还原/删除影响的其它板块 */
async function syncOthers(): Promise<void> {
  const jobs: Promise<unknown>[] = [useWorkspaceStore.getState().refresh()];

  // 历史区从没打开过就不用管它 —— 等它挂载时自然会 load 一次
  const history = useHistoryStore.getState();
  if (history.status !== "idle") jobs.push(history.load());

  await Promise.all(jobs);
}

function currentWorkspaceId(): number | null {
  return useWorkspaceStore.getState().workspace?.id ?? null;
}
