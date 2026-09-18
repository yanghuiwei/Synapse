//! 数据模型。
//!
//! **与前端 `src/lib/types.ts` 一一对应**，字段名一律 camelCase
//! （`#[serde(rename_all = "camelCase")]`），DB 里的 snake_case 列名在 repo 里映射。
//!
//! JSON 列的处理原则：
//! * 结构稳定、多处复用（params / headers / settings）→ 定义具体结构体
//! * 结构还没定下来（历史条目 / 导入预览的快照）→ 用 [`serde_json::Value`] 占位
//!
//! 🔴 这里**没有 `auth` 这个东西**。007 迁移之后「带什么凭据」就是 `headers`
//! 里的一行（见 [`ApiRequest::headers`]），分类层不再持有任何授权配置。

use std::collections::BTreeMap;

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
    /// 文件字段的**磁盘路径**（只有 `multipart/form-data` 用得上）。
    ///
    /// 为什么不新开一个 `FormField` 类型：multipart 的每一行要么是文本、
    /// 要么是文件，两者共用「键 / 值 / 是否启用」这套形状。拆成两个类型意味着
    /// 两张表、两套占位符扫描、两处解析 —— 而它们的差别只有一个字段。
    ///
    /// 非空即表示这一行是文件：`value` 被忽略，发送时才去读这个路径
    /// （不在编辑期读，是因为大文件不该被搬进内存再由 IPC 传一遍）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

impl KeyValueItem {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            enabled: true,
            desc: None,
            file: None,
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
    /// 系统分类（「token刷新」）：**不可删除 / 改名 / 移动**。
    ///
    /// 与 `is_token_source` 是两件事：那个是用户可以随时切换的引导标记，
    /// 这个是「这一行由程序托管」的硬约束。Token 来源固定配在系统分类下，
    /// 所以系统分类同时也是 Token 分组（建的时候顺手置上）。
    pub is_system: bool,
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
    /// 授权**不是**一个字段 —— 它就是这个数组里的一行。
    ///
    /// `Authorization: Bearer {{token.access}}` 与 `X-Access-Token: <裸 JWT>`
    /// 都是普通请求头，值为普通模板串。原先那套 `AuthConfig`
    /// （inherit / none / bearer / basic / apiKey）连同 `folder.auth` 继承链
    /// 在 007 迁移里被拆掉了：实测继承链深度恒为 1，而它的代价是
    /// 「4 条请求因为自己写了显式授权、反而静默切断了继承、一条凭据都不带」。
    pub headers: Vec<KeyValueItem>,
    pub body_type: BodyType,
    /// 当前 `body_type` 对应的那份内容，**原样发出**（见 `http::execute::build_body`）。
    pub body: String,
    /// 每种 `body_type` 各一份的内容快照，形如 `{"json":"{…}","raw":"hello"}`。
    ///
    /// 键用 `String` 而不是 `BodyType`：用户库里可能留着更早版本写下的类型名，
    /// 拿枚举当键会让**整条记录**反序列化失败，而这里既定的语义是
    /// 「未知值退化成 none、其余字段照读」（见 [`BodyType::parse`]）。
    ///
    /// 执行路径**从不读它** —— `build_body` 只看 `body`；这一列纯粹服务于
    /// 编辑器「切走再切回来内容还在」。用 `BTreeMap` 是为了键有序，
    /// 写回库里时 diff 稳定。
    pub body_by_type: BTreeMap<String, String>,
    /// 键值型请求体（当前只有 urlencoded）的行。与 `params` / `headers` 同构，
    /// 所以键值表组件、占位符高亮都能直接复用。
    ///
    /// ⚠️ 它**只服务 `urlencoded`**。`form` 的行在 [`Self::form_fields`] ——
    /// 两种类型共用一列会让「切类型」把上一种表格的行带过去（005 迁移的说明里有完整推演）。
    pub body_rows: Vec<KeyValueItem>,
    /// `multipart/form-data` 的字段行（005 起）。
    ///
    /// 与 `body_rows` 同构，差别只在多一点：`file` 非空的行是**文件字段**，
    /// 值是一个磁盘路径，发送时才读。
    pub form_fields: Vec<KeyValueItem>,
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

impl TokenSource {
    /// 「**这份 token 是由哪套配置取的**」的指纹。
    ///
    /// 它只有两个用途，而且必须**只有这一处**定义：
    /// * `storage::repo::token::cache_owner` —— 同指纹的来源共用一行 `token_cache`
    /// * `http::token::TokenRefresher::flight_for` —— 同指纹的来源共用一把单飞锁
    ///
    /// 于是「配置相同就共用同一个 token、同一个倒计时；配置不同就各自独立」
    /// 这条规则在后端只有一个来源，不会出现「缓存说共享、锁说独立」这种半拉子状态。
    ///
    /// ## 进指纹的：决定「发出什么请求 + 从响应里取什么」的字段
    ///
    /// 取 token 的请求、三条提取路径、过期字段的单位、刷新时实际会打的那个请求。
    ///
    /// 刷新请求取的是**有效值**（`refresh_request_id.unwrap_or(fetch_request_id)`）
    /// 而不是原字段：`None` 的语义就是「刷新时复用取 token 那个请求」，
    /// 所以「显式填成同一个」与「不填」在行为上完全一样，
    /// 拿原字段拼指纹会让这两条等价配置变成两份独立缓存。
    ///
    /// ## 刻意不进指纹的
    ///
    /// * `name` / `folder_id` —— 它们说的是「这条配置叫什么、放在哪」，
    ///   与拿到的 token 无关。算进去，则「同一个取 token 接口按业务拆成两条来源」
    ///   又会各打一次网络、各走一个倒计时 —— 那正是这套指纹要消灭的情形。
    /// * `auto_refresh` / `skew_seconds` —— 它们是**使用策略**（什么时候该刷、
    ///   提前多久算过期），不是「token 是什么」。两条来源共用同一个 token 时，
    ///   「这次该不该自动刷」仍各自按自己那份配置判定，这样反而更准。
    ///
    /// `workspace_id` 进指纹：两个工作区里同名同路径的配置是两份互不相干的东西，
    /// 而 `token_cache` 表**没有工作区列**（它挂在来源行上），不隔开就会串。
    ///
    /// 用 `\u{1f}`（单元分隔符）拼接而不是 `:` 或 `-`：JSONPath 与 URL 里
    /// 都满是冒号，拿它当分隔符会让 `a:b`+`c` 与 `a`+`b:c` 撞成同一个键 ——
    /// 那种碰撞的后果是「两条配置不同的来源共用了一个 token」。
    pub fn share_key(&self) -> String {
        let refresh = self.refresh_request_id.unwrap_or(self.fetch_request_id);
        [
            self.workspace_id.to_string(),
            self.fetch_request_id.to_string(),
            self.access_path.clone(),
            self.refresh_path.clone(),
            self.expires_in_path.clone(),
            self.expires_in_unit.as_db_str().to_string(),
            refresh.to_string(),
        ]
        .join("\u{1f}")
    }
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
    /// 这次执行**真实发出**的请求对应的 curl（凭据为明文）。
    ///
    /// `None` 有两种来源：这条历史发生在 006 之前，或者写入方（MCP / CLI）没有
    /// 这个入口。前端据此退化成「按当前内容重新生成」并标出来源 ——
    /// 而不是把空字符串当成「当时就是一个空请求」。
    pub curl: Option<String>,
    /// 同上，凭据打码。给「要截图 / 要贴出去」的场合用。
    pub curl_masked: Option<String>,
}

/// 写历史时的入参。
///
/// 单独一个结构而不是给 `insert` 塞十个参数：调用点在发请求的热路径上，
/// 参数顺序写错一个（比如 duration 和 size 换位）编译器不会拦，
/// 但字段名写错会。
///
/// 它也直接当 IPC 的入参用（`record_history` 命令），所以带 serde 派生 ——
/// 字段名与前端 `types.ts` 的 `NewHistoryEntry` 一一对应。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// 本次真实发出的请求对应的 curl（见 [`HistoryEntry::curl`]）。
    ///
    /// 由前端从发送结果里原样带过来 —— 后端在 `send_request` 里已经算好了，
    /// 前端只负责搬运，不重新拼一遍（两处拼法迟早会不一样）。
    #[serde(default)]
    pub curl: Option<String>,
    /// 同上，凭据打码那一份。
    #[serde(default)]
    pub curl_masked: Option<String>,
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
            is_system: true,
            created_at: "c".into(),
            updated_at: "u".into(),
            deleted_at: None,
        };
        let json = serde_json::to_value(&f).unwrap();
        assert!(json.get("workspaceId").is_some(), "workspaceId missing");
        assert!(json.get("isTokenSource").is_some(), "isTokenSource missing");
        assert!(json.get("isSystem").is_some(), "isSystem missing");
        assert!(json.get("sortOrder").is_some(), "sortOrder missing");
        assert!(json.get("workspace_id").is_none(), "snake_case leaked");
    }

    /// 凭据头是**普通请求头**，序列化形状必须与前端 `KeyValueItem` 逐字段对齐 ——
    /// `enabled` 必须是 JSON 布尔，写成 `1` 会让 serde 拒掉整条请求
    /// （007 迁移里 `json_object(…, 'enabled', json('true'))` 那个 `json()` 就是为它）。
    #[test]
    fn credential_headers_serialize_as_plain_key_value_items() {
        let item = KeyValueItem::new("Authorization", "Bearer {{token.access}}");
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["key"], serde_json::json!("Authorization"));
        assert_eq!(json["value"], serde_json::json!("Bearer {{token.access}}"));
        assert_eq!(
            json["enabled"],
            serde_json::json!(true),
            "enabled 必须是 JSON 布尔而不是数字 1"
        );

        let back: KeyValueItem = serde_json::from_value(json).expect("roundtrip");
        assert_eq!(back, item);
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
