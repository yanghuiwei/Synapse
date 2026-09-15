//! MCP 侧的协议适配层（design.md §8.2）。
//!
//! ## 13 个操作的实现不在这里了
//!
//! P8 把它们整块搬去了 `reqly_core::service::ops`。理由是**签名**：原先它们返回
//! rmcp 的 `ToolResult<T>`（= `Result<Json<T>, ErrorData>`），CLI 想复用就得把
//! 编排重写一遍 —— 而那正是「两边悄悄分叉」的来源。剥掉协议类型之后，MCP 与 CLI
//! 调的是同一个 `Ops`。
//!
//! 这里只剩三样东西：
//!
//! 1. [`Tools`] —— 把 `Ops` 的结果包成 `ToolResult<T>`。`server.rs` 因此可以
//!    **一行都不改**，P7 那份验收过的工具表（13 个名字 + 注解）仍然原样；
//! 2. [`to_error_data`] / [`permission_denied`] —— `AppError` → JSON-RPC 错误；
//! 3. 把 core 的 DTO 与 `Ops` 转出来，`server.rs` 的 import 清单保持不变。
//!
//! ## 为什么错误映射留在 MCP 侧，不放进 core
//!
//! `E_READ_ONLY` 是后端内部的错误码，而 MCP 对外承诺的是 P7 定下的
//! `permission_denied`（数字码 `-32003` + `data.code` 双写）。这层翻译只对 MCP
//! 有意义 —— CLI 看的是退出码与 stderr，不需要这套形状。放 here 就不会有
//! 「core 被迫认识 JSON-RPC」这种反向依赖。
//!
//! ## 装不下东西的边界
//!
//! 只读闸门、64KB 截断、`••••` 打码这些是**安全语义**，跟着实现体留在 core ——
//! 换任何一个入口（MCP / CLI）它们都必须成立，不能只挂在 MCP 这一侧。

use std::sync::Arc;

use rmcp::handler::server::wrapper::Json;
use rmcp::model::ErrorCode;
use rmcp::ErrorData;
use serde::Serialize;
use serde_json::{json, Value};

use reqly_core::error::{AppError, AppResult};
use reqly_core::AppState;

pub use reqly_core::service::ops::{
    CreateRequestRequest, DeleteRequestRequest, DeleteRequestResponse, EnvironmentDto,
    GetRequestRequest, ImportCurlRequest, ImportCurlResponse, ListEnvironmentsRequest,
    ListEnvironmentsResponse, ListFoldersRequest, ListFoldersResponse, ListRequestsRequest,
    ListRequestsResponse, MoveRequestRequest, Ops, RefreshTokenRequest, RefreshTokenResponse,
    RequestDetail, RequestSummary, RunLoadTestRequest, RunLoadTestResponse, SendRequestRequest,
    SendRequestResponse, SetEnvVarRequest, UpdateRequestRequest,
};

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
    // core 的只读闸门抛的是 `E_READ_ONLY`，但对外的承诺是 P7 的 `permission_denied`
    // ——在这里折回那个形状，客户端的判断逻辑不用改。
    if let AppError::ReadOnly { tool } = &err {
        return permission_denied(tool);
    }

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

/// `structuredContent`（见 rmcp 的 `CallToolResult::structured`），
/// 所以只认文本的老客户端也读得到。
pub type ToolResult<T> = Result<Json<T>, ErrorData>;

/// `Ops` 的结果 → MCP 的 `ToolResult`。
///
/// 一行做完两件事：把 DTO 包进 `Json`（进 `structuredContent` 与文本两处），
/// 把 `AppError` 翻成 JSON-RPC 错误。13 个转发方法都只用它。
fn json_result<T: Serialize>(result: AppResult<T>) -> ToolResult<T> {
    result.map(Json).map_err(to_error_data)
}

/// MCP 的工具集外壳：只包一个 [`Ops`]，把结果转成协议形状。
///
/// 为什么要有这层壳而不是让 `server.rs` 直接持 `Ops`：`server.rs` 是**协议声明**
/// （13 条 `#[tool]` 属性 + 注解表 + `instructions`），那份文件是 P7 实测过的对外
/// 契约。多一层转发的代价是十几行样板，换来的是它一行不用改
/// —— 而返回值类型不匹配是编译期错误，不会静默分叉。
///
/// `Ops` 里的 13 个方法签名与这里逐个对应，改一个不改另一个会直接编译不过。
#[derive(Clone)]
pub struct Tools(Ops);

impl Tools {
    pub fn new(state: Arc<AppState>, read_only: bool) -> Self {
        Self(Ops::new(state, read_only))
    }

    // ---- 只读的 4 个 ----

    pub async fn list_folders(&self, req: ListFoldersRequest) -> ToolResult<ListFoldersResponse> {
        json_result(self.0.list_folders(req).await)
    }

    pub async fn list_requests(
        &self,
        req: ListRequestsRequest,
    ) -> ToolResult<ListRequestsResponse> {
        json_result(self.0.list_requests(req).await)
    }

    pub async fn get_request(&self, req: GetRequestRequest) -> ToolResult<RequestDetail> {
        json_result(self.0.get_request(req).await)
    }

    pub async fn list_environments(
        &self,
        req: ListEnvironmentsRequest,
    ) -> ToolResult<ListEnvironmentsResponse> {
        json_result(self.0.list_environments(req).await)
    }

    // ---- 可写的 9 个 ----

    pub async fn create_request(&self, req: CreateRequestRequest) -> ToolResult<RequestDetail> {
        json_result(self.0.create_request(req).await)
    }

    pub async fn update_request(&self, req: UpdateRequestRequest) -> ToolResult<RequestDetail> {
        json_result(self.0.update_request(req).await)
    }

    pub async fn set_env_var(&self, req: SetEnvVarRequest) -> ToolResult<EnvironmentDto> {
        json_result(self.0.set_env_var(req).await)
    }

    pub async fn send_request(&self, req: SendRequestRequest) -> ToolResult<SendRequestResponse> {
        json_result(self.0.send_request(req).await)
    }

    pub async fn run_load_test(&self, req: RunLoadTestRequest) -> ToolResult<RunLoadTestResponse> {
        json_result(self.0.run_load_test(req).await)
    }

    pub async fn refresh_token(
        &self,
        req: RefreshTokenRequest,
    ) -> ToolResult<RefreshTokenResponse> {
        json_result(self.0.refresh_token(req).await)
    }

    pub async fn import_curl(&self, req: ImportCurlRequest) -> ToolResult<ImportCurlResponse> {
        json_result(self.0.import_curl(req).await)
    }

    pub async fn move_request(&self, req: MoveRequestRequest) -> ToolResult<RequestSummary> {
        json_result(self.0.move_request(req).await)
    }

    pub async fn delete_request(
        &self,
        req: DeleteRequestRequest,
    ) -> ToolResult<DeleteRequestResponse> {
        json_result(self.0.delete_request(req).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 入参契约的回归网：`create_request` 的三个必填字段在 schema 里必须是
    /// 「必填的、非空 string」。
    ///
    /// 这条测试是补上一个真实缺陷的：最初 `CreateRequestRequest` 用
    /// `#[serde(flatten)]` 复用了 `RequestFields`，而 `name` 在两边同名 ——
    /// 外层是 `String`、flatten 进来的是 `Option<String>`，schemars 把两者合并成
    /// `type: ["string","null"]` 且 `default: null`，同时 `required` 又列着 `name`。
    /// 一份自相矛盾的 schema 会把 AI 引向「传 null 是合法的」这个错误结论，
    /// 而反序列化会当场拒掉它。
    ///
    /// 留在 MCP 侧而不是跟着 DTO 走：验的是**服务端对外宣告的 schema**
    /// （`schema_for_input` 是 rmcp 的入口），CLI 不经过它。
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

    /// core 的只读闸门抛 `E_READ_ONLY`，但对外的形状必须是 P7 的
    /// `permission_denied`。这条测试守的就是那次翻译 —— 漏了它，
    /// 客户端会看到一个从没见过的 `E_READ_ONLY`，然后按「未知错误」处理。
    #[test]
    fn read_only_refusals_keep_the_p7_permission_denied_shape() {
        let data = to_error_data(AppError::ReadOnly {
            tool: "delete_request",
        });
        assert_eq!(data.code, ErrorCode(PERMISSION_DENIED_CODE));
        assert_eq!(data.message, "permission_denied");
        let payload = data.data.expect("data 里必须带结构化信息");
        assert_eq!(payload["code"], "permission_denied");
        assert_eq!(payload["reason"], "readOnlyMode");
        assert_eq!(payload["tool"], "delete_request");
    }

    #[test]
    fn permission_denied_is_recognisable_without_the_numeric_code() {
        let data = permission_denied("create_request");
        assert_eq!(data.message, "permission_denied");
        let payload = data.data.expect("data 里必须带结构化信息");
        assert_eq!(payload["code"], "permission_denied");
        assert_eq!(payload["tool"], "create_request");
    }
}
