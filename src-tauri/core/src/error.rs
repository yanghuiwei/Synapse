//! 统一错误类型与错误码。
//!
//! **契约（design.md §9.1）**：后端**只返回错误码**，不返回面向用户的文案。
//! 前端 `src/lib/ipc.ts` 把 `code` 映射成 `src/lib/i18n/zh-CN.ts` 里的 key。
//!
//! 所以这里出现的所有人类可读字符串都只是**给开发者排查用的 debug 信息**，
//! UI 一律不展示。字段名固定为 `code` + `debug`（+ 可选 `params`）。

use serde::ser::{Serialize, SerializeStruct, Serializer};
use serde_json::json;

/// 错误码常量。加新码必须同步在 `src/lib/ipc.ts` 的 `ERROR_KEYS` 里登记，
/// 否则前端只能退化成「未知错误」。
pub mod codes {
    /// 数据库操作失败（兜底）
    pub const DB: &str = "E_DB";
    /// SQLITE_BUSY：库被占用，可重试
    pub const DB_LOCKED: &str = "E_DB_LOCKED";
    /// 迁移脚本执行失败
    pub const MIGRATION: &str = "E_MIGRATION";
    /// 目标记录不存在
    pub const NOT_FOUND: &str = "E_NOT_FOUND";
    /// 入参不合法（字段级信息在 params 里）
    pub const INVALID_INPUT: &str = "E_INVALID_INPUT";
    /// 文件系统 / 数据目录相关失败
    pub const IO: &str = "E_IO";
    /// 不该发生的内部错误（panic、任务丢失等）
    pub const INTERNAL: &str = "E_INTERNAL";

    // ---- 请求层（P2） ----
    //
    // 为什么 DNS / 连接被拒 / TLS 要分成三个码：
    // 它们的**用户动作完全不同** —— 域名打错了要去改地址，
    // 连接被拒说明服务没起或端口不对，TLS 失败通常是证书或自签问题
    // （对应的解法是打开「校验 TLS」开关）。合成一个"网络错误"等于让用户自己猜。
    /// 域名解析失败
    pub const DNS: &str = "E_DNS";
    /// TCP 层连不上（端口没监听 / 被防火墙拦）
    pub const CONN_REFUSED: &str = "E_CONN_REFUSED";
    /// TLS 握手 / 证书校验失败
    pub const TLS: &str = "E_TLS";
    /// 请求超时
    pub const TIMEOUT: &str = "E_TIMEOUT";
    /// 重定向次数超过上限
    pub const REDIRECT: &str = "E_REDIRECT";
    /// 响应体解码失败（压缩流损坏、body 不完整等）
    pub const DECODE: &str = "E_DECODE";
    /// 请求地址本身不合法
    pub const URL: &str = "E_URL";
    /// 请求还在构造阶段就失败了（header 值非法等）
    pub const REQUEST: &str = "E_REQUEST";
    /// 请求体不是合法 JSON（本地就能判出来，不必白跑一趟网络）
    pub const BODY_NOT_JSON: &str = "E_BODY_NOT_JSON";
    /// 用户主动取消
    pub const CANCELLED: &str = "E_CANCELLED";
    /// 网络层兜底（上面几个都不是时）
    pub const NETWORK: &str = "E_NETWORK";
    /// 该功能还没实现（params.feature 说明是哪个）
    pub const NOT_IMPLEMENTED: &str = "E_NOT_IMPLEMENTED";

    // ---- 进程模式（P8） ----
    /// 进程被起成了只读模式（MCP `--read-only` / CLI `--read-only`），写操作被拒。
    ///
    /// 与「权限不足」分开：这条**重试永远不会变好**，所以不进 `retryable()`。
    pub const READ_ONLY: &str = "E_READ_ONLY";

    // ---- 变量与 Token（P4） ----
    //
    // 为什么这三个要分开：它们指向的**修改动作完全不同** ——
    // 配置缺失要去 Token 源面板补配置；刷新请求失败要去看那个接口本身能不能通
    // （或者看网络/证书）；提取失败说明配置里的 JSONPath 与真实响应对不上。
    // 合成一句「Token 出错了」等于让用户自己从头猜一遍。
    /// Token 源配置本身不成立（没有可用来源 / 找不到取 token 的请求）
    pub const TOKEN_SOURCE: &str = "E_TOKEN_SOURCE";
    /// 取 token 的请求本身失败了（网络层错误 / 非 2xx 响应）
    pub const TOKEN_REFRESH: &str = "E_TOKEN_REFRESH";
    /// 取到了响应，但按配置的路径提不出 access_token
    pub const TOKEN_EXTRACT: &str = "E_TOKEN_EXTRACT";
    /// 没有可用缓存，且该 Token 源关闭了自动刷新（需要用户手动刷一次）
    pub const TOKEN_EXPIRED: &str = "E_TOKEN_EXPIRED";

    // ---- 系统分类（P8 打磨） ----
    /// 对系统分类（「token刷新」）做了删除 / 改名 / 移动。
    ///
    /// 单开一个码而不是复用 `E_INVALID_INPUT`：这条**不是用户填错了什么**，
    /// 而是「这个分类本来就不归你管」，界面上该说的是「系统分类不可删除」
    /// 而不是「输入不合法」。前端也靠它把菜单项直接置灰，而不是等用户点了才报错。
    pub const SYSTEM_FOLDER: &str = "E_SYSTEM_FOLDER";

    // ---- 导入器（P5） ----
    //
    // 三个码分开的理由是**用户的下一个动作不同**：
    // 识别失败 = 这段东西我们压根不认（可能粘错了，或格式还没支持）；
    // JSON 不合法 = 文件选错了或文件坏了（去重新导出一次）；
    // 解析失败 = 格式认出来了，但里面某一条解不动（去看警告列表定位到哪一条）。
    /// 没能从内容里识别出格式
    pub const IMPORT_DETECT: &str = "E_IMPORT_DETECT";
    /// 内容是 JSON 但不合法（或不是我们认得的那种 JSON）
    pub const IMPORT_JSON: &str = "E_IMPORT_JSON";
    /// 认出了格式，但内容本身解不动（引号没闭合、必填字段缺失等）
    pub const IMPORT_PARSE: &str = "E_IMPORT_PARSE";
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("sqlite error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("connection pool error: {0}")]
    Pool(#[from] r2d2::Error),

    #[error("migration failed at version {version}: {reason}")]
    Migration { version: i64, reason: String },

    #[error("{entity} #{id} not found")]
    NotFound { entity: &'static str, id: i64 },

    #[error("invalid input: {field} = {reason}")]
    InvalidInput { field: &'static str, reason: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("internal error: {0}")]
    Internal(String),

    // ---- 请求层（P2） ----
    /// 请求失败。
    ///
    /// 用一个带 `code` 的变体、而不是「每个错误码一个变体」：请求层的失败模式有十来种，
    /// 但它们的**结构完全一样**（一个码 + 给开发者看的调试信息 + 可选参数）。
    /// 拆成十来个变体只会把 `code()` / `params()` 变成两张需要同步维护的大表。
    /// `code` 必须是 [`codes`] 里的常量。
    #[error("http request failed ({code}): {debug}")]
    Http {
        code: &'static str,
        debug: String,
        /// 结构化补充信息，供前端插值（如 DNS 失败时的 host）
        params: Option<serde_json::Value>,
    },

    #[error("invalid request url `{raw}`: {reason}")]
    InvalidUrl { raw: String, reason: String },

    #[error("system folder #{id} cannot be {action}")]
    SystemFolder { id: i64, action: &'static str },

    #[error("not implemented: {feature}")]
    NotImplemented { feature: &'static str },

    #[error("request cancelled by user")]
    Cancelled,

    /// 只读模式下拒绝了写操作。`tool` 是发起方的操作名（MCP 的 tool 名 / CLI 的子命令名），
    /// 用于让调用者说清「是哪个操作被拦了」而不是给一句笼统的「操作被拒」。
    #[error("write refused in read-only mode: {tool}")]
    ReadOnly { tool: &'static str },
}

impl AppError {
    /// 映射成稳定的错误码。前端只认这个字段。
    ///
    /// 注意 `Db` 要细分：SQLITE_BUSY / SQLITE_LOCKED 是可重试的瞬时故障，
    /// 语义上与「SQL 写错了」完全不同，前端提示语也不一样。
    pub fn code(&self) -> &'static str {
        match self {
            AppError::Db(err) => match err {
                rusqlite::Error::SqliteFailure(e, _) => match e.code {
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                        codes::DB_LOCKED
                    }
                    _ => codes::DB,
                },
                _ => codes::DB,
            },
            AppError::Pool(_) => codes::DB,
            AppError::Migration { .. } => codes::MIGRATION,
            AppError::NotFound { .. } => codes::NOT_FOUND,
            AppError::InvalidInput { .. } => codes::INVALID_INPUT,
            AppError::Io(_) => codes::IO,
            AppError::Internal(_) => codes::INTERNAL,
            AppError::Http { code, .. } => code,
            AppError::InvalidUrl { .. } => codes::URL,
            AppError::SystemFolder { .. } => codes::SYSTEM_FOLDER,
            AppError::NotImplemented { .. } => codes::NOT_IMPLEMENTED,
            AppError::Cancelled => codes::CANCELLED,
            AppError::ReadOnly { .. } => codes::READ_ONLY,
        }
    }

    /// 请求层错误的快捷构造（不带 params）
    pub fn http(code: &'static str, debug: impl Into<String>) -> Self {
        AppError::Http {
            code,
            debug: debug.into(),
            params: None,
        }
    }

    /// 请求层错误的快捷构造（带 params，用于给前端插值）
    pub fn http_with(
        code: &'static str,
        debug: impl Into<String>,
        params: serde_json::Value,
    ) -> Self {
        AppError::Http {
            code,
            debug: debug.into(),
            params: Some(params),
        }
    }

    /// 结构化补充信息，供前端拼更精确的提示（如「找不到 request #12」）。
    pub fn params(&self) -> Option<serde_json::Value> {
        match self {
            AppError::NotFound { entity, id } => Some(json!({ "entity": entity, "id": id })),
            AppError::InvalidInput { field, .. } => Some(json!({ "field": field })),
            AppError::Migration { version, .. } => Some(json!({ "version": version })),
            AppError::Http { params, .. } => params.clone(),
            AppError::InvalidUrl { raw, .. } => Some(json!({ "raw": raw })),
            AppError::SystemFolder { id, .. } => Some(json!({ "id": id })),
            AppError::NotImplemented { feature } => Some(json!({ "feature": feature })),
            AppError::ReadOnly { tool } => Some(json!({ "tool": tool })),
            _ => None,
        }
    }

    /// 这个错误是否值得让用户重试（前端据此决定要不要显示「重试」按钮）
    pub fn retryable(&self) -> bool {
        matches!(
            self.code(),
            codes::DB_LOCKED | codes::TIMEOUT | codes::NETWORK | codes::CONN_REFUSED
        )
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let has_params = self.params().is_some();
        let mut st = serializer.serialize_struct("AppError", if has_params { 3 } else { 2 })?;
        st.serialize_field("code", self.code())?;
        if has_params {
            st.serialize_field("params", &self.params())?;
        }
        // 仅用于日志/开发排查，UI 不展示
        st.serialize_field("debug", &self.to_string())?;
        st.end()
    }
}

pub type AppResult<T> = Result<T, AppError>;
