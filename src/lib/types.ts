/**
 * 数据模型 —— 与 `src-tauri/src/storage/models.rs` **一一对应**。
 *
 * 约定：
 * - 字段名两边都是 camelCase（Rust 侧用 `#[serde(rename_all = "camelCase")]`）
 * - 时间戳是 ISO8601 带毫秒 + `Z`（如 `2026-09-11T06:00:00.000Z`），
 *   直接 `new Date(s)` 即可，不需要补时区
 * - `null` 表示的「无」在两边都用 `| null`，不用 `undefined`
 *   （JSON 里 undefined 会消失，null 不会，语义更可靠）
 *
 * ⚠️ 改这里必须同步改 Rust，反之亦然。两边没有自动同步机制，
 *    所以 `src/lib/ipc.ts` 提供了可选的 zod 校验，用来在运行时抓出漂移。
 */

// ============================================================
// 请求内的值类型
// ============================================================

/** params / headers / form 共用的键值对行 */
export interface KeyValueItem {
  key: string;
  value: string;
  /** 未勾选的行不参与请求 */
  enabled: boolean;
  /** 备注（Postman 的 description） */
  desc?: string;
}

export type BodyType =
  | "none"
  | "json"
  | "form"
  | "urlencoded"
  | "raw"
  | "binary"
  | "graphql";

export type HttpVersion = "auto" | "http1" | "http2";

export type ExpiryUnit = "seconds" | "millis" | "absolute";

/** 逐请求的发送设置，对应 `request.settings` JSON 列 */
export interface RequestSettings {
  followRedirects: boolean;
  maxRedirects: number;
  /** 是否校验 TLS 证书；公司内网自签证书场景要关掉 */
  verifyTls: boolean;
  timeoutMs: number;
  proxy?: string | null;
  httpVersion: HttpVersion;
}

// ============================================================
// 实体
// ============================================================

export interface Workspace {
  id: number;
  /** 可能为空串 —— 此时前端用 i18n 的 `workspace.default` 显示 */
  name: string;
  sortOrder: number;
  createdAt: string;
  updatedAt: string;
  deletedAt: string | null;
}

export interface Folder {
  id: number;
  workspaceId: number;
  /** null = 根级 */
  parentId: number | null;
  name: string;
  sortOrder: number;
  /** 是否被标记为 Token 源分组 */
  isTokenSource: boolean;
  createdAt: string;
  updatedAt: string;
  deletedAt: string | null;
}

export interface ApiRequest {
  id: number;
  folderId: number | null;
  workspaceId: number;
  name: string;
  method: string;
  url: string;
  params: KeyValueItem[];
  headers: KeyValueItem[];
  bodyType: BodyType;
  body: string;
  /** TODO(P2/P4)：AuthConfig 结构待定稿，当前只做透传 */
  auth: Record<string, unknown>;
  settings: RequestSettings;
  preScript: string;
  testScript: string;
  sortOrder: number;
  createdAt: string;
  updatedAt: string;
  deletedAt: string | null;
}

export interface Environment {
  id: number;
  workspaceId: number;
  name: string;
  isActive: boolean;
  createdAt: string;
  updatedAt: string;
  deletedAt: string | null;
}

export interface EnvVar {
  id: number;
  environmentId: number;
  key: string;
  value: string;
  isSecret: boolean;
  enabled: boolean;
  sortOrder: number;
}

export interface TokenSource {
  id: number;
  workspaceId: number;
  folderId: number;
  name: string;
  fetchRequestId: number;
  accessPath: string;
  refreshPath: string;
  expiresInPath: string;
  expiresInUnit: ExpiryUnit;
  refreshRequestId: number | null;
  autoRefresh: boolean;
  skewSeconds: number;
  createdAt: string;
  updatedAt: string;
}

export interface TokenCache {
  id: number;
  tokenSourceId: number;
  accessToken: string;
  refreshToken: string | null;
  /** ISO8601；null = 永不过期 */
  expiresAt: string | null;
  rawResponse: string | null;
  updatedAt: string;
}

export interface HistoryEntry {
  id: number;
  workspaceId: number;
  requestId: number | null;
  requestName: string;
  snapshot: Record<string, unknown>;
  /** null = 网络层就失败了 */
  status: number | null;
  durationMs: number | null;
  respSize: number | null;
  error: string | null;
  environment: string | null;
  executedAt: string;
  /** 非空 = 在回收站 */
  deletedAt: string | null;
}

export interface LoadtestRun {
  id: number;
  requestId: number | null;
  requestName: string;
  config: Record<string, unknown>;
  summary: Record<string, unknown>;
  startedAt: string;
  finishedAt: string | null;
}

export interface LoadtestSample {
  id: number;
  runId: number;
  seq: number;
  status: number | null;
  durationMs: number;
  bytes: number | null;
  error: string | null;
  startedOffsetMs: number;
}

export interface Setting {
  key: string;
  value: string;
  updatedAt: string;
}

/** `get_app_info` 的返回 */
export interface AppInfo {
  dbPath: string;
  schemaVersion: number;
  latestSchemaVersion: number;
  appVersion: string;
}

// ============================================================
// 请求执行（P2）
// ============================================================

/**
 * 发给后端的请求定义，对应 Rust 的 `http::RequestSpec`。
 *
 * 刻意**不是** `ApiRequest`：这样「还没保存的新请求」也能直接发 ——
 * 这是 Postman 类工具的核心体验，保存和发送是两件事。
 */
export interface RequestSpec {
  method: string;
  url: string;
  params: KeyValueItem[];
  headers: KeyValueItem[];
  bodyType: BodyType;
  body: string;
  settings: RequestSettings;
}

export interface HttpHeader {
  name: string;
  value: string;
}

/**
 * 响应体的解读方式。
 *
 * `binary` 时 `body` 是空串：一份二进制内容被有损转成字符串后界面就是乱码，
 * 用户会以为是工具坏了。宁可明说「这是二进制，共 N 字节」。
 */
export type BodyEncoding = "text" | "lossy" | "binary";

/** 一次请求的结果，对应 Rust 的 `http::HttpResponsePayload` */
export interface HttpResponsePayload {
  status: number;
  statusText: string;
  /** 实际协商出来的版本（`HTTP/1.1` / `HTTP/2.0`），不是设置里选的那个 */
  httpVersion: string;
  headers: HttpHeader[];
  /** 最终生效的 URL。跟随重定向后与请求 URL 不同 —— 排查 302 的关键信息 */
  url: string;
  redirected: boolean;
  body: string;
  bodyEncoding: BodyEncoding;
  /** 实际收到的字节数（**截断前**） */
  bodySize: number;
  truncated: boolean;
  /** 发出 → 收到响应头 */
  ttfbMs: number;
  /** 响应头 → 读完 body */
  downloadMs: number;
  /** 端到端总耗时（含组装与解析，比 ttfb + download 略大） */
  totalMs: number;
}
