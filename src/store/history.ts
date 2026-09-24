import { create } from "zustand";

import * as ipc from "@/lib/ipc";
import type { HistoryEntry, NewHistoryEntry } from "@/lib/types";
import { useWorkspaceStore } from "@/store/workspace";

/** 单页条数。与后端 `DEFAULT_PAGE_SIZE` 一致；后端还会再夹到 200 以内。 */
export const HISTORY_PAGE_SIZE = 50;

export type LoadStatus = "idle" | "loading" | "ready" | "error";

export interface HistoryState {
  status: LoadStatus;
  error: ipc.IpcError | null;

  items: HistoryEntry[];
  /** **匹配**总数（带搜索时也只数匹配项）—— 用来判断还有没有下一页 */
  total: number;
  /** 当前搜索词；空串 = 不过滤 */
  keyword: string;
  /** 正在续页（与首屏 loading 分开，避免续页时整个列表变成加载态） */
  loadingMore: boolean;

  /**
   * 当前选中的那一条历史。
   *
   * 放在 store 而不是 `HistoryList` 的局部 state：详情现在由**主视图右侧**渲染，
   * 而列表住在左侧那一列 —— 两棵子树隔着一层，共享字段比提状态直接。
   * `null` = 还没选中任何一条（详情区显示空态）。
   */
  selectedId: number | null;

  load: () => Promise<void>;
  loadMore: () => Promise<void>;
  setKeyword: (keyword: string) => void;
  select: (id: number | null) => void;

  record: (entry: NewHistoryEntry) => Promise<void>;
  remove: (id: number) => Promise<void>;
  clearAll: () => Promise<number>;
}

/**
 * 历史列表。
 *
 * 分页用「追加」而不是「翻页」：历史是**时间倒序的流**，
 * 用户往下滚是想看更早的事，而不是想看「第 3 页」。所以只有
 * `load`（重置到第一页）与 `loadMore`（往后接一截）。
 *
 * `loadMore` 期间不动 `status` —— 否则续页会把已经渲染出来的列表
 * 整块换成加载态，视觉上像「刷新了」。
 */
export const useHistoryStore = create<HistoryState>((set, get) => ({
  status: "idle",
  error: null,
  items: [],
  total: 0,
  keyword: "",
  loadingMore: false,
  selectedId: null,

  load: async () => {
    const workspaceId = currentWorkspaceId();
    if (workspaceId === null) return;

    // 记下这次请求用的关键字。**必须**在 await 之前取 ——
    // 用户每敲一个字就会发起一次 load，两个请求并行时后到的可能是前一个的结果，
    // 列表就会显示上一个关键字的内容（看起来像"搜索慢了一拍"）。
    const sent = get().keyword;

    set({ status: "loading", error: null });
    try {
      const page = await ipc.history.list(workspaceId, {
        keyword: sent,
        limit: HISTORY_PAGE_SIZE,
        offset: 0,
      });
      if (get().keyword !== sent) return; // 过期结果，丢掉
      set({ status: "ready", error: null, items: page.items, total: page.total });
    } catch (e) {
      if (get().keyword !== sent) return;
      set({ status: "error", error: ipc.toIpcError(e) });
    }
  },

  loadMore: async () => {
    const { items, total, loadingMore, keyword, status } = get();
    const workspaceId = currentWorkspaceId();
    if (workspaceId === null || loadingMore) return;
    // 第一页还在路上时不能续页：`offset` 要用当前列表长度，
    // 而那一刻的列表长度还是**上一个关键字**的，续上来的是错的那一页
    if (status !== "ready") return;
    if (items.length >= total) return; // 已经没有下一页了，别再白跑一趟

    set({ loadingMore: true });
    try {
      const page = await ipc.history.list(workspaceId, {
        keyword,
        limit: HISTORY_PAGE_SIZE,
        offset: items.length,
      });
      if (get().keyword !== keyword) return; // 期间换了关键字，这一页已经不属于当前列表
      // 用 id 去重再追加：边滚动边发请求时，同一页可能被拉两次
      // （`items.length` 在两次请求之间没变），直接 concat 会出现重复行
      const seen = new Set(items.map((i) => i.id));
      const fresh = page.items.filter((i) => !seen.has(i.id));
      set({ items: [...items, ...fresh], total: page.total, loadingMore: false });
    } catch (e) {
      if (get().keyword !== keyword) return;
      set({ loadingMore: false, error: ipc.toIpcError(e) });
    }
  },

  setKeyword: (keyword) => {
    set({ keyword });
    // 搜索本身就是「重新开始一次列表」，直接重置到第一页
    void get().load();
  },

  /**
   * 选中一条。**不动 `load`**，也**不回填编辑器** ——
   * 选中只决定右侧显示哪一条的详情。
   */
  select: (id) => set({ selectedId: id }),

  /**
   * 写一条历史。
   *
   * 搜索状态下**不**动列表：新记录未必匹配当前关键字，
   * 硬插进去会让人以为搜索失灵了。等清空搜索时 `load()` 自然会带上它。
   * 这个函数不抛错 —— 它跑在发请求之后的收尾路径上，
   * 让「写历史失败」把「请求已经成功」这件事变成异常，是本末倒置。
   */
  record: async (entry) => {
    try {
      const saved = await ipc.history.record(entry);
      if (get().keyword !== "") return;
      set((s) => ({ items: [saved, ...s.items], total: s.total + 1 }));
    } catch (e) {
      console.warn("[reqly] 写历史失败：", ipc.toIpcError(e).debug);
    }
  },

  remove: async (id) => {
    await ipc.history.remove(id);
    // 本地就能算准：删的是一条确定存在的行，不涉及级联
    set((s) => ({
      items: s.items.filter((i) => i.id !== id),
      total: Math.max(0, s.total - 1),
    }));
  },

  clearAll: async () => {
    const workspaceId = currentWorkspaceId();
    if (workspaceId === null) return 0;
    const removed = await ipc.history.clearAll(workspaceId);
    set({ items: [], total: 0 });
    return removed;
  },
}));

/**
 * 当前工作区 id。
 *
 * 用 `getState()` 现取而不是订阅：这些操作都是命令式的（用户点了才发生），
 * 订阅只会让组件在无关的工作区变更上重渲染。
 *
 * 依赖方向是**单向**的（history → workspace），所以这里直接 import 不会成环。
 * 工作区拿不到时返回 null，调用方直接不做 —— 启动序列还没跑完时
 * 历史列表本来就是空的。
 */
function currentWorkspaceId(): number | null {
  return useWorkspaceStore.getState().workspace?.id ?? null;
}
