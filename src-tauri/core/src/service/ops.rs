//! 15 个操作的实现体 —— MCP tool 与 CLI 子命令共用同一份。
//!
//! ## 13 与 15：暴露的数量与实现的数量不是一回事
//!
//! `Ops` 现在有 **15 个**操作，而 MCP 只暴露其中 **13 个**（那张 tool 表有测试
//! 钉着「数量必须是 13」，见 `mcp/src/server.rs` 的 `EXPECTED`）。多出来的两条是
//! [`Ops::create_folder`] 与 [`Ops::create_environment`]：桌面端早就能建，
//! 但 MCP 与 CLI 都没有入口，于是「按需求建目录、按测试/预发切环境」这条自动化
//! 路走不通（缺陷单 B3 / B4 —— 一个工作流跑通全流程时实测出来的）。
//!
//! 收在 `Ops` 而不是另开一份「CLI 专用实现」：这一层本来就是 MCP 与 CLI
//! **共用的编排**，暴露其中哪几个子集是 MCP 的事 —— 编排逻辑不该因此分叉。
//! 将来要开给 AI，只需在 `server.rs` 补两条 `#[tool]` 并同步那张注解表。
//!
//! ## 为什么在这里而不是在 `mcp/`
//!
//! 这里原本是 `mcp/tools.rs`。P7 时它只为 MCP 服务，签名上绑着 rmcp 的
//! `ToolResult<T>`（= `Result<Json<T>, ErrorData>`）—— 于是 CLI 想用这 13 个操作
//! 就得把编排重写一遍，而那正是「两边悄悄分叉」的来源。
//!
//! 剥掉协议类型、改成返回 `AppResult<T>` 之后两边都调得动：MCP 侧包一层 `Json`
//! 加错误映射，CLI 侧直接序列化这些 DTO 就是 `--json` 的输出。
//!
//! ## 入参出参为什么自成一套 DTO
//!
//! 刻意**不**直接把 `storage::models` 的类型当返回：那些类型要加 `JsonSchema`
//! 派生就得动共享数据模型，而这里对外暴露的形状本来就该比库表更克制
//! （比如凭据头的值一律不外泄；代理只报「有没有配」）。
//!
//! ## 三条安全边界（design.md §8.3）在本文件的落点
//!
//! 1. `list_environments` 对 `is_secret = 1` 的值一律返回 `••••`，
//!    其余任何方法也没有「读原始 secret」的入口。
//! 2. `send_request` 的响应体截断到 64KB（按 UTF-8 边界切），
//!    凭据类响应头（`set-cookie` / `authorization` / `*-key` / `*-token` …）默认打码。
//! 3. `writable` 是只读模式的唯一闸门：9 个写操作的第一行都是它，
//!    4 个只读操作不经过它。
//!
//! ## 错误一律是 `AppError`（稳定错误码）
//!
//! 后端不产出面向用户的中文文案（见 `crate::error` 的契约），所以这里只说
//! 「哪个字段不合法」「哪条记录不存在」。MCP 与 CLI 各自决定怎么呈现出去。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{codes, AppError, AppResult};
use crate::http::token::remaining_seconds;
use crate::http::{HttpResponsePayload, Pipeline, RequestSpec};
use crate::loadtest::{engine, LoadtestConfig, LoadtestDeps};
use crate::redact::MASK;
use crate::service::collection::ensure_usable_folder;
use crate::service::import::{import_into_folder, Conflict, ImportLabels, ImportSource};
use crate::service::loadtest::{discard_run, persist_outcome, stored_run_config};
use crate::service::request::send_request_with;
use crate::service::validate::clean_name;
use crate::storage::models::{
    ApiRequest, BodyType, EnvVar, Environment, Folder, HttpVersion, KeyValueItem,
};
use crate::storage::{repo, with_conn};
use crate::AppState;

/// 回给 AI 的响应体上限（design.md §8.2）。
///
/// 64KB 是「够看清一个 JSON 结构，又不至于把 MCP 的 stdio 通道塞爆」的量级。
/// 超出的部分**丢掉但如实标记**（`bodyTruncated` + `bodySize`），不静默截断。
///
/// CLI 可以把这个数字调大甚至关掉（[`Ops::with_body_limit`]）——
/// 它的输出是文件或管道，而 MCP 的 stdout 是一条共享的 JSON-RPC 通道，
/// 两者的约束本来就不一样。
pub const DEFAULT_MAX_BODY_BYTES: usize = 64 * 1024;

/// `run_load_test` 的总超时上限。
///
/// 为什么必须有：这个 tool 会阻塞到跑完，而 MCP 的 stdio 通道是**一条连接**——
/// 一轮 50 万请求能跑十几分钟，期间客户端以为服务死了。宁可报错让它拆细了再跑。
const LOADTEST_TIMEOUT: Duration = Duration::from_secs(120);

/// 一次 tool 调用的返回。`Json<T>` 会把结构化结果同时写进 `content` 与
// ============================================================
// 请求/响应的 DTO
// ============================================================
//
// 刻意**不**直接把 `storage::models` 的类型当返回值：那些类型要加 JsonSchema
// 派生就得动共享的数据模型，而 MCP 对外暴露的形状本来就该比库表更克制
// （比如凭据头的值一律不外泄）。

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FolderSummary {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub sort_order: i64,
    /// 被标记成 Token 源分组（design.md §4.3）
    pub is_token_source: bool,
    /// 系统分类（「token刷新」）：删不掉也改不了名。
    ///
    /// 必须透出来 —— 否则 AI 看到列表里有个空分类，会照着「清理一下」的思路
    /// 试着删它，然后撞一个 `E_SYSTEM_FOLDER` 回来，白跑一趟。
    pub is_system: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RequestSummary {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub name: String,
    pub method: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KeyValueDto {
    pub key: String,
    pub value: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
}

/// 凭据**没有**单独的 DTO 字段 —— 它就是 `headers` / `params` 里的一行。
///
/// §8.3 写着「MCP 层不提供读取原始 secret 的能力」。旧的 `AuthSummary`
/// 靠「只回类型标签、不回值」来兜这条线，而 007 之后凭据就是普通的行 ——
/// 兜法必须换成[按头名打码][crate::redact]（见 [`key_value`]），
/// 否则 `get_request` 会把用户粘进去的长期 token 原样交给 AI，
/// 那就是绕过 §8.3 的后门。
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RequestSettingsDto {
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub verify_tls: bool,
    pub timeout_ms: u64,
    pub http_version: String,
    /// 代理地址同样可能内嵌 `user:pass`，只报「有没有配」。
    pub has_proxy: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RequestDetail {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub workspace_id: i64,
    pub name: String,
    pub method: String,
    pub url: String,
    pub params: Vec<KeyValueDto>,
    pub headers: Vec<KeyValueDto>,
    pub body_type: String,
    pub body: String,
    pub settings: RequestSettingsDto,
    pub pre_script: String,
    pub test_script: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EnvVarDto {
    pub key: String,
    /// `is_secret = true` 时恒为 `••••`
    pub value: String,
    pub is_secret: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentDto {
    pub id: i64,
    pub name: String,
    pub is_active: bool,
    pub vars: Vec<EnvVarDto>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HeaderDto {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedDto {
    pub raw: String,
    pub namespace: String,
    pub key: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadtestCountsDto {
    pub total: u64,
    pub succeeded: u64,
    pub failed: u64,
}

// ============================================================
// tool 入参
// ============================================================
//
// 每个字段都给了 `alias`：design.md §8.2 的参数表写的是 camelCase
// （`folderId`），而 MCP 生态里更常见的是 snake_case。两种都收，
// 声明的 schema 只有一份（snake_case），少一次「AI 按文档传却校验失败」。

/// 入参里的键值对行。
///
/// 为什么不直接用 `storage::models::KeyValueItem`：那个类型是**库里那一行**的
/// 形状，没有 `JsonSchema` 派生（给它加派生就得动共享数据模型 + 让 schemars
/// 渗进存储层）。而且「AI 写的入参」本来就该有自己的契约 ——
/// 比如这里 `value` 允许省略（只写 key 的空行是合法的），
/// 库层没这个自由度。
///
/// 两个形状之间只有一次转换（[`From`]），字段名与默认值刻意保持一致。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KeyValueInput {
    pub key: String,
    #[serde(default)]
    pub value: String,
    /// 省略时视为启用 —— 让 AI 写 `{"key":"X-Trace","value":"1"}` 就能用
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub desc: Option<String>,
}

fn enabled_by_default() -> bool {
    true
}

impl From<KeyValueInput> for KeyValueItem {
    fn from(input: KeyValueInput) -> Self {
        KeyValueItem {
            key: input.key,
            value: input.value,
            enabled: input.enabled,
            desc: input.desc,
            // 入参里没有「文件」这一说：AI 要发 multipart 的文件字段，
            // 走 `body` 的 `k=@路径` 写法（curl `-F` 那套，见 `http::execute`）。
            // 不在这里另开一个参数，是因为那样就有两条互相竞争的文件表达方式，
            // 而「哪个赢」会成为下一个要解释的规则。
            file: None,
        }
    }
}

fn to_key_values(items: &[KeyValueInput]) -> Vec<KeyValueItem> {
    items.iter().cloned().map(KeyValueItem::from).collect()
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListFoldersRequest {
    /// 工作区 id。省略时用当前（也是唯一）工作区。
    #[serde(default, alias = "workspaceId")]
    pub workspace_id: Option<i64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListRequestsRequest {
    /// 只看某个文件夹下的请求。省略时列出整个工作区的存活请求。
    #[serde(default, alias = "folderId")]
    pub folder_id: Option<i64>,
    /// 按名称模糊搜（大小写不敏感的子串匹配）。
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetRequestRequest {
    pub request_id: i64,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListEnvironmentsRequest {
    #[serde(default, alias = "workspaceId")]
    pub workspace_id: Option<i64>,
}

/// 创建与局部更新共用的字段集。
///
/// 共用一个结构体是刻意的：两边的字段名、别名、校验规则必须一致，
/// 分成两份迟早会出现「创建时能填 body_type，更新时填了被忽略」这种裂缝。
///
/// [`CreateRequestRequest`] 里 `name` / `method` / `url` 是**必填**的，
/// 所以那边不 flatten 这个结构，而是自己声明那三个字段再拼一个 `RequestFields`
/// 出来（见 [`CreateRequestRequest::into_fields`]）。这不是多余的绕路：
/// 直接 flatten 会让「必填的 `String`」和「可选的 `Option<String>`」同名相撞，
/// 生成的 JSON Schema 会把 `name` 报成 `["string","null"]` 且 `default: null`
/// —— 一份在说谎的 schema，客户端按它传 `null` 会被反序列化拒掉。
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RequestFields {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub params: Option<Vec<KeyValueInput>>,
    #[serde(default)]
    pub headers: Option<Vec<KeyValueInput>>,
    /// `none` / `json` / `form` / `urlencoded` / `raw` / `binary` / `graphql`
    #[serde(default, alias = "bodyType")]
    pub body_type: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequestRequest {
    /// 落到哪个文件夹。省略 = 工作区根（未分类）。
    #[serde(default, alias = "folderId")]
    pub folder_id: Option<i64>,
    #[serde(default, alias = "workspaceId")]
    pub workspace_id: Option<i64>,
    pub name: String,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub params: Option<Vec<KeyValueInput>>,
    #[serde(default)]
    pub headers: Option<Vec<KeyValueInput>>,
    #[serde(default, alias = "bodyType")]
    pub body_type: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
}

impl CreateRequestRequest {
    /// 把三个必填字段与其余可选字段收成同一个 [`RequestFields`]，
    /// 好让创建与更新共用同一个 [`apply_fields`]。
    fn into_fields(self) -> RequestFields {
        RequestFields {
            name: Some(self.name),
            method: Some(self.method),
            url: Some(self.url),
            params: self.params,
            headers: self.headers,
            body_type: self.body_type,
            body: self.body,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRequestRequest {
    pub request_id: i64,
    /// 只改这里给出的字段，未给出的保持原样。
    #[serde(flatten)]
    pub fields: RequestFields,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetEnvVarRequest {
    pub environment_id: i64,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendRequestRequest {
    /// 已保存请求的 id。给了它就以内联字段**覆盖**其上的对应字段。
    #[serde(default)]
    pub request_id: Option<i64>,
    /// 内联定义（`request_id` 省略时必填）
    #[serde(default)]
    pub method: Option<String>,
    /// 内联定义（`request_id` 省略时必填）
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub params: Option<Vec<KeyValueInput>>,
    #[serde(default)]
    pub headers: Option<Vec<KeyValueInput>>,
    #[serde(default, alias = "bodyType")]
    pub body_type: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    /// 内联定义挂到哪个文件夹下。
    ///
    /// 不是可有可无的元数据：Token 源与授权继承都要**沿文件夹链向上找**，
    /// 不写它就解析不到 `{{token.access}}`。
    #[serde(default, alias = "folderId")]
    pub folder_id: Option<i64>,
    /// 指定用哪个环境。当前只支持「就是那个激活的环境」——
    /// 见 `Tools::check_environment` 的说明。
    #[serde(default)]
    pub environment: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunLoadTestRequest {
    pub request_id: i64,
    pub concurrency: u32,
    pub total: u64,
    /// 单请求超时，默认 30 秒
    #[serde(default, alias = "timeoutMs")]
    pub timeout_ms: Option<u64>,
    /// 预热请求数（不计入统计），默认 0
    #[serde(default, alias = "warmupRequests")]
    pub warmup_requests: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefreshTokenRequest {
    pub token_source_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportCurlRequest {
    /// 从浏览器 DevTools 复制来的那段 `curl ...` 原文
    #[serde(alias = "curlText")]
    pub curl_text: String,
    /// 落到哪个文件夹。省略 = 工作区根。
    #[serde(default, alias = "targetFolderId")]
    pub target_folder_id: Option<i64>,
    #[serde(default, alias = "workspaceId")]
    pub workspace_id: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MoveRequestRequest {
    pub request_id: i64,
    pub target_folder_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRequestRequest {
    pub request_id: i64,
}

/// 新建文件夹。
///
/// **MCP 不暴露这条**（见模块头「13 与 15」）—— 它只服务于 CLI 的 `folders create`。
/// 仍然派生 `JsonSchema`：「出入参只定义一份」是这一层的原则，
/// 将来要开给 AI 就不必再动类型。
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderRequest {
    #[serde(default, alias = "workspaceId")]
    pub workspace_id: Option<i64>,
    /// 父文件夹。省略 = 工作区根
    #[serde(default, alias = "parentId")]
    pub parent_id: Option<i64>,
    pub name: String,
}

/// 新建环境。**MCP 不暴露这条**，理由同 [`CreateFolderRequest`]。
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateEnvironmentRequest {
    #[serde(default, alias = "workspaceId")]
    pub workspace_id: Option<i64>,
    pub name: String,
}

// ============================================================
// tool 出参
// ============================================================

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListFoldersResponse {
    pub workspace_id: i64,
    pub folders: Vec<FolderSummary>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListRequestsResponse {
    pub workspace_id: i64,
    pub requests: Vec<RequestSummary>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListEnvironmentsResponse {
    pub workspace_id: i64,
    pub environments: Vec<EnvironmentDto>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendRequestResponse {
    pub status: u16,
    pub status_text: String,
    pub http_version: String,
    pub url: String,
    pub redirected: bool,
    /// **这次实际采用的请求体类型**。
    ///
    /// 回显它是为了消掉「body 到底发出去没有」这个猜：给了 body 而没指定类型时
    /// 这里是推断出来的 `json`（见 [`infer_body_type`]）；是 `none` 就说明一个字节
    /// 都没发。曾经的静默丢弃正是靠这个字段变得可见。
    pub body_type: String,
    pub headers: Vec<HeaderDto>,
    /// 响应体。超过 64KB 时只给前面一段，`bodyTruncated` 为 true。
    pub body: String,
    pub body_encoding: String,
    /// **截断前**的真实字节数
    pub body_size: u64,
    pub body_truncated: bool,
    pub ttfb_ms: u64,
    pub download_ms: u64,
    pub total_ms: u64,
    /// 没能解析掉的占位符（它们的原文已经原样发出去了）
    pub unresolved_placeholders: Vec<UnresolvedDto>,
    // ⚠️ 这里**刻意**没有 `tokenUsage`（桌面端的 `HttpResponsePayload` 有）。
    //
    // 它是给人看的展示字段（「本次刷了没、走的是哪个接口」），服务于桌面端
    // 「发送前能看见、发送后能复核」这条交互；而 MCP / CLI 的调用方要看 Token
    // 状态有自己的入口（`refresh_token` 直接返回剩余有效期）。
    // 加它要多一份 Dto + 一次转换，对当前使用场景没有增量 —— 需要时再补，不是漏。
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunLoadTestResponse {
    /// 落库的那一轮 id，可用桌面端的历史压测列表打开
    pub run_id: i64,
    pub counts: LoadtestCountsDto,
    /// 完整概要（延迟分位、错误分布、吞吐）
    pub summary: Value,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RefreshTokenResponse {
    pub source_id: i64,
    pub source_name: String,
    /// ISO8601；`null` = 该 Token 源配的是「永不过期」
    pub expires_at: Option<String>,
    /// 距过期还有多少秒（`expires_at` 为空时也是 null）
    pub expires_in_seconds: Option<i64>,
    pub has_refresh_token: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportCountsDto {
    pub created: i64,
    pub updated: i64,
    pub skipped: i64,
    pub removed: i64,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImportCurlResponse {
    pub format: String,
    pub counts: ImportCountsDto,
    /// 本次真正新建出来的请求（靠导入前后的 id 水位线算出来）
    pub created: Vec<RequestSummary>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeleteRequestResponse {
    pub request_id: i64,
    /// 软删除（进回收站），桌面端可还原
    pub restorable: bool,
}

// ============================================================
// 实现
// ============================================================

/// 13 个操作的实现体。可克隆（`Arc<AppState>` 与几个 usize/bool 都是廉价克隆），
/// HTTP 通道的每个会话各拿一份。
#[derive(Clone)]
pub struct Ops {
    state: Arc<AppState>,
    read_only: bool,
    /// 回出去之前响应体截到多少字节。
    max_body_bytes: usize,
}

impl Ops {
    /// 桌面端与 MCP 用的默认档（64KB，见 [`DEFAULT_MAX_BODY_BYTES`]）。
    pub fn new(state: Arc<AppState>, read_only: bool) -> Self {
        Self::with_body_limit(state, read_only, DEFAULT_MAX_BODY_BYTES)
    }

    /// 换一个响应体上限。`usize::MAX` 表示不在这里截（执行层自己还有一道 8MB 上限，
    /// 那一道不归这一层管）。
    ///
    /// 之所以把它做成入参而不是常量：MCP 的 stdout 是**一条**共享的 JSON-RPC 通道，
    /// 往里塞一个 10MB 响应会把整条连接堵住；而 CLI 的输出是文件或管道，没有这个
    /// 约束 —— 这正是「大响应该走 CLI 而不是 MCP」的那条理由。
    ///
    /// 能调的只是**数字**：「超了就切、并且如实标记」这条判断写在 [`truncate_body`] 里，
    /// 换任何一个入口都不会被跳过。
    pub fn with_body_limit(state: Arc<AppState>, read_only: bool, max_body_bytes: usize) -> Self {
        Self {
            state,
            read_only,
            max_body_bytes,
        }
    }

    /// 只读模式的唯一闸门：11 个写操作的第一行都是它。
    ///
    /// `tool` 是**调用方**的操作名（MCP 传 tool 名，CLI 传子命令名）——
    /// 错误里带上它，调用者才能说清「是哪个操作被拦了」，而不是给一句笼统的拒绝。
    fn writable(&self, tool: &'static str) -> AppResult<()> {
        if self.read_only {
            return Err(AppError::ReadOnly { tool });
        }
        Ok(())
    }

    // ---- 只读的 4 个 ----

    /// 列出文件夹树（平铺返回，前端/AI 按 `parentId` 自己组树）。
    pub async fn list_folders(&self, req: ListFoldersRequest) -> AppResult<ListFoldersResponse> {
        let workspace_id = self.resolve_workspace(req.workspace_id).await?;

        let folders = with_conn(&self.state.pool, move |conn| {
            repo::folder::list_by_workspace(conn, workspace_id)
        })
        .await?;

        Ok(ListFoldersResponse {
            workspace_id,
            folders: folders.into_iter().map(summarise_folder).collect(),
        })
    }

    /// 列出请求。给了 `folder_id` 就只看那个文件夹，否则整个工作区。
    pub async fn list_requests(&self, req: ListRequestsRequest) -> AppResult<ListRequestsResponse> {
        let workspace_id = self.resolve_workspace(None).await?;
        let folder_id = req.folder_id;
        let needle = req
            .query
            .map(|q| q.trim().to_lowercase())
            .filter(|q| !q.is_empty());

        let requests = with_conn(&self.state.pool, move |conn| match folder_id {
            Some(id) => repo::request::list_by_folder(conn, Some(id)),
            None => repo::request::list_by_workspace(conn, workspace_id),
        })
        .await?;

        let requests = requests
            .into_iter()
            .filter(|r| match &needle {
                Some(q) => r.name.to_lowercase().contains(q.as_str()),
                None => true,
            })
            .map(summarise)
            .collect();

        Ok(ListRequestsResponse {
            workspace_id,
            requests,
        })
    }

    /// 取完整请求定义。凭据头的值**一律打码**（见 [`key_value`]）。
    pub async fn get_request(&self, req: GetRequestRequest) -> AppResult<RequestDetail> {
        let request = self.load_request(req.request_id).await?;
        Ok(detail(&request))
    }

    /// 列出环境与变量。`is_secret` 的值一律 `••••`（design.md §8.3）。
    pub async fn list_environments(
        &self,
        req: ListEnvironmentsRequest,
    ) -> AppResult<ListEnvironmentsResponse> {
        let workspace_id = self.resolve_workspace(req.workspace_id).await?;

        let rows = with_conn(&self.state.pool, move |conn| {
            let mut out = Vec::new();
            for env in repo::env::list_by_workspace(conn, workspace_id)? {
                let vars = repo::env::list_vars(conn, env.id)?;
                out.push((env, vars));
            }
            Ok(out)
        })
        .await?;

        let environments = rows
            .into_iter()
            .map(|(env, vars)| environment_dto(env, vars))
            .collect();

        Ok(ListEnvironmentsResponse {
            workspace_id,
            environments,
        })
    }

    // ---- 写类的 9 个 ----

    /// 新建请求（一开始就是可发送的完整定义，不是「空壳再补字段」）。
    pub async fn create_request(&self, req: CreateRequestRequest) -> AppResult<RequestDetail> {
        self.writable("create_request")?;

        let workspace_id = self.resolve_workspace(req.workspace_id).await?;

        let folder_id = req.folder_id;
        let name = require_text(req.name.clone(), "name")?;
        // method / url 的校验（非空、方法转大写）统一交给 `apply_fields` ——
        // 与 update 走同一条路，避免「创建时接受 method = ""，更新时拒绝」这种裂缝
        let fields = req.into_fields();

        let created = with_conn(&self.state.pool, move |conn| {
            if let Some(folder_id) = folder_id {
                ensure_usable_folder(conn, folder_id, workspace_id, "folderId")?;
            }
            let mut full = repo::request::create(conn, workspace_id, folder_id, &name)?;
            apply_fields(&mut full, &fields)?;
            repo::request::save(conn, &full)
        })
        .await?;

        Ok(detail(&created))
    }

    /// 局部更新：只改传进来的字段，其余原样保留。
    pub async fn update_request(&self, req: UpdateRequestRequest) -> AppResult<RequestDetail> {
        self.writable("update_request")?;

        let request_id = req.request_id;
        let fields = req.fields;
        let saved = with_conn(&self.state.pool, move |conn| {
            let mut full = repo::request::get(conn, request_id)?;
            apply_fields(&mut full, &fields)?;
            repo::request::save(conn, &full)
        })
        .await?;

        Ok(detail(&saved))
    }

    /// 写一个环境变量：同名改写，没有就追加（整表提交语义，见 `repo::env::replace_vars`）。
    pub async fn set_env_var(&self, req: SetEnvVarRequest) -> AppResult<EnvironmentDto> {
        self.writable("set_env_var")?;

        let key = require_text(req.key, "key")?;
        let environment_id = req.environment_id;
        let value = req.value;

        let (env, vars) = with_conn(&self.state.pool, move |conn| {
            let existing = repo::env::get(conn, environment_id)?;
            let mut vars = repo::env::list_vars(conn, environment_id)?;
            match vars.iter_mut().find(|v| v.key == key) {
                Some(slot) => slot.value = value,
                None => vars.push(crate::storage::models::EnvVar {
                    id: 0,
                    environment_id,
                    key,
                    value,
                    // 通过 MCP 写进来的变量不开 secret：与 UI 的默认一致，
                    // 而且要开 secret 得先有「值不外泄」的读写闭环，目前没有
                    is_secret: false,
                    enabled: true,
                    sort_order: 0,
                }),
            }
            let saved = repo::env::replace_vars(conn, environment_id, &vars)?;
            Ok((existing, saved))
        })
        .await?;

        Ok(environment_dto(env, vars))
    }

    /// 发一次真实请求。响应体 64KB 截断、敏感响应头打码。
    pub async fn send_request(&self, req: SendRequestRequest) -> AppResult<SendRequestResponse> {
        self.writable("send_request")?;

        self.check_environment(req.environment.as_deref()).await?;

        let spec = self.build_spec(&req).await?;
        // runId 只服务于「取消」：MCP 这边没有取消入口，但登记表要求它非空且唯一，
        // 否则 `InflightRegistry` 里两条并发请求会互相覆盖。
        let run_id = next_run_id();

        // 先取出来：`send_request_with` 之后 `spec` 还活着，但把「这次用的类型」
        // 显式传给 DTO 而不是让 DTO 去猜，能让回显这件事只有一个来源。
        let body_type = spec.body_type;
        // `emit_curl = false`：MCP / CLI 的输出会进 AI 的上下文，
        // 而那份 curl 里带着展开后的 token。桌面端的「查看 curl」是另一条路
        // （`send_request` 命令传 `true`），凭据不从这里出去。
        let payload = send_request_with(&self.state, &run_id, &spec, false).await?;

        Ok(response_dto(payload, self.max_body_bytes, body_type))
    }

    /// 跑一轮压测，阻塞到结束（最多 [`LOADTEST_TIMEOUT`]）。
    pub async fn run_load_test(&self, req: RunLoadTestRequest) -> AppResult<RunLoadTestResponse> {
        self.writable("run_load_test")?;

        let config = LoadtestConfig {
            concurrency: req.concurrency,
            total: req.total,
            timeout_ms: req.timeout_ms.unwrap_or(30_000),
            // 压测要一次定死「跟随不跟随重定向」与超时，否则不同请求之间的
            // 「什么算超时」不一致，分位数就没法解释（见 engine 的注释）
            follow_redirects: false,
            keep_alive: true,
            think_time_ms: 0,
            http_version: HttpVersion::Auto,
            warmup_requests: req.warmup_requests.unwrap_or(0),
        };
        // 先校验再建 run 行：一个 concurrency = 0 的输入不该在历史里留下一行残骸
        config.validate()?;

        let request = self.load_request(req.request_id).await?;
        let spec = RequestSpec::from(&request);
        // 与单发同一道校验。压测这里比单发更要紧：一整轮几百个请求都不带 body，
        // 测出来的是「服务端 400 有多快」，而报告上只体现为成功率不高 ——
        // 人会去查服务端，不会想到是 body 没发出去。
        ensure_body_type_consistent(spec.body_type, &spec.body)?;
        let stored = stored_run_config(&spec, &config)?;

        let run = with_conn(&self.state.pool, move |conn| {
            repo::loadtest::create_run(conn, Some(req.request_id), &request.name, &stored)
        })
        .await?;
        let run_id = run.id;

        // 占坑。跨进程拦不住（桌面端同时在跑一轮我们就无从得知），
        // 但同一个 MCP 进程里两次并发调用必须被挡住 —— 两轮叠在一起，
        // 双方测到的都是对方造成的劣化。
        let Some(active) = self.state.loadtests.begin(run_id) else {
            discard_run(&self.state.pool, run_id).await;
            return Err(AppError::InvalidInput {
                field: "loadtest",
                reason: "another load test is already running in this process".into(),
            });
        };

        let previous = self
            .state
            .last_response
            .lock()
            .ok()
            .and_then(|slot| slot.clone());

        let prepared =
            match Pipeline::new(&self.state.pool, &self.state.clients, &self.state.tokens)
                .prepare(&spec, previous.as_deref(), Some(active.token()))
                .await
            {
                Ok(prepared) => prepared,
                Err(err) => {
                    discard_run(&self.state.pool, run_id).await;
                    return Err(err);
                }
            };

        let client = match self.state.clients.get_for_loadtest(
            &prepared.spec.settings,
            config.concurrency,
            config.keep_alive,
        ) {
            Ok(client) => client,
            Err(err) => {
                discard_run(&self.state.pool, run_id).await;
                return Err(err);
            }
        };

        let outcome = tokio::time::timeout(
            LOADTEST_TIMEOUT,
            engine::run(LoadtestDeps {
                client,
                spec: prepared.spec,
                config,
                cancel: active.token().clone(),
                // 没有 AppHandle，进度事件无处可发 —— 调用方要的就是「跑完给我数字」
                on_progress: None,
            }),
        )
        .await;

        match outcome {
            Ok(Ok(outcome)) => {
                let detail = persist_outcome(self.state.pool.clone(), run_id, outcome).await?;
                let summary = detail.summary.ok_or_else(|| {
                    AppError::Internal("loadtest finished without a summary".into())
                })?;
                let value = serde_json::to_value(&summary)
                    .map_err(|e| AppError::Internal(format!("serialize loadtest summary: {e}")))?;
                Ok(RunLoadTestResponse {
                    run_id,
                    counts: LoadtestCountsDto {
                        total: summary.total,
                        succeeded: summary.succeeded,
                        failed: summary.failed,
                    },
                    summary: value,
                })
            }
            Ok(Err(err)) => {
                discard_run(&self.state.pool, run_id).await;
                Err(err)
            }
            Err(_elapsed) => {
                // 超时：先广播取消（在飞的请求自然结束，不打断），
                // 再把这一轮从历史里抹掉 —— 留一行 finished_at 为空、
                // 没有概要的记录，用户看到的是「跑着跑着没了」。
                active.token().cancel();
                discard_run(&self.state.pool, run_id).await;
                Err(AppError::http_with(
                    codes::TIMEOUT,
                    "load test exceeded the tool timeout budget",
                    json!({ "ms": LOADTEST_TIMEOUT.as_millis() as u64 }),
                ))
            }
        }
    }

    /// 强制刷新一个 Token 源，返回剩余有效期。**不返回 token 本身**（§8.3）。
    pub async fn refresh_token(&self, req: RefreshTokenRequest) -> AppResult<RefreshTokenResponse> {
        self.writable("refresh_token")?;

        let source = with_conn(&self.state.pool, move |conn| {
            repo::token::get(conn, req.token_source_id)
        })
        .await?;

        let workspace_id = source.workspace_id;
        let env = with_conn(&self.state.pool, move |conn| {
            repo::env::active_vars(conn, workspace_id)
        })
        .await?;

        let deps = crate::http::TokenDeps {
            pool: &self.state.pool,
            clients: &self.state.clients,
            source: &source,
            env: &env,
            cancel: None,
        };
        let values = self.state.tokens.refresh_now(&deps).await?.values;

        Ok(RefreshTokenResponse {
            source_id: source.id,
            source_name: source.name,
            // 剩余秒数走 `http::token` 那一份实现（含「时间戳读不懂就是 None」），
            // 不在这里再解析一次 ISO 字符串 —— 同一件事两处算，边界上迟早分叉
            expires_in_seconds: remaining_seconds(values.expires_at.as_deref(), chrono::Utc::now()),
            expires_at: values.expires_at,
            has_refresh_token: values.refresh_token.is_some(),
        })
    }

    /// 粘贴 curl 即导入。复用与 UI 完全相同的解析 + 落库路径。
    pub async fn import_curl(&self, req: ImportCurlRequest) -> AppResult<ImportCurlResponse> {
        self.writable("import_curl")?;

        let workspace_id = self.resolve_workspace(req.workspace_id).await?;

        let source = ImportSource {
            text: req.curl_text,
            workspace_id,
            target_folder_id: req.target_folder_id,
            file_name: None,
            // 占位名是**会写进库的数据**，而后端不产出面向用户的文案
            // （见 `crate::error` 的契约）。桌面端从 i18n 传中文进来，
            // MCP 这边没有 i18n 上下文，给一个中性词。
            labels: ImportLabels {
                untitled_folder: "untitled".into(),
                untitled_request: "untitled".into(),
                untitled_collection: "untitled".into(),
            },
        };

        let (outcome, created) = with_conn(&self.state.pool, move |conn| {
            // 导入前后的 id 水位线：`apply_plan` 只回计数不回 id，
            // 而「返回创建的请求」是 design.md §8.2 明确要求的。
            // 放在同一个闭包里（同一个连接、紧挨着执行）不会有并发插入混进来。
            let mark: i64 =
                conn.query_row("SELECT COALESCE(MAX(id), 0) FROM request", [], |r| r.get(0))?;
            let outcome = import_into_folder(conn, &source, Conflict::Merge, false)?;
            let created = repo::request::list_by_workspace(conn, workspace_id)?
                .into_iter()
                .filter(|r| r.id > mark)
                .map(summarise)
                .collect::<Vec<_>>();
            Ok((outcome, created))
        })
        .await?;

        Ok(ImportCurlResponse {
            format: outcome.format.as_str().to_string(),
            counts: ImportCountsDto {
                created: outcome.counts.created as i64,
                updated: outcome.counts.updated as i64,
                skipped: outcome.counts.skipped as i64,
                removed: outcome.counts.removed as i64,
            },
            created,
            // 后端不产出面向用户的文案，而警告恰恰是「给人看的一小段解释」。
            // 这里只把**机器可读的部分**（原因码 + 出问题的对象 + 开发者细节）拼出来，
            // 具体怎么措辞由 AI 自己决定 —— 与桌面端拿 `kind` 去查 i18n 是同一个契约。
            warnings: outcome.warnings.iter().map(describe_warning).collect(),
        })
    }

    /// 移动到其他分类。
    pub async fn move_request(&self, req: MoveRequestRequest) -> AppResult<RequestSummary> {
        self.writable("move_request")?;

        let request_id = req.request_id;
        let target = req.target_folder_id;
        let moved = with_conn(&self.state.pool, move |conn| {
            let request = repo::request::get(conn, request_id)?;
            ensure_usable_folder(conn, target, request.workspace_id, "targetFolderId")?;
            repo::request::move_to_folder(conn, request_id, Some(target))
        })
        .await?;

        Ok(summarise(moved))
    }

    /// 软删除（进回收站，可还原）。
    pub async fn delete_request(
        &self,
        req: DeleteRequestRequest,
    ) -> AppResult<DeleteRequestResponse> {
        self.writable("delete_request")?;

        let request_id = req.request_id;
        with_conn(&self.state.pool, move |conn| {
            repo::request::soft_delete(conn, request_id)
        })
        .await?;

        Ok(DeleteRequestResponse {
            request_id,
            restorable: true,
        })
    }

    // ---- 只给 CLI 的 2 个（MCP 的 tool 表刻意不暴露，见模块头「13 与 15」） ----

    /// 新建文件夹。父节点必须是「存活 + 同工作区」—— 与建请求共用同一条校验
    /// （[`ensure_usable_folder`]），否则会建出树里看不见的孤儿。
    pub async fn create_folder(&self, req: CreateFolderRequest) -> AppResult<FolderSummary> {
        self.writable("create_folder")?;

        let workspace_id = self.resolve_workspace(req.workspace_id).await?;
        // 名字规则与桌面端是同一份实现（长度上限必须是同一个数，见 `service::validate`）
        let name = clean_name(&req.name, "name")?;
        let parent_id = req.parent_id;

        let folder = with_conn(&self.state.pool, move |conn| {
            if let Some(parent_id) = parent_id {
                ensure_usable_folder(conn, parent_id, workspace_id, "parentId")?;
            }
            repo::folder::create(conn, workspace_id, parent_id, &name)
        })
        .await?;

        Ok(summarise_folder(folder))
    }

    /// 新建环境。建出来是个空环境且 `is_active = 0`，变量接着用 `set_env_var` 写。
    ///
    /// 刻意不提供「顺手激活它」：激活位是**用户的状态**，不是这次创建的一部分。
    /// 这与 [`Ops::check_environment`] 拒绝隐式切环境是同一条理由。
    pub async fn create_environment(
        &self,
        req: CreateEnvironmentRequest,
    ) -> AppResult<EnvironmentDto> {
        self.writable("create_environment")?;

        let workspace_id = self.resolve_workspace(req.workspace_id).await?;
        let name = clean_name(&req.name, "name")?;

        let (env, vars) = with_conn(&self.state.pool, move |conn| {
            let env = repo::env::create(conn, workspace_id, &name)?;
            let vars = repo::env::list_vars(conn, env.id)?;
            Ok((env, vars))
        })
        .await?;

        Ok(environment_dto(env, vars))
    }

    // ---- 内部工具 ----

    /// 解析要用哪个工作区：给了 id 就校验它存在，没给就用当前（唯一）的那个。
    async fn resolve_workspace(&self, requested: Option<i64>) -> AppResult<i64> {
        with_conn(&self.state.pool, move |conn| match requested {
            Some(id) => Ok(repo::workspace::get(conn, id)?.id),
            None => repo::workspace::active(conn)?
                .map(|ws| ws.id)
                .ok_or_else(|| AppError::Internal("this database has no active workspace".into())),
        })
        .await
    }

    async fn load_request(&self, request_id: i64) -> AppResult<ApiRequest> {
        with_conn(&self.state.pool, move |conn| {
            repo::request::get(conn, request_id)
        })
        .await
    }

    /// 把「已保存请求 / 内联定义」两种输入收成同一个 [`RequestSpec`]。
    async fn build_spec(&self, req: &SendRequestRequest) -> AppResult<RequestSpec> {
        let mut spec = match req.request_id {
            Some(id) => RequestSpec::from(&self.load_request(id).await?),
            None => RequestSpec {
                method: req.method.clone().ok_or_else(|| AppError::InvalidInput {
                    field: "method",
                    reason: "no requestId given, so an inline method is required".into(),
                })?,
                url: req.url.clone().ok_or_else(|| AppError::InvalidInput {
                    field: "url",
                    reason: "no requestId given, so an inline url is required".into(),
                })?,
                params: Vec::new(),
                headers: Vec::new(),
                body_type: BodyType::None,
                body: String::new(),
                settings: crate::storage::models::RequestSettings::default(),
                folder_id: req.folder_id,
                // 内联定义里的 multipart 字段：留空，让 `body` 的
                // `k=v&k=@路径` 回退解析来接管（见 `http::execute::build_body`）。
                // 这条路径上多一个参数就是多一处要解释「谁覆盖谁」的地方。
                form_fields: Vec::new(),
            },
        };

        if let Some(value) = &req.method {
            spec.method = value.clone();
        }
        if let Some(value) = &req.url {
            spec.url = value.clone();
        }
        if let Some(value) = &req.params {
            spec.params = to_key_values(value);
        }
        if let Some(value) = &req.headers {
            spec.headers = to_key_values(value);
        }
        if let Some(value) = &req.body {
            spec.body = value.clone();
        }
        if let Some(raw) = &req.body_type {
            spec.body_type = parse_body_type(raw)?;
        }
        // folder_id 只在内联定义时生效：已保存请求的归属文件夹是它的属性，
        // 不该被一次发送临时改写（Token 源与授权继承都挂在它上面）
        if req.request_id.is_none() {
            spec.folder_id = req.folder_id;
        }

        // ⚠️ 下面两行的**顺序是契约：先推断、后校验**。反过来的话
        // `--body '{\"a\":1}'`（没给 type）会先被判成「none + 非空 body」直接报错，
        // 于是 B1 等于没修 —— 从一个坑换到另一个坑。
        //
        // 推断只在**调用方这次给了 body** 时生效。已保存请求自带的 `body_type`
        // 是用户的持久选择（`none` 就是「不发请求体」），一次 send 不该改写它；
        // 那种情况交给下面的硬校验去拦，同样是显式失败而不是静默。
        if req.body.is_some() {
            infer_body_type(&mut spec.body_type, &spec.body, req.body_type.is_some());
        }
        ensure_body_type_consistent(spec.body_type, &spec.body)?;

        Ok(spec)
    }

    /// 校验 `environment` 参数。
    ///
    /// 目前只支持「就是那个激活的环境」。为什么不做别的：占位符解析用的是
    /// `Pipeline` 里的**激活环境**（它背后是 `environment.is_active` 这一列），
    /// 要支持按名字临时切换，就得给流水线加一条环境注入口。
    /// 与其偷偷把用户激活的环境改掉（一次 AI 调用顺手改了桌面端的状态，
    /// 而且只读模式之外无人可察），不如明确报 `E_NOT_IMPLEMENTED` 并把
    /// 可用环境列给它 —— AI 可以据此提示用户去切，或者干脆不带这个参数。
    async fn check_environment(&self, requested: Option<&str>) -> AppResult<()> {
        let Some(name) = requested.map(str::trim).filter(|n| !n.is_empty()) else {
            return Ok(());
        };

        let workspace_id = self.resolve_workspace(None).await?;
        let (active, available) = with_conn(&self.state.pool, move |conn| {
            let envs = repo::env::list_by_workspace(conn, workspace_id)?;
            let active = envs
                .iter()
                .find(|env| env.is_active)
                .map(|env| env.name.clone());
            Ok((
                active,
                envs.into_iter().map(|env| env.name).collect::<Vec<_>>(),
            ))
        })
        .await?;

        if active.as_deref() == Some(name) {
            return Ok(());
        }

        Err(AppError::http_with(
            codes::NOT_IMPLEMENTED,
            format!(
                "environment override is not supported: requested `{name}`, active is {active:?}"
            ),
            json!({
                "feature": "environmentOverride",
                "requested": name,
                "active": active,
                "available": available,
            }),
        ))
    }
}

/// 把一次响应收成回给 AI 的形状（截断 + 打码）。
///
/// `body_type` **不来自响应**（HTTP 响应里没有这个概念）—— 它是这次请求实际用的
/// 那个类型，由调用方从 `RequestSpec` 传进来，只为回显。
fn response_dto(
    payload: HttpResponsePayload,
    max_body_bytes: usize,
    body_type: BodyType,
) -> SendRequestResponse {
    let headers = payload
        .headers
        .iter()
        .map(|header| HeaderDto {
            name: header.name.clone(),
            value: mask_secret_value(&header.name, &header.value),
        })
        .collect();

    let (body, cut) = truncate_body(payload.body, max_body_bytes);

    SendRequestResponse {
        status: payload.status,
        status_text: payload.status_text,
        http_version: payload.http_version,
        url: payload.url,
        redirected: payload.redirected,
        body_type: body_type.as_db_str().to_string(),
        headers,
        body,
        body_encoding: match payload.body_encoding {
            crate::http::BodyEncoding::Text => "text",
            crate::http::BodyEncoding::Lossy => "lossy",
            crate::http::BodyEncoding::Binary => "binary",
        }
        .to_string(),
        body_size: payload.body_size,
        // 执行层自己那道 8MB 上限也要算进来：它先切过一刀的话，
        // 这里看着「没超 64KB」其实内容已经不全了
        body_truncated: cut || payload.truncated,
        ttfb_ms: payload.ttfb_ms,
        download_ms: payload.download_ms,
        total_ms: payload.total_ms,
        unresolved_placeholders: payload
            .unresolved_placeholders
            .into_iter()
            .map(|hit| UnresolvedDto {
                raw: hit.raw,
                namespace: hit.namespace.to_string(),
                key: hit.key,
                reason: hit.reason.to_string(),
            })
            .collect(),
    }
}

/// 按名字判定「这一行是不是凭据」并打码，其余原样。
///
/// 用在两处：
/// * 响应头 —— `set-cookie` / `authorization` / `*-key` / `*-token` 之类；
/// * **`get_request` 回给 AI 的请求头 / 查询参数** —— 007 之后凭据就住在
///   那两个数组里，不打码等于把用户粘进去的长期 token 直接交出去（§8.3）。
///
/// 名单只有一份（[`crate::redact`]，与 curl 导出、日志脱敏共用），
/// 这里只是把「按名字判」换成调用点读得顺的写法。
fn mask_secret_value(name: &str, value: &str) -> String {
    crate::redact::mask_value(name, value)
}

/// 按 **UTF-8 字符边界**截断响应体。
///
/// 为什么不能直接 `body.truncate(limit)`：`String::truncate` 在非边界处
/// 会 panic。一个 64KB 处正好落着半个汉字的响应就能把整个 MCP 进程打挂 ——
/// 而 `http::execute` 里那段 `valid_up_to()` 的写法正是同一个问题的另一半
/// （那里是在字节切片上切，这里是在 `String` 上切）。
///
/// `limit` 由调用入口给（见 [`Ops::with_body_limit`]），截与不截的**判断**在这里
/// —— 上限可以变，截断这件事不能漏。
fn truncate_body(body: String, limit: usize) -> (String, bool) {
    if body.len() <= limit {
        return (body, false);
    }
    let mut end = limit;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = body;
    out.truncate(end);
    (out, true)
}

/// `body` 去掉首尾空白后是否真的有内容。
///
/// 判据刻意不是 `is_some()`：`--body ""` 与 `--body "  "` 都是「用户写了、
/// 但什么都没写」，把它们当成有内容会推断出一个没人要的 `json`。
fn has_body(body: &str) -> bool {
    !body.trim().is_empty()
}

/// 补齐「给了 body 却没给 bodyType」这一步。
///
/// ## 这就是 B1 的修法，以及为什么只在 `type_given == false` 时才动
///
/// 原来的行为是：`body_type` 默认 `none`，而 `none` 在执行层等于
/// 「一个字节都不发」（`http::execute::build_body` 的 `None => (None, None)`）。
/// 于是 `send --body '{"empId":0}'` 把 body 存进 spec、发出去却是空的，
/// **零报错** —— 现场看起来像服务端坏了（服务端回 400「no JSON tokens」）。
///
/// 推断成 `json` 而不是 `raw`：`--body` 的用法十有八九是贴一段 JSON，
/// 猜错的代价也只是多一个 `Content-Type: application/json`（用户显式给了
/// `Content-Type` 时那个头不会被覆盖，见 `build_headers` 之后的那段）。
///
/// `type_given` 为真时一概不动 —— 用户说了 `raw` 就是 `raw`；
/// 说了 `none` 又带 body 属自相矛盾，交给 [`ensure_body_type_consistent`] 报错，
/// 而不是替他改一个。
fn infer_body_type(body_type: &mut BodyType, body: &str, type_given: bool) {
    if !type_given && *body_type == BodyType::None && has_body(body) {
        *body_type = BodyType::Json;
    }
}

/// `body_type = none` 却又带着非空 `body` —— 这个状态一律拒绝。
///
/// 三个来源、同一个处置（显式失败，绝不静默）：
/// 1. 调用方显式写了 `--body-type none` 同时给了 body（自相矛盾的输入）；
/// 2. 库里的**存量数据**：老版本的 `create --body` 会把 `body_type` 落成 `none`
///    （正是 B1 的另一半），发送时不能装作没看见；
/// 3. 桌面端把一条请求的 `bodyType` 改成 `none` 但 body 文本还在的历史行。
///
/// ⚠️ **反向用例是硬约束**：`none` + **空** body 完全合法，那是「确实不发请求体」
/// 的正常表达，必须原样放行 —— 判据是 [`has_body`]，不是 `is_some()`。
fn ensure_body_type_consistent(body_type: BodyType, body: &str) -> AppResult<()> {
    if body_type == BodyType::None && has_body(body) {
        return Err(AppError::InvalidInput {
            field: "bodyType",
            reason: "bodyType is `none` but body is not empty;                      set bodyType to json/raw/urlencoded, or clear the body"
                .into(),
        });
    }
    Ok(())
}

/// 请求内的字段覆盖。`None` 一律表示「不动这个字段」。
fn apply_fields(target: &mut ApiRequest, fields: &RequestFields) -> AppResult<()> {
    if let Some(name) = fields.name.as_deref() {
        target.name = require_text(name.to_string(), "name")?;
    }
    if let Some(method) = fields.method.as_deref() {
        target.method = require_text(method.to_string(), "method")?.to_uppercase();
    }
    if let Some(url) = &fields.url {
        target.url = url.clone();
    }
    if let Some(params) = &fields.params {
        target.params = to_key_values(params);
    }
    if let Some(headers) = &fields.headers {
        target.headers = to_key_values(headers);
    }
    if let Some(raw) = fields.body_type.as_deref() {
        target.body_type = parse_body_type(raw)?;
    }
    if let Some(body) = &fields.body {
        target.body = body.clone();
    }

    // 先推断、后校验 —— 顺序是契约，理由见 `build_spec` 里那段说明。
    //
    // 推断只在调用方**这次给了 body** 时生效：`update 5 --name x` 这种没碰请求体的
    // 调用不该顺手改写一条存量记录的 `body_type`。
    if fields.body.is_some() {
        infer_body_type(
            &mut target.body_type,
            &target.body,
            fields.body_type.is_some(),
        );
    }
    // 校验同样只在用户碰了 body 相关字段时才做。否则「库里那条本来就是
    // none + 有内容」的历史记录会让一次改名也跟着报错 —— 那种记录该在
    // **发送**时被拦（那才是它会静默丢数据的地方），而不是在无关的写操作上。
    if fields.body.is_some() || fields.body_type.is_some() {
        ensure_body_type_consistent(target.body_type, &target.body)?;
    }
    Ok(())
}

/// 正文类型解析。**不用 `BodyType::parse`** —— 那个函数对未知值是「退化成 none」，
/// 对存量数据是宽容，对 tool 入参是静默错误：AI 写了 `application/json`，
/// 我们会当成「无正文」照发，而它不会知道。
fn parse_body_type(raw: &str) -> AppResult<BodyType> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" | "" => Ok(BodyType::None),
        "json" => Ok(BodyType::Json),
        "form" => Ok(BodyType::Form),
        "urlencoded" => Ok(BodyType::Urlencoded),
        "raw" => Ok(BodyType::Raw),
        "binary" => Ok(BodyType::Binary),
        "graphql" => Ok(BodyType::Graphql),
        other => Err(AppError::InvalidInput {
            field: "bodyType",
            reason: format!("`{other}` is not one of none/json/form/urlencoded/raw/binary/graphql"),
        }),
    }
}

fn require_text(raw: String, field: &'static str) -> AppResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidInput {
            field,
            reason: "must not be empty".into(),
        });
    }
    Ok(trimmed.to_string())
}

fn summarise(request: ApiRequest) -> RequestSummary {
    RequestSummary {
        id: request.id,
        folder_id: request.folder_id,
        name: request.name,
        method: request.method,
        url: request.url,
    }
}

fn summarise_folder(folder: Folder) -> FolderSummary {
    FolderSummary {
        id: folder.id,
        parent_id: folder.parent_id,
        name: folder.name,
        sort_order: folder.sort_order,
        is_token_source: folder.is_token_source,
        is_system: folder.is_system,
    }
}

/// `Environment` + 变量 → 对外形状。密文变量的值一律 `••••`。
///
/// 抽出来是因为它有三个调用点（`list_environments` / `set_env_var` /
/// `create_environment`）。打码那行抄三遍的话，迟早有一处会漏 ——
/// 而漏掉的那一处就是「secret 直接进了 AI 的上下文」，§8.3 明令禁止的事。
fn environment_dto(env: Environment, vars: Vec<EnvVar>) -> EnvironmentDto {
    EnvironmentDto {
        id: env.id,
        name: env.name,
        is_active: env.is_active,
        vars: vars
            .into_iter()
            .map(|var| EnvVarDto {
                key: var.key,
                // ★ 不打码就是把 secret 直接交给 AI —— §8.3 明令禁止
                value: if var.is_secret {
                    MASK.to_string()
                } else {
                    var.value
                },
                is_secret: var.is_secret,
                enabled: var.enabled,
            })
            .collect(),
    }
}

fn detail(request: &ApiRequest) -> RequestDetail {
    RequestDetail {
        id: request.id,
        folder_id: request.folder_id,
        workspace_id: request.workspace_id,
        name: request.name.clone(),
        method: request.method.clone(),
        url: request.url.clone(),
        params: request.params.iter().map(key_value).collect(),
        headers: request.headers.iter().map(key_value).collect(),
        body_type: request.body_type.as_db_str().to_string(),
        body: request.body.clone(),
        settings: RequestSettingsDto {
            follow_redirects: request.settings.follow_redirects,
            max_redirects: request.settings.max_redirects,
            verify_tls: request.settings.verify_tls,
            timeout_ms: request.settings.timeout_ms,
            http_version: request.settings.http_version.as_db_str().to_string(),
            has_proxy: request.settings.proxy.is_some(),
        },
        pre_script: request.pre_script.clone(),
        test_script: request.test_script.clone(),
    }
}

/// 一行键值 → DTO。
///
/// 🔴 **`value` 要按头名打码**。凭据现在是 `headers` / `params` 里的一行，
/// 而 MCP 的 `get_request` 把这些行直接回给 AI —— 不打码就等于
/// 把用户粘进去的长期 token 交出去（design.md §8.3 的边界）。
/// 判据复用 [`crate::redact`] 那份名单：与 curl 导出、日志脱敏同一套，
/// **只存在一份**，改一处三处都跟着动。
fn key_value(item: &KeyValueItem) -> KeyValueDto {
    KeyValueDto {
        key: item.key.clone(),
        value: mask_secret_value(&item.key, &item.value),
        enabled: item.enabled,
        desc: item.desc.clone(),
    }
}

/// 同一个进程里给每次发送发一个不重复的 runId。
///
/// 用「进程 id + 自增序号」而不是 UUID：项目没有 uuid 依赖，而这里需要的只是
/// **进程内唯一**（`InflightRegistry` 的键）。跨进程不需要唯一 —— 两个进程
/// 各有各的登记表。
fn next_run_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!(
        "run-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// 把一条导入警告压成一行可读文本。
///
/// `ImportWarning` 的三个字段各自有明确分工（`kind` 机器可读、`subject` 出问题的
/// 对象、`detail` 给开发者的细节），但 MCP 这边没有 i18n 上下文去把 `kind`
/// 翻成完整句子。所以只做一件无害的事：把三者**原样拼起来**——
/// 既不替客户端编文案，也不把细节丢掉。
fn describe_warning(warning: &crate::importer::ImportWarning) -> String {
    match warning.subject.as_deref() {
        Some(subject) => format!("{:?}: {subject} ({})", warning.kind, warning.detail),
        None => format!("{:?}: {}", warning.kind, warning.detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_truncation_keeps_utf8_boundaries() {
        // 每个汉字 3 字节，默认上限必然落在某个字中间
        let body = "中".repeat(DEFAULT_MAX_BODY_BYTES);
        assert!(body.len() > DEFAULT_MAX_BODY_BYTES);

        let (cut, truncated) = truncate_body(body, DEFAULT_MAX_BODY_BYTES);
        assert!(truncated);
        assert!(cut.len() <= DEFAULT_MAX_BODY_BYTES);
        assert_eq!(cut.len() % 3, 0, "截断必须落在字符边界上");
        assert!(cut.chars().all(|c| c == '中'));
    }

    #[test]
    fn a_body_within_the_limit_is_untouched() {
        let (body, truncated) = truncate_body("hello".to_string(), DEFAULT_MAX_BODY_BYTES);
        assert_eq!(body, "hello");
        assert!(!truncated);
    }

    /// 上限是**入口给**的（CLI 会把它调大甚至设成 `usize::MAX`），
    /// 但截断这条判断必须在 core 里 —— 换个数字不该换掉行为。
    #[test]
    fn the_body_limit_follows_the_caller_but_truncation_still_happens() {
        let body = "x".repeat(100);

        // 调大：100 字节的响应在 4KB 上限下原样通过
        let (kept, cut) = truncate_body(body.clone(), 4096);
        assert_eq!(kept, body);
        assert!(!cut);

        // 调小：同样的响应被切到 10 字节
        let (sliced, cut) = truncate_body(body, 10);
        assert_eq!(sliced.len(), 10);
        assert!(cut);
    }

    #[test]
    fn sensitive_headers_are_masked_and_others_are_not() {
        assert_eq!(mask_secret_value("Set-Cookie", "sid=1"), MASK);
        assert_eq!(mask_secret_value("Authorization", "Bearer x"), MASK);
        // 凭据类自定义头：真实接口里这些和 Authorization 等价，不能漏
        assert_eq!(mask_secret_value("X-Api-Key", "k"), MASK);
        assert_eq!(mask_secret_value("Api-Key", "k"), MASK);
        assert_eq!(mask_secret_value("Proxy-Authorization", "Basic x"), MASK);
        assert_eq!(mask_secret_value("X-Auth-Token", "t"), MASK);
        assert_eq!(mask_secret_value("X-Client-Secret", "s"), MASK);
        assert_eq!(mask_secret_value("X-User-Password", "p"), MASK);
        // 反向：不能打过头 —— 排障要看的头必须原样透出
        assert_eq!(mask_secret_value("X-Request-Id", "abc-123"), "abc-123");
        assert_eq!(
            mask_secret_value("Content-Type", "application/json"),
            "application/json"
        );
    }

    #[test]
    fn unknown_body_type_is_rejected_instead_of_silently_becoming_none() {
        assert_eq!(parse_body_type("JSON").unwrap(), BodyType::Json);
        assert_eq!(parse_body_type("").unwrap(), BodyType::None);
        assert!(matches!(
            parse_body_type("application/json").unwrap_err(),
            AppError::InvalidInput {
                field: "bodyType",
                ..
            }
        ));
    }

    #[test]
    fn run_ids_are_unique_within_the_process() {
        let first = next_run_id();
        let second = next_run_id();
        assert_ne!(first, second);
        assert!(first.starts_with("run-"));
    }

    // ============================================================
    // body 与 bodyType 的一致性（纯函数层）
    // ============================================================

    /// 核心用例：给了 body 没给 type —— 推断成 json，**不能**静默丢弃。
    #[test]
    fn a_body_without_a_type_is_inferred_as_json() {
        let mut body_type = BodyType::None;
        infer_body_type(&mut body_type, r#"{"a":1}"#, false);
        assert_eq!(body_type, BodyType::Json);
    }

    /// **反向用例**（缺陷单特意点名的那个）：`none` + 空 body 完全合法，
    /// 修的时候不能把「确实不发请求体」也判成错误。
    #[test]
    fn none_with_an_empty_body_stays_legal() {
        assert!(ensure_body_type_consistent(BodyType::None, "").is_ok());
        // 只有空白也算「什么都没写」—— 否则 `--body "  "` 会被推断成 json
        assert!(ensure_body_type_consistent(BodyType::None, " \n\t ").is_ok());

        let mut body_type = BodyType::None;
        infer_body_type(&mut body_type, " \n\t ", false);
        assert_eq!(body_type, BodyType::None);
    }

    /// 显式给的类型一概不被推断改写（`raw` 就是 `raw`）。
    #[test]
    fn an_explicit_type_is_never_overwritten_by_the_inference() {
        for given in [BodyType::Json, BodyType::Raw, BodyType::Urlencoded] {
            let mut body_type = given;
            infer_body_type(&mut body_type, "x=1", true);
            assert_eq!(body_type, given);
        }
    }

    /// 显式的 `none` + body 不会被偷偷升级成 json —— 它该走报错那条路。
    #[test]
    fn an_explicit_none_is_not_upgraded_by_the_inference() {
        let mut body_type = BodyType::None;
        infer_body_type(&mut body_type, r#"{"a":1}"#, true);
        assert_eq!(body_type, BodyType::None);
    }

    /// `none` + 非空 body 是自相矛盾的状态：报错，而不是替用户选一个。
    #[test]
    fn none_with_a_body_is_rejected_instead_of_silently_dropped() {
        match ensure_body_type_consistent(BodyType::None, r#"{"a":1}"#).unwrap_err() {
            AppError::InvalidInput { field, .. } => assert_eq!(field, "bodyType"),
            other => panic!("期望 InvalidInput，实际 {other}"),
        }
    }

    // ============================================================
    // 带库的端到端：create / send / 两条 CLI 专有操作
    // ============================================================

    /// 「析构即删」的临时目录。
    struct TempDir(std::path::PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            if let Err(e) = std::fs::remove_dir_all(&self.0) {
                println!("[reqly-test] 临时目录没能删掉 {}: {e}", self.0.display());
            }
        }
    }

    struct TempOps {
        // ⚠️ 顺序有意义：`ops` 持有连接池，声明必须在 `_dir` 之前 ——
        // 否则删目录时文件还被打开着，Windows 删不掉，每个测试都在 TEMP 里
        // 留一个目录（与 `storage::testkit` 的 `TestDb` 是同一个坑）。
        ops: Ops,
        _dir: TempDir,
    }

    fn temp_ops(tag: &str, read_only: bool) -> TempOps {
        let dir = std::env::temp_dir().join(format!(
            "reqly-ops-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        // 上一次跑崩了会留下同名目录
        let _ = std::fs::remove_dir_all(&dir);
        let state = Arc::new(AppState::new(dir.join("reqly.db")).expect("建库"));
        TempOps {
            ops: Ops::new(state, read_only),
            _dir: TempDir(dir),
        }
    }

    /// 建一条带 body 的请求，返回它的 id。直接走 `create_request`，
    /// 因为要测的正是这条路径。
    async fn seed_request(ops: &Ops, name: &str) -> i64 {
        ops.create_request(CreateRequestRequest {
            folder_id: None,
            workspace_id: None,
            name: name.into(),
            method: "post".into(),
            url: "http://127.0.0.1:1/x".into(),
            params: None,
            headers: None,
            body_type: None,
            body: Some(r#"{"a":1}"#.into()),
        })
        .await
        .expect("建请求")
        .id
    }

    /// `requests create --body`（不给 type）过去会落成 `bodyType: "none"` ——
    /// 请求存着 body 却发不出去。这是缺陷单里第 3 个实测点。
    #[tokio::test]
    async fn a_created_request_keeps_its_type_in_sync_with_its_body() {
        let t = temp_ops("create-body-type", false);

        let created = t
            .ops
            .create_request(CreateRequestRequest {
                folder_id: None,
                workspace_id: None,
                name: "BUGTEST".into(),
                method: "post".into(),
                url: "http://127.0.0.1:1/x".into(),
                params: None,
                headers: None,
                body_type: None,
                body: Some(r#"{"a":1}"#.into()),
            })
            .await
            .expect("建请求");

        assert_eq!(
            created.body_type, "json",
            "落库的 bodyType 必须与 body 一致"
        );
        assert_eq!(created.body, r#"{"a":1}"#);
        // method 仍然走 `apply_fields` 统一大写
        assert_eq!(created.method, "POST");
    }

    /// 内联 `send --body`（不给 type）：`build_spec` 出来的 spec 必须是 json，
    /// 否则执行层的 `BodyType::None => (None, None)` 会把 body 吞掉。
    #[tokio::test]
    async fn an_inline_send_with_a_body_gets_a_type_that_can_be_sent() {
        let t = temp_ops("inline-body-type", false);

        let spec = t
            .ops
            .build_spec(&SendRequestRequest {
                method: Some("POST".into()),
                url: Some("http://127.0.0.1:1/x".into()),
                body: Some(r#"{"a":1}"#.into()),
                ..Default::default()
            })
            .await
            .expect("建 spec");
        assert_eq!(spec.body_type, BodyType::Json);

        // 反向：两者都不给 —— 照旧不发请求体，**不能**变成报错
        let spec = t
            .ops
            .build_spec(&SendRequestRequest {
                method: Some("GET".into()),
                url: Some("http://127.0.0.1:1/x".into()),
                ..Default::default()
            })
            .await
            .expect("不带 body 的内联发送必须仍然合法");
        assert_eq!(spec.body_type, BodyType::None);
    }

    /// 存量数据：库里已经有一条 `body_type = 'none'` 却存着 body 的请求
    /// （老版本 `create --body` 的产物）。发送时必须显式报错，不能静默发空 body。
    #[tokio::test]
    async fn a_stale_none_request_is_rejected_at_send_time() {
        let t = temp_ops("stale-none", false);
        let request_id = seed_request(&t.ops, "legacy").await;

        // 复刻老版本落库的形状
        t.ops
            .state
            .pool
            .get()
            .expect("连接")
            .execute(
                "UPDATE request SET body_type = 'none' WHERE id = ?1",
                rusqlite::params![request_id],
            )
            .expect("改回 none");

        match t
            .ops
            .build_spec(&SendRequestRequest {
                request_id: Some(request_id),
                ..Default::default()
            })
            .await
            .unwrap_err()
        {
            AppError::InvalidInput { field, .. } => assert_eq!(field, "bodyType"),
            other => panic!("期望 InvalidInput，实际 {other}"),
        }
    }

    /// 反过来：同样 `body_type = 'none'`，把 body 清空之后就该能发 ——
    /// 「不发请求体」是合法表达，不能一并拦掉。
    #[tokio::test]
    async fn a_none_request_with_an_empty_body_still_builds_a_spec() {
        let t = temp_ops("none-empty-body", false);
        let request_id = seed_request(&t.ops, "no-body").await;

        t.ops
            .state
            .pool
            .get()
            .expect("连接")
            .execute(
                "UPDATE request SET body_type = 'none', body = '' WHERE id = ?1",
                rusqlite::params![request_id],
            )
            .expect("清 body");

        let spec = t
            .ops
            .build_spec(&SendRequestRequest {
                request_id: Some(request_id),
                ..Default::default()
            })
            .await
            .expect("空 body + none 必须放行");
        assert_eq!(spec.body_type, BodyType::None);
    }

    /// 建文件夹（B3），含父节点校验 —— 与建请求共用 `ensure_usable_folder`。
    #[tokio::test]
    async fn folders_can_be_created_from_ops() {
        let t = temp_ops("create-folder", false);

        let root = t
            .ops
            .create_folder(CreateFolderRequest {
                workspace_id: None,
                parent_id: None,
                name: "RMS-报价权限".into(),
            })
            .await
            .expect("建根文件夹");
        assert_eq!(root.parent_id, None);

        let child = t
            .ops
            .create_folder(CreateFolderRequest {
                workspace_id: None,
                parent_id: Some(root.id),
                name: "FP-01".into(),
            })
            .await
            .expect("建子文件夹");
        assert_eq!(child.parent_id, Some(root.id));

        // 父节点不存在 → 走仓储的 NotFound，而不是建出一个树里看不见的孤儿
        let err = t
            .ops
            .create_folder(CreateFolderRequest {
                workspace_id: None,
                parent_id: Some(9999),
                name: "x".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), codes::NOT_FOUND);

        // 名字规则与桌面端是同一份实现
        let err = t
            .ops
            .create_folder(CreateFolderRequest {
                workspace_id: None,
                parent_id: None,
                name: "   ".into(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), codes::INVALID_INPUT);

        let listed = t
            .ops
            .list_folders(ListFoldersRequest::default())
            .await
            .expect("列文件夹");
        assert_eq!(
            listed.folders.len(),
            3,
            "两个自建 + 启动时补的系统分类「token刷新」"
        );

        // 系统分类必须出现在列表里，而且排在最前 —— AI 靠 `isSystem` 认出
        // 「这个别动」，靠 sort_order 认出「这是那个固定分类」
        let first = &listed.folders[0];
        assert!(first.is_system, "系统分类要透出 isSystem");
        assert_eq!(first.name, crate::storage::repo::folder::SYSTEM_FOLDER_NAME);
    }

    /// 建环境（B4）。建出来是空的，且**不是**激活状态。
    #[tokio::test]
    async fn environments_can_be_created_from_ops() {
        let t = temp_ops("create-env", false);

        let created = t
            .ops
            .create_environment(CreateEnvironmentRequest {
                workspace_id: None,
                name: "预发".into(),
            })
            .await
            .expect("建环境");

        assert_eq!(created.name, "预发");
        assert!(created.vars.is_empty());
        // 激活位是用户的状态，创建不该顺手改它
        assert!(!created.is_active);

        // 建完能直接往里写变量（两条命令刚好接得上）
        let with_var = t
            .ops
            .set_env_var(SetEnvVarRequest {
                environment_id: created.id,
                key: "BASE_URL".into(),
                value: "https://pre.example.com".into(),
            })
            .await
            .expect("写变量");
        assert_eq!(with_var.vars.len(), 1);
        assert_eq!(with_var.vars[0].key, "BASE_URL");
    }

    /// 两条 CLI 专有操作同样受只读闸门管 —— 闸门就是 `writable()` 那一行，
    /// 新加操作时最容易漏的正是它。
    #[tokio::test]
    async fn the_cli_only_operations_obey_the_read_only_gate() {
        let t = temp_ops("cli-only-read-only", true);

        let folder_err = t
            .ops
            .create_folder(CreateFolderRequest {
                workspace_id: None,
                parent_id: None,
                name: "x".into(),
            })
            .await
            .unwrap_err();
        let env_err = t
            .ops
            .create_environment(CreateEnvironmentRequest {
                workspace_id: None,
                name: "x".into(),
            })
            .await
            .unwrap_err();

        for err in [folder_err, env_err] {
            assert_eq!(err.code(), codes::READ_ONLY);
        }
    }
}
