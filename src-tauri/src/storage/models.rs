//! 数据模型。
//!
//! **与前端 `src/lib/types.ts` 一一对应**，字段名一律 camelCase
//! （`#[serde(rename_all = "camelCase")]`），DB 里的 snake_case 列名在 repo 里映射。
//!
//! JSON 列的处理原则：
//! * 结构稳定、多处复用（params / headers / settings）→ 定义具体结构体
//! * 结构还在演化（auth）→ 先用 [`serde_json::Value`] 占位并标注 TODO，
//!   避免现在拍一个 P2/P4 就会推翻的 schema

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================
// 请求内的值类型
// ============================================================

/// 键值对行。params / headers / form 共用一套结构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyValueItem {
    pub key: String,
    pub value: String,
    /// 勾选开关；未勾选的行不参与请求
    #[serde(default = "yes")]
    pub enabled: bool,
    /// 备注（Postman 的 description）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
}

impl KeyValueItem {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            enabled: true,
            desc: None,
        }
    }
}

/// 请求体类型，对应 `request.body_type`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BodyType {
    #[default]
    None,
    Json,
    Form,
    Urlencoded,
    Raw,
    Binary,
    Graphql,
}

impl BodyType {
    pub fn as_db_str(self) -> &'static str {
        match self {
            BodyType::None => "none",
            BodyType::Json => "json",
            BodyType::Form => "form",
            BodyType::Urlencoded => "urlencoded",
            BodyType::Raw => "raw",
            BodyType::Binary => "binary",
            BodyType::Graphql => "graphql",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "json" => BodyType::Json,
            "form" => BodyType::Form,
            "urlencoded" => BodyType::Urlencoded,
            "raw" => BodyType::Raw,
            "binary" => BodyType::Binary,
            "graphql" => BodyType::Graphql,
            // 未知值一律当 none，而不是报错 —— 旧数据不该让整个列表打不开
            _ => BodyType::None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HttpVersion {
    #[default]
    Auto,
    Http1,
    Http2,
}

impl HttpVersion {
    pub fn as_db_str(self) -> &'static str {
        match self {
            HttpVersion::Auto => "auto",
            HttpVersion::Http1 => "http1",
            HttpVersion::Http2 => "http2",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "http1" => HttpVersion::Http1,
            "http2" => HttpVersion::Http2,
            _ => HttpVersion::Auto,
        }
    }
}

/// Token 过期字段的单位，对应 `token_source.expires_in_unit`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExpiryUnit {
    /// `expires_in` 是「还有多少秒」（OAuth 标准）
    #[default]
    Seconds,
    /// 毫秒
    Millis,
    /// 绝对时间（ISO8601 字符串）
    Absolute,
}

impl ExpiryUnit {
    pub fn as_db_str(self) -> &'static str {
        match self {
            ExpiryUnit::Seconds => "seconds",
            ExpiryUnit::Millis => "millis",
            ExpiryUnit::Absolute => "absolute",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "millis" => ExpiryUnit::Millis,
            "absolute" => ExpiryUnit::Absolute,
            _ => ExpiryUnit::Seconds,
        }
    }
}

/// 逐请求的发送设置，对应 `request.settings` 这一 JSON 列。
///
/// 每个字段都有 serde default，所以历史数据里的 `{}` 能平滑升级成完整配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSettings {
    #[serde(default = "yes")]
    pub follow_redirects: bool,
    #[serde(default = "default_max_redirects")]
    pub max_redirects: u32,
    /// 是否校验 TLS 证书。公司内网自签证书场景要关掉（design.md §8.5）
    #[serde(default = "yes")]
    pub verify_tls: bool,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// 形如 `http://127.0.0.1:7890`；None = 直连
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    #[serde(default)]
    pub http_version: HttpVersion,
}

impl Default for RequestSettings {
    fn default() -> Self {
        Self {
            follow_redirects: true,
            max_redirects: default_max_redirects(),
            verify_tls: true,
            timeout_ms: default_timeout_ms(),
            proxy: None,
            http_version: HttpVersion::Auto,
        }
    }
}

fn yes() -> bool {
    true
}

fn default_max_redirects() -> u32 {
    10
}

fn default_timeout_ms() -> u64 {
    30_000
}

// ============================================================
// 实体
// ============================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: i64,
    /// 允许为空 —— 默认工作区不给名字，由前端查 i18n 文案（`workspace.default`）显示
    pub name: String,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: i64,
    pub workspace_id: i64,
    /// None = 根级
    pub parent_id: Option<i64>,
    pub name: String,
    pub sort_order: i64,
    /// 是否被标记为 Token 源分组（design.md §4.3）
    pub is_token_source: bool,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiRequest {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub workspace_id: i64,
    pub name: String,
    pub method: String,
    pub url: String,
    pub params: Vec<KeyValueItem>,
    pub headers: Vec<KeyValueItem>,
    pub body_type: BodyType,
    pub body: String,
    /// TODO(P2/P4)：AuthConfig 的结构要到 Token 那一轮才定稿，
    /// 现在只做透传，不提前固化 schema。
    pub auth: Value,
    pub settings: RequestSettings,
    pub pre_script: String,
    pub test_script: String,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Environment {
    pub id: i64,
    pub workspace_id: i64,
    pub name: String,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvVar {
    pub id: i64,
    pub environment_id: i64,
    pub key: String,
    pub value: String,
    /// 1 时值走系统密钥串加密存储（加密实现在 P8）
    pub is_secret: bool,
    pub enabled: bool,
    pub sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenSource {
    pub id: i64,
    pub workspace_id: i64,
    pub folder_id: i64,
    pub name: String,
    /// 用哪个请求去取 token
    pub fetch_request_id: i64,
    pub access_path: String,
    pub refresh_path: String,
    pub expires_in_path: String,
    pub expires_in_unit: ExpiryUnit,
    /// None = 刷新时复用 fetch_request_id
    pub refresh_request_id: Option<i64>,
    pub auto_refresh: bool,
    /// 提前多少秒视为已过期
    pub skew_seconds: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenCache {
    pub id: i64,
    pub token_source_id: i64,
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// ISO8601；None = 永不过期
    pub expires_at: Option<String>,
    /// 原始响应快照，调试用
    pub raw_response: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: i64,
    pub workspace_id: i64,
    pub request_id: Option<i64>,
    /// 冗余快照：请求被删后列表仍显示得出名字
    pub request_name: String,
    /// 完整请求快照（JSON），点历史可回填
    pub snapshot: Value,
    /// HTTP 状态码；None = 网络层就失败了
    pub status: Option<i64>,
    pub duration_ms: Option<i64>,
    pub resp_size: Option<i64>,
    pub error: Option<String>,
    /// 当时使用的环境名
    pub environment: Option<String>,
    pub executed_at: String,
    /// 非空 = 在回收站
    pub deleted_at: Option<String>,
}

/// 写历史时的入参。
///
/// 单独一个结构而不是给 `insert` 塞十个参数：调用点在发请求的热路径上，
/// 参数顺序写错一个（比如 duration 和 size 换位）编译器不会拦，
/// 但字段名写错会。
#[derive(Debug, Clone)]
pub struct NewHistoryEntry {
    pub workspace_id: i64,
    pub request_id: Option<i64>,
    pub request_name: String,
    /// 完整请求快照，点历史时回填用
    pub snapshot: Value,
    /// None = 网络层就失败了
    pub status: Option<i64>,
    pub duration_ms: Option<i64>,
    pub resp_size: Option<i64>,
    pub error: Option<String>,
    pub environment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadtestRun {
    pub id: i64,
    pub request_id: Option<i64>,
    pub request_name: String,
    pub config: Value,
    pub summary: Value,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadtestSample {
    pub id: i64,
    pub run_id: i64,
    pub seq: i64,
    pub status: Option<i64>,
    pub duration_ms: i64,
    pub bytes: Option<i64>,
    pub error: Option<String>,
    /// 相对压测开始的偏移，画时间线用
    pub started_offset_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
    pub key: String,
    pub value: String,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_type_roundtrips_through_db_string() {
        for bt in [
            BodyType::None,
            BodyType::Json,
            BodyType::Form,
            BodyType::Urlencoded,
            BodyType::Raw,
            BodyType::Binary,
            BodyType::Graphql,
        ] {
            assert_eq!(BodyType::parse(bt.as_db_str()), bt);
        }
    }

    #[test]
    fn unknown_db_values_fall_back_to_default_instead_of_failing() {
        assert_eq!(BodyType::parse("something-new"), BodyType::None);
        assert_eq!(HttpVersion::parse("http3"), HttpVersion::Auto);
        assert_eq!(ExpiryUnit::parse("weeks"), ExpiryUnit::Seconds);
    }

    #[test]
    fn empty_settings_json_parses_to_defaults() {
        let parsed: RequestSettings = serde_json::from_str("{}").expect("{} must be valid");
        assert_eq!(parsed, RequestSettings::default());
        assert_eq!(parsed.timeout_ms, 30_000);
        assert!(parsed.verify_tls);
        assert!(parsed.follow_redirects);
    }

    /// 前端看到的是 camelCase，别被 rename_all 悄悄改坏
    #[test]
    fn serializes_to_camel_case() {
        let f = Folder {
            id: 1,
            workspace_id: 2,
            parent_id: None,
            name: "n".into(),
            sort_order: 3,
            is_token_source: true,
            created_at: "c".into(),
            updated_at: "u".into(),
            deleted_at: None,
        };
        let json = serde_json::to_value(&f).unwrap();
        assert!(json.get("workspaceId").is_some(), "workspaceId missing");
        assert!(json.get("isTokenSource").is_some(), "isTokenSource missing");
        assert!(json.get("sortOrder").is_some(), "sortOrder missing");
        assert!(json.get("workspace_id").is_none(), "snake_case leaked");
    }

    #[test]
    fn token_source_roundtrips_expiry_unit() {
        for unit in [
            ExpiryUnit::Seconds,
            ExpiryUnit::Millis,
            ExpiryUnit::Absolute,
        ] {
            assert_eq!(ExpiryUnit::parse(unit.as_db_str()), unit);
        }
    }
}
