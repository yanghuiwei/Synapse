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

import { DEFAULT_REQUEST_SETTINGS } from "./http";

// ---------------------------------------------------------------
// 请求内的值类型
// ---------------------------------------------------------------

export const keyValueItemSchema = z.looseObject({
  key: z.string(),
  value: z.string(),
  enabled: z.boolean(),
  desc: z.string().optional(),
  // multipart 的文件列。老库里没有它，所以是可选的
  file: z.string().optional(),
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

// 🔴 这里**没有 `authConfigSchema`**。
//
// 迁移 007 把「授权」拍平成了请求头里的一行 —— 库里既没有 `auth` 列，
// 线上也没有 `auth` 字段，所以没有任何东西需要校验。Bearer / Basic /
// API Key 现在只是请求头页上三个快捷插入按钮产生的普通 `KeyValueItem`。
//
// 于是老库里的 `{"type":"bearer",...}` 会被当成一个不认识的额外键**丢掉**
// （`z.looseObject` 只校验声明过的字段）—— 这是对的：那行的内容
// 迁移时已经被搬进 `headers` 了，这里再解一次反而会重复。

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
  /**
   * 004 迁移新增的列。带 `.catch()` 兜底：浏览器里开着旧 dev server
   * （或旧后端）时少这一个字段，不该让整棵集合树打不开 ——
   * 退回「是普通分类」即可，表现与升级前一致。
   */
  isSystem: z.boolean().catch(false),
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
  /**
   * 003 迁移新增的两列。带 `.catch()` 兜底：**前端可能先于后端更新**
   * （浏览器里还开着旧 dev server），那时少这两个字段不该让整个集合树
   * 打不开 —— 退回空快照即可，表现与升级前一致。
   */
  bodyByType: z.record(z.string(), z.string()).catch({}),
  bodyRows: z.array(keyValueItemSchema).catch([]),
  // 005 迁移新增的列，兜底理由同上
  formFields: z.array(keyValueItemSchema).catch([]),
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
  /** 当时真实发出的 curl；null = 这条历史发生在本版之前 */
  curl: z.string().nullable(),
  curlMasked: z.string().nullable(),
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

export const mcpSetupSchema = z.looseObject({
  jsonSnippet: z.string(),
  yamlSnippet: z.string(),
  commandPath: z.string(),
  dbPath: z.string(),
  commandExists: z.boolean(),
});

// ---------------------------------------------------------------
// 请求执行（P2）
// ---------------------------------------------------------------

export const httpHeaderSchema = z.looseObject({
  name: z.string(),
  value: z.string(),
});

/**
 * `BodyFile` —— 「文件型」响应在后端临时区的位置。
 *
 * 三项都是必填且不兜底：缺任何一项这个入口都没法用（没有路径就无从复制），
 * 与其退化成一个点了报错的文件名，不如让整份 payload 校验失败、当场暴露漂移。
 */
export const bodyFileSchema = z.looseObject({
  path: z.string(),
  name: z.string(),
  size: z.number(),
});

export const savedFileSchema = z.looseObject({
  path: z.string(),
  bytes: z.number(),
});

/**
 * 发给后端执行的请求定义。
 *
 * 有它的理由：历史记录里存的是**原样 JSON 快照**，回填编辑器时要先把
 * 「一坨 unknown」变成可信的 `RequestSpec`。少了这层，一条字段缺失的旧记录
 * 会让编辑器在 `spec.params.map(...)` 上当场炸掉，而用户只会看到白屏。
 *
 * 除 `url` 外每个字段都带 `.catch()` 兜底：快照是**过去写下的数据**，
 * 它可能来自更早的版本，用当前 schema 去卡它必然会出现缺口。
 * 缺口处宁可退回默认值（用户看得见、也改得动），也不要整条记录打不开。
 * `url` 不兜底 —— 没有地址的请求回填进编辑器也没有任何意义。
 */
export const requestSpecSchema = z.looseObject({
  method: z.string().catch("GET"),
  url: z.string(),
  params: z.array(keyValueItemSchema).catch([]),
  headers: z.array(keyValueItemSchema).catch([]),
  bodyType: bodyTypeSchema.catch("none"),
  body: z.string().catch(""),
  // 分槽快照与键值行同样兜底：P2/P3 时期的历史快照里没有它们
  bodyByType: z.record(z.string(), z.string()).catch({}),
  bodyRows: z.array(keyValueItemSchema).catch([]),
  // multipart 的字段行。同理兜底 —— 它是本轮才加上的字段
  formFields: z.array(keyValueItemSchema).catch([]),
  settings: requestSettingsSchema.catch(() => ({ ...DEFAULT_REQUEST_SETTINGS })),
  // 老快照里没有这个字段 —— 缺失按「无分类」处理，而不是整条记录打不开
  folderId: z.number().nullable().optional().catch(null),
});

export const bodyEncodingSchema = z.enum(["text", "lossy", "binary"]);

export const placeholderNamespaceSchema = z.enum([
  "func",
  "env",
  "token",
  "resp",
  "prev",
  "folder",
  "unknown",
]);

export const unresolvedReasonSchema = z.enum([
  "unknownNamespace",
  "badFunction",
  "envNotFound",
  "noTokenSource",
  "tokenUnavailable",
  "noResponse",
  "respPathMiss",
  "notSupported",
]);

/** 未解析的占位符。`.catch()` 兜底的理由同 `requestSpecSchema`：后端加变体时前端不该整体崩 */
export const unresolvedPlaceholderSchema = z.looseObject({
  raw: z.string(),
  namespace: placeholderNamespaceSchema.catch("unknown"),
  key: z.string().catch(""),
  reason: unresolvedReasonSchema.catch("unknownNamespace"),
});

/**
 * `send_request` 响应里的 `tokenUsage`。
 *
 * 与 `tokenStatusSchema` 是**两个不同的东西**，别合并：
 * 这个是「**这一次发送**用了什么」（执行期的事实，事后从库里查不到），
 * 那个是「某个 Token 来源**现在**是什么状态」（随时可查的缓存快照）。
 */
export const tokenUsageSchema = z.looseObject({
  sourceId: z.number(),
  sourceName: z.string(),
  refreshed: z.boolean(),
  endpointRequestId: z.number(),
  endpointName: z.string().nullable(),
  expiresAt: z.string().nullable(),
  remainingSeconds: z.number().nullable(),
  autoRefresh: z.boolean(),
});

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
  // 只有「看起来是文件」的响应才有它（见 Rust `http::execute::looks_like_file`）。
  // 缺省 undefined 而不是 null：没有它就是「没有下载入口」，与 `bodyFile: null`
  // 是同义，多一种表示只会让判断写成两处。
  bodyFile: bodyFileSchema.optional(),
  // 缺省空数组而不是必填：P2/P3 的后端没有这个字段，
  // 而「没有未解析项」本来就是这个字段的绝大多数取值
  unresolvedPlaceholders: z.array(unresolvedPlaceholderSchema).catch([]),
  // 同上：缺省 `undefined` 而不是 `null` —— 它就是「本次没用上 Token」，
  // 与 `bodyFile` 一个约定
  tokenUsage: tokenUsageSchema.optional(),
  // 本次真实发出的 curl。缺省 `undefined` = 这条路径不生成它（MCP / CLI 的输出
  // 会进 AI 的上下文，凭据不从那里出去）
  curl: z.string().optional(),
  curlPlain: z.string().optional(),
});

/** `preview_curl` 的返回：按当前编辑内容**推演**出来的一条命令 */
export const previewCurlResultSchema = z.looseObject({
  curl: z.string(),
  curlPlain: z.string(),
  unresolvedPlaceholders: z.array(unresolvedPlaceholderSchema).catch([]),
  tokenUsage: tokenUsageSchema.optional(),
});

/** `get_token_status` / `refresh_token` 的返回 */
export const tokenStatusSchema = z.looseObject({
  hasCache: z.boolean(),
  expired: z.boolean(),
  expiresAt: z.string().nullable(),
  remainingSeconds: z.number().nullable(),
  updatedAt: z.string().nullable(),
  accessToken: z.string().nullable(),
  hasRefreshToken: z.boolean(),
  autoRefresh: z.boolean(),
  /**
   * 这一份 token 落在哪条**代表行**上（配置相同的来源会给同一个值）。
   *
   * 界面靠它把「同一个 token、同一个倒计时」体现出来：刷新其一之后要把同组的
   * 条目一起更新，计数也不能把一份 token 数成两份。
   *
   * 用 `.catch(-1)` 而不是必填：老版本后端（或别的前端调用点）不带这个字段时，
   * 退化成「每条来源各算一组」—— 也就是这套共享机制出现之前的行为，
   * 界面对得上、不会炸。`-1` 是「不知道归属」，不是一个真实的行 id。
   */
  cacheOwnerId: z.number().catch(-1),
});

// ---------------------------------------------------------------
// 集合管理 / 历史 / 回收站（P3）
// ---------------------------------------------------------------

/** `delete_folder` 的返回 */
export const deleteFolderResultSchema = z.looseObject({
  folders: z.number(),
  requests: z.number(),
});

/** `purge_folder` 的返回 */
export const purgeTreeResultSchema = z.looseObject({
  folders: z.number(),
  requests: z.number(),
});

/** `purge_trash` 的返回 */
export const purgeTrashResultSchema = z.looseObject({
  folders: z.number(),
  requests: z.number(),
  history: z.number(),
});

/**
 * `list_trash` 的返回。
 *
 * 三块都用**各自的实体 schema**（而不是 `z.array(z.unknown())`）：
 * 回收站是「用户最后确认一次再删」的地方，这里的数据错了会让人删错东西，
 * 所以宁可当场报 schema 漂移。
 */
export const trashContentsSchema = z.looseObject({
  folders: z.array(folderSchema),
  requests: z.array(apiRequestSchema),
  history: z.array(historyEntrySchema),
});

/**
 * `list_history` 的返回。
 *
 * `total` 是**匹配**总数（带搜索时也只数匹配项），前端据此判断还有没有下一页。
 */
export const historyPageSchema = z.looseObject({
  items: z.array(historyEntrySchema),
  total: z.number(),
});

// ---------------------------------------------------------------
// 导入器（P5）
// ---------------------------------------------------------------

export const importFormatSchema = z.enum(["curl", "postman", "apipost", "reqly", "har"]);

export const conflictModeSchema = z.enum(["merge", "duplicate", "overwrite"]);

export const diffStatusSchema = z.enum(["new", "update", "skip", "keep", "remove"]);

export const previewKindSchema = z.enum(["folder", "request"]);

/**
 * 警告种类用 `.catch("unsupportedField")` 兜底。
 *
 * 后端的 `WarningKind` 会随支持的格式增多而变长，而前端可能先于后端更新
 * （或者反过来）。认不出的一律当成「有东西没导进来」，至少不会整屏报错。
 */
export const importWarningSchema = z.looseObject({
  subject: z.string().optional(),
  kind: z
    .enum([
      "unknownArgument",
      "missingArgumentValue",
      "unsupportedField",
      "renamedDuplicate",
      "truncatedName",
      "generatedName",
      "skippedItem",
    ])
    .catch("unsupportedField"),
  detail: z.string(),
});

export const importCountsSchema = z.looseObject({
  created: z.number(),
  updated: z.number(),
  skipped: z.number(),
  removed: z.number(),
});

/**
 * 预览树是**递归**的，类型要显式写出来（zod 的推断在这里帮不上忙）。
 * `children` 用 `.optional()`：后端对空数组省略了它（`skip_serializing_if`）。
 */
export const previewNodeSchema: z.ZodType<import("./types").PreviewNode> = z.lazy(() =>
  z.looseObject({
    name: z.string(),
    kind: previewKindSchema,
    status: diffStatusSchema,
    method: z.string().optional(),
    url: z.string().optional(),
    children: z.array(previewNodeSchema).optional(),
  }),
);

export const envPreviewSchema = z.looseObject({
  name: z.string(),
  varCount: z.number(),
  merged: z.boolean(),
});

export const importPreviewSchema = z.looseObject({
  format: importFormatSchema,
  counts: importCountsSchema,
  nodes: z.array(previewNodeSchema),
  environments: z.array(envPreviewSchema),
  warnings: z.array(importWarningSchema),
});

export const importOutcomeSchema = z.looseObject({
  format: importFormatSchema,
  counts: importCountsSchema,
  environments: z.number(),
  warnings: z.array(importWarningSchema),
});

// ---------------------------------------------------------------
// 压测（P6）
// ---------------------------------------------------------------

export const latencySummarySchema = z.looseObject({
  minMs: z.number(),
  meanMs: z.number(),
  p50Ms: z.number(),
  p90Ms: z.number(),
  p95Ms: z.number(),
  p99Ms: z.number(),
  maxMs: z.number(),
});

export const throughputSummarySchema = z.looseObject({
  avgRps: z.number(),
  peakRps: z.number(),
  peakWindowIsPartial: z.boolean(),
  totalBytes: z.number(),
  avgBytes: z.number(),
});

export const statusBucketSchema = z.looseObject({
  // 可空是刻意的：null = 没拿到响应，见 types.ts 里的说明
  status: z.number().nullable(),
  count: z.number(),
});

export const errorBucketSchema = z.looseObject({
  code: z.string(),
  count: z.number(),
  slowestSeqs: z.array(z.number()),
  slowestMs: z.number(),
});

/**
 * 压测汇总。
 *
 * `frozenFunctions` 用 `.default([])`：它是 P6 后期才加进 `LoadtestSummary` 的，
 * 而 P6 之前存下来的 run 行里没有这个键 —— 不加默认值的话，
 * 那些历史记录的概要会整份解析失败，变成「打不开的旧压测」。
 */
export const loadtestSummarySchema = z.looseObject({
  total: z.number(),
  succeeded: z.number(),
  failed: z.number(),
  successRate: z.number(),
  elapsedMs: z.number(),
  tps: z.number(),
  latency: latencySummarySchema,
  throughput: throughputSummarySchema,
  statuses: z.array(statusBucketSchema),
  errors: z.array(errorBucketSchema),
  cancelled: z.boolean(),
  frozenFunctions: z.array(z.string()).default([]),
});

/**
 * 截断的失败日志行。
 *
 * 定义在 `loadtestProgressSchema` **之前**：zod 的 schema 是运行期常量，
 * 引用晚于自己的常量会撞上暂时性死区（TDZ），而且报的是 `Cannot access
 * before initialization` 这种跟「顺序」看不出关系的错。
 * 类型声明（`types.ts`）不受这个限制，所以那边可以随意前后放置。
 */
export const failureLineSchema = z.looseObject({
  seq: z.number(),
  status: z.number().nullable(),
  code: z.string().nullable(),
  durationMs: z.number(),
  offsetMs: z.number(),
});

export const loadtestProgressSchema = z.looseObject({
  sent: z.number(),
  succeeded: z.number(),
  failed: z.number(),
  p95Ms: z.number(),
  currentRps: z.number(),
  elapsedMs: z.number(),
  recentFailures: z.array(failureLineSchema).default([]),
});

export const loadtestProgressEventSchema = z.looseObject({
  token: z.string(),
  snapshot: loadtestProgressSchema,
});

export const loadtestRunDetailSchema: z.ZodType<import("./types").LoadtestRunDetail> =
  z.looseObject({
    run: loadtestRunSchema,
    // null = 还没跑完 / 跑失败没产出概要，是正常状态而不是故障
    summary: loadtestSummarySchema.nullable(),
    sampleCount: z.number(),
  });

export const exportFormatSchema = z.enum(["html", "csv", "json"]);

export const exportResultSchema: z.ZodType<import("./types").ExportResult> = z.looseObject({
  path: z.string(),
  bytes: z.number(),
  format: exportFormatSchema,
});

export const timelinePointSchema = z.looseObject({
  second: z.number(),
  count: z.number(),
  p95Ms: z.number(),
});

export const histogramBucketSchema = z.looseObject({
  lowerMs: z.number(),
  upperMs: z.number(),
  count: z.number(),
});

export const loadtestChartSchema: z.ZodType<import("./types").LoadtestChart> = z.looseObject({
  timeline: z.array(timelinePointSchema),
  histogram: z.array(histogramBucketSchema),
  slowest: z.array(loadtestSampleSchema),
  failures: z.array(loadtestSampleSchema),
});
