//! MCP 工具集（design.md §8.2）：13 个 tool 的参数、注解与实现。
//!
//! ## 为什么实现放在这里，`server.rs` 只留一层壳
//!
//! `#[tool_router]` 要求所有 `#[tool]` 方法长在**同一个 `impl` 块**里，而那个
//! 块的形状（`&self` + `Parameters<T>` + `Json<T>`）是给协议看的样板。把 13 个
//! 业务实现也塞进去，接口形状与数据编排会缠在一起 —— 改一个查询要动协议层代码。
//! 所以这里是一个普通的 `Tools` 结构（持有 `Arc<AppState>` + `read_only`），
//! `server.rs` 的每个 `#[tool]` 方法只做一次转发。
//!
//! ## 数据访问：直接组合 `repo` 层与 `http` 层，不调 `commands`
//!
//! `commands::*` 里那些函数都要 `tauri::State<'_, AppState>`，而 `tauri::State`
//! **没有公开构造函数** —— 非 Tauri 进程根本造不出来（这一点 `send_request_with`
//! 的注释里也写过）。所以这里直连 `repo` + `Pipeline` + `send_request_with`。
//! 唯一复用的是那些**本来就只吃 `&Connection`** 的编排：导入的 `import_into_folder`、
//! 压测收尾的 `persist_outcome`。
//!
//! ## 三条安全边界（design.md §8.3）在本文件的落点
//!
//! 1. [`Tools::list_environments`] 对 `is_secret = 1` 的值一律返回 `••••`，
//!    其余任何 tool 也没有「读原始 secret」的入口。
//! 2. [`Tools::send_request`] 的响应体截断到 64KB（按 UTF-8 边界切），
//!    凭据类响应头（`set-cookie` / `authorization` / `*-key` / `*-token` …）默认打码。
//! 3. [`Tools::writable`] 是只读模式的唯一闸门：9 个写类 tool 的第一行都是它，
//!    只读的 4 个 tool 不经过它。
//!
//! ## 错误一律是结构化错误码
//!
//! 后端不产出面向用户的中文文案（见 `crate::error` 的契约），所以 [`to_error_data`]
//! 把 `AppError` 的**错误码**放进 `message`，把 `params` / `debug` 放进 `data`。
//! tool 的 `description` 用中文没问题 —— 那是给 AI 看的契约，不是 UI 文案。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::wrapper::Json;
use rmcp::model::ErrorCode;
use rmcp::schemars::{self, JsonSchema};
use rmcp::ErrorData;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::commands::collection::ensure_usable_folder;
use crate::commands::import_::{import_into_folder, Conflict, ImportLabels, ImportSource};
use crate::commands::loadtest::{discard_run, persist_outcome, stored_run_config};
use crate::commands::request::send_request_with;
use crate::error::{codes, AppError, AppResult};
use crate::http::{HttpResponsePayload, Pipeline, RequestSpec};
use crate::loadtest::{engine, LoadtestConfig, LoadtestDeps};
use crate::storage::models::{ApiRequest, BodyType, HttpVersion, KeyValueItem};
use crate::storage::{repo, with_conn};
use crate::AppState;

/// 回给 AI 的响应体上限（design.md §8.2）。
///
/// 64KB 是「够看清一个 JSON 结构，又不至于把 MCP 的 stdio 通道塞爆」的量级。
/// 超出的部分**丢掉但如实标记**（`bodyTruncated` + `bodySize`），不静默截断。
const MAX_BODY_BYTES: usize = 64 * 1024;

/// 敏感值的打码结果，与 design.md §8.3 写的字符一致。
const MASK: &str = "••••";

/// 响应头里默认打码的那些（小写字面量比较）。
///
/// design.md §8.3 只点名了 `set-cookie` 与 `authorization`，但**照字面做只防了一半**：
/// 真实接口里 `x-api-key` / `x-auth-token` 这类自定义头与 `Authorization` 完全等价，
/// 漏掉它们，凭据照样进 AI 的上下文。
const MASKED_HEADERS: [&str; 6] = [
    "set-cookie",
    "cookie",
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
];

/// 再加一条后缀规则，省得穷举每个厂商的命名（`x-auth-token`、`x-client-secret`…）。
///
/// 方向刻意偏向「宁可多打」：多打一个非敏感头，代价是 AI 少看见一个值；
/// 漏掉一个凭据头，代价是它进了 AI 的上下文。两者不对称，所以这里不追求精确白名单。
const MASKED_HEADER_SUFFIXES: [&str; 4] = ["-key", "-token", "-secret", "-password"];

/// `run_load_test` 的总超时上限。
///
/// 为什么必须有：这个 tool 会阻塞到跑完，而 MCP 的 stdio 通道是**一条连接**——
/// 一轮 50 万请求能跑十几分钟，期间客户端以为服务死了。宁可报错让它拆细了再跑。
const LOADTEST_TIMEOUT: Duration = Duration::from_secs(120);

/// 只读模式下拒绝写操作时用的 JSON-RPC 错误码。
///
/// MCP 规范没有定义 `permission_denied` 这个**协议级**错误码（标准码只到
/// `-32603`），而 JSON-RPC 2.0 把 `-32000 ~ -32099` 留给实现自定义。
/// 所以这里在这个区间里取一个，并在 `data.code` 里给出稳定的 `permission_denied`——
/// 客户端要判断「是不是被只读模式拦了」认的是那个字符串，不是这个数字。
const PERMISSION_DENIED_CODE: i32 = -32003;

/// 只读模式下拒绝一次写操作。
pub fn permission_denied(tool: &'static str) -> ErrorData {
    ErrorData::new(
        ErrorCode(PERMISSION_DENIED_CODE),
        "permission_denied",
        Some(json!({
            "code": "permission_denied",
            "reason": "readOnlyMode",
            "tool": tool,
        })),
    )
}

/// `AppError` → MCP 错误。
///
/// **不丢错误码**：`message` 就是 `crate::error::codes` 里那个稳定标识（如 `E_DB_LOCKED`），
/// `params` 原样带上（前端插值用的那套结构化信息），`debug` 留给排查。
/// 把 `err.to_string()` 当 `message` 是不行的 —— 那是给人读的调试句子，
/// AI 拿它做分支判断会碎。
pub fn to_error_data(err: AppError) -> ErrorData {
    // 先按错误类别挑一个协议级码，客户端据此决定「能不能重试/是不是参数问题」
    let code = match &err {
        AppError::NotFound { .. } => ErrorCode::RESOURCE_NOT_FOUND,
        AppError::InvalidInput { .. } | AppError::InvalidUrl { .. } => ErrorCode::INVALID_PARAMS,
        _ => ErrorCode::INTERNAL_ERROR,
    };

    let mut data = serde_json::Map::new();
    data.insert("code".into(), json!(err.code()));
    if let Some(params) = err.params() {
        data.insert("params".into(), params);
    }
    data.insert("debug".into(), json!(err.to_string()));
    if err.retryable() {
        data.insert("retryable".into(), json!(true));
    }

    ErrorData::new(code, err.code(), Some(Value::Object(data)))
}

/// 一次 tool 调用的返回。`Json<T>` 会把结构化结果同时写进 `content` 与
/// `structuredContent`（见 rmcp 的 `CallToolResult::structured`），
/// 所以只认文本的老客户端也读得到。
pub type ToolResult<T> = Result<Json<T>, ErrorData>;

// ============================================================
// 请求/响应的 DTO
// ============================================================
//
// 刻意**不**直接把 `storage::models` 的类型当返回值：那些类型要加 JsonSchema
// 派生就得动共享的数据模型，而 MCP 对外暴露的形状本来就该比库表更克制
// （比如 auth 的值一律不外泄，见 `AuthSummary`）。

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FolderSummary {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub sort_order: i64,
    /// 被标记成 Token 源分组（design.md §4.3）
    pub is_token_source: bool,
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

/// 请求的授权配置 —— **只报类型，不报值**。
///
/// §8.3 写着「MCP 层不提供读取原始 secret 的能力」，而 `AuthConfig::Bearer { token }`
/// 里的 token 可能是用户直接粘进去的长期凭据（不全是 `{{token.access}}` 占位符）。
/// 把整个 auth 结构回给 AI 等于开了一个绕过 §8.3 的后门，所以这里只给类型标签。
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthSummary {
    /// `inherit` / `none` / `bearer` / `basic` / `apiKey`
    pub r#type: String,
}

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
    pub auth: AuthSummary,
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

/// 13 个 tool 的实现体。可克隆（`Arc<AppState>` 与 bool 都是廉价克隆），
/// HTTP 通道的每个会话各拿一份。
#[derive(Clone)]
pub struct Tools {
    state: Arc<AppState>,
    read_only: bool,
}

impl Tools {
    pub fn new(state: Arc<AppState>, read_only: bool) -> Self {
        Self { state, read_only }
    }

    /// 只读模式的唯一闸门：9 个写类 tool 的第一行都是它。
    fn writable(&self, tool: &'static str) -> Result<(), ErrorData> {
        if self.read_only {
            return Err(permission_denied(tool));
        }
        Ok(())
    }

    // ---- 只读的 4 个 ----

    /// 列出文件夹树（平铺返回，前端/AI 按 `parentId` 自己组树）。
    pub async fn list_folders(&self, req: ListFoldersRequest) -> ToolResult<ListFoldersResponse> {
        let workspace_id = self
            .resolve_workspace(req.workspace_id)
            .await
            .map_err(to_error_data)?;

        let folders = with_conn(&self.state.pool, move |conn| {
            repo::folder::list_by_workspace(conn, workspace_id)
        })
        .await
        .map_err(to_error_data)?;

        Ok(Json(ListFoldersResponse {
            workspace_id,
            folders: folders
                .into_iter()
                .map(|folder| FolderSummary {
                    id: folder.id,
                    parent_id: folder.parent_id,
                    name: folder.name,
                    sort_order: folder.sort_order,
                    is_token_source: folder.is_token_source,
                })
                .collect(),
        }))
    }

    /// 列出请求。给了 `folder_id` 就只看那个文件夹，否则整个工作区。
    pub async fn list_requests(
        &self,
        req: ListRequestsRequest,
    ) -> ToolResult<ListRequestsResponse> {
        let workspace_id = self.resolve_workspace(None).await.map_err(to_error_data)?;
        let folder_id = req.folder_id;
        let needle = req
            .query
            .map(|q| q.trim().to_lowercase())
            .filter(|q| !q.is_empty());

        let requests = with_conn(&self.state.pool, move |conn| match folder_id {
            Some(id) => repo::request::list_by_folder(conn, Some(id)),
            None => repo::request::list_by_workspace(conn, workspace_id),
        })
        .await
        .map_err(to_error_data)?;

        let requests = requests
            .into_iter()
            .filter(|r| match &needle {
                Some(q) => r.name.to_lowercase().contains(q.as_str()),
                None => true,
            })
            .map(summarise)
            .collect();

        Ok(Json(ListRequestsResponse {
            workspace_id,
            requests,
        }))
    }

    /// 取完整请求定义。auth 只报类型（见 [`AuthSummary`]）。
    pub async fn get_request(&self, req: GetRequestRequest) -> ToolResult<RequestDetail> {
        let request = self
            .load_request(req.request_id)
            .await
            .map_err(to_error_data)?;
        Ok(Json(detail(&request)))
    }

    /// 列出环境与变量。`is_secret` 的值一律 `••••`（design.md §8.3）。
    pub async fn list_environments(
        &self,
        req: ListEnvironmentsRequest,
    ) -> ToolResult<ListEnvironmentsResponse> {
        let workspace_id = self
            .resolve_workspace(req.workspace_id)
            .await
            .map_err(to_error_data)?;

        let rows = with_conn(&self.state.pool, move |conn| {
            let mut out = Vec::new();
            for env in repo::env::list_by_workspace(conn, workspace_id)? {
                let vars = repo::env::list_vars(conn, env.id)?;
                out.push((env, vars));
            }
            Ok(out)
        })
        .await
        .map_err(to_error_data)?;

        let environments = rows
            .into_iter()
            .map(|(env, vars)| EnvironmentDto {
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
            })
            .collect();

        Ok(Json(ListEnvironmentsResponse {
            workspace_id,
            environments,
        }))
    }

    // ---- 写类的 9 个 ----

    /// 新建请求（一开始就是可发送的完整定义，不是「空壳再补字段」）。
    pub async fn create_request(&self, req: CreateRequestRequest) -> ToolResult<RequestDetail> {
        self.writable("create_request")?;

        let workspace_id = self
            .resolve_workspace(req.workspace_id)
            .await
            .map_err(to_error_data)?;

        let folder_id = req.folder_id;
        let name = require_text(req.name.clone(), "name").map_err(to_error_data)?;
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
        .await
        .map_err(to_error_data)?;

        Ok(Json(detail(&created)))
    }

    /// 局部更新：只改传进来的字段，其余原样保留。
    pub async fn update_request(&self, req: UpdateRequestRequest) -> ToolResult<RequestDetail> {
        self.writable("update_request")?;

        let request_id = req.request_id;
        let fields = req.fields;
        let saved = with_conn(&self.state.pool, move |conn| {
            let mut full = repo::request::get(conn, request_id)?;
            apply_fields(&mut full, &fields)?;
            repo::request::save(conn, &full)
        })
        .await
        .map_err(to_error_data)?;

        Ok(Json(detail(&saved)))
    }

    /// 写一个环境变量：同名改写，没有就追加（整表提交语义，见 `repo::env::replace_vars`）。
    pub async fn set_env_var(&self, req: SetEnvVarRequest) -> ToolResult<EnvironmentDto> {
        self.writable("set_env_var")?;

        let key = require_text(req.key, "key").map_err(to_error_data)?;
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
        .await
        .map_err(to_error_data)?;

        Ok(Json(EnvironmentDto {
            id: env.id,
            name: env.name,
            is_active: env.is_active,
            vars: vars
                .into_iter()
                .map(|var| EnvVarDto {
                    key: var.key,
                    value: if var.is_secret {
                        MASK.to_string()
                    } else {
                        var.value
                    },
                    is_secret: var.is_secret,
                    enabled: var.enabled,
                })
                .collect(),
        }))
    }

    /// 发一次真实请求。响应体 64KB 截断、敏感响应头打码。
    pub async fn send_request(&self, req: SendRequestRequest) -> ToolResult<SendRequestResponse> {
        self.writable("send_request")?;

        self.check_environment(req.environment.as_deref())
            .await
            .map_err(to_error_data)?;

        let spec = self.build_spec(&req).await.map_err(to_error_data)?;
        // runId 只服务于「取消」：MCP 这边没有取消入口，但登记表要求它非空且唯一，
        // 否则 `InflightRegistry` 里两条并发请求会互相覆盖。
        let run_id = next_run_id();

        let payload = send_request_with(&self.state, &run_id, &spec)
            .await
            .map_err(to_error_data)?;

        Ok(Json(response_dto(payload)))
    }

    /// 跑一轮压测，阻塞到结束（最多 [`LOADTEST_TIMEOUT`]）。
    pub async fn run_load_test(&self, req: RunLoadTestRequest) -> ToolResult<RunLoadTestResponse> {
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
        config.validate().map_err(to_error_data)?;

        let request = self
            .load_request(req.request_id)
            .await
            .map_err(to_error_data)?;
        let spec = RequestSpec::from(&request);
        let stored = stored_run_config(&spec, &config).map_err(to_error_data)?;

        let run = with_conn(&self.state.pool, move |conn| {
            repo::loadtest::create_run(conn, Some(req.request_id), &request.name, &stored)
        })
        .await
        .map_err(to_error_data)?;
        let run_id = run.id;

        // 占坑。跨进程拦不住（桌面端同时在跑一轮我们就无从得知），
        // 但同一个 MCP 进程里两次并发调用必须被挡住 —— 两轮叠在一起，
        // 双方测到的都是对方造成的劣化。
        let Some(active) = self.state.loadtests.begin(run_id) else {
            discard_run(&self.state.pool, run_id).await;
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                codes::INVALID_INPUT,
                Some(json!({
                    "code": codes::INVALID_INPUT,
                    "params": { "field": "loadtest" },
                    "debug": "another load test is already running in this process",
                })),
            ));
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
                    return Err(to_error_data(err));
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
                return Err(to_error_data(err));
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
                let detail = persist_outcome(self.state.pool.clone(), run_id, outcome)
                    .await
                    .map_err(to_error_data)?;
                let summary = detail.summary.ok_or_else(|| {
                    ErrorData::internal_error("loadtest finished without a summary", None)
                })?;
                let value = serde_json::to_value(&summary).map_err(|e| {
                    ErrorData::internal_error(format!("serialize loadtest summary: {e}"), None)
                })?;
                Ok(Json(RunLoadTestResponse {
                    run_id,
                    counts: LoadtestCountsDto {
                        total: summary.total,
                        succeeded: summary.succeeded,
                        failed: summary.failed,
                    },
                    summary: value,
                }))
            }
            Ok(Err(err)) => {
                discard_run(&self.state.pool, run_id).await;
                Err(to_error_data(err))
            }
            Err(_elapsed) => {
                // 超时：先广播取消（在飞的请求自然结束，不打断），
                // 再把这一轮从历史里抹掉 —— 留一行 finished_at 为空、
                // 没有概要的记录，用户看到的是「跑着跑着没了」。
                active.token().cancel();
                discard_run(&self.state.pool, run_id).await;
                Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    codes::TIMEOUT,
                    Some(json!({
                        "code": codes::TIMEOUT,
                        "params": { "ms": LOADTEST_TIMEOUT.as_millis() as u64 },
                        "debug": "load test exceeded the MCP tool timeout budget",
                    })),
                ))
            }
        }
    }

    /// 强制刷新一个 Token 源，返回剩余有效期。**不返回 token 本身**（§8.3）。
    pub async fn refresh_token(
        &self,
        req: RefreshTokenRequest,
    ) -> ToolResult<RefreshTokenResponse> {
        self.writable("refresh_token")?;

        let source = with_conn(&self.state.pool, move |conn| {
            repo::token::get(conn, req.token_source_id)
        })
        .await
        .map_err(to_error_data)?;

        let workspace_id = source.workspace_id;
        let env = with_conn(&self.state.pool, move |conn| {
            repo::env::active_vars(conn, workspace_id)
        })
        .await
        .map_err(to_error_data)?;

        let deps = crate::http::TokenDeps {
            pool: &self.state.pool,
            clients: &self.state.clients,
            source: &source,
            env: &env,
            cancel: None,
        };
        let values = self
            .state
            .tokens
            .refresh_now(&deps)
            .await
            .map_err(to_error_data)?;

        let expires_in_seconds = values.expires_at.as_deref().and_then(|raw| {
            chrono::DateTime::parse_from_rfc3339(raw)
                .ok()
                .map(|at| (at.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_seconds())
        });

        Ok(Json(RefreshTokenResponse {
            source_id: source.id,
            source_name: source.name,
            expires_at: values.expires_at,
            expires_in_seconds,
            has_refresh_token: values.refresh_token.is_some(),
        }))
    }

    /// 粘贴 curl 即导入。复用与 UI 完全相同的解析 + 落库路径。
    pub async fn import_curl(&self, req: ImportCurlRequest) -> ToolResult<ImportCurlResponse> {
        self.writable("import_curl")?;

        let workspace_id = self
            .resolve_workspace(req.workspace_id)
            .await
            .map_err(to_error_data)?;

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
        .await
        .map_err(to_error_data)?;

        Ok(Json(ImportCurlResponse {
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
        }))
    }

    /// 移动到其他分类。
    pub async fn move_request(&self, req: MoveRequestRequest) -> ToolResult<RequestSummary> {
        self.writable("move_request")?;

        let request_id = req.request_id;
        let target = req.target_folder_id;
        let moved = with_conn(&self.state.pool, move |conn| {
            let request = repo::request::get(conn, request_id)?;
            ensure_usable_folder(conn, target, request.workspace_id, "targetFolderId")?;
            repo::request::move_to_folder(conn, request_id, Some(target))
        })
        .await
        .map_err(to_error_data)?;

        Ok(Json(summarise(moved)))
    }

    /// 软删除（进回收站，可还原）。
    pub async fn delete_request(
        &self,
        req: DeleteRequestRequest,
    ) -> ToolResult<DeleteRequestResponse> {
        self.writable("delete_request")?;

        let request_id = req.request_id;
        with_conn(&self.state.pool, move |conn| {
            repo::request::soft_delete(conn, request_id)
        })
        .await
        .map_err(to_error_data)?;

        Ok(Json(DeleteRequestResponse {
            request_id,
            restorable: true,
        }))
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
                auth: crate::storage::models::AuthConfig::Inherit,
                settings: crate::storage::models::RequestSettings::default(),
                folder_id: req.folder_id,
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
fn response_dto(payload: HttpResponsePayload) -> SendRequestResponse {
    let headers = payload
        .headers
        .iter()
        .map(|header| HeaderDto {
            name: header.name.clone(),
            value: mask_header_value(&header.name, &header.value),
        })
        .collect();

    let (body, cut) = truncate_body(payload.body);

    SendRequestResponse {
        status: payload.status,
        status_text: payload.status_text,
        http_version: payload.http_version,
        url: payload.url,
        redirected: payload.redirected,
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

/// 敏感响应头打码。其余原样。
fn mask_header_value(name: &str, value: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let sensitive = MASKED_HEADERS.contains(&lower.as_str())
        || MASKED_HEADER_SUFFIXES
            .iter()
            .any(|suffix| lower.ends_with(suffix));

    if sensitive {
        MASK.to_string()
    } else {
        value.to_string()
    }
}

/// 按 **UTF-8 字符边界**截断响应体。
///
/// 为什么不能直接 `body.truncate(MAX_BODY_BYTES)`：`String::truncate` 在非边界处
/// 会 panic。一个 64KB 处正好落着半个汉字的响应就能把整个 MCP 进程打挂 ——
/// 而 `http::execute` 里那段 `valid_up_to()` 的写法正是同一个问题的另一半
/// （那里是在字节切片上切，这里是在 `String` 上切）。
fn truncate_body(body: String) -> (String, bool) {
    if body.len() <= MAX_BODY_BYTES {
        return (body, false);
    }
    let mut end = MAX_BODY_BYTES;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = body;
    out.truncate(end);
    (out, true)
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
        auth: AuthSummary {
            r#type: auth_type(&request.auth).to_string(),
        },
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

fn key_value(item: &KeyValueItem) -> KeyValueDto {
    KeyValueDto {
        key: item.key.clone(),
        value: item.value.clone(),
        enabled: item.enabled,
        desc: item.desc.clone(),
    }
}

/// 只报类型标签，值一概不回（见 [`AuthSummary`]）。
fn auth_type(auth: &crate::storage::models::AuthConfig) -> &'static str {
    use crate::storage::models::AuthConfig;
    match auth {
        AuthConfig::Inherit => "inherit",
        AuthConfig::None => "none",
        AuthConfig::Bearer { .. } => "bearer",
        AuthConfig::Basic { .. } => "basic",
        AuthConfig::ApiKey { .. } => "apiKey",
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
        "mcp-{}-{}",
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
        // 每个汉字 3 字节，MAX_BODY_BYTES 必然落在某个字中间
        let body = "中".repeat(MAX_BODY_BYTES);
        assert!(body.len() > MAX_BODY_BYTES);

        let (cut, truncated) = truncate_body(body);
        assert!(truncated);
        assert!(cut.len() <= MAX_BODY_BYTES);
        assert_eq!(cut.len() % 3, 0, "截断必须落在字符边界上");
        assert!(cut.chars().all(|c| c == '中'));
    }

    #[test]
    fn a_body_within_the_limit_is_untouched() {
        let (body, truncated) = truncate_body("hello".to_string());
        assert_eq!(body, "hello");
        assert!(!truncated);
    }

    #[test]
    fn sensitive_headers_are_masked_and_others_are_not() {
        assert_eq!(mask_header_value("Set-Cookie", "sid=1"), MASK);
        assert_eq!(mask_header_value("Authorization", "Bearer x"), MASK);
        // 凭据类自定义头：真实接口里这些和 Authorization 等价，不能漏
        assert_eq!(mask_header_value("X-Api-Key", "k"), MASK);
        assert_eq!(mask_header_value("Api-Key", "k"), MASK);
        assert_eq!(mask_header_value("Proxy-Authorization", "Basic x"), MASK);
        assert_eq!(mask_header_value("X-Auth-Token", "t"), MASK);
        assert_eq!(mask_header_value("X-Client-Secret", "s"), MASK);
        assert_eq!(mask_header_value("X-User-Password", "p"), MASK);
        // 反向：不能打过头 —— 排障要看的头必须原样透出
        assert_eq!(mask_header_value("X-Request-Id", "abc-123"), "abc-123");
        assert_eq!(
            mask_header_value("Content-Type", "application/json"),
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

    /// 入参契约的回归网：`create_request` 的三个必填字段在 schema 里必须是
    /// 「必填的、非空 string」。
    ///
    /// 这条测试是补上一个真实缺陷的：最初 `CreateRequestRequest` 用
    /// `#[serde(flatten)]` 复用了 `RequestFields`，而 `name` 在两边同名 ——
    /// 外层是 `String`、flatten 进来的是 `Option<String>`，schemars 把两者合并成
    /// `type: ["string","null"]` 且 `default: null`，同时 `required` 又列着 `name`。
    /// 一份自相矛盾的 schema 会把 AI 引向「传 null 是合法的」这个错误结论，
    /// 而反序列化会当场拒掉它。
    #[test]
    fn create_request_schema_keeps_its_required_fields_required() {
        let schema = rmcp::handler::server::common::schema_for_input::<CreateRequestRequest>()
            .expect("create_request 的入参 schema 必须能生成");

        let required = schema
            .get("required")
            .and_then(Value::as_array)
            .expect("schema 必须声明 required");
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("schema 必须声明 properties");

        for field in ["name", "method", "url"] {
            assert!(
                required.iter().any(|entry| entry == field),
                "`{field}` 必须是必填：{required:?}"
            );
            assert_eq!(
                properties[field].get("type"),
                Some(&Value::from("string")),
                "`{field}` 不该是可空类型：一份说「可以传 null」而实现会拒的 schema，\
                 会把调用方引到一个它自己看不出来的错误上"
            );
        }
    }

    /// `send_request` 允许「内联定义」，所以 method / url 在那里是**可选**的
    /// —— 与 `create_request` 相反。这条测试把两者的差别钉住，
    /// 免得有人「顺手统一一下」。
    #[test]
    fn send_request_schema_leaves_method_and_url_optional() {
        let schema = rmcp::handler::server::common::schema_for_input::<SendRequestRequest>()
            .expect("send_request 的入参 schema 必须能生成");

        let required = schema
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        assert!(
            !required
                .iter()
                .any(|entry| entry == "method" || entry == "url"),
            "send_request 可以只给 requestId，method/url 不能是必填：{required:?}"
        );
    }

    #[test]
    fn app_errors_keep_their_code_in_the_mcp_error() {
        let err = AppError::NotFound {
            entity: "request",
            id: 7,
        };
        let data = to_error_data(err);
        assert_eq!(data.code, ErrorCode::RESOURCE_NOT_FOUND);
        assert_eq!(data.message, "E_NOT_FOUND", "错误码不能被丢掉");

        let invalid = to_error_data(AppError::InvalidInput {
            field: "name",
            reason: "must not be empty".into(),
        });
        assert_eq!(invalid.code, ErrorCode::INVALID_PARAMS);
        assert_eq!(invalid.message, "E_INVALID_INPUT");
    }

    #[test]
    fn permission_denied_is_recognisable_without_the_numeric_code() {
        let data = permission_denied("create_request");
        assert_eq!(data.message, "permission_denied");
        let payload = data.data.expect("data 里必须带结构化信息");
        assert_eq!(payload["code"], "permission_denied");
        assert_eq!(payload["tool"], "create_request");
    }

    #[test]
    fn run_ids_are_unique_within_the_process() {
        let first = next_run_id();
        let second = next_run_id();
        assert_ne!(first, second);
        assert!(first.starts_with("mcp-"));
    }
}
