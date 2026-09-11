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

import { interpolate, type TranslateParams } from "./i18n";
import { zhCN, type MessageKey } from "./i18n/zh-CN";
import {
  apiRequestSchema,
  appInfoSchema,
  folderSchema,
  responsePayloadSchema,
  settingSchema,
  workspaceSchema,
} from "./schemas";
import type {
  ApiRequest,
  AppInfo,
  Folder,
  HttpResponsePayload,
  RequestSpec,
  Setting,
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
  // ---- 仅前端 ----
  E_NOT_TAURI: "error.notTauri",
  E_SCHEMA_DRIFT: "error.schemaDrift",
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

export const app = {
  info: (): Promise<AppInfo> =>
    call("get_app_info", undefined, { parse: (v) => appInfoSchema.parse(v) }),
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
};
