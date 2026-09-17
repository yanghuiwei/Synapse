/* ============================================================
   IPC 薄封装 —— 前端访问后端的**唯一**入口
   ------------------------------------------------------------
   这一层只做四件事，不做业务：
   1. 环境探测：不在 Tauri 里（比如浏览器直接开 dev server）时
      直接抛 E_NOT_TAURI，而不是让 invoke 抛一句 "undefined is not a function"
   2. 错误归一：把后端 `{code, params, debug}` 变成 `IpcError`，
      文案一律由前端查 `zh-CN` 表（契约见 design.md §9.1）
   3. 结构校验：可选地过一遍 zod，抓 Rust ↔ TS 的类型漂移
   4. 命令名集中登记：命令名写错时在**这里**改一次，而不是散落各处

   ⚠️ 契约：后端**只返回错误码**。这里出现的任何英文（如 entity 名）
      都必须翻译后再给用户看 —— 见 ENTITY_LABELS。
   ============================================================ */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { interpolate, type TranslateParams } from "./i18n";
import { zhCN, type MessageKey } from "./i18n/zh-CN";
import {
  apiRequestSchema,
  appInfoSchema,
  deleteFolderResultSchema,
  envVarSchema,
  environmentSchema,
  exportResultSchema,
  folderSchema,
  historyEntrySchema,
  historyPageSchema,
  importOutcomeSchema,
  importPreviewSchema,
  loadtestProgressEventSchema,
  loadtestChartSchema,
  loadtestRunDetailSchema,
  loadtestRunSchema,
  loadtestSampleSchema,
  mcpSetupSchema,
  purgeTrashResultSchema,
  purgeTreeResultSchema,
  previewCurlResultSchema,
  responsePayloadSchema,
  savedFileSchema,
  settingSchema,
  tokenSourceSchema,
  tokenStatusSchema,
  trashContentsSchema,
  workspaceSchema,
} from "./schemas";
import type {
  ApiRequest,
  AppInfo,
  AuthConfig,
  ConflictMode,
  DeleteFolderResult,
  EnvVar,
  Environment,
  ExportFormat,
  ExportResult,
  Folder,
  HistoryEntry,
  HistoryPage,
  HttpResponsePayload,
  ImportOutcome,
  ImportPreview,
  ImportSourceInput,
  LoadtestConfig,
  LoadtestChart,
  LoadtestProgressEvent,
  LoadtestRun,
  LoadtestRunDetail,
  LoadtestSample,
  LoadtestTarget,
  McpSetup,
  NewHistoryEntry,
  PreviewCurlResult,
  PurgeTrashResult,
  PurgeTreeResult,
  ReportLabels,
  RequestSpec,
  SavedFile,
  Setting,
  TokenSource,
  TokenStatus,
  TrashContents,
  Workspace,
} from "./types";

// ---------------------------------------------------------------
// 错误码
// ---------------------------------------------------------------

/**
 * 错误码 → 文案 key 的登记表。
 *
 * 上半部分必须与 `src-tauri/src/error.rs` 的 `codes` 模块**逐条对齐**；
 * 下半部分是前端自己产生的，后端永远不会返回。
 * 漏登记的码会退化成 `E_UNKNOWN`（并且这里 `satisfies` 会挡住写错的 key）。
 */
export const ERROR_KEYS = {
  // ---- 后端 codes（改这里必须同步改 error.rs） ----
  E_DB: "error.db",
  E_DB_LOCKED: "error.dbLocked",
  E_MIGRATION: "error.migration",
  E_NOT_FOUND: "error.notFound",
  E_INVALID_INPUT: "error.invalidInput",
  E_IO: "error.io",
  E_INTERNAL: "error.internal",
  // ---- 请求层（P2） ----
  //
  // DNS / 连接被拒 / TLS 分成三个码，是因为**用户动作完全不同**：
  // 域名错 → 改地址；连接被拒 → 服务没起或端口不对；TLS 失败 → 证书或自签。
  // 合成一句「网络错误」等于让用户自己猜。
  E_DNS: "error.dns",
  E_CONN_REFUSED: "error.connRefused",
  E_TLS: "error.tls",
  E_TIMEOUT: "error.timeout",
  E_REDIRECT: "error.redirect",
  E_DECODE: "error.decode",
  E_URL: "error.invalidUrl",
  E_REQUEST: "error.request",
  E_BODY_NOT_JSON: "error.bodyNotJson",
  E_CANCELLED: "error.cancelled",
  E_NETWORK: "error.network",
  E_NOT_IMPLEMENTED: "error.notImplemented",
  // ---- Token（P4） ----
  //
  // 这四条分得细，是因为**用户动作完全不同**：
  // 「没有 Token 来源」→ 去建一个；「刷新失败」→ 去修那个取 token 的请求；
  // 「提取失败」→ 改提取路径；「已过期且关掉了自动刷新」→ 打开开关或手动刷。
  // 合成一句「Token 出错了」等于让用户自己猜。
  E_TOKEN_SOURCE: "error.tokenSource",
  E_TOKEN_REFRESH: "error.tokenRefresh",
  E_TOKEN_EXTRACT: "error.tokenExtract",
  E_TOKEN_EXPIRED: "error.tokenExpired",
  // ---- 系统分类 ----
  //
  // 单开一个码而不是复用 `E_INVALID_INPUT`：这条不是「用户填错了什么」，
  // 而是「这行由程序托管」。界面该说的是「系统分类不能删」，
  // 而且前端靠它把菜单项直接置灰，不必等用户点了才报错。
  E_SYSTEM_FOLDER: "error.systemFolder",
  // ---- 导入器（P5） ----
  //
  // 三个码分开是因为**用户的下一个动作不同**：识别失败 = 这段东西我们压根不认
  // （可能粘错了）；JSON 不合法 = 文件选错了或坏了（重新导一次）；
  // 解析失败 = 格式认出来了，但里面某条解不动（去看警告列表定位）。
  E_IMPORT_DETECT: "error.importDetect",
  E_IMPORT_JSON: "error.importJson",
  E_IMPORT_PARSE: "error.importParse",
  // ---- 仅前端 ----
  E_NOT_TAURI: "error.notTauri",
  E_SCHEMA_DRIFT: "error.schemaDrift",
  /** 启动序列还没跑完就发起了写操作（正常操作流里不该出现） */
  E_WORKSPACE_MISSING: "error.workspaceMissing",
  E_UNKNOWN: "error.unknown",
} as const satisfies Record<string, MessageKey>;

export type ErrorCode = keyof typeof ERROR_KEYS;

/**
 * 可重试的瞬时故障。
 *
 * ⚠️ 必须与 Rust 侧 `AppError::retryable()` **逐条对齐**。两边不一致的话，
 * 就会出现「后端说能重试、前端不给重试按钮」这种查半天查不出来的鬼故事。
 */
const RETRYABLE_CODES: ReadonlySet<ErrorCode> = new Set([
  "E_DB_LOCKED",
  "E_TIMEOUT",
  "E_NETWORK",
  "E_CONN_REFUSED",
]);

/**
 * 后端 `AppError::NotFound { entity }` 里的 entity 是 **Rust 侧英文名**，
 * 直接透给用户就是「找不到 loadtest_run #12」这种半中半英。
 * 这张表负责把它翻成人话。
 */
const ENTITY_LABELS: Record<string, string> = {
  workspace: "工作区",
  folder: "文件夹",
  request: "请求",
  environment: "环境",
  env_var: "环境变量",
  token_source: "Token 来源",
  history: "历史记录",
  loadtest_run: "压测记录",
  setting: "设置项",
};

export function isErrorCode(value: unknown): value is ErrorCode {
  return typeof value === "string" && value in ERROR_KEYS;
}

/** zod（或任何同形状的校验器）只要能 parse 出 T 就够用 */
export interface Parser<T> {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  parse: (value: any) => T;
}

export interface IpcErrorInit {
  code: string;
  params?: Record<string, unknown>;
  debug?: string;
}

/**
 * 统一的错误类型。
 *
 * `Error.message` 只给控制台看（保持英文/原始信息，方便搜 issue），
 * 用户可见的文案走 `localized`。
 */
export class IpcError extends Error {
  readonly code: ErrorCode;
  readonly params: Record<string, unknown>;
  readonly debug: string;

  constructor(init: IpcErrorInit) {
    const code: ErrorCode = isErrorCode(init.code) ? init.code : "E_UNKNOWN";
    super(init.debug ? `${code}: ${init.debug}` : code);
    this.name = "IpcError";
    this.code = code;
    this.params = init.params ?? {};
    this.debug = init.debug ?? "";
  }

  /**
   * 是否值得让用户重试。
   *
   * 与 Rust 侧 `AppError::retryable()` 是同一份清单 —— 见 `RETRYABLE_CODES`。
   */
  get retryable(): boolean {
    return RETRYABLE_CODES.has(this.code);
  }

  /**
   * 面向用户的文案（已插值）。
   *
   * 目前直接查中文表：这一版只有一种语言。
   * 将来做多语言时，**渲染处**改用 `useI18n().t(ERROR_KEYS[err.code], err.uiParams)`
   * 即可 —— `code` 是稳定标识，本来就不该被语言绑死。
   */
  get localized(): string {
    // 少数错误码在**带了额外参数**时有更精确的说法。这里显式列举而不是
    // 给插值器加「缺参数就删掉那句话」的花招 —— 那种隐式行为会让
    // `zh-CN.ts` 里读不出文案到底长什么样。
    if (this.code === "E_TOKEN_REFRESH" && typeof this.params.status === "number") {
      return interpolate(zhCN["error.tokenRefreshHttp"], this.uiParams);
    }
    return interpolate(zhCN[ERROR_KEYS[this.code]], this.uiParams);
  }

  /** 插值参数：把 English entity 翻成中文，非字符串的值转成字符串 */
  get uiParams(): TranslateParams {
    const out: TranslateParams = {};
    for (const [k, v] of Object.entries(this.params)) {
      if (k === "entity" && typeof v === "string") {
        out[k] = ENTITY_LABELS[v] ?? v;
      } else if (typeof v === "string" || typeof v === "number") {
        out[k] = v;
      } else if (v !== null && v !== undefined) {
        out[k] = String(v);
      }
    }
    return out;
  }
}

/** 把任意 catch 到的东西归一成 IpcError */
export function toIpcError(raw: unknown): IpcError {
  if (raw instanceof IpcError) return raw;

  // 后端序列化过来的是 `{ code, params?, debug? }`
  if (typeof raw === "object" && raw !== null && "code" in raw) {
    const o = raw as Record<string, unknown>;
    return new IpcError({
      code: String(o.code),
      params:
        typeof o.params === "object" && o.params !== null
          ? (o.params as Record<string, unknown>)
          : undefined,
      debug: typeof o.debug === "string" ? o.debug : undefined,
    });
  }

  // 字符串 / Error / 其它 —— 都没有错误码，只能落到 E_UNKNOWN，
  // 但把原文留在 debug 里，便于在控制台定位
  const debug =
    typeof raw === "string"
      ? raw
      : raw instanceof Error
        ? raw.message
        : JSON.stringify(raw);
  return new IpcError({ code: "E_UNKNOWN", debug });
}

// ---------------------------------------------------------------
// 调用入口
// ---------------------------------------------------------------

/** 是否运行在 Tauri 里。Tauri 2 会在 window 上挂 `__TAURI_INTERNALS__`。 */
export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * 调一个后端命令。
 *
 * @param command Rust 侧注册的命令名（snake_case，与 `generate_handler!` 一致）
 * @param args    参数名用 **camelCase**（Tauri 2 自动映射到 Rust 的 snake_case）
 * @param parser  可选的 zod schema，用来抓类型漂移
 */
export async function call<T>(
  command: string,
  args?: Record<string, unknown>,
  parser?: Parser<T>,
): Promise<T> {
  if (!isTauriRuntime()) {
    throw new IpcError({
      code: "E_NOT_TAURI",
      debug: `command \`${command}\`: window.__TAURI_INTERNALS__ is missing`,
    });
  }

  let raw: unknown;
  try {
    raw = await invoke(command, args);
  } catch (e) {
    throw toIpcError(e);
  }

  if (!parser) return raw as T;

  try {
    return parser.parse(raw);
  } catch (e) {
    const detail = e instanceof Error ? e.message : String(e);
    throw new IpcError({
      code: "E_SCHEMA_DRIFT",
      debug: `command \`${command}\` returned unexpected shape: ${detail}`,
    });
  }
}

// ---------------------------------------------------------------
// 命令封装 —— 名字只在这里出现一次
// ---------------------------------------------------------------

export const settings = {
  /** 取单个设置；不存在时返回 null（不是错误） */
  get: (key: string): Promise<string | null> => call("get_setting", { key }),

  set: async (key: string, value: string): Promise<void> => {
    await call<null>("set_setting", { key, value });
  },

  /** 启动时一次拉全部，避免为每个键各跑一趟 IPC */
  all: (): Promise<Setting[]> =>
    call("get_all_settings", undefined, { parse: (v) => settingSchema.array().parse(v) }),

  remove: (key: string): Promise<boolean> => call("delete_setting", { key }),
};

/**
 * 「库被别的进程改过」的事件名。
 *
 * ⚠️ 必须与 Rust 侧 `src-tauri/src/external_changes.rs` 的 `DB_CHANGED_EVENT`
 * 逐字一致 —— 它是两边的唯一约定，写错了不会有任何报错，只是界面永远不刷新。
 */
const DB_CHANGED_EVENT = "reqly://db-changed";

export const app = {
  info: (): Promise<AppInfo> =>
    call("get_app_info", undefined, { parse: (v) => appInfoSchema.parse(v) }),

  /**
   * 订阅「库被别的进程改过」。
   *
   * 载荷是空的：这一层只回答「变了」，**不回答「什么变了」**。
   * 想知道那种粒度就得让后端算出一份结构化 diff，而重拉一次文件夹 + 请求列表
   * 在本地库上是亚毫秒级的 —— 不值得为省这点开销去维护一份变更日志。
   *
   * 返回退订函数（与 `loadtests.onProgress` 同一套约定），不在 Tauri 里时为 no-op。
   */
  onDbChanged: (handler: () => void): Promise<UnlistenFn> => {
    if (!isTauriRuntime()) return Promise.resolve(() => {});
    return listen<null>(DB_CHANGED_EVENT, () => handler());
  },
};

/**
 * 文件对话框（P8）。
 *
 * 两件事都在 Rust 侧弹框 / 落盘，前端只拿结果 —— 理由见
 * `src-tauri/src/commands/file.rs` 的模块头。这里只保持「取消」与「失败」
 * 可区分：取消返回 `null`，不抛错。
 *
 * 「打开所在目录」不在这里：`@tauri-apps/plugin-opener` 的 `revealItemInDir`
 * 前端本来就能直接调（`opener:default` 已开），不必再包一层。
 */
export const files = {
  /** 选一个文件，回绝对路径。用户取消 → `null` */
  pickFile: async (): Promise<string | null> => {
    const raw = await call<unknown>("pick_file");
    return typeof raw === "string" ? raw : null;
  },

  /**
   * 把一份「文件型」响应存到用户选的位置。
   *
   * `sourcePath` 必须是后端在本次执行里落的临时文件（后端会校验前缀）——
   * 拿别处的路径调它会直接被拒。
   */
  saveResponseBody: (sourcePath: string, fileName: string): Promise<SavedFile | null> =>
    call("save_response_body", { sourcePath, fileName }, {
      parse: (v) => savedFileSchema.nullable().parse(v),
    }),
};

/**
 * MCP Server 的连接信息（P7）。
 *
 * 只有「取片段」这一个读操作 —— MCP 进程的启停由客户端负责，
 * 桌面端既不需要也不该去管它（设计上两者只共享那个 SQLite 文件）。
 */
export const mcp = {
  setup: (): Promise<McpSetup> =>
    call("get_mcp_setup", undefined, { parse: (v) => mcpSetupSchema.parse(v) }),
};

export const workspaces = {
  list: (): Promise<Workspace[]> =>
    call("list_workspaces", undefined, { parse: (v) => workspaceSchema.array().parse(v) }),

  /** 当前工作区。首次启动时后端已经保证存在，不需要处理「没有」的情况。 */
  active: (): Promise<Workspace> =>
    call("get_active_workspace", undefined, { parse: (v) => workspaceSchema.parse(v) }),
};

export const folders = {
  list: (workspaceId: number): Promise<Folder[]> =>
    call("list_folders", { workspaceId }, { parse: (v) => folderSchema.array().parse(v) }),

  create: (workspaceId: number, parentId: number | null, name: string): Promise<Folder> =>
    call("create_folder", { workspaceId, parentId, name }, { parse: (v) => folderSchema.parse(v) }),

  rename: (id: number, name: string): Promise<Folder> =>
    call("rename_folder", { id, name }, { parse: (v) => folderSchema.parse(v) }),

  /**
   * 设置分类的授权（P4）。这是**继承链的数据源** ——
   * 子分类/请求上配 `inherit` 就是沿链向上找第一个非 `inherit` 的。
   */
  setAuth: (id: number, auth: AuthConfig): Promise<Folder> =>
    call("set_folder_auth", { id, auth }, { parse: (v) => folderSchema.parse(v) }),

  /** `parentId = null` 表示移到根。移进自己的子孙会被后端拒绝。 */
  move: (id: number, parentId: number | null): Promise<Folder> =>
    call("move_folder", { id, parentId }, { parse: (v) => folderSchema.parse(v) }),

  /** 级联软删除：整棵子树 + 子树下的请求一起进回收站 */
  remove: (id: number): Promise<DeleteFolderResult> =>
    call("delete_folder", { id }, { parse: (v) => deleteFolderResultSchema.parse(v) }),

  restore: (id: number): Promise<Folder> =>
    call("restore_folder", { id }, { parse: (v) => folderSchema.parse(v) }),

  /** 彻底删除（含子树与请求）。**不可撤销** */
  purge: (id: number): Promise<PurgeTreeResult> =>
    call("purge_folder", { id }, { parse: (v) => purgeTreeResultSchema.parse(v) }),
};

export const requests = {
  list: (workspaceId: number): Promise<ApiRequest[]> =>
    call("list_requests", { workspaceId }, { parse: (v) => apiRequestSchema.array().parse(v) }),

  /**
   * 发一次请求。
   *
   * `runId` 由调用方生成并保管 —— `cancel` 要靠它找回这次执行。
   * 这个 id 是一次执行的**身份**：将来写历史、跑压测都靠它把
   * 「请求 → 响应」串起来，所以不要用数组下标之类会重号的东西。
   */
  send: (runId: string, spec: RequestSpec): Promise<HttpResponsePayload> =>
    call("send_request", { runId, spec }, { parse: (v) => responsePayloadSchema.parse(v) }),

  /**
   * 打断一次进行中的请求。
   *
   * 返回 `false` = 那次执行已经结束了（或 id 从来不存在），**不是错误** ——
   * 用户点取消时请求刚好回完是很常见的事，按「本来就没在跑」处理即可。
   */
  cancel: (runId: string): Promise<boolean> => call("cancel_request", { runId }),

  /**
   * 按**当前编辑内容**推演出一条 curl，不发送。
   *
   * 用在「这条请求还没跑过」或「历史里那条太老」的时候。
   *
   * ⚠️ 它不是纯只读的：解析过程里 token 若已过期会**顺带刷新一次**
   * （不然生成出来的是一串已经过期的 token，粘出去跑还是 401，
   * 「核对」这件事就没意义了）。但它**不会**写历史 ——
   * 历史只记录真实发送过的东西。
   */
  previewCurl: (spec: RequestSpec): Promise<PreviewCurlResult> =>
    call("preview_curl", { spec }, { parse: (v) => previewCurlResultSchema.parse(v) }),

  create: (workspaceId: number, folderId: number | null, name: string): Promise<ApiRequest> =>
    call("create_request", { workspaceId, folderId, name }, { parse: (v) => apiRequestSchema.parse(v) }),

  /** 全量保存。编辑器每次改动后落库走这里。 */
  save: (request: ApiRequest): Promise<ApiRequest> =>
    call("save_request", { request }, { parse: (v) => apiRequestSchema.parse(v) }),

  rename: (id: number, name: string): Promise<ApiRequest> =>
    call("rename_request", { id, name }, { parse: (v) => apiRequestSchema.parse(v) }),

  /** 复制到同一个分类下。名字由前端拼（后端不产出「副本」这类文案）。 */
  duplicate: (id: number, name: string): Promise<ApiRequest> =>
    call("duplicate_request", { id, name }, { parse: (v) => apiRequestSchema.parse(v) }),

  /** `folderId = null` 表示移出到「未分类」 */
  move: (id: number, folderId: number | null): Promise<ApiRequest> =>
    call("move_request", { id, folderId }, { parse: (v) => apiRequestSchema.parse(v) }),

  /** 软删除，返回被删的那一行 —— 界面要用它显示「已移到回收站」 */
  remove: (id: number): Promise<ApiRequest> =>
    call("delete_request", { id }, { parse: (v) => apiRequestSchema.parse(v) }),

  restore: (id: number): Promise<ApiRequest> =>
    call("restore_request", { id }, { parse: (v) => apiRequestSchema.parse(v) }),

  /** 彻底删除。**不可撤销** */
  purge: async (id: number): Promise<void> => {
    await call<null>("purge_request", { id });
  },
};

export const history = {
  /**
   * 一页历史。`keyword` 为空白等同不过滤。
   *
   * `limit` / `offset` 后端会夹紧（limit ∈ [1,200]，offset ≥ 0），
   * 前端算错也不会拿到空列表。
   */
  list: (
    workspaceId: number,
    opts: { keyword?: string; limit?: number; offset?: number } = {},
  ): Promise<HistoryPage> =>
    call(
      "list_history",
      {
        workspaceId,
        keyword: opts.keyword ?? null,
        limit: opts.limit ?? null,
        offset: opts.offset ?? null,
      },
      { parse: (v) => historyPageSchema.parse(v) },
    ),

  get: (id: number): Promise<HistoryEntry> =>
    call("get_history", { id }, { parse: (v) => historyEntrySchema.parse(v) }),

  /**
   * 某条请求**最近一次带 curl 的**执行。
   *
   * 返回 `null` 是正常情况（这条请求还没跑过，或跑过但都在本版之前）——
   * 调用方据此退化成「按当前内容生成」，不要当成错误。
   */
  latestCurl: (requestId: number): Promise<HistoryEntry | null> =>
    call(
      "latest_history_curl",
      { requestId },
      { parse: (v) => (v === null ? null : historyEntrySchema.parse(v)) },
    ),

  /**
   * 写一条历史（发送结束后调用）。
   *
   * **成功与失败都要写**：`status = null` + `error` 有值代表网络层失败，
   * 那恰恰是最值得回看的几条。用户主动取消不写。
   */
  record: (entry: NewHistoryEntry): Promise<HistoryEntry> =>
    call("record_history", { entry }, { parse: (v) => historyEntrySchema.parse(v) }),

  /** 移除一条（进回收站） */
  remove: async (id: number): Promise<void> => {
    await call<null>("delete_history", { id });
  },

  restore: (id: number): Promise<HistoryEntry> =>
    call("restore_history", { id }, { parse: (v) => historyEntrySchema.parse(v) }),

  /** 彻底删除一条。**不可撤销** */
  purge: async (id: number): Promise<void> => {
    await call<null>("purge_history", { id });
  },

  /** 清空历史（全部进回收站），返回条数 */
  clearAll: (workspaceId: number): Promise<number> =>
    call("clear_history", { workspaceId }),
};

export const trash = {
  /** 回收站的三块内容一次拿回 */
  list: (workspaceId: number): Promise<TrashContents> =>
    call("list_trash", { workspaceId }, { parse: (v) => trashContentsSchema.parse(v) }),

  /** 清空回收站（**硬删除**） */
  purge: (workspaceId: number): Promise<PurgeTrashResult> =>
    call("purge_trash", { workspaceId }, { parse: (v) => purgeTrashResultSchema.parse(v) }),
};

// ---------------------------------------------------------------
// 环境与变量（P4）
// ---------------------------------------------------------------

export const envs = {
  list: (workspaceId: number): Promise<Environment[]> =>
    call("list_environments", { workspaceId }, {
      parse: (v) => environmentSchema.array().parse(v),
    }),

  /** 当前激活的环境。`null` = 不使用环境（**不是错误**） */
  active: (workspaceId: number): Promise<Environment | null> =>
    call("get_active_environment", { workspaceId }, {
      parse: (v) => environmentSchema.nullable().parse(v),
    }),

  create: (workspaceId: number, name: string): Promise<Environment> =>
    call("create_environment", { workspaceId, name }, {
      parse: (v) => environmentSchema.parse(v),
    }),

  rename: (id: number, name: string): Promise<Environment> =>
    call("rename_environment", { id, name }, { parse: (v) => environmentSchema.parse(v) }),

  /** `id = null` 表示「不使用环境」。唯一真源是 `environment.is_active` 列。 */
  setActive: async (workspaceId: number, id: number | null): Promise<void> => {
    await call<null>("set_active_environment", { workspaceId, id });
  },

  /** 软删除（进回收站） */
  remove: async (id: number): Promise<void> => {
    await call<null>("delete_environment", { id });
  },

  restore: (id: number): Promise<Environment> =>
    call("restore_environment", { id }, { parse: (v) => environmentSchema.parse(v) }),

  /** 彻底删除。**不可撤销**（变量由外键级联带走） */
  purge: async (id: number): Promise<void> => {
    await call<null>("purge_environment", { id });
  },

  vars: (environmentId: number): Promise<EnvVar[]> =>
    call("list_env_vars", { environmentId }, { parse: (v) => envVarSchema.array().parse(v) }),

  /**
   * 整表替换变量。
   *
   * 整表提交而不是逐行增删改：少一套 API 就少一套需要保持一致的语义。
   * 空白键由后端**丢弃**（不报错）—— 「加一行」的默认状态就是空行。
   */
  replaceVars: (environmentId: number, vars: EnvVar[]): Promise<EnvVar[]> =>
    call("replace_env_vars", { environmentId, vars }, {
      parse: (v) => envVarSchema.array().parse(v),
    }),
};

// ---------------------------------------------------------------
// Token 来源（P4）
// ---------------------------------------------------------------

export const tokens = {
  list: (workspaceId: number): Promise<TokenSource[]> =>
    call("list_token_sources", { workspaceId }, {
      parse: (v) => tokenSourceSchema.array().parse(v),
    }),

  create: (
    workspaceId: number,
    folderId: number,
    name: string,
    fetchRequestId: number,
  ): Promise<TokenSource> =>
    call("create_token_source", { workspaceId, folderId, name, fetchRequestId }, {
      parse: (v) => tokenSourceSchema.parse(v),
    }),

  /** 全量保存 */
  save: (source: TokenSource): Promise<TokenSource> =>
    call("save_token_source", { source }, { parse: (v) => tokenSourceSchema.parse(v) }),

  /** 硬删除（Token 配置属于「机器用的配置」，不进回收站） */
  remove: async (id: number): Promise<void> => {
    await call<null>("delete_token_source", { id });
  },

  /**
   * 某条请求**实际生效**的 Token 来源（沿分类链向上找第一个）。
   *
   * 用它把「当前生效的是哪个」摆到界面上 —— 否则用户对着三个配置
   * 只能靠猜哪个在起作用。
   */
  resolve: (folderId: number | null): Promise<TokenSource | null> =>
    call("resolve_token_source", { folderId }, {
      parse: (v) => tokenSourceSchema.nullable().parse(v),
    }),

  status: (id: number): Promise<TokenStatus> =>
    call("get_token_status", { id }, { parse: (v) => tokenStatusSchema.parse(v) }),

  /** 强制刷新，忽略缓存。走的是同一个单飞锁 —— 不会变成第 6 次网络请求。 */
  refresh: (id: number): Promise<TokenStatus> =>
    call("refresh_token", { id }, { parse: (v) => tokenStatusSchema.parse(v) }),

  /**
   * 一次拿回所有来源的状态。
   *
   * 没有做成一条后端命令：来源本来就只有个位数（一个工作区通常 1~3 个），
   * 而 `get_token_status` 是纯读缓存、不触发网络。为省几次 IPC 加一条
   * 聚合命令，换来的是两处需要同步的语义。
   */
  statuses: async (sources: readonly TokenSource[]): Promise<Map<number, TokenStatus>> => {
    const rows = await Promise.all(
      sources.map(async (s) => [s.id, await tokens.status(s.id)] as const),
    );
    return new Map(rows);
  },
};

// ---------------------------------------------------------------
// 导入器（P5）
// ---------------------------------------------------------------

export const imports = {
  /**
   * 只看不写：解析 + 与库里比对，回一棵差异树。
   *
   * `conflict` 是参数而不是常量：差异树**随冲突模式变**（副本模式会把
   * 同名的算成新增，覆盖模式会多出一批「移除」），所以切换模式要重算。
   *
   * 这里**没有**「是否导入环境」的参数：预览永远列出文件里带的环境，
   * 用户才知道原来还有一批变量可以一起导。真写不写由提交那一步的开关决定。
   */
  preview: (source: ImportSourceInput, conflict: ConflictMode): Promise<ImportPreview> =>
    call(
      "preview_import",
      { source, conflict },
      { parse: (v) => importPreviewSchema.parse(v) },
    ),

  /**
   * 落库。
   *
   * 会**重新解析一遍**（预览到落库之间用户可能换了文件），
   * 但走的是同一套计划逻辑，所以名字与冲突判定和预览完全一致 ——
   * 界面上看到什么，落库就是什么。
   */
  commit: (
    source: ImportSourceInput,
    conflict: ConflictMode,
    includeEnvironments: boolean,
  ): Promise<ImportOutcome> =>
    call(
      "commit_import",
      { source, conflict, includeEnvironments },
      { parse: (v) => importOutcomeSchema.parse(v) },
    ),
};

// ---------------------------------------------------------------
// 压测（P6）
// ---------------------------------------------------------------

/**
 * 进度事件名。**必须与 Rust 侧 `commands::loadtest::PROGRESS_EVENT` 逐字一致** ——
 * 它是两边的唯一约定，写错了不会有任何报错，只是进度条永远不动。
 */
const LOADTEST_PROGRESS_EVENT = "reqly://loadtest-progress";

export const loadtests = {
  /**
   * 跑一轮压测。**这个 Promise 要等到整轮结束才 resolve**（可能几十秒），
   * 过程中的数字靠 `onProgress` 推。
   *
   * `token` 由调用方生成并保管：它是这一轮在事件流与取消里的身份。
   * 后端在跑的过程中还不知道数据库的 run id，所以不能拿它当标识。
   */
  start: (
    token: string,
    target: LoadtestTarget,
    spec: RequestSpec,
    config: LoadtestConfig,
  ): Promise<LoadtestRunDetail> =>
    call(
      "start_loadtest",
      { token, target, spec, config },
      { parse: (v) => loadtestRunDetailSchema.parse(v) },
    ),

  /**
   * 取消当前这一轮。返回 `false` = 本来就没在跑（**不是错误**）。
   *
   * 不传 token：同时只允许一轮压测，所以「取消谁」没有歧义。
   * 取消后已经在飞的请求不打断、自然结束，所以超时 30 秒时最多还要等 30 秒。
   */
  cancel: (): Promise<boolean> => call("cancel_loadtest"),

  /** 历史压测列表，新的在前 */
  runs: (limit?: number, offset?: number): Promise<LoadtestRun[]> =>
    call(
      "list_loadtest_runs",
      { limit, offset },
      { parse: (v) => loadtestRunSchema.array().parse(v) },
    ),

  /** 一轮的详情。打开历史记录时用 */
  run: (runId: number): Promise<LoadtestRunDetail> =>
    call("get_loadtest_run", { runId }, { parse: (v) => loadtestRunDetailSchema.parse(v) }),

  /** 一轮的采样明细（分页）。列表与图表用 */
  samples: (runId: number, limit?: number, offset?: number): Promise<LoadtestSample[]> =>
    call(
      "get_loadtest_samples",
      { runId, limit, offset },
      { parse: (v) => loadtestSampleSchema.array().parse(v) },
    ),

  remove: async (runId: number): Promise<void> => {
    await call<null>("delete_loadtest_run", { runId });
  },

  /**
   * 一轮的图表数据（时间线 / 分布 / 最慢 / 失败）。
   *
   * 聚合在后端算 —— 前端拿到的只有几十个点，而不是几十万条明细。
   */
  chart: (runId: number): Promise<LoadtestChart> =>
    call("get_loadtest_chart", { runId }, { parse: (v) => loadtestChartSchema.parse(v) }),
  /**
   * 导出报告。返回**落盘的绝对路径** —— 后端写在用户的下载目录里，
   * 前端拿这个路径去「打开所在目录」。
   *
   * `labels` 是报表里的全部文案，来自 `zh-CN.ts`。报表是唯一「后端产文本」
   * 的场景，但文案来源仍然只有一个。
   */
  export: (runId: number, format: ExportFormat, labels: ReportLabels): Promise<ExportResult> =>
    call(
      "export_loadtest_report",
      { runId, format, labels },
      { parse: (v) => exportResultSchema.parse(v) },
    ),

  /**
   * 订阅进度事件。返回退订函数，**必须在组件卸载 / 换轮次时调用** ——
   * 不退订的表现是每跑一轮就多一个监听者，进度回调被重复触发 N 次。
   */
  onProgress: (handler: (event: LoadtestProgressEvent) => void): Promise<UnlistenFn> => {
    if (!isTauriRuntime()) return Promise.resolve(() => {});

    return listen<unknown>(LOADTEST_PROGRESS_EVENT, (e) => {
      const parsed = loadtestProgressEventSchema.safeParse(e.payload);
      if (!parsed.success) {
        // 结构对不上就丢掉这一帧，而不是往上抛：进度只是显示，
        // 一个坏的进度帧不该把正在跑的那一轮（或整个面板）弄崩。
        console.warn("[reqly] 压测进度事件结构异常：", parsed.error.message);
        return;
      }
      handler(parsed.data);
    });
  },
};
