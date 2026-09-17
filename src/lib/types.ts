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
  /**
   * 文件字段的**磁盘路径**，只有 multipart 表单用得上。
   *
   * 非空即表示这一行是文件：`value` 会被忽略，发送时由后端去读这个路径
   * （不在这里读，是因为大文件不该先进内存再走一遍 IPC）。
   */
  file?: string;
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

/** API Key 注入到哪儿 */
export type ApiKeyLocation = "header" | "query";

/**
 * 授权配置，与 Rust 的 `storage::models::AuthConfig` 一一对应。
 *
 * 内部标签枚举（`type` 字段判别），变体名两边都是 camelCase。
 *
 * `inherit` 的语义是「沿文件夹链向上找最近的非 inherit 配置」，由后端在
 * 发送时解析（`http::pipeline`）—— 前端只负责显示「继承自哪里」。
 *
 * ⚠️ **前端的 `Record<string, unknown>` 时代结束了**：老库里存的是 `{}`，
 * 后端把它宽松解析成 `inherit`（见 `AuthConfig::from_json`）。前端这边
 * `schemas.ts` 同样给了兜底，读到认不出的形状就退回 `inherit`。
 */
export type AuthConfig =
  | { type: "inherit" }
  | { type: "none" }
  | { type: "bearer"; token: string }
  | { type: "basic"; username: string; password: string }
  | { type: "apiKey"; key: string; value: string; location: ApiKeyLocation };

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
  /**
   * 系统分类（「token刷新」）：**不可删除 / 改名 / 移动**。
   *
   * 每个工作区固定有一个，由后端在启动时建好（`folder::ensure_system_folder`）——
   * Token 来源就配在它下面，所以它同时也是 Token 分组。
   */
  isSystem: boolean;
  /**
   * 这个分类给其下所有请求提供的授权（P4）。
   *
   * 之所以放在**分类**上：一个团队的内网接口往往共用一套 Bearer，
   * 逐请求配一遍就是逐请求写错一遍的机会。
   */
  auth: AuthConfig;
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
  /**
   * **当前** `bodyType` 对应的那份内容，也是唯一会被真正发出去的一份。
   *
   * 它与下面两个字段的关系是「镜子与原件」：`bodyByType` 是每种类型各存
   * 一份的原件，`body` 永远是当前选中那一格的副本。这样后端 `build_body`、
   * 压测、MCP 三条读取路径都只需要看 `body`，一行都不用改。
   */
  body: string;
  /**
   * 每种请求体类型各存一份的内容快照，形如 `{ json: "…", raw: "hello" }`。
   *
   * 键是 `string` 而不是 `BodyType`：库里可能留着更早版本写下的类型名，
   * 用枚举做键会让整条记录读不出来（对应 Rust 侧 `BTreeMap<String, String>`）。
   */
  bodyByType: Record<string, string>;
  /**
   * 键值型请求体（当前只有 urlencoded）的行，形状与 `params` / `headers` 一致，
   * 所以能直接复用键值表组件与占位符高亮。
   *
   * 只服务 `urlencoded` —— multipart 的行在 `formFields`，两者刻意分槽。
   */
  bodyRows: KeyValueItem[];
  /**
   * `multipart/form-data` 的字段行（005 起）。
   *
   * 与 `bodyRows` 分槽的理由：共用一列的话，从 urlencoded 切到 form
   * 会把上一种表格的行原样带过去，而用户以为那是 form 的内容。
   */
  formFields: KeyValueItem[];
  auth: AuthConfig;
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

/**
 * MCP 客户端的连接信息（P7）。
 *
 * 两个配置片段由后端拼（`mcp::config_gen`）—— 路径转义（反斜杠、空格）是
 * **格式相关**的知识，前端再拼一遍就是第二份实现，两份迟早分叉。
 */
export interface McpSetup {
  jsonSnippet: string;
  yamlSnippet: string;
  commandPath: string;
  dbPath: string;
  /** MCP 可执行文件是否真的在那个路径上；开发态没编过 bin 时为 false */
  commandExists: boolean;
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
  /**
   * 每种请求体类型各存一份的内容快照。
   *
   * 后端**不读**这两个字段（执行只看 `body`）。放在这里是因为历史记录
   * 存的就是一份 `RequestSpec` 快照 —— 不带上的话，从历史里打开一条
   * urlencoded 请求，表格会退化成一行 `a=1&b=2` 纯文本。
   */
  bodyByType: Record<string, string>;
  /**
   * 键值型请求体（urlencoded）的行。后端同样不读
   */
  bodyRows: KeyValueItem[];
  /**
   * `multipart/form-data` 的字段行（含文件）。后端**只在这个类型下读它**。
   *
   * 为什么一个字段装不下、非要再开一条通道：multipart 的成品不是一段字符串
   * —— boundary 必须由发送方现生成，文件内容也不该跟着 IPC 走一趟。
   * `formFields` 非空时它是权威；为空而后端拿到非空 `body` 时，
   * 会退回解析 `body`（`k=v&k=@路径`，curl 的 `-F` 那套）。
   *
   * 老快照（P2/P3 写下的）里没有这个字段 —— 由 `schemas.ts` 兜底成 `[]`，
   * 而不是让整条历史记录打不开。
   */
  formFields: KeyValueItem[];
  auth: AuthConfig;
  settings: RequestSettings;
  /**
   * 这条请求挂在哪个分类下。
   *
   * 不是可有可无的元数据：Token 源分组（§4.3）与授权继承（§4.4）都要
   * **沿分类链向上找**，所以它是这次发送语义的一部分。
   * P2/P3 存下来的历史快照里没有这个字段 —— 缺失时按「无分类」处理。
   */
  folderId?: number | null;
}

export interface HttpHeader {
  name: string;
  value: string;
}

/** 占位符的命名空间。与 Rust `template::resolver::Namespace::as_str()` 一一对应 */
export type PlaceholderNamespace =
  | "func"
  | "env"
  | "token"
  | "resp"
  | "prev"
  | "folder"
  | "unknown";

/**
 * 这个占位符为什么没解析出来。
 *
 * 与 Rust `UnresolvedReason::as_str()` 一一对应，是**码**不是文案 ——
 * 中文在 `zh-CN.ts` 的 `placeholder.reason.*` 里查。
 */
export type UnresolvedReason =
  | "unknownNamespace"
  | "badFunction"
  | "envNotFound"
  | "noTokenSource"
  | "tokenUnavailable"
  | "noResponse"
  | "respPathMiss"
  | "notSupported";

/**
 * 本次发送里**没能解析**的占位符。
 *
 * 它们的原文被原样留在了请求里 —— 也就是「你发出去的不是你以为的那串地址」。
 * 挂在响应上而不是单开一条事件：它就是「这一次发送」的属性。
 */
export interface UnresolvedPlaceholder {
  /** 原文，含花括号，如 `{{env.nope}}` */
  raw: string;
  namespace: PlaceholderNamespace;
  /** 命名空间之后的部分，如 `nope` / `$.data.id` */
  key: string;
  reason: UnresolvedReason;
}

/**
 * 响应体的解读方式。
 *
 * `binary` 时 `body` 是空串：一份二进制内容被有损转成字符串后界面就是乱码，
 * 用户会以为是工具坏了。宁可明说「这是二进制，共 N 字节」。
 */
export type BodyEncoding = "text" | "lossy" | "binary";

/**
 * 一份「看起来是文件」的响应落在后端临时区的位置。
 *
 * 它**不是**用户磁盘上的最终文件：执行时先落一份，用户点「保存到文件」
 * 才由后端弹框、复制到他选的地方。分两步是因为落盘发生在执行那一刻，
 * 而问用户存哪儿是个交互动作 —— 合成一步就等于每个疑似文件的响应都弹一次框。
 */
export interface BodyFile {
  /** 后端临时区里的绝对路径 */
  path: string;
  /** 建议的文件名（Content-Disposition → URL 末段 → 兜底） */
  name: string;
  /** 完整字节数，不受内联截断影响 */
  size: number;
}

/** 另存成功后的结果 */
export interface SavedFile {
  path: string;
  bytes: number;
}

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
  /**
   * 非空 = 这份响应是个文件，完整字节已经落在后端临时区，界面上要给「保存到文件」。
   *
   * 判据是三者取并集：解不出 UTF-8、`Content-Disposition: attachment`、
   * Content-Type 落在二进制清单里 —— 所以 **CSV 这类合法 UTF-8 的导出也会有它**
   * （那时 `bodyEncoding` 是 `text`，内容照样在 `body` 里能看）。
   */
  bodyFile?: BodyFile;
  /**
   * 本次发送里没解析出来的占位符。
   *
   * 前端要把它显眼地摆出来：这些占位符的**原文**已经跟着请求发出去了，
   * 99% 的情况是用户想要的接口根本没被调到（打到 `https://api.x/{{env.path}}`）。
   */
  unresolvedPlaceholders: UnresolvedPlaceholder[];
}

/**
 * Token 源的运行状态（`get_token_status` / `refresh_token` 的返回）。
 *
 * 「还有多少秒过期」由**后端**算（判定逻辑只有一份，见 `http::token::is_expired`）。
 * 前端自己拿 `expiresAt` 与 `skewSeconds` 再算一遍，两边迟早会在边界上分叉，
 * 表现为「徽标说还有 5 秒，后端说已经过期了」。
 */
export interface TokenStatus {
  /** 有没有缓存行。`false` = 从没取过 */
  hasCache: boolean;
  /** 已经过期（含 `skewSeconds` 提前量） */
  expired: boolean;
  /** 过期时间的绝对值；`null` = 永不过期 */
  expiresAt: string | null;
  /** 还剩多少秒。负数表示已经过去多久；`null` = 永不过期或没有缓存 */
  remainingSeconds: number | null;
  updatedAt: string | null;
  /** 当前缓存的 access token。**打码是展示层的事**（见 `env.secretHint` 的同一套约定） */
  accessToken: string | null;
  hasRefreshToken: boolean;
  autoRefresh: boolean;
}

// ============================================================
// 集合管理 / 历史 / 回收站（P3）
// ============================================================

/** 级联软删除的计数（`delete_folder`） */
export interface DeleteFolderResult {
  folders: number;
  requests: number;
}

/** 彻底删除一棵子树的计数（`purge_folder`） */
export interface PurgeTreeResult {
  folders: number;
  requests: number;
}

/** 清空回收站的计数（`purge_trash`） */
export interface PurgeTrashResult {
  folders: number;
  requests: number;
  history: number;
}

/**
 * 回收站的全部内容。
 *
 * 三块列表一次拿回：分三次请求会出现「分类已经出来了，历史还在转」
 * 这种半截界面，而且用户在回收站里做的多半是「整体清空」，不是逐块看。
 */
export interface TrashContents {
  folders: Folder[];
  requests: ApiRequest[];
  history: HistoryEntry[];
}

/** 一页历史 + 匹配总数（`list_history`） */
export interface HistoryPage {
  items: HistoryEntry[];
  total: number;
}

/**
 * 写一条历史的入参（`record_history`）。
 *
 * 与 `HistoryEntry` 的差别：没有 `id` / `deletedAt`（由后端生成），
 * 也没有「执行时间」（后端用 `now_iso()` 取，避免前端时钟不准）。
 */
export interface NewHistoryEntry {
  workspaceId: number;
  /** 快照来自哪个已保存请求；null = 从未保存过的临时请求 */
  requestId: number | null;
  /** 冗余的名字快照 —— 请求被删掉后历史仍要能读 */
  requestName: string;
  snapshot: RequestSpec;
  /** null = 网络层就失败了（此时 `error` 有值） */
  status: number | null;
  durationMs: number | null;
  respSize: number | null;
  /** 错误码字符串（如 `E_DNS`），不是给人读的句子 */
  error: string | null;
  environment: string | null;
}

// ============================================================
// 导入器（P5）
// ============================================================

/** 识别出来的来源格式。加一种要同步 `importer::Format`。 */
export type ImportFormat = "curl" | "postman" | "apipost" | "reqly" | "har";

/**
 * 同名冲突的处理方式。语义差异见 Rust 侧 `commands::import_::Conflict`：
 * `merge` 更新、`duplicate` 建副本、`overwrite` 以文件为准（多出来的进回收站）。
 */
export type ConflictMode = "merge" | "duplicate" | "overwrite";

/**
 * 警告的种类，**机器可读**。文案在前端查表（后端只回码，不回句子）。
 * 加一种要同步 `importer::WarningKind`。
 */
export type ImportWarningKind =
  | "unknownArgument"
  | "missingArgumentValue"
  | "unsupportedField"
  | "renamedDuplicate"
  | "truncatedName"
  | "generatedName"
  | "skippedItem";

export interface ImportWarning {
  /** 出问题的对象名（接口名 / 分类名），没有就是 undefined */
  subject?: string;
  kind: ImportWarningKind;
  /** 只给开发者看的细节，界面不展示 */
  detail: string;
}

/** 一个节点在预览树里的处境 */
export type DiffStatus = "new" | "update" | "skip" | "keep" | "remove";

export type PreviewKind = "folder" | "request";

export interface ImportCounts {
  created: number;
  updated: number;
  skipped: number;
  removed: number;
}

export interface PreviewNode {
  name: string;
  kind: PreviewKind;
  status: DiffStatus;
  method?: string;
  url?: string;
  children?: PreviewNode[];
}

export interface EnvPreview {
  name: string;
  varCount: number;
  /** 同名环境已存在 → 变量会并进去，不会再多个环境出来 */
  merged: boolean;
}

export interface ImportPreview {
  format: ImportFormat;
  counts: ImportCounts;
  nodes: PreviewNode[];
  environments: EnvPreview[];
  warnings: ImportWarning[];
}

export interface ImportOutcome {
  format: ImportFormat;
  counts: ImportCounts;
  environments: number;
  warnings: ImportWarning[];
}

/**
 * 导入时给「文件里没名字」的东西用的占位文案。
 *
 * 由**前端**送过去：这些名字会写进库、显示在侧栏上，属于界面文案。
 * 后端硬编码一份中文的话，以后加英文界面就会露馅。
 */
export interface ImportLabels {
  untitledFolder: string;
  untitledRequest: string;
  untitledCollection: string;
}

/** 一次导入的输入：内容 + 落点 + 占位文案 */
export interface ImportSourceInput {
  text: string;
  workspaceId: number;
  /** null = 工作区根 */
  targetFolderId: number | null;
  fileName?: string;
  labels: ImportLabels;
}

// ============================================================
// 压测（P6，design.md §7）
// ============================================================

/**
 * 压测配置（design.md §7.1）。字段名与 Rust `LoadtestConfig` 一一对应。
 *
 * 刻意**没有**「持续时间 / 爬坡 / 停止条件」这几项：design.md §7.1 定稿的模型是
 * 「固定并发 + 固定总请求数」，本工具不做开环加压（§7.5 把「恒定 RPS 加压」
 * 和「精确的 QPS 阶梯爬坡」明确列为不做的事）。
 */
export interface LoadtestConfig {
  concurrency: number;
  total: number;
  timeoutMs: number;
  followRedirects: boolean;
  /** 复用连接。关掉之后每个请求都要重做握手 —— 那是在测握手 */
  keepAlive: boolean;
  /** 每个 worker 两轮请求之间的间隔 */
  thinkTimeMs: number;
  httpVersion: HttpVersion;
  /** 预热请求数，不计入统计 */
  warmupRequests: number;
}

export interface LatencySummary {
  minMs: number;
  meanMs: number;
  p50Ms: number;
  p90Ms: number;
  p95Ms: number;
  p99Ms: number;
  maxMs: number;
}

export interface ThroughputSummary {
  /** **有效**吞吐：只数成功的请求 */
  avgRps: number;
  peakRps: number;
  /** 峰值是「按不足 1 秒的实际时长折算」出来的 —— 界面上要说明 */
  peakWindowIsPartial: boolean;
  totalBytes: number;
  avgBytes: number;
}

export interface StatusBucket {
  /**
   * `null` = **没拿到响应**（连接失败 / 超时 / TLS）。
   *
   * 后端刻意用 null 而不是 0 当哨兵：0 不是合法状态码，但拿它当特殊值
   * 意味着前端必须记住「0 是特殊的那一个」。
   */
  status: number | null;
  count: number;
}

export interface ErrorBucket {
  /** 后端错误码（`E_TIMEOUT` / `E_DNS` …），**不是文案** */
  code: string;
  count: number;
  /** 最慢的若干条的序号，供界面直接跳到那一条 */
  slowestSeqs: number[];
  slowestMs: number;
}

export interface LoadtestSummary {
  total: number;
  succeeded: number;
  failed: number;
  /** 0.0 ~ 1.0。**不是百分数** —— 谁显示谁乘 100 */
  successRate: number;
  elapsedMs: number;
  /** 实际吞吐（含失败）。与 `throughput.avgRps` 不是同一个数，见 Rust `stats` 模块头 */
  tps: number;
  latency: LatencySummary;
  throughput: ThroughputSummary;
  statuses: StatusBucket[];
  errors: ErrorBucket[];
  /** 被用户取消的。**必须显示** —— 否则这份数字会被当成完整结果引用 */
  cancelled: boolean;
  /**
   * 整轮里被冻结成同一个值的动态函数占位符（`{{$uuid}}` 这类）。
   *
   * 占位符只在开工前解析一次（见 Rust `loadtest::engine`），所以这些本该
   * 「每个请求都不一样」的占位符在整轮里是同一个值。不一定错，但用户必须知道。
   */
  frozenFunctions: string[];
}

/** 压测进行中的一次进度快照（每 200ms 推一次） */
export interface LoadtestProgress {
  sent: number;
  succeeded: number;
  failed: number;
  p95Ms: number;
  /** 最近一个采样窗口的速率，不是从头算的平均值 */
  currentRps: number;
  elapsedMs: number;
  /**
   * 最近 50 条失败，**最新的在后**。
   *
   * 跟着进度事件一起推，而不是另开一个查询：失败日志的意义就是「跟着跑」，
   * 每 200ms 多发一次 IPC 取它等于自己给压测加负载。
   * 代价是每帧多带几十条，而正常压测里这个数组是空的。
   */
  recentFailures: FailureLine[];
}

/** 进度事件的载荷 */
export interface LoadtestProgressEvent {
  /** 前端生成的 runToken。同时只允许一轮，它用来**丢掉过期事件** */
  token: string;
  snapshot: LoadtestProgress;
}

/** 这一轮压的是哪个请求 */
export interface LoadtestTarget {
  /** 从哪条保存的请求发起；null = 对着未保存的草稿压测 */
  requestId: number | null;
  /** 请求的显示名，作为快照存进这一轮。空则后端退化成 `METHOD url` */
  requestName: string | null;
}

/**
 * 一轮压测的详情。
 *
 * `summary` 为 `null` = 这一轮还没跑完或跑失败没产出概要；`sampleCount` 为 0
 * 而 `summary.total > 0` = 明细已被保留策略裁掉（图表画不出来，但概要可信）。
 */
export interface LoadtestRunDetail {
  run: LoadtestRun;
  summary: LoadtestSummary | null;
  sampleCount: number;
}

export type ExportFormat = "html" | "csv" | "json";

export interface ExportResult {
  /** 落盘的绝对路径 */
  path: string;
  bytes: number;
  format: ExportFormat;
}

/**
 * 报表文案。
 *
 * 由前端送过去 —— 报表是唯一「后端产文本」的场景（一份要发给别人的成品文档
 * 不能只有数字没有标签），但文案来源仍然只有一个：`zh-CN.ts`。
 * 字段可缺省：缺的落到后端那份语言中立的兜底（P50 / RPS / Bytes 这类写法）。
 */
export interface ReportLabels {
  lang?: string;
  title?: string;
  runId?: string;
  target?: string;
  startedAt?: string;
  finishedAt?: string;
  elapsed?: string;
  sectionConfig?: string;
  sectionSummary?: string;
  sectionCharts?: string;
  sectionLatency?: string;
  sectionThroughput?: string;
  sectionStatus?: string;
  sectionErrors?: string;
  sectionSlowest?: string;
  sectionNotes?: string;
  chartLatency?: string;
  chartRps?: string;
  chartHistogram?: string;
  requestsSent?: string;
  successRate?: string;
  tps?: string;
  p95?: string;
  min?: string;
  mean?: string;
  p50?: string;
  p90?: string;
  p99?: string;
  max?: string;
  avgRps?: string;
  peakRps?: string;
  totalBytes?: string;
  avgBytes?: string;
  status?: string;
  count?: string;
  code?: string;
  seq?: string;
  durationMs?: string;
  concurrency?: string;
  total?: string;
  timeout?: string;
  followRedirects?: string;
  keepAlive?: string;
  thinkTime?: string;
  httpVersion?: string;
  warmup?: string;
  yes?: string;
  no?: string;
  noResponse?: string;
  cancelledNotice?: string;
  samplesPrunedNotice?: string;
  frozenNotice?: string;
  peakPartialNotice?: string;
  notes?: string[];
}

/** 截断的失败日志行（进度事件里推的） */
export interface FailureLine {
  seq: number;
  /** 服务端答了 4xx/5xx 时才有；没拿到响应时为 null */
  status: number | null;
  /** 错误码。服务端正常应答时为 null —— 那不是「没拿到响应」 */
  code: string | null;
  durationMs: number;
  offsetMs: number;
}

/** 时间线上的一个点（一秒） */
export interface TimelinePoint {
  second: number;
  /** 这一秒内**完成**的请求数（含失败） */
  count: number;
  p95Ms: number;
}

/** 延迟分布的一个桶，按 2 的幂划分 */
export interface HistogramBucket {
  lowerMs: number;
  /** 上界（含） */
  upperMs: number;
  count: number;
}

/**
 * 一轮压测的图表数据。
 *
 * **聚合在后端算好再给前端**：一轮 50 万条明细整体走 IPC 是几十 MB 的 JSON，
 * 而图表真正需要的只是几十个聚合点。明细的去处是 CSV 导出与「最慢 N 条」表格。
 */
export interface LoadtestChart {
  timeline: TimelinePoint[];
  histogram: HistogramBucket[];
  slowest: LoadtestSample[];
  failures: LoadtestSample[];
}
