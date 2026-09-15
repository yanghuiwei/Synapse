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
import { rowsFrom, serializeUrlencoded } from "@/lib/urlencoded";
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
  /**
   * **当前** `bodyType` 对应的那份内容，也是唯一会被发出去的一份。
   *
   * 它与 [`RequestDraft.bodyByType`] 是「镜子与原件」：切类型时先把旧值
   * 写回原件、再从新那格取出来，所以切走再切回来内容还在。
   */
  body: string;
  /** 每种请求体类型各存一份（键是 `bodyType` 的字面量） */
  bodyByType: Record<string, string>;
  /**
   * 键值型请求体（urlencoded）的行。
   *
   * 只有一份 —— 当前也只有 urlencoded 一种类型用到它。将来 multipart
   * (`form`) 落地时**必须**改成按类型分槽，否则两者会串。
   */
  bodyRows: KeyValueItem[];
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
    bodyByType: {},
    bodyRows: [],
    // 新请求默认**继承分类**：绝大多数时候用户会在分类上配一次授权，
    // 然后希望下面所有请求自动用上
    auth: { type: "inherit" },
    settings: { ...DEFAULT_REQUEST_SETTINGS },
    folderId: null,
  };
}

/**
 * 把分槽快照补齐成「一定包含当前类型」的一份。
 *
 * 当前类型**无条件以 `body` 列的值覆盖**，而不是反过来 —— `body` 是库里
 * 其他写入者（MCP 的 `update_request`、curl 导入，以及以后任何脚本）唯一
 * 会碰的列，拿快照当权威会把它们刚写进去的内容吃掉，症状是
 * 「MCP 明明改了请求体，界面上还是旧的」。
 */
function bodySnapshot(
  bodyType: BodyType,
  body: string,
  raw: Record<string, string>,
): Record<string, string> {
  return { ...raw, [bodyType]: body };
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
    bodyByType: bodySnapshot(req.bodyType, req.body, req.bodyByType),
    bodyRows: rowsFrom(req.bodyType, req.bodyRows, req.body),
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
 *
 * `bodyByType` / `bodyRows` 后端**不读**（执行只看 `body`），带上它们是因为
 * 同一份 `RequestSpec` 也被当历史快照存下来 —— 不带的话，从历史里打开一条
 * urlencoded 请求，键值表会退化成一行纯文本。
 */
export function draftToSpec(draft: RequestDraft): RequestSpec {
  return {
    method: draft.method,
    url: draft.url,
    params: draft.params,
    headers: draft.headers,
    bodyType: draft.bodyType,
    body: draft.body,
    bodyByType: bodySnapshot(draft.bodyType, draft.body, draft.bodyByType),
    bodyRows: draft.bodyRows,
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
    bodyByType: bodySnapshot(spec.bodyType, spec.body, spec.bodyByType),
    bodyRows: rowsFrom(spec.bodyType, spec.bodyRows, spec.body),
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

/** 一次执行的结果快照（就是 state 里那四个字段） */
interface ResultSlot {
  response: HttpResponsePayload | null;
  error: ipc.IpcError | null;
  cancelled: boolean;
  finishedAt: number | null;
}

/**
 * 每条请求最近一次执行的结果。
 *
 * ## 为什么要存
 *
 * 响应区是按**请求身份**存在的：发完 A 去看 B，再切回 A，上次的响应应该还在。
 * 而 `openRequest` 会把草稿整个换掉 —— 结果不跟着存一份的话，切回来只剩空态，
 * 用户得重发一次才能再看一眼刚才的返回。
 *
 * ## 为什么放模块级 `Map` 而不是 store 的 state
 *
 * 它是**只写不读的旁存**：组件读的始终是 store 顶层那几个字段，
 * 槽只在「切走」「切回」两个瞬间被碰。放进 state 只会让每次发送都多触发
 * 一轮与渲染无关的更新。
 */
const resultSlots = new Map<string, ResultSlot>();

/**
 * 槽的上限，超了丢最早放入的。
 *
 * 一个槽里躺着完整的响应体（可能是几 MB），不设上限就是慢性泄漏 ——
 * 点开 200 个请求之后，内存里就真的压着 200 份响应。
 */
const MAX_RESULT_SLOTS = 40;

/**
 * 一条请求对应哪个槽。
 *
 * 只有**已保存的请求**才有槽：没保存的草稿没有稳定身份，切走再切回来
 * 连 URL 都可能变了，拿旧结果填上去是张冠李戴 —— 比空着更坏。
 * 历史快照同理（每次 `openSpec` 都是另一份内容），所以两者一律返回 `null`。
 */
function slotKey(sourceId: number | null): string | null {
  return sourceId === null ? null : `req:${sourceId}`;
}

/** 把一份结果存进它的槽。`key` 为 `null` 时什么都不做（见 `slotKey`） */
function stashResult(key: string | null, slot: ResultSlot): void {
  if (key === null) return;
  // 先删再插：`Map` 的迭代顺序就是插入顺序，这样「最早放入的」始终排在开头，
  // 反复读写的那个槽不会因为一直没动过而被当成旧的淘汰掉
  resultSlots.delete(key);
  resultSlots.set(key, slot);
  while (resultSlots.size > MAX_RESULT_SLOTS) {
    const oldest = resultSlots.keys().next();
    if (oldest.done) break;
    resultSlots.delete(oldest.value);
  }
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
    // 两个分槽字段**必须**一起落库：漏了 `bodyByType` 的表现是
    // 「保存过再切类型，别的类型的内容还是没了」—— 而那正是这次要修的问题本身
    bodyByType: bodySnapshot(draft.bodyType, draft.body, draft.bodyByType),
    bodyRows: draft.bodyRows,
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

  /**
   * 当前展示的那次执行的结果。
   *
   * 这几个字段**是按请求存的**：切到别的请求时它们会被存进 `resultSlots`，
   * 切回来再取出来 —— 所以「发完 A 去看 B，再切回 A」时 A 的响应还在。
   * 存取的时机只在 `openRequest` / `openSpec` 两处。
   */
  response: HttpResponsePayload | null;
  error: ipc.IpcError | null;
  /** 上一次执行是被用户取消的（不是失败，所以要区别对待） */
  cancelled: boolean;
  /** 上一次执行结束的时间戳，用于「刚刚 / 3 秒前」这类相对时间 */
  finishedAt: number | null;
  /**
   * 上面四个字段属于哪一条请求。
   *
   * `null` = 没有稳定归属（新建的空白草稿、历史快照），那两种情况的结果
   * 不缓存 —— 见 `slotKey` 里的理由。
   */
  resultKey: string | null;

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
  /**
   * 把当前草稿「另存为」一条正式请求，落到指定分类下。
   *
   * 为什么必须有这个动作：草稿（`sourceId === null`）没有归属分类，
   * 而 Token 来源**必须**挂在分类上（沿分类链向上找，见 Rust `http::pipeline`）——
   * 于是「粘个地址就发」这条路走到授权页就断了：那里只说「先在左侧选中一个分类」，
   * 而左侧根本没有「把这份内容收进某个分类」这个动作。
   */
  saveAs: (folderId: number | null, name: string) => Promise<ApiRequest>;
  patch: (patch: Partial<RequestDraft>) => void;
  /** 整块替换键值表（params / headers 共用一套编辑器） */
  setRows: (which: "params" | "headers", rows: KeyValueItem[]) => void;
  /**
   * 切换请求体类型。
   *
   * 这是**唯一**允许改 `bodyType` 的入口 —— 走 `patch({ bodyType })` 会跳过
   * 「把当前内容写回它自己那一格」这一步，切走再切回来内容就空了。
   */
  setBodyType: (bodyType: BodyType) => void;
  /** 整块替换 urlencoded 的键值行，并同步 `body`（后端只认它） */
  setBodyRows: (rows: KeyValueItem[]) => void;

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
  resultKey: null,

  openRequest: (req) => {
    // 换请求前先把没落库的改动写回去 —— 否则「改完立刻点下一个」
    // 会静默丢改动，而且丢得毫无痕迹（切回来是旧值）
    cancelAutosave();
    void flushAutosave();

    // 换请求前把在跑的那次掐掉：否则「A 的响应」会落到「B 的界面」上。
    // 即发即忘 —— 后端把它当成一次正常的用户取消。
    void get().cancel();

    // 结果跟着请求走：先把上一条的结果存进它的槽，再从新那条的槽里取回来
    const before = get();
    stashResult(before.resultKey, {
      response: before.response,
      error: before.error,
      cancelled: before.cancelled,
      finishedAt: before.finishedAt,
    });

    const resultKey = slotKey(req?.id ?? null);
    const slot = resultKey === null ? undefined : resultSlots.get(resultKey);

    set({
      draft: req ? draftFrom(req) : emptyDraft(),
      sourceId: req?.id ?? null,
      sourceName: req?.name ?? null,
      dirty: false,
      resultKey,
      // `runId` 必须在这里清掉：上面那次 `cancel()` 是异步的，它靠 `send()`
      // 收尾时「runId 还是不是自己」来判断有没有被顶掉。不清的话，
      // 被取消的旧请求会把「已取消」写到刚打开的新请求头上 ——
      // 而新请求的槽里明明还存着一份好好的响应，切回来就变成「已取消」了
      runId: null,
      cancelling: false,
      response: slot?.response ?? null,
      error: slot?.error ?? null,
      cancelled: slot?.cancelled ?? false,
      finishedAt: slot?.finishedAt ?? null,
    });
  },

  openSpec: (spec, name) => {
    cancelAutosave();
    void flushAutosave();
    void get().cancel();

    // 历史快照是一次性的（`slotKey` 拿到 null 就返回 null），这一份的结果不缓存；
    // 但**上一条**的结果要照常存好 —— 用户在历史里来回翻，切回某条请求时
    // 它上次的响应还得在
    const before = get();
    stashResult(before.resultKey, {
      response: before.response,
      error: before.error,
      cancelled: before.cancelled,
      finishedAt: before.finishedAt,
    });

    set({
      draft: draftFromSpec(spec),
      // 关键：`sourceId = null` → 自动保存不会去写任何一行
      // （它没有可写的目标），这份内容只存在于编辑器里，
      // 用户想留下它得自己「另存为」
      sourceId: null,
      sourceName: name,
      dirty: false,
      resultKey: null,
      // 与 `openRequest` 同理：在跑的那次已被上面掐掉，
      // 它的收尾不该再往这一份内容上写状态
      runId: null,
      cancelling: false,
      response: null,
      error: null,
      cancelled: false,
      finishedAt: null,
    });
  },

  /**
   * 两步落库不是啰嗦：`create` 只建一行**空**请求（名字与位置由它定），
   * 内容得靠 `save` 全量写一次 —— 合成一条 IPC 会让后端的 `create` 语义变复杂。
   */
  saveAs: async (folderId, name) => {
    // 先掐掉待写的自动保存：它要写的是「旧草稿 → 旧请求」那条路径，
    // 而这份内容马上要换个身份了
    cancelAutosave();

    const draft = get().draft;
    const created = await useWorkspaceStore.getState().createRequest(folderId, name);
    const saved = await ipc.requests.save({ ...created, ...draftFields(draft) });

    // 只补那一行，比整块 refresh 便宜（同 `flushAutosave` 的做法）
    useWorkspaceStore.getState().patchRequest(saved);
    get().openRequest(saved);
    return saved;
  },

  // 下面几个是唯一会改草稿的入口，所以「标脏 + 排自动保存」写在这里就够，
  // 不需要每个调用点都记得调一次
  patch: (next) => {
    set((s) => {
      const draft = { ...s.draft, ...next };
      // `body` 是当前类型那一格的**镜子**，改它就得同步原件 ——
      // 否则切走再切回来，取回的是改动之前的值
      if (next.body !== undefined) {
        draft.bodyByType = bodySnapshot(s.draft.bodyType, next.body, s.draft.bodyByType);
      }
      return { draft, dirty: true };
    });
    scheduleAutosave();
  },

  setRows: (which, rows) => {
    set((s) => ({ draft: { ...s.draft, [which]: rows }, dirty: true }));
    scheduleAutosave();
  },

  setBodyType: (bodyType) => {
    if (get().draft.bodyType === bodyType) return;

    set((s) => {
      const d = s.draft;
      // 先把当前这份写回它自己的格子，再取新那格 —— 顺序反过来等于没存
      const bodyByType = bodySnapshot(d.bodyType, d.body, d.bodyByType);
      // `bodyRows` 刻意**不动**：它只服务 urlencoded 一种类型，切到别处再切
      // 回来时那几行得原样还在（包括「未勾选」这种只存在于界面上的状态）。
      // 若在这里从 body 重新解析，未勾选的行就悄悄没了。
      return {
        draft: { ...d, bodyType, bodyByType, body: bodyByType[bodyType] ?? "" },
        dirty: true,
      };
    });
    scheduleAutosave();
  },

  setBodyRows: (rows) => {
    // 拼串在这里做一次，此后 `body` 就是权威 —— 保存、发送、压测三条路径
    // 都只读它（见 lib/urlencoded 里「为什么序列化放在前端」）
    const body = serializeUrlencoded(rows);
    set((s) => ({
      draft: {
        ...s.draft,
        bodyRows: rows,
        body,
        bodyByType: bodySnapshot(s.draft.bodyType, body, s.draft.bodyByType),
      },
      dirty: true,
    }));
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

  clearResult: () => {
    // 连槽一起清：只清顶层字段的话，切走再切回来那个被「关掉」的响应
    // 会从槽里复活，看起来像是清不掉
    const { resultKey } = get();
    if (resultKey !== null) resultSlots.delete(resultKey);
    set({
      response: null,
      error: null,
      cancelled: false,
      finishedAt: null,
    });
  },
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
