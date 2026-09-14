import { create } from "zustand";

import { DEFAULT_REQUEST_SETTINGS } from "@/lib/http";
import * as ipc from "@/lib/ipc";
import type {
  ApiRequest,
  AuthConfig,
  BodyType,
  HttpResponsePayload,
  KeyValueItem,
  RequestSettings,
  RequestSpec,
} from "@/lib/types";
import { useEnvStore } from "@/store/env";
import { useHistoryStore } from "@/store/history";
import { useWorkspaceStore } from "@/store/workspace";

/** 自动保存的静默期。敲字时每 700ms 落一次库，手感上等于「不用保存」 */
const AUTOSAVE_DEBOUNCE_MS = 700;

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
  auth: AuthConfig;
  settings: RequestSettings;
  /**
   * 这条请求挂在哪个分类下。
   *
   * 草稿里放它不是为了编辑（界面上没有这个输入框），而是因为它是
   * **发送语义的一部分**：Token 来源与授权继承都要沿分类链向上找
   * （见 Rust `http::pipeline`）。历史快照回填时它是 `null` ——
   * 那时确实没有分类上下文，这是如实反映而不是丢信息。
   */
  folderId: number | null;
}

function emptyDraft(): RequestDraft {
  return {
    method: "GET",
    url: "",
    params: [],
    headers: [],
    bodyType: "none",
    body: "",
    // 新请求默认**继承分类**：绝大多数时候用户会在分类上配一次授权，
    // 然后希望下面所有请求自动用上
    auth: { type: "inherit" },
    settings: { ...DEFAULT_REQUEST_SETTINGS },
    folderId: null,
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
    // 授权配置也是对象（apiKey 变体有字段），同样得拷贝
    auth: { ...req.auth },
    settings: { ...req.settings },
    folderId: req.folderId,
  };
}

/**
 * 草稿 → 后端要的请求定义。字段名与 Rust `RequestSpec` 的 camelCase 一一对应。
 *
 * 导出给压测（P6）复用：那边也要把当前草稿发成真正的请求。
 * 两边各写一份的话，将来 `RequestSpec` 多一个字段时必然只改一处 ——
 * 而症状是「压测打的请求和单发的不一样」，很难查。
 */
export function draftToSpec(draft: RequestDraft): RequestSpec {
  return {
    method: draft.method,
    url: draft.url,
    params: draft.params,
    headers: draft.headers,
    bodyType: draft.bodyType,
    body: draft.body,
    auth: draft.auth,
    settings: draft.settings,
    folderId: draft.folderId,
  };
}

/**
 * 历史快照 → 草稿。
 *
 * 深拷贝的理由与 [`draftFrom`] 一样，而且这里更要紧：快照是
 * **原样存下来的 JSON**，直接引用的话「在界面上加一行参数」
 * 会改到 store 里那份历史记录对象上。
 */
function draftFromSpec(spec: RequestSpec): RequestDraft {
  return {
    method: spec.method,
    url: spec.url,
    params: spec.params.map((row) => ({ ...row })),
    headers: spec.headers.map((row) => ({ ...row })),
    bodyType: spec.bodyType,
    body: spec.body,
    auth: { ...spec.auth },
    settings: { ...spec.settings },
    // 历史快照可能来自 P2/P3，那时没有这个字段 —— 缺失就是「无分类」。
    // 刻意**不**猜成「当前选中的分类」：猜错会让请求带上一个它从未有过的
    // Token 来源，而界面上的「生效来源」看起来还挺合理。
    folderId: spec.folderId ?? null,
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

/**
 * 一次「落库」的目标。
 *
 * 为什么要先快照再写：`loadFrom` 会**立刻**把草稿换成新请求，
 * 而保存是异步的。不拿快照的话，写库时会读到已经换掉的草稿，
 * 结果把 B 的内容存进 A 的行里。
 */
interface SaveTarget {
  id: number;
  draft: RequestDraft;
}

/** 待写盘的定时器。放模块级而不是 state：它不需要触发重渲染 */
let autosaveTimer: ReturnType<typeof setTimeout> | null = null;

function cancelAutosave(): void {
  if (autosaveTimer !== null) {
    clearTimeout(autosaveTimer);
    autosaveTimer = null;
  }
}

function scheduleAutosave(): void {
  cancelAutosave();
  autosaveTimer = setTimeout(() => {
    autosaveTimer = null;
    void flushAutosave();
  }, AUTOSAVE_DEBOUNCE_MS);
}

/**
 * 把当前草稿写回库。
 *
 * 三处调用：静默期到了、切换请求之前、以及手动 `flushSave()`。
 * 失败**不**打扰用户：自动保存是背景行为，弹一个红框出来只会让人
 * 以为请求出了问题；控制台留线索就够了。
 */
async function flushAutosave(): Promise<void> {
  const state = useRequestStore.getState();
  if (!state.dirty) return;
  if (state.sourceId === null) return; // 没保存过的请求没有可写的行

  const target: SaveTarget = { id: state.sourceId, draft: state.draft };
  const base = findSaved(target.id);
  if (!base) return; // 已经被删掉了，没什么可写

  try {
    const saved = await ipc.requests.save({ ...base, ...draftFields(target.draft) });
    // 只清掉「这一轮」的 dirty：写盘期间用户又敲了字的话，标记要留着
    if (useRequestStore.getState().draft === target.draft) {
      useRequestStore.setState({ dirty: false });
    }
    // 侧栏的名字/方法色会跟着变，就地替换那一行比整块刷新便宜
    useWorkspaceStore.getState().patchRequest(saved);
  } catch (e) {
    console.warn("[reqly] 自动保存失败：", ipc.toIpcError(e).debug);
  }
}

/** 草稿里那几个能直接落到 `ApiRequest` 上的字段 */
function draftFields(draft: RequestDraft): Partial<ApiRequest> {
  return {
    method: draft.method,
    url: draft.url,
    params: draft.params,
    headers: draft.headers,
    bodyType: draft.bodyType,
    body: draft.body,
    // 授权**必须**落库：用户在授权页签改完之后多半不会点保存，
    // 靠的就是自动保存。漏了这一个字段的表现是「切走再切回来，授权没了」——
    // 而请求本身看起来完全正常
    auth: draft.auth,
    settings: draft.settings,
  };
}

/** 从工作区 store 里取一行已保存的请求（拿它的 id / 名字 / 脚本等非草稿字段） */
function findSaved(id: number): ApiRequest | undefined {
  return useWorkspaceStore.getState().requests.find((r) => r.id === id);
}

export interface RequestState {
  /** 当前编辑的草稿 */
  draft: RequestDraft;
  /** 草稿来自哪个已保存的请求；null = 还没保存过 */
  sourceId: number | null;
  /**
   * 草稿的来源名字。
   *
   * 已保存的请求用 `sourceId` 去查工作区就能拿到名字；
   * 这个字段只服务于「历史快照回填」—— 那时没有任何 id 可查，
   * 但界面上总得告诉用户现在编辑的是哪一条。
   */
  sourceName: string | null;

  /** 有改动尚未落库（自动保存的静默期里为 true） */
  dirty: boolean;

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

  // ---- 打开内容 ----
  //
  // 「打开哪个请求」是**显式动作**，不是从 `activeRequestId` 推出来的副作用。
  // 为什么这么设计：历史记录里的快照可能对应一个**已经删掉的**请求，
  // 那时要把内容灌进编辑器却没有 id 可挂。若由「选中项变了」驱动，
  // 把选中项设成 null 就会立刻把刚灌进去的内容清掉 ——
  // 一个必然出现的时序坑，不如从结构上避免。
  /** 打开一个已保存的请求；传 null 表示清空成新草稿 */
  openRequest: (req: ApiRequest | null) => void;
  /** 打开一份**没有对应已保存请求**的内容（历史快照回填） */
  openSpec: (spec: RequestSpec, name: string) => void;
  patch: (patch: Partial<RequestDraft>) => void;
  /** 整块替换键值表（params / headers 共用一套编辑器） */
  setRows: (which: "params" | "headers", rows: KeyValueItem[]) => void;

  send: () => Promise<void>;
  cancel: () => Promise<void>;
  /** 立刻落库（不等静默期）。切走/关窗前调 */
  flushSave: () => Promise<void>;
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
export const useRequestStore = create<RequestState>((set, get) => ({
  draft: emptyDraft(),
  sourceId: null,
  sourceName: null,
  dirty: false,
  runId: null,
  cancelling: false,
  response: null,
  error: null,
  cancelled: false,
  finishedAt: null,

  openRequest: (req) => {
    // 换请求前先把没落库的改动写回去 —— 否则「改完立刻点下一个」
    // 会静默丢改动，而且丢得毫无痕迹（切回来是旧值）
    cancelAutosave();
    void flushAutosave();

    // 换请求前把在跑的那次掐掉：否则「A 的响应」会落到「B 的界面」上。
    // 即发即忘 —— 后端把它当成一次正常的用户取消。
    void get().cancel();

    set({
      draft: req ? draftFrom(req) : emptyDraft(),
      sourceId: req?.id ?? null,
      sourceName: req?.name ?? null,
      dirty: false,
      // 结果跟着请求走：换了请求就不该再显示上一个的响应
      response: null,
      error: null,
      cancelled: false,
      finishedAt: null,
    });
  },

  openSpec: (spec, name) => {
    cancelAutosave();
    void flushAutosave();
    void get().cancel();

    set({
      draft: draftFromSpec(spec),
      // 关键：`sourceId = null` → 自动保存不会去写任何一行
      // （它没有可写的目标），这份内容只存在于编辑器里，
      // 用户想留下它得自己「另存为」
      sourceId: null,
      sourceName: name,
      dirty: false,
      response: null,
      error: null,
      cancelled: false,
      finishedAt: null,
    });
  },

  // 下面两个是唯一会改草稿的入口，所以「标脏 + 排自动保存」写在这里就够，
  // 不需要每个调用点都记得调一次
  patch: (patch) => {
    set((s) => ({ draft: { ...s.draft, ...patch }, dirty: true }));
    scheduleAutosave();
  },

  setRows: (which, rows) => {
    set((s) => ({ draft: { ...s.draft, [which]: rows }, dirty: true }));
    scheduleAutosave();
  },

  send: async () => {
    const { draft, runId, sourceId } = get();
    if (runId) return; // 已经在跑了，不叠加

    const id = newRunId();
    const spec = draftToSpec(draft);
    set({ runId: id, cancelling: false, error: null, cancelled: false });

    try {
      const response = await ipc.requests.send(id, spec);

      // 这一轮可能已经被顶掉（取消后马上又发了一次）——
      // 只认自己那一轮的结果，否则旧响应会盖到新请求头上
      if (get().runId !== id) return;
      set({
        response,
        runId: null,
        cancelling: false,
        finishedAt: Date.now(),
      });

      void recordHistory({ spec, sourceId, status: response.status, response });
    } catch (e) {
      const err = ipc.toIpcError(e);
      if (get().runId !== id) return;

      // 用户主动取消**不是错误**：弹一句「已取消」就够，不该走红色报错。
      // 也不写历史 —— 一次被取消的请求等于「没发生过」，
      // 让它占一行只会把真正有用的记录挤下去。
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

      void recordHistory({ spec, sourceId, status: null, error: err });
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

  flushSave: async () => {
    cancelAutosave();
    await flushAutosave();
  },

  clearResult: () =>
    set({
      response: null,
      error: null,
      cancelled: false,
      finishedAt: null,
    }),
}));

/**
 * 把一次执行写进历史。
 *
 * **成功与失败都写**：`status = null` + `error` 有值代表网络层失败，
 * 那恰恰是最值得回看的几条 —— 只记成功的历史等于把排查线索删掉了。
 *
 * 名字的取法：已保存的请求用它的名字（用户认的是这个名字），
 * 没保存过的临时请求退回用地址 —— 总不能让它显示成「未命名」，
 * 那样一列表全是同名行，等于没记。
 */
function recordHistory(input: {
  spec: RequestSpec;
  sourceId: number | null;
  status: number | null;
  /** 成功时的响应（用来取耗时与体积） */
  response?: HttpResponsePayload;
  /** 失败时的错误（消息只留**错误码**，文案归前端查表） */
  error?: ipc.IpcError;
}): void {
  const workspaceId = useWorkspaceStore.getState().workspace?.id;
  if (workspaceId === undefined) return; // 工作区还没加载完，静默跳过

  const saved = input.sourceId === null ? undefined : findSaved(input.sourceId);

  void useHistoryStore.getState().record({
    workspaceId,
    requestId: input.sourceId,
    requestName: saved?.name || input.spec.url,
    snapshot: input.spec,
    status: input.status,
    durationMs: input.response?.totalMs ?? null,
    respSize: input.response?.bodySize ?? null,
    error: input.error?.code ?? null,
    // P4 起记录了：历史列表里能看出「这条是用哪个环境跑的」。
    // 环境没激活时是 null —— 「不使用环境」是一个真实的、需要被记录的状态。
    environment: useEnvStore.getState().active?.name ?? null,
  });
}
