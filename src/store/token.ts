import { create } from "zustand";

import * as ipc from "@/lib/ipc";
import type { TokenSource, TokenStatus } from "@/lib/types";
import { useWorkspaceStore } from "@/store/workspace";

export type LoadStatus = "idle" | "loading" | "ready" | "error";

export interface TokenState {
  status: LoadStatus;
  error: ipc.IpcError | null;
  /** 当前工作区的全部 Token 来源 */
  sources: TokenSource[];
  /** tokenSourceId → 运行状态。没有键 = 还没查过 */
  statuses: Record<number, TokenStatus>;
  /** 正在刷新哪个来源（`null` = 没有）。用来把「立即刷新」按钮置灰 */
  refreshingId: number | null;

  load: () => Promise<void>;
  /** 只重读某一个来源的状态（刷新完之后用） */
  reloadStatus: (id: number) => Promise<void>;
  /** 强制刷新 —— 走的是与业务请求同一个单飞锁 */
  refresh: (id: number) => Promise<boolean>;

  /**
   * 新建。收的是表单那份完整草稿，而不只是「名字 + 获取请求」——
   * 后端那条命令只写最小的那一行，剩下的由这里补（理由见实现里的注释）。
   */
  create: (
    draft: Omit<TokenSource, "id" | "createdAt" | "updatedAt">,
  ) => Promise<TokenSource | null>;
  save: (source: TokenSource) => Promise<TokenSource | null>;
  remove: (id: number) => Promise<boolean>;
}

/**
 * Token 来源。
 *
 * ## 为什么状态也要一次全拉
 *
 * 状态栏要显示「N 个待刷新」，而那个数字必须来自后端的判定
 * （`http::token::is_expired`，含 `skew_seconds`）。前端自己拿
 * `expiresAt` 再算一遍，两边迟早在边界上分叉，表现为
 * 「徽标说还有 5 秒，后端说已经过期了」。
 *
 * 来源数量是个位数，逐条 `get_token_status` 都是**纯读缓存、不触发网络**，
 * 所以并发拉一轮的代价可以忽略。
 */
export const useTokenStore = create<TokenState>((set, get) => ({
  status: "idle",
  error: null,
  sources: [],
  statuses: {},
  refreshingId: null,

  load: async () => {
    const workspaceId = currentWorkspaceId();
    if (workspaceId === null) return;

    set({ status: "loading", error: null });
    try {
      const sources = await ipc.tokens.list(workspaceId);
      const statuses = await ipc.tokens.statuses(sources);
      set({
        status: "ready",
        error: null,
        sources,
        statuses: Object.fromEntries(statuses),
      });
    } catch (e) {
      set({ status: "error", error: ipc.toIpcError(e) });
    }
  },

  reloadStatus: async (id) => {
    try {
      const next = await ipc.tokens.status(id);
      set((s) => ({ statuses: { ...s.statuses, [id]: next } }));
    } catch (e) {
      console.warn("[reqly] 读取 Token 状态失败：", ipc.toIpcError(e).debug);
    }
  },

  refresh: async (id) => {
    // 同一来源不允许并发刷新：后端有单飞锁兜着（多按几次也不会多打网络），
    // 但界面上要能看出「正在刷」，所以这一层也挡一下
    if (get().refreshingId !== null) return false;
    set({ refreshingId: id });

    try {
      const next = await ipc.tokens.refresh(id);
      set((s) => ({ statuses: { ...s.statuses, [id]: next } }));
      return true;
    } catch (e) {
      const error = ipc.toIpcError(e);
      console.warn("[reqly] 刷新 Token 失败：", error.debug);
      // 失败时**保留旧状态**（可能已经是过期态），只把错误码留给界面 ——
      // 把它清成「没缓存」会看起来像配置丢了
      set({ error });
      return false;
    } finally {
      set({ refreshingId: null });
    }
  },

  create: async (draft) => {
    const workspaceId = currentWorkspaceId();
    if (workspaceId === null) return fail(null);

    try {
      const created = await ipc.tokens.create(
        workspaceId,
        draft.folderId,
        draft.name,
        draft.fetchRequestId,
      );

      // `create_token_source` 只写「名字 + 获取请求」两个字段（其余落到列默认值），
      // 所以这里接着补一次全量保存，把用户在表单里填的提取路径、刷新请求、
      // 提前量一起落库。
      //
      // 为什么不给那条命令再加 8 个参数：那要连着改 Rust 命令、仓储和
      // 一整组测试，而 `save_token_source` 早就在往同一张表写同一批字段了 ——
      // 两条路径维护同一份数据，迟早会有一份忘了改。
      //
      // 没配「刷新请求」时跳过这一趟：那是绝大多数配置的形状，
      // 不值得为此多走一次 IPC。
      const saved =
        draft.refreshRequestId === null
          ? created
          : await ipc.tokens.save({ ...created, ...draft, workspaceId });

      await get().load();
      return saved;
    } catch (e) {
      return fail(e);
    }
  },

  save: async (source) => {
    try {
      const saved = await ipc.tokens.save(source);
      await get().load();
      return saved;
    } catch (e) {
      return fail(e);
    }
  },

  remove: async (id) => {
    try {
      await ipc.tokens.remove(id);
      await get().load();
      return true;
    } catch (e) {
      fail(e);
      return false;
    }
  },
}));

/**
 * 状态栏 / 顶栏要的两个数字。
 *
 * 判据是**后端给的 `expired`**，不是前端自己比时间戳。
 * 从没取过的来源（`hasCache === false`）算**待刷新** —— 它一定会触发一次网络请求；
 * 状态还没查回来的来源**两个数字都不计**（宁可不显示，也别先报一个假数字）。
 *
 * 刻意不做成 store 的派生字段：它是 `sources × statuses` 的函数，
 * 存一份就得在每次刷新后同步更新，而这种同步迟早会漏。
 *
 * ⚠️ 调用方的选择器必须落到**基本类型**上（`(s) => tokenCounts(s).pending`）。
 * zustand 比的是返回值引用，`(s) => tokenCounts(s)` 每次都新建对象 → 无限重渲染。
 */
export interface TokenCounts {
  /** 已判定过期 + 从没取过的来源数 */
  pending: number;
  /** 已判定还有效的来源数 */
  valid: number;
}

export function tokenCounts(state: TokenState): TokenCounts {
  let pending = 0;
  let valid = 0;
  for (const source of state.sources) {
    const status = state.statuses[source.id];
    if (!status) continue;
    if (status.expired) pending += 1;
    else valid += 1;
  }
  return { pending, valid };
}

function fail(raw: unknown): null {
  const error = ipc.toIpcError(raw);
  console.warn("[reqly] Token 来源操作失败：", error.debug);
  void useTokenStore.getState().load();
  useTokenStore.setState({ error });
  return null;
}

function currentWorkspaceId(): number | null {
  return useWorkspaceStore.getState().workspace?.id ?? null;
}
