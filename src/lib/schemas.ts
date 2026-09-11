/* ============================================================
   运行时校验 schema
   ------------------------------------------------------------
   为什么要这一层：`src/lib/types.ts` 与 `src-tauri/src/storage/models.rs`
   是**两份手写**的类型定义，没有任何自动同步机制。少写一个字段、
   把 `number` 写成 `string`，TS 和 Rust 都不会报错 ——
   错误只会在某个深夜以「undefined 点不出来」的形式爆出来。

   所以每个 IPC 返回都过一次 parse：
   * 结构对 → 拿到**经过校验**的值（顺便把类型收窄）
   * 结构错 → 抛 E_SCHEMA_DRIFT，在控制台点名是哪个字段

   ⚠️ 用 `z.looseObject`（不是 `z.object`）：后端将来加字段时，
      旧前端不应该因为「多了个不认识的字断」而整个列表打不开。
      只校验我们**需要**的字段必须存在且类型正确。
   ============================================================ */

import { z } from "zod";

// ---------------------------------------------------------------
// 请求内的值类型
// ---------------------------------------------------------------

export const keyValueItemSchema = z.looseObject({
  key: z.string(),
  value: z.string(),
  enabled: z.boolean(),
  desc: z.string().optional(),
});

export const bodyTypeSchema = z.enum([
  "none",
  "json",
  "form",
  "urlencoded",
  "raw",
  "binary",
  "graphql",
]);

export const httpVersionSchema = z.enum(["auto", "http1", "http2"]);

export const expiryUnitSchema = z.enum(["seconds", "millis", "absolute"]);

export const requestSettingsSchema = z.looseObject({
  followRedirects: z.boolean(),
  maxRedirects: z.number(),
  verifyTls: z.boolean(),
  timeoutMs: z.number(),
  // 可选 + 可空：JSON 里可能整个键不存在，也可能是 null
  proxy: z.string().nullish(),
  httpVersion: httpVersionSchema,
});

// ---------------------------------------------------------------
// 实体
// ---------------------------------------------------------------

export const workspaceSchema = z.looseObject({
  id: z.number(),
  /** 空串是合法的 —— 此时前端显示 i18n 的 `workspace.default` */
  name: z.string(),
  sortOrder: z.number(),
  createdAt: z.string(),
  updatedAt: z.string(),
  deletedAt: z.string().nullable(),
});

export const folderSchema = z.looseObject({
  id: z.number(),
  workspaceId: z.number(),
  parentId: z.number().nullable(),
  name: z.string(),
  sortOrder: z.number(),
  isTokenSource: z.boolean(),
  createdAt: z.string(),
  updatedAt: z.string(),
  deletedAt: z.string().nullable(),
});

export const apiRequestSchema = z.looseObject({
  id: z.number(),
  folderId: z.number().nullable(),
  workspaceId: z.number(),
  name: z.string(),
  method: z.string(),
  url: z.string(),
  params: z.array(keyValueItemSchema),
  headers: z.array(keyValueItemSchema),
  bodyType: bodyTypeSchema,
  body: z.string(),
  // auth 结构还没定稿（见 types.ts 的 TODO），这里只要求「是个对象」
  auth: z.record(z.string(), z.unknown()),
  settings: requestSettingsSchema,
  preScript: z.string(),
  testScript: z.string(),
  sortOrder: z.number(),
  createdAt: z.string(),
  updatedAt: z.string(),
  deletedAt: z.string().nullable(),
});

export const environmentSchema = z.looseObject({
  id: z.number(),
  workspaceId: z.number(),
  name: z.string(),
  isActive: z.boolean(),
  createdAt: z.string(),
  updatedAt: z.string(),
  deletedAt: z.string().nullable(),
});

export const envVarSchema = z.looseObject({
  id: z.number(),
  environmentId: z.number(),
  key: z.string(),
  value: z.string(),
  isSecret: z.boolean(),
  enabled: z.boolean(),
  sortOrder: z.number(),
});

export const tokenSourceSchema = z.looseObject({
  id: z.number(),
  workspaceId: z.number(),
  folderId: z.number(),
  name: z.string(),
  fetchRequestId: z.number(),
  accessPath: z.string(),
  refreshPath: z.string(),
  expiresInPath: z.string(),
  expiresInUnit: expiryUnitSchema,
  refreshRequestId: z.number().nullable(),
  autoRefresh: z.boolean(),
  skewSeconds: z.number(),
  createdAt: z.string(),
  updatedAt: z.string(),
});

export const tokenCacheSchema = z.looseObject({
  id: z.number(),
  tokenSourceId: z.number(),
  accessToken: z.string(),
  refreshToken: z.string().nullable(),
  /** null = 永不过期 */
  expiresAt: z.string().nullable(),
  rawResponse: z.string().nullable(),
  updatedAt: z.string(),
});

export const historyEntrySchema = z.looseObject({
  id: z.number(),
  workspaceId: z.number(),
  requestId: z.number().nullable(),
  requestName: z.string(),
  snapshot: z.record(z.string(), z.unknown()),
  /** null = 网络层就失败了 */
  status: z.number().nullable(),
  durationMs: z.number().nullable(),
  respSize: z.number().nullable(),
  error: z.string().nullable(),
  environment: z.string().nullable(),
  executedAt: z.string(),
  /** 非空 = 在回收站 */
  deletedAt: z.string().nullable(),
});

export const loadtestRunSchema = z.looseObject({
  id: z.number(),
  requestId: z.number().nullable(),
  requestName: z.string(),
  config: z.record(z.string(), z.unknown()),
  summary: z.record(z.string(), z.unknown()),
  startedAt: z.string(),
  finishedAt: z.string().nullable(),
});

export const loadtestSampleSchema = z.looseObject({
  id: z.number(),
  runId: z.number(),
  seq: z.number(),
  status: z.number().nullable(),
  durationMs: z.number(),
  bytes: z.number().nullable(),
  error: z.string().nullable(),
  startedOffsetMs: z.number(),
});

export const settingSchema = z.looseObject({
  key: z.string(),
  value: z.string(),
  updatedAt: z.string(),
});

export const appInfoSchema = z.looseObject({
  dbPath: z.string(),
  schemaVersion: z.number(),
  latestSchemaVersion: z.number(),
  appVersion: z.string(),
});

// ---------------------------------------------------------------
// 请求执行（P2）
// ---------------------------------------------------------------

export const httpHeaderSchema = z.looseObject({
  name: z.string(),
  value: z.string(),
});

export const bodyEncodingSchema = z.enum(["text", "lossy", "binary"]);

/**
 * `send_request` 的返回。
 *
 * 这一层校验的价值在这条链路上最高：Rust 侧字段多、又是手写两份定义，
 * 少一个 `bodySize` 就会在界面上显示成「NaN B」而不是当场报错。
 */
export const responsePayloadSchema = z.looseObject({
  status: z.number(),
  statusText: z.string(),
  httpVersion: z.string(),
  headers: z.array(httpHeaderSchema),
  url: z.string(),
  redirected: z.boolean(),
  body: z.string(),
  bodyEncoding: bodyEncodingSchema,
  bodySize: z.number(),
  truncated: z.boolean(),
  ttfbMs: z.number(),
  downloadMs: z.number(),
  totalMs: z.number(),
});
