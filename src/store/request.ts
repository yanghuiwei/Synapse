import { create } from "zustand";

import { DEFAULT_REQUEST_SETTINGS } from "@/lib/http";
import * as ipc from "@/lib/ipc";
import type {
  ApiRequest,
  BodyType,
  HttpResponsePayload,
  KeyValueItem,
  RequestSettings,
  RequestSpec,
} from "@/lib/types";

/**
 * 编辑中的请求草稿。
 *
 * 与 `ApiRequest` 的区别：没有 id / 名字 / 时间戳 / 脚本。
 * 草稿可以存在（也能直接发送）而**没有**对应的已保存请求 ——
 * 「粘个地址就发」是这个工具最高频的用法，不该被"先保存"卡住。
 */
export interface RequestDraft {
  method: string;
  url: string;
  params: KeyValueItem[];
  headers: KeyValueItem[];
  bodyType: BodyType;
  body: string;
  settings: RequestSettings;
}

function emptyDraft(): RequestDraft {
  return {
    method: "GET",
    url: "",
    params: [],
    headers: [],
    bodyType: "none",
    body: "",
    settings: { ...DEFAULT_REQUEST_SETTINGS },
  };
}

/**
 * 从已保存的请求灌一份草稿。
 *
 * 这里**必须深拷贝** `params` / `headers`：它们是数组对象，
 * 直接引用的话「在界面上加一行」会同时改到 store 里那份原始数据 ——
 * 之后取消选中再选回来，就会看到一串没保存过的脏行。
 */
function draftFrom(req: ApiRequest): RequestDraft {
  return {
    method: req.method,
    url: req.url,
    params: req.params.map((row) => ({ ...row })),
    headers: req.headers.map((row) => ({ ...row })),
    bodyType: req.bodyType,
    body: req.body,
    settings: { ...req.settings },
  };
}

/** 草稿 → 后端要的请求定义。字段名与 Rust `RequestSpec` 的 camelCase 一一对应 */
function toSpec(draft: RequestDraft): RequestSpec {
  return {
    method: draft.method,
    url: draft.url,
    params: draft.params,
    headers: draft.headers,
    bodyType: draft.bodyType,
    body: draft.body,
    settings: draft.settings,
  };
}

/**
 * 生成一次执行的 id。
 *
 * 用 `crypto.randomUUID()`；拿不到就退回 `getRandomValues`。
 * 这个 id 不需要全局唯一，但**必须**避免两次并发执行撞车 ——
 * 撞了的话「取消」会打到另一次请求身上。
 */
function newRunId(): string {
  // `randomUUID` 只在**安全上下文**里有（https / localhost / tauri 协议）。
  // Tauri 的 WebView 满足，但用 http:// 打开 dev server 时不保证 —— 所以留条退路。
  const c = globalThis.crypto;
  if (typeof c?.randomUUID === "function") return c.randomUUID();

  const bytes = c.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export interface RequestState {
  /** 当前编辑的草稿 */
  draft: RequestDraft;
  /** 草稿来自哪个已保存的请求；null = 还没保存过 */
  sourceId: number | null;

  /** 正在跑的那次执行的 id；null = 空闲 */
  runId: string | null;
  /** 已经点了取消、但后端还没回话 —— 按钮要立刻有反应 */
  cancelling: boolean;

  response: HttpResponsePayload | null;
  error: ipc.IpcError | null;
  /** 上一次执行是被用户取消的（不是失败，所以要区别对待） */
  cancelled: boolean;
  /** 上一次执行结束的时间戳，用于「刚刚 / 3 秒前」这类相对时间 */
  finishedAt: number | null;

  /** 载入一个已保存请求；传 null 表示清空成新草稿 */
  loadFrom: (req: ApiRequest | null) => void;
  patch: (patch: Partial<RequestDraft>) => void;
  /** 整块替换键值表（params / headers 共用一套编辑器） */
  setRows: (which: "params" | "headers", rows: KeyValueItem[]) => void;

  send: () => Promise<void>;
  cancel: () => Promise<void>;
  clearResult: () => void;
}

/**
 * 请求闭环的状态机。
 *
 * 为什么把「发送 / 取消」放在 store 而不是组件里：
 * 快捷键（Ctrl+Enter / Esc）、地址栏按钮、响应区空态按钮三处都要能触发它，
 * 状态放在组件里就得靠 ref 往上提，还不如一开始就放 store。
 *
 * 并发语义：**同一时刻只允许一次执行**（`runId` 非空时 `send` 直接返回）。
 * 想连发两次就等第一次结束 —— 否则两个响应抢同一个展示位，界面会闪。
 */
// ===== TEMP(仅用于无头截图核对)：注入假数据，核完必须删掉 =====
const __DEV: string | null =
  typeof location === "undefined"
    ? null
    : new URLSearchParams(location.search).get("__dev");

function __devSeed(): Partial<RequestState> {
  if (!__DEV) return {};

  const draft: RequestDraft = {
    method: "POST",
    url: "{{env.baseUrl}}/v1/orders?page=1&size=20",
    params: [
      { key: "page", value: "1", enabled: true },
      { key: "size", value: "20", enabled: true },
      { key: "trace", value: "1", enabled: false },
    ],
    headers: [
      {
        key: "Authorization",
        value: "Bearer {{token.accessToken}}",
        enabled: true,
        desc: "过期前自动刷新",
      },
      { key: "X-Trace-Id", value: "{{$uuid}}", enabled: true, desc: "每次生成" },
    ],
    bodyType: "json",
    body: '{\n  "name": "reqly",\n  "n": 7\n}',
    settings: { ...DEFAULT_REQUEST_SETTINGS },
  };

  if (__DEV === "err") {
    return {
      draft,
      error: new ipc.IpcError({
        code: "E_DNS",
        params: { host: "api.example.com" },
        debug:
          "dns: failed to lookup address information: Name or service not known " +
          "(https://api.example.com/v1/orders?page=1)",
      }),
      finishedAt: Date.now(),
    };
  }

  if (__DEV === "binary") {
    return {
      draft,
      response: {
        status: 200,
        statusText: "OK",
        httpVersion: "HTTP/1.1",
        headers: [
          { name: "content-type", value: "image/png" },
          { name: "content-length", value: "4213" },
        ],
        url: "https://cdn.example.com/logo.png",
        redirected: false,
        body: "",
        bodyEncoding: "binary",
        bodySize: 4213,
        truncated: false,
        ttfbMs: 61,
        downloadMs: 8,
        totalMs: 74,
      },
      finishedAt: Date.now(),
    };
  }

  if (__DEV === "cancel") {
    return { draft, cancelled: true, finishedAt: Date.now() };
  }

  const body = JSON.stringify(
    {
      code: 0,
      message: "ok",
      data: {
        total: 137,
        page: 1,
        size: 20,
        items: [
          {
            id: 90124,
            orderNo: "SO-2026-0911-0012",
            status: "PAID",
            amount: 1299.5,
            currency: "CNY",
            paidAt: "2026-09-11T02:13:44.000Z",
            tags: ["urgent", "vip"],
          },
          {
            id: 90125,
            orderNo: "SO-2026-0911-0013",
            status: "PENDING",
            amount: 88,
            currency: "CNY",
            paidAt: null,
            tags: [],
          },
        ],
        cursor: null,
        traceId: "8f0c1a2b-4d5e-6f70-8192-a3b4c5d6e7f8",
      },
    },
    null,
    2,
  );

  return {
    draft,
    response: {
      status: 200,
      statusText: "OK",
      httpVersion: "HTTP/2.0",
      headers: [
        { name: "content-type", value: "application/json; charset=utf-8" },
        { name: "content-length", value: String(body.length) },
        { name: "date", value: "Fri, 11 Sep 2026 07:45:12 GMT" },
        { name: "server", value: "envoy" },
        { name: "x-request-id", value: "8f0c1a2b-4d5e-6f70-8192-a3b4c5d6e7f8" },
        { name: "set-cookie", value: "session=abc123; Path=/; HttpOnly; SameSite=Lax" },
        { name: "set-cookie", value: "theme=dark; Path=/; Max-Age=31536000" },
      ],
      url: "https://api.example.com/v1/orders?page=1&size=20",
      redirected: true,
      body,
      bodyEncoding: "text",
      bodySize: body.length,
      truncated: false,
      ttfbMs: 143,
      downloadMs: 27,
      totalMs: 174,
    },
    finishedAt: Date.now(),
  };
}
// ===== /TEMP =====

export const useRequestStore = create<RequestState>((set, get) => ({
  draft: emptyDraft(),
  sourceId: null,
  runId: null,
  cancelling: false,
  response: null,
  error: null,
  cancelled: false,
  finishedAt: null,
  ...__devSeed(),

  loadFrom: (req) => {
    if (__DEV) return; // TEMP

    // 换请求前先把在跑的那次掐掉：否则「A 的响应」会落到「B 的界面」上。
    // 即发即忘 —— 后端把它当成一次正常的用户取消。
    void get().cancel();

    set({
      draft: req ? draftFrom(req) : emptyDraft(),
      sourceId: req?.id ?? null,
      // 结果跟着请求走：换了请求就不该再显示上一个的响应
      response: null,
      error: null,
      cancelled: false,
      finishedAt: null,
    });
  },

  patch: (patch) => set((s) => ({ draft: { ...s.draft, ...patch } })),

  setRows: (which, rows) => set((s) => ({ draft: { ...s.draft, [which]: rows } })),

  send: async () => {
    const { draft, runId } = get();
    if (runId) return; // 已经在跑了，不叠加

    const id = newRunId();
    set({ runId: id, cancelling: false, error: null, cancelled: false });

    try {
      const response = await ipc.requests.send(id, toSpec(draft));

      // 这一轮可能已经被顶掉（取消后马上又发了一次）——
      // 只认自己那一轮的结果，否则旧响应会盖到新请求头上
      if (get().runId !== id) return;
      set({
        response,
        runId: null,
        cancelling: false,
        finishedAt: Date.now(),
      });
    } catch (e) {
      const err = ipc.toIpcError(e);
      if (get().runId !== id) return;

      // 用户主动取消**不是错误**：弹一句「已取消」就够，不该走红色报错
      if (err.code === "E_CANCELLED") {
        set({
          response: null,
          error: null,
          cancelled: true,
          runId: null,
          cancelling: false,
          finishedAt: Date.now(),
        });
        return;
      }

      set({
        response: null,
        error: err,
        cancelled: false,
        runId: null,
        cancelling: false,
        finishedAt: Date.now(),
      });
    }
  },

  cancel: async () => {
    const { runId } = get();
    if (!runId) return;
    set({ cancelling: true });

    // 取消可能"来得太早"：`send` 的 IPC 刚发出去，后端的登记表里还没这一条，
    // 此时 cancel 返回 false，但请求其实正在跑。所以只要这次执行还挂着
    // （runId 没变），就小睡一下再试几次 —— 后端登记发生在第一次 await 之前，
    // 几十毫秒足够覆盖这个窗口。
    for (let attempt = 0; attempt < 5; attempt += 1) {
      try {
        if (await ipc.requests.cancel(runId)) return;
      } catch (e) {
        // 取消本身失败没什么可补救的：这次执行要么自己结束，要么本来就结束了。
        // 留个控制台线索，不打扰用户。
        console.warn("[reqly] 取消请求失败：", ipc.toIpcError(e).debug);
        return;
      }

      if (get().runId !== runId) return; // 已经结束了，不用再试
      await sleep(40);
    }
  },

  clearResult: () =>
    set({
      response: null,
      error: null,
      cancelled: false,
      finishedAt: null,
    }),
}));
