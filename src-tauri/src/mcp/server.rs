//! MCP 服务端装配（design.md §8.1/§8.2）。
//!
//! ## 这一层只做三件事
//!
//! 1. 把 13 个 tool **登记**进 `ToolRouter`，并声明各自的注解（readOnly / destructive / idempotent）；
//! 2. 声明 server 身份与 `instructions`（AI 客户端握手时拿到的「这个 server 是干什么的」）；
//! 3. 把调用转发给 [`crate::mcp::tools::Tools`]。
//!
//! 业务逻辑一律不在这里 —— 理由见 `tools.rs` 的模块头。
//!
//! ## 为什么注解必须逐个手写，而不是按「写得像只读」去推
//!
//! design.md §8.2 的注解表是**审批模型的输入**：Hermes 在 `untrusted` 模式下
//! 靠 `readOnlyHint` 决定「这条不用问用户」，靠 `destructiveHint` 决定「这条必须问」。
//! 猜错的后果是不对称的 —— 把 `delete_request` 标成只读，用户就不会被问；
//! 把 `send_request` 标成只读，AI 能静默把请求打到生产环境（§8.2 的注里专门写了这一条）。
//! 所以注解取值严格照表，不按直觉。

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};

use crate::mcp::tools::{
    CreateRequestRequest, DeleteRequestRequest, DeleteRequestResponse, GetRequestRequest,
    ImportCurlRequest, ImportCurlResponse, ListEnvironmentsRequest, ListEnvironmentsResponse,
    ListFoldersRequest, ListFoldersResponse, ListRequestsRequest, ListRequestsResponse,
    MoveRequestRequest, RefreshTokenRequest, RefreshTokenResponse, RequestDetail, RequestSummary,
    RunLoadTestRequest, RunLoadTestResponse, SendRequestRequest, SendRequestResponse,
    SetEnvVarRequest, ToolResult, UpdateRequestRequest,
};
use crate::mcp::tools::{EnvironmentDto, Tools};
use crate::AppState;

/// MCP server 本体。
///
/// 只包一个 [`Tools`]：`ToolRouter` 不是字段而是 `#[tool_router]` 生成的关联函数
/// （`Self::tool_router()`），每次 `list_tools` 现搭 —— 13 条路由的构造是纳秒级的，
/// 而省掉一个字段就省掉了「router 与 tools 不同步」这类问题。
#[derive(Clone)]
pub struct ReqlyMcp {
    tools: Tools,
}

impl ReqlyMcp {
    pub fn new(state: Arc<AppState>, read_only: bool) -> Self {
        Self {
            tools: Tools::new(state, read_only),
        }
    }

    /// 13 条路由。**给测试用**：不依赖 `AppState` 也能把对外契约（名字 + 注解）验一遍，
    /// 恰好这是 P7 验收里最容易在重构中悄悄坏掉的部分。
    ///
    /// 生产路径不调它 —— `#[tool_handler]` 已经把 `Self::tool_router()` 接好了。
    #[cfg(test)]
    fn routes() -> Vec<rmcp::model::Tool> {
        Self::tool_router().list_all()
    }
}

#[tool_router]
impl ReqlyMcp {
    #[tool(
        title = "列出文件夹",
        description = "列出工作区里的文件夹（平铺返回，按 parentId 自行组树）。tokenSource 为真的那些还挂着 Token 源。",
        annotations(read_only_hint = true)
    )]
    async fn list_folders(
        &self,
        Parameters(req): Parameters<ListFoldersRequest>,
    ) -> ToolResult<ListFoldersResponse> {
        self.tools.list_folders(req).await
    }

    #[tool(
        title = "列出请求",
        description = "列出请求。给 folderId 就只看那个文件夹，不给就整个工作区；query 按名称做大小写不敏感的子串匹配。",
        annotations(read_only_hint = true)
    )]
    async fn list_requests(
        &self,
        Parameters(req): Parameters<ListRequestsRequest>,
    ) -> ToolResult<ListRequestsResponse> {
        self.tools.list_requests(req).await
    }

    #[tool(
        title = "读取请求详情",
        description = "按 requestId 取完整请求定义。authorization 只回类型（inherit/none/bearer/basic/apiKey），不回值。",
        annotations(read_only_hint = true)
    )]
    async fn get_request(
        &self,
        Parameters(req): Parameters<GetRequestRequest>,
    ) -> ToolResult<RequestDetail> {
        self.tools.get_request(req).await
    }

    #[tool(
        title = "列出环境与变量",
        description = "列出环境及其变量。isSecret 为真的变量值恒为 ••••，原文不会经过 MCP 通道。",
        annotations(read_only_hint = true)
    )]
    async fn list_environments(
        &self,
        Parameters(req): Parameters<ListEnvironmentsRequest>,
    ) -> ToolResult<ListEnvironmentsResponse> {
        self.tools.list_environments(req).await
    }

    #[tool(
        title = "新建请求",
        description = "新建一条立刻可发送的请求（不是空壳）。name/method/url 必填，其余字段可一并给出。",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn create_request(
        &self,
        Parameters(req): Parameters<CreateRequestRequest>,
    ) -> ToolResult<RequestDetail> {
        self.tools.create_request(req).await
    }

    #[tool(
        title = "局部更新请求",
        description = "按 requestId 局部更新：只改你给出的字段，未给出的原样保留。bodyType 只认 none/json/form/urlencoded/raw/binary/graphql。",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn update_request(
        &self,
        Parameters(req): Parameters<UpdateRequestRequest>,
    ) -> ToolResult<RequestDetail> {
        self.tools.update_request(req).await
    }

    #[tool(
        title = "写环境变量",
        description = "在某个环境里写一个变量：同名改写，没有就追加。写进来的变量按「非密文」处理。",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn set_env_var(
        &self,
        Parameters(req): Parameters<SetEnvVarRequest>,
    ) -> ToolResult<EnvironmentDto> {
        self.tools.set_env_var(req).await
    }

    #[tool(
        title = "发送请求",
        description = "真实发出一次请求并返回响应。可用 requestId 取已保存的请求，也可内联 method+url（内联字段会覆盖已保存请求上的对应字段）。响应体超 64KB 截断并标记。",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn send_request(
        &self,
        Parameters(req): Parameters<SendRequestRequest>,
    ) -> ToolResult<SendRequestResponse> {
        self.tools.send_request(req).await
    }

    #[tool(
        title = "跑一轮压测",
        description = "对一条已保存请求跑一轮压测，阻塞到结束（上限 120 秒），返回落库的 runId 与完整概要（延迟分位、错误分布、吞吐）。同一进程同时只允许一轮。",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn run_load_test(
        &self,
        Parameters(req): Parameters<RunLoadTestRequest>,
    ) -> ToolResult<RunLoadTestResponse> {
        self.tools.run_load_test(req).await
    }

    #[tool(
        title = "强制刷新 Token",
        description = "强制刷新一个 Token 源并返回剩余有效期。只回有效期与是否有 refresh token，不回 access token 本身。",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn refresh_token(
        &self,
        Parameters(req): Parameters<RefreshTokenRequest>,
    ) -> ToolResult<RefreshTokenResponse> {
        self.tools.refresh_token(req).await
    }

    #[tool(
        title = "导入 curl",
        description = "把一段 curl 命令原文导入成请求（走与界面导入完全相同的解析与落库路径），返回四个计数与本次新建的请求。",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn import_curl(
        &self,
        Parameters(req): Parameters<ImportCurlRequest>,
    ) -> ToolResult<ImportCurlResponse> {
        self.tools.import_curl(req).await
    }

    #[tool(
        title = "移动请求",
        description = "把请求移动到另一个分类下。targetFolderId 必须在同一个工作区且未被删除。",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn move_request(
        &self,
        Parameters(req): Parameters<MoveRequestRequest>,
    ) -> ToolResult<RequestSummary> {
        self.tools.move_request(req).await
    }

    // ★ destructiveHint 是这张表里唯一的 true：软删除虽然能还原，
    //   但它会把请求从用户的视野里拿掉 —— 审批提示必须弹出来。
    #[tool(
        title = "删除请求",
        description = "软删除一条请求（进回收站，桌面端可还原）。不会真正抹掉数据。",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn delete_request(
        &self,
        Parameters(req): Parameters<DeleteRequestRequest>,
    ) -> ToolResult<DeleteRequestResponse> {
        self.tools.delete_request(req).await
    }
}

// `instructions` / `name` 在宏里只能是字面量，所以这里原样再写一遍，而不是引用常量
// —— 属性宏的参数不展开常量，也不接受 `env!()`（它要的是 lit）。
// `version` 刻意不写：省略时 `tool_handler` 会自己用 `env!("CARGO_PKG_VERSION")`。
#[tool_handler(
    router = Self::tool_router(),
    name = "reqly",
    instructions = "Reqly 是一个本地优先的 API 调试工作台。你操作的是用户本机上同一个 SQLite 库，桌面端打开着的话会实时刷新。推荐顺序：先用 list_requests（可按名模糊搜）或 list_folders 找到目标请求的 requestId；再 get_request 看完整定义；想先试一下就直接 send_request（内联覆盖几个字段即可，不必先保存），确认无误再用 update_request 落库；要压测用 run_load_test（阻塞到跑完，上限 120 秒）；新建用 create_request，整理位置用 move_request，不要了用 delete_request（进回收站可还原）；从浏览器 DevTools 复制的 curl 直接交给 import_curl。约束：isSecret 为真的环境变量值一律返回 ••••，原文不会经过 MCP 通道，要用它请在请求里写 {{env.KEY}} 占位符；send_request 的响应体超 64KB 会截断（bodyTruncated/bodySize 会如实标记），set-cookie 与 authorization 响应头会打码；environment 参数目前只接受用户当前激活的那个环境名，传别的会返回 E_NOT_IMPLEMENTED；只读模式下所有写类 tool 返回 permission_denied。"
)]
impl ServerHandler for ReqlyMcp {}

#[cfg(test)]
mod tests {
    use super::*;

    /// design.md §8.2 注解表的一行。`None` = **未声明这个 hint**。
    ///
    /// 三态而不是 bool：§8.2 里那些 `—` 格子写的是「不声明」，而不是「声明成 false」。
    /// 两者的差别不是学术性的 —— MCP 规范给 `destructiveHint` 的**默认值是 true**，
    /// 而只读类 tool 不声明它正是为了让「readOnlyHint = true 时没人在乎 destructiveHint」
    /// 这条规则生效。全写成 false 会多送一个没意义的断言给客户端。
    #[derive(Clone, Copy)]
    struct Expected {
        name: &'static str,
        read_only: Option<bool>,
        destructive: Option<bool>,
        idempotent: Option<bool>,
    }

    /// 这张表就是 design.md §8.2 的注解表，抄在这里当**回归网**：
    /// 谁把某个 tool 顺手改成 read_only = true，这里会立刻红。
    const EXPECTED: [Expected; 13] = [
        Expected::reader("list_folders"),
        Expected::reader("list_requests"),
        Expected::reader("get_request"),
        Expected::reader("list_environments"),
        Expected::writer("create_request"),
        Expected::writer("update_request"),
        Expected::writer("set_env_var"),
        Expected::writer("send_request"),
        Expected::writer("run_load_test"),
        Expected::writer("refresh_token"),
        Expected::writer("import_curl"),
        Expected::writer("move_request"),
        Expected {
            name: "delete_request",
            read_only: Some(false),
            destructive: Some(true),
            idempotent: Some(false),
        },
    ];

    impl Expected {
        /// 只读类：只声明 `readOnlyHint`，另外两个留空（表格里的 `—`）
        const fn reader(name: &'static str) -> Self {
            Self {
                name,
                read_only: Some(true),
                destructive: None,
                idempotent: None,
            }
        }

        /// 写类：三个 hint 都显式声明成 false
        const fn writer(name: &'static str) -> Self {
            Self {
                name,
                read_only: Some(false),
                destructive: Some(false),
                idempotent: Some(false),
            }
        }
    }

    #[test]
    fn exactly_the_thirteen_designed_tools_are_exposed() {
        let names: Vec<String> = ReqlyMcp::routes()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();

        assert_eq!(names.len(), EXPECTED.len(), "tool 数量必须是 13：{names:?}");
        for expected in EXPECTED {
            let name = expected.name;
            assert!(names.contains(&name.to_string()), "缺少 tool `{name}`");
        }
    }

    #[test]
    fn every_tool_declares_the_designed_annotations() {
        let routes = ReqlyMcp::routes();

        for Expected {
            name,
            read_only,
            destructive,
            idempotent,
        } in EXPECTED
        {
            let tool = routes
                .iter()
                .find(|tool| tool.name == name)
                .unwrap_or_else(|| panic!("找不到 tool `{name}`"));
            let annotations = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("`{name}` 必须声明注解：客户端靠它决定审批策略"));

            assert_eq!(
                annotations.read_only_hint, read_only,
                "`{name}` 的 readOnlyHint 与 design.md §8.2 不一致"
            );
            assert_eq!(
                annotations.destructive_hint, destructive,
                "`{name}` 的 destructiveHint 与 design.md §8.2 不一致"
            );
            assert_eq!(
                annotations.idempotent_hint, idempotent,
                "`{name}` 的 idempotentHint 与 design.md §8.2 不一致"
            );
        }
    }

    #[test]
    fn only_delete_request_is_destructive_and_nothing_is_both_read_only_and_writable() {
        for tool in ReqlyMcp::routes() {
            let annotations = tool.annotations.as_ref().expect("注解必须存在");
            let read_only = annotations.read_only_hint.unwrap_or(false);
            let destructive = annotations.destructive_hint.unwrap_or(false);

            // 只读的 tool 不该标 destructive：客户端可能只读 destructiveHint 就弹审批，
            // 一条纯查询被拦下来问「确定要破坏吗」比不弹还烦。
            assert!(
                !(read_only && destructive),
                "`{}` 同时标了 readOnly 与 destructive",
                tool.name
            );
            // §8.2 唯一的 destructive 就是软删除
            assert_eq!(
                destructive,
                tool.name == "delete_request",
                "`{}` 的 destructiveHint 不该是 {destructive}",
                tool.name
            );
        }
    }

    #[test]
    fn tool_names_avoid_dots_and_hyphens() {
        // §8.2 的硬要求：Hermes 会把带点号的名字规整成下划线，容易与其他 server 撞名。
        for tool in ReqlyMcp::routes() {
            let name = tool.name.to_string();
            assert!(
                !name.contains('.') && !name.contains('-'),
                "tool 名 `{name}` 含点号或连字符"
            );
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "tool 名 `{name}` 不是纯 snake_case"
            );
        }
    }

    #[test]
    fn every_tool_has_a_description_for_the_client() {
        for tool in ReqlyMcp::routes() {
            let description = tool.description.as_deref().unwrap_or("");
            assert!(
                !description.trim().is_empty(),
                "tool `{}` 没有 description：AI 只能靠它判断这个工具做什么",
                tool.name
            );
        }
    }
}
