import { create } from "zustand";

import * as ipc from "@/lib/ipc";
import type {
  ExportFormat,
  ExportResult,
  LoadtestChart,
  LoadtestConfig,
  LoadtestProgress,
  LoadtestRun,
  LoadtestRunDetail,
  LoadtestSummary,
  ReportLabels,
} from "@/lib/types";
import { draftToSpec, useRequestStore } from "@/store/request";

/**
 * 进度的**本地时间线**最多留多少个点。
 *
 * 后端推的是「当前的累计数字」（sent / p95 / rps），而折线图要的是历史。
 * 所以这里自己攒一份：200ms 一个点，600 个就是两分钟 —— 再长的压测
 * 折线图也早就压在屏幕左边看不出细节了。
 */
const MAX_LIVE_POINTS = 600;

/**
 * 本轮在界面上的阶段。
 *
 * 刻意把「被取消且没产出」和「被取消但有产出」分开：前者什么都没有，
 * 后者有一份带 `cancelled` 标记的完整报告 —— 而那份报告**必须显示**，
 * 否则用户会拿半截数字当完整结果。
 */
export type LoadtestPhase = "idle" | "running" | "done" | "stopped" | "failed";

/** 折线上的一个点（前端自己攒的） */
export interface LivePoint {
  /** 相对于开始的秒 */
  second: number;
  sent: number;
  p95Ms: number;
  rps: number;
}

export interface LoadtestState {
  config: LoadtestConfig;

  phase: LoadtestPhase;
  /** 当前这一轮的 token；null = 没有在跑 */
  token: string | null;
  /** 已经点了停止、等后端回话 */
  cancelling: boolean;
  progress: LoadtestProgress | null;
  /** 前端自己攒的时间线（后端只推当前值） */
  livePoints: LivePoint[];

  /** 当前展示的报告（刚跑完的，或从历史里打开的） */
  detail: LoadtestRunDetail | null;
  chart: LoadtestChart | null;
  detailLoading: boolean;

  error: ipc.IpcError | null;
  /** 最近一次导出的结果 */
  exported: ExportResult | null;

  runs: LoadtestRun[];
  runsLoading: boolean;

  patchConfig: (patch: Partial<LoadtestConfig>) => void;
  resetConfig: () => void;

  start: () => Promise<void>;
  cancel: () => Promise<void>;

  refreshRuns: () => Promise<void>;
  openRun: (runId: number) => Promise<void>;
  deleteRun: (runId: number) => Promise<void>;
  exportCurrent: (format: ExportFormat, labels: ReportLabels) => Promise<void>;
  clearResult: () => void;
}

/**
 * 默认配置。
 *
 * 与 Rust `LoadtestConfig::default()` **刻意不同**：那边是「最小能跑起来」的
 * 保守值（10 并发 / 100 请求），这里是「打开面板就能直接点开始」的值。
 * 两边的用途不同 —— 后端那份是校验与兜底，前端这份是初始体验。
 *
 * 初值不大是刻意的：压测是能真把别人的服务打挂的操作。
 * 让人工改一次数字再点开始，这点摩擦是有价值的。
 */
export const DEFAULT_LOADTEST_CONFIG: LoadtestConfig = {
  concurrency: 10,
  total: 100,
  timeoutMs: 30_000,
  followRedirects: false,
  keepAlive: true,
  thinkTimeMs: 0,
  httpVersion: "auto",
  warmupRequests: 0,
};

/**
 * 生成一轮的 token。
 *
 * 与 `store/request` 的 `newRunId` 同样的理由：它只服务于「进度事件对不对得上号」
 * 和「取消」，不需要全局唯一，但**不能**两次并发撞车。
 */
function newRunToken(): string {
  const c = globalThis.crypto;
  if (typeof c?.randomUUID === "function") return c.randomUUID();
  const bytes = c.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/**
 * 进度事件的订阅句柄。放模块级而不是 state：
 * 它不参与渲染，进 state 只会让每次 setState 都多拎一个函数引用
 * （与 `store/request` 里那个 autosave 定时器同一个理由）。
 */
let unsubscribe: (() => void) | null = null;

async function ensureSubscribed(): Promise<void> {
  if (unsubscribe) return;
  unsubscribe = await ipc.loadtests.onProgress((event) => {
    const state = useLoadtestStore.getState();
    // 只认当前这一轮的事件。取消之后可能还有一两个 tick 已经在飞，
    // 而那时界面已经换了状态 —— 让进度条回跳一下比丢帧更糟
    if (state.token !== event.token) return;

    const snapshot = event.snapshot;
    const second = Math.floor(snapshot.elapsedMs / 1000);
    const last = state.livePoints[state.livePoints.length - 1];

    // 同一个秒内只保留最后一个点：后端 200ms 推一次、图上 1s 一个刻度，
    // 不合并的话一秒里会塞 5 个点，横轴会被压缩 5 倍
    const next =
      last && last.second === second
        ? [...state.livePoints.slice(0, -1), { second, sent: snapshot.sent, p95Ms: snapshot.p95Ms, rps: snapshot.currentRps }]
        : [
            ...state.livePoints,
            { second, sent: snapshot.sent, p95Ms: snapshot.p95Ms, rps: snapshot.currentRps },
          ];

    useLoadtestStore.setState({
      progress: snapshot,
      livePoints: next.length > MAX_LIVE_POINTS ? next.slice(next.length - MAX_LIVE_POINTS) : next,
    });
  });
}

export const useLoadtestStore = create<LoadtestState>((set, get) => ({
  config: { ...DEFAULT_LOADTEST_CONFIG },
  phase: "idle",
  token: null,
  cancelling: false,
  progress: null,
  livePoints: [],
  detail: null,
  chart: null,
  detailLoading: false,
  error: null,
  exported: null,
  runs: [],
  runsLoading: false,

  patchConfig: (patch) => set((s) => ({ config: { ...s.config, ...patch } })),
  resetConfig: () => set({ config: { ...DEFAULT_LOADTEST_CONFIG } }),

  start: async () => {
    const { phase, config } = get();
    if (phase === "running") return;

    // 压的是**当前编辑器里的草稿**，与「发送」按钮走同一份数据 ——
    // 否则会出现"我改完 URL 点开始，打的还是旧地址"
    const request = useRequestStore.getState();
    const spec = draftToSpec(request.draft);
    if (spec.url.trim() === "") {
      set({
        error: new ipc.IpcError({
          code: "E_INVALID_INPUT",
          params: { field: "url" },
          debug: "loadtest started with an empty url",
        }),
      });
      return;
    }

    const token = newRunToken();
    set({
      phase: "running",
      token,
      cancelling: false,
      progress: null,
      livePoints: [],
      detail: null,
      chart: null,
      error: null,
      exported: null,
    });

    // 先挂监听再发命令：反过来会丢掉最开始那一两个进度事件，
    // 而那正是"按下按钮之后有没有反应"最要紧的一刻
    await ensureSubscribed();

    try {
      const detail = await ipc.loadtests.start(
        token,
        {
          requestId: request.sourceId,
          requestName: request.sourceName,
        },
        spec,
        config,
      );

      // 这一轮可能已经被顶掉（取消后马上又开了一轮）——
      // 只认自己那一轮的结果，否则旧结果会盖到新一轮的界面上
      if (get().token !== token) return;
      set({ detail, chart: null, phase: "done", token: null, progress: null });
      void loadChart(detail.run.id);
      void get().refreshRuns();
    } catch (e) {
      const err = ipc.toIpcError(e);
      if (get().token !== token) return;

      // 用户主动取消**不是错误**。走到这里说明后端一个请求都没发出去
      // （预热阶段就被取消了）—— 那一轮的记录已经被后端删掉，没有报告可给。
      //
      // 注意：如果取消发生在正式计时阶段，后端会返回一份**带 cancelled 标记的
      // 完整报告**，走的是上面那条成功分支 —— 那份数字必须显示出来。
      if (err.code === "E_CANCELLED") {
        set({ phase: "stopped", token: null, cancelling: false, progress: null, error: null });
        return;
      }

      set({ phase: "failed", token: null, cancelling: false, progress: null, error: err });
    }
  },

  cancel: async () => {
    if (get().phase !== "running") return;
    set({ cancelling: true });

    try {
      // 返回 false = 后端那边已经结束了。这不代表出错，
      // 只是 start 的 promise 还没落地 —— 把 cancelling 撤掉，
      // 真正的状态转移仍然由那次 promise 完成。
      const found = await ipc.loadtests.cancel();
      if (!found) set({ cancelling: false });
    } catch (e) {
      // 取消失败没什么可补救的：这一轮要么自己结束，要么本来就结束了。
      // 留个控制台线索，不打扰用户。
      console.warn("[reqly] 取消防测失败：", ipc.toIpcError(e).debug);
      set({ cancelling: false });
    }
  },

  refreshRuns: async () => {
    set({ runsLoading: true });
    try {
      const runs = await ipc.loadtests.runs(50, 0);
      set({ runs, runsLoading: false });
    } catch (e) {
      console.warn("[reqly] 压测历史加载失败：", ipc.toIpcError(e).debug);
      set({ runsLoading: false });
    }
  },

  openRun: async (runId) => {
    set({ detailLoading: true, error: null });
    try {
      const detail = await ipc.loadtests.run(runId);
      set({ detail, chart: null, phase: "done", token: null, detailLoading: false });
      void loadChart(runId);
    } catch (e) {
      set({ detailLoading: false, error: ipc.toIpcError(e) });
    }
  },

  deleteRun: async (runId) => {
    try {
      await ipc.loadtests.remove(runId);
      const current = get().detail?.run.id;
      set((s) => ({
        runs: s.runs.filter((r) => r.id !== runId),
        // 删掉的正好是正在看的那一轮 → 把报告一起收掉，
        // 否则界面上会留着一份已经不存在的数据
        detail: current === runId ? null : s.detail,
        chart: current === runId ? null : s.chart,
        phase: current === runId ? "idle" : s.phase,
      }));
    } catch (e) {
      set({ error: ipc.toIpcError(e) });
    }
  },

  exportCurrent: async (format, labels) => {
    const runId = get().detail?.run.id;
    if (runId === undefined) return;

    // 先清掉上一次的结果：成败还没定，留着旧的会让「导出成功（到某个路径）」
    // 和「导出失败」同时挂在界面上
    set({ exported: null });

    // 失败时**往上抛**，不写进共享的 `error`：那个字段还承载着
    // 「打开某轮报告失败」，混用会让两条不同的错误共用一句提示。
    // 调用方（`ExportBar`）自己 catch 并就地显示。
    const exported = await ipc.loadtests.export(runId, format, labels);
    set({ exported });
  },

  clearResult: () =>
    set({ detail: null, chart: null, progress: null, livePoints: [], exported: null, error: null, phase: "idle" }),
}));

/**
 * 取图表数据。
 *
 * 单独一个函数而不是写进 `start`/`openRun` 里：它是个**可以失败但不该拖累主流程**
 * 的附加请求 —— 图表挂了，概要数字还得显示出来。
 */
async function loadChart(runId: number): Promise<void> {
  try {
    const chart = await ipc.loadtests.chart(runId);
    // 期间可能已经切到别的轮次了，只认当前这一份
    if (useLoadtestStore.getState().detail?.run.id !== runId) return;
    useLoadtestStore.setState({ chart });
  } catch (e) {
    console.warn("[reqly] 压测图表加载失败：", ipc.toIpcError(e).debug);
  }
}

/**
 * 一轮的概要。`detail.summary` 本来就是解析好的，
 * 这个函数只是把「可能为 null」这件事收在一处，省得每个组件都判一次。
 */
export function summaryOf(detail: LoadtestRunDetail | null): LoadtestSummary | null {
  return detail?.summary ?? null;
}
