//! 占位符解析（design.md §4.2）。
//!
//! ## 解析顺序：不写成链式 try
//!
//! 设计稿写的是「动态函数 → env → token → resp」。实现上按**前缀分派**：
//! 这几个命名空间的前缀互不重叠（`$` / `env.` / `token.` / `resp.` / `prev.` /
//! `folder.`），分派与按顺序试的结果完全一致 —— 而分派能准确说出
//! 「为什么没命中」（是变量不存在？还是这个命名空间本轮就不支持？），
//! 链式 try 只能得到一句「都试过了，不知道」。
//!
//! ## 未命中一律原样保留
//!
//! 绝不把 `{{env.nope}}` 换成空串。空的 header、空的 query 参数会让请求
//! **以另一种方式失败**（服务端 400，或者更糟：静默用了默认值），
//! 而留着原文至少还能让人一眼看出自己拼错了。（plan.md §4 验收第 4 条）
//!
//! ## 不做二次展开
//!
//! `{{env.a}}` 解析出来的值里若含 `{{env.b}}`，**不会**再被解析。理由有二：
//! ① 否则 `a = "{{env.a}}"` 就是一个死循环；② 用户填的值里本来就可能
//! 合法地含 `{{ }}`（比如一段模板化的 JSON body）。
//!
//! ## 同步、无 IO
//!
//! 解析器不碰数据库、不碰网络 —— Token 的值由 `http::token` 在解析之前
//! 备好（见 [`TokenValues`]）。这既是「单测不需要 tokio 运行时」的前提，
//! 也是「同一份请求里 `{{env.a}}` 两处必然得到同一个值」的保证。

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use chrono::{DateTime, Local};
use serde_json::Value;

use crate::http::execute::RequestSpec;
use crate::storage::models::KeyValueItem;
use crate::template::functions::{self, Entropy};
use crate::template::jsonpath;

/// 占位符的命名空间。`as_str()` 的值会原样出现在 IPC 里，前端靠它选文案。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    Func,
    Env,
    Token,
    Resp,
    /// 上一条历史响应 —— **P4 不做**，历史表没存响应体，没有数据源
    Prev,
    /// 上级文件夹变量 —— design.md §4.1 标着「可选」，P4 不做
    Folder,
    Unknown,
}

impl Namespace {
    pub fn as_str(self) -> &'static str {
        match self {
            Namespace::Func => "func",
            Namespace::Env => "env",
            Namespace::Token => "token",
            Namespace::Resp => "resp",
            Namespace::Prev => "prev",
            Namespace::Folder => "folder",
            Namespace::Unknown => "unknown",
        }
    }
}

/// 这个占位符为什么没解析出来。
///
/// 是**码**不是文案（契约见 `crate::error`）：文案归前端 i18n 查表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnresolvedReason {
    /// 命名空间不认识，如 `{{foo.bar}}`
    UnknownNamespace,
    /// 写成函数的样子但参数不合法，如 `{{$datetime}}`（缺格式串）
    BadFunction,
    /// 没有激活环境，或变量不存在，或变量被停用
    EnvNotFound,
    /// 这条请求的文件夹链上没有 Token 源
    NoTokenSource,
    /// 有 Token 源但当下拿不出值（键名不认识 / 关闭了自动刷新且已过期）
    TokenUnavailable,
    /// 本次会话还没有可参考的响应
    NoResponse,
    /// 响应里没有这个路径（或路径语法超出支持的子集）
    RespPathMiss,
    /// 本阶段明确不支持的命名空间（`prev.*` / `folder.*`）
    NotSupported,
}

impl UnresolvedReason {
    pub fn as_str(self) -> &'static str {
        match self {
            UnresolvedReason::UnknownNamespace => "unknownNamespace",
            UnresolvedReason::BadFunction => "badFunction",
            UnresolvedReason::EnvNotFound => "envNotFound",
            UnresolvedReason::NoTokenSource => "noTokenSource",
            UnresolvedReason::TokenUnavailable => "tokenUnavailable",
            UnresolvedReason::NoResponse => "noResponse",
            UnresolvedReason::RespPathMiss => "respPathMiss",
            UnresolvedReason::NotSupported => "notSupported",
        }
    }
}

/// 扫到的一处占位符引用（还没有值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanHit {
    pub namespace: Namespace,
    /// 命名空间之后的部分：`baseUrl` / `access` / `$.data.id` / 整个 `$datetime '%Y'`
    pub key: String,
    /// 原文里的完整字面量（含 `{{` `}}`）—— 未命中时要**原样**回填
    pub raw: String,
    /// 在原文中的字节区间
    pub span: Range<usize>,
}

/// 扫描结果里带上了值 / 未命中的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPlaceholder {
    pub hit: ScanHit,
    /// `None` = 未命中，此时原文原样保留
    pub value: Option<String>,
    pub reason: Option<UnresolvedReason>,
}

/// 解析后的一段文本。
#[derive(Debug, Clone)]
pub struct ResolvedText {
    pub text: String,
    pub placeholders: Vec<ResolvedPlaceholder>,
}

impl ResolvedText {
    /// 没解析出来的那些（它们的原文原样留在 `text` 里）
    pub fn unresolved(&self) -> impl Iterator<Item = &ResolvedPlaceholder> {
        self.placeholders.iter().filter(|p| p.value.is_none())
    }

    pub fn unresolved_count(&self) -> usize {
        self.unresolved().count()
    }
}

/// 解析时刻可用的 Token 值。
///
/// 由 `http::token` 在解析**之前**备好 —— 解析器自己不碰网络，
/// 所以它可以是同步纯函数，也就能被单测直接钉死。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenValues {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// ISO8601；`None` = 永不过期
    pub expires_at: Option<String>,
}

impl TokenValues {
    /// 认的键名与 design.md §4.2 的 `{{token.*}}` 一致。
    pub fn get(&self, key: &str) -> Option<&str> {
        match key {
            "access" => Some(self.access_token.as_str()),
            "refresh" => self.refresh_token.as_deref(),
            "expiresAt" => self.expires_at.as_deref(),
            _ => None,
        }
    }
}

/// 解析上下文。字段全是**已备好的数据**，解析过程不会回头去取。
pub struct TemplateContext<'a> {
    /// 激活环境里 `enabled = 1` 的变量
    pub env: &'a HashMap<String, String>,
    /// 本次发送可用的 Token 值；这条请求的文件夹链上没有 Token 源时为 `None`
    pub token: Option<&'a TokenValues>,
    /// 最近一次成功的响应（`{{resp.$.path}}` 的来源）
    pub resp: Option<&'a Value>,
    /// 注入的当前时间。`$timestamp` / `$datetime` 用它 ——
    /// 同一份请求里两处 `{{$timestamp}}` 因此必然相同。
    pub now: DateTime<Local>,
    /// 注入的熵源。`$uuid` / `$randomInt` 用它
    pub entropy: Entropy,
}

/// 扫出文本里所有的占位符引用。**不做值替换**。
///
/// 单飞锁那一步要靠它先判断「这次发送到底用不用得上 token」——
/// 用不上就不该为了它去打一次刷新请求。
pub fn scan(text: &str) -> Vec<ScanHit> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // 找不到收尾的 `}}` 就当它不是占位符（用户可能正在输入）
            let Some(close) = find_closing(bytes, i + 2) else {
                break;
            };
            let inner = text[i + 2..close].trim();
            let end = close + 2;
            let (namespace, key) = classify(inner);
            out.push(ScanHit {
                namespace,
                key,
                raw: text[i..end].to_string(),
                span: i..end,
            });
            i = end;
        } else {
            i += 1;
        }
    }

    out
}

/// `{{` 之后找 `}}`，返回 `}}` 中前一个 `}` 的字节下标。
fn find_closing(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'}' && bytes[i + 1] == b'}' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// 按前缀分派命名空间 —— 见文件头对「为什么不写链式 try」的说明。
fn classify(inner: &str) -> (Namespace, String) {
    if functions::looks_like_function(inner) {
        return (Namespace::Func, inner.to_string());
    }
    match inner.split_once('.') {
        Some(("env", key)) => (Namespace::Env, key.to_string()),
        Some(("token", key)) => (Namespace::Token, key.to_string()),
        Some(("resp", key)) => (Namespace::Resp, key.to_string()),
        Some(("prev", key)) => (Namespace::Prev, key.to_string()),
        Some(("folder", key)) => (Namespace::Folder, key.to_string()),
        _ => (Namespace::Unknown, inner.to_string()),
    }
}

/// 解析一段文本。未命中的占位符**原样保留**在结果里。
pub fn resolve(text: &str, ctx: &mut TemplateContext<'_>) -> ResolvedText {
    let hits = scan(text);
    if hits.is_empty() {
        return ResolvedText {
            text: text.to_string(),
            placeholders: Vec::new(),
        };
    }

    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut placeholders = Vec::with_capacity(hits.len());

    for hit in hits {
        out.push_str(&text[cursor..hit.span.start]);
        let resolved = lookup(hit, ctx);
        match &resolved.value {
            Some(value) => out.push_str(value),
            // 未命中 → 原样保留（见文件头）
            None => out.push_str(&resolved.hit.raw),
        }
        cursor = resolved.hit.span.end;
        placeholders.push(resolved);
    }
    out.push_str(&text[cursor..]);

    ResolvedText {
        text: out,
        placeholders,
    }
}

fn lookup(hit: ScanHit, ctx: &mut TemplateContext<'_>) -> ResolvedPlaceholder {
    let (value, reason) = match hit.namespace {
        Namespace::Func => match functions::evaluate(&hit.key, &ctx.now, &mut ctx.entropy) {
            Some(v) => (Some(v), None),
            None => (None, Some(UnresolvedReason::BadFunction)),
        },
        Namespace::Env => match ctx.env.get(&hit.key) {
            Some(v) => (Some(v.clone()), None),
            None => (None, Some(UnresolvedReason::EnvNotFound)),
        },
        Namespace::Token => match ctx.token {
            None => (None, Some(UnresolvedReason::NoTokenSource)),
            // 「键名不认识」与「有源但取不到值」对用户而言动作是一样的：
            // 去 Token 源配置里看一眼。所以不拆成两个码。
            Some(values) => match values.get(&hit.key) {
                Some(v) => (Some(v.to_string()), None),
                None => (None, Some(UnresolvedReason::TokenUnavailable)),
            },
        },
        Namespace::Resp => match ctx.resp {
            None => (None, Some(UnresolvedReason::NoResponse)),
            Some(resp) => match jsonpath::extract(resp, &hit.key) {
                // `null` 也当没值：往 URL 里塞一个 "null" 字符串只会更误导
                Some(Value::Null) | None => (None, Some(UnresolvedReason::RespPathMiss)),
                Some(found) => (Some(jsonpath::to_placeholder_string(found)), None),
            },
        },
        Namespace::Prev | Namespace::Folder => (None, Some(UnresolvedReason::NotSupported)),
        Namespace::Unknown => (None, Some(UnresolvedReason::UnknownNamespace)),
    };

    ResolvedPlaceholder { hit, value, reason }
}

/// 一份可以发出去的请求定义 + 本次的占位符报告。
pub struct ResolvedSpec {
    pub spec: RequestSpec,
    /// 去重后的未解析占位符。同一处占位符出现在多个字段里只报一次 ——
    /// 否则改一个 `{{env.nope}}` 会看到十几条一模一样的提示。
    pub unresolved: Vec<UnresolvedPlaceholder>,
}

/// 未解析占位符的 IPC 形态。字段名与前端 `types.ts` 对齐。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedPlaceholder {
    pub raw: String,
    pub namespace: &'static str,
    pub key: String,
    pub reason: &'static str,
}

/// 扫描一份请求定义里**所有**可能含占位符的文本。
///
/// 与 [`resolve_spec`] 共用同一批字段（经由 [`spec_text_fields`]），
/// 漏一处就会出现「单飞锁以为用不上 token，解析时却发现用上了」。
///
/// 凭据头天然被覆盖：它就是 `headers` 里的一行，而 `spec_text_fields`
/// 连每个键值行的 `key` / `value` / `file` 都扫。**这正是 007 迁移的收益之一** ——
/// 旧代码要单独列一遍 `AuthConfig` 的文本字段，那条清单被漏改的后果
/// 不是编译错误，而是「29 条请求一起静默降级」。
pub fn scan_spec(spec: &RequestSpec) -> Vec<ScanHit> {
    let mut out = Vec::new();
    for text in spec_text_fields(spec) {
        out.extend(scan(text));
    }
    out
}

pub fn resolve_spec(spec: &RequestSpec, ctx: &mut TemplateContext<'_>) -> ResolvedSpec {
    let mut acc: Vec<ResolvedPlaceholder> = Vec::new();

    let params = resolve_rows(&spec.params, ctx, &mut acc);
    let headers = resolve_rows(&spec.headers, ctx, &mut acc);
    let url = resolve_into(&spec.url, ctx, &mut acc);
    let body = resolve_into(&spec.body, ctx, &mut acc);
    let form_fields = resolve_rows(&spec.form_fields, ctx, &mut acc);

    let resolved = RequestSpec {
        method: spec.method.clone(),
        url,
        params,
        headers,
        body_type: spec.body_type,
        body,
        settings: spec.settings.clone(),
        request_id: spec.request_id,
        form_fields,
    };

    // 去重：按原文（`{{env.nope}}`）判重，保留第一次出现的那条
    let mut seen = HashSet::new();
    let unresolved = acc
        .iter()
        .filter(|p| p.value.is_none())
        .filter(|p| seen.insert(p.hit.raw.clone()))
        .map(|p| UnresolvedPlaceholder {
            raw: p.hit.raw.clone(),
            namespace: p.hit.namespace.as_str(),
            key: p.hit.key.clone(),
            reason: p
                .reason
                .unwrap_or(UnresolvedReason::UnknownNamespace)
                .as_str(),
        })
        .collect();

    ResolvedSpec {
        spec: resolved,
        unresolved,
    }
}

/// 请求定义里所有可能含占位符的字符串。**改字段时这里和 `resolve_spec` 一起改。**
fn spec_text_fields(spec: &RequestSpec) -> Vec<&str> {
    let mut out = vec![spec.url.as_str(), spec.body.as_str()];
    for row in spec
        .params
        .iter()
        .chain(spec.headers.iter())
        .chain(spec.form_fields.iter())
    {
        out.push(row.key.as_str());
        out.push(row.value.as_str());
        // 文件路径也扫：`{{env.dir}}/模板.xlsx` 是很自然的写法，
        // 而漏扫它的表现是「文件读不出来」，错的地方离原因很远
        if let Some(file) = row.file.as_deref() {
            out.push(file);
        }
    }
    out
}

fn resolve_into(
    text: &str,
    ctx: &mut TemplateContext<'_>,
    acc: &mut Vec<ResolvedPlaceholder>,
) -> String {
    let resolved = resolve(text, ctx);
    acc.extend(resolved.placeholders);
    resolved.text
}

fn resolve_rows(
    rows: &[KeyValueItem],
    ctx: &mut TemplateContext<'_>,
    acc: &mut Vec<ResolvedPlaceholder>,
) -> Vec<KeyValueItem> {
    rows.iter()
        .map(|row| KeyValueItem {
            key: resolve_into(&row.key, ctx, acc),
            value: resolve_into(&row.value, ctx, acc),
            enabled: row.enabled,
            // 备注是给人看的，不参与请求 —— 解析它只会让「未命中」列表变噪
            desc: row.desc.clone(),
            // 文件路径参与解析（`{{env.dir}}/a.xlsx`），读盘发生在发送前一刻
            file: row.file.as_deref().map(|path| resolve_into(path, ctx, acc)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::{BodyType, RequestSettings};
    use chrono::TimeZone;
    use std::path::PathBuf;

    fn clock() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 9, 11, 16, 46, 17)
            .single()
            .unwrap()
    }

    fn env() -> HashMap<String, String> {
        HashMap::from([
            ("baseUrl".to_string(), "https://api.example.com".to_string()),
            // 值里故意带一个占位符：验证「不做二次展开」
            ("tricky".to_string(), "{{env.baseUrl}}/x".to_string()),
        ])
    }

    fn tokens() -> TokenValues {
        TokenValues {
            access_token: "t-abc".into(),
            refresh_token: Some("r-def".into()),
            expires_at: Some("2026-09-11T18:00:00.000Z".into()),
        }
    }

    fn response() -> Value {
        serde_json::json!({
            "data": { "id": 7, "token": "from-resp", "ids": [1, 2] },
            "nothing": null
        })
    }

    /// 一个「什么都齐了」的上下文。要测缺失场景时再按需置 None。
    fn with_ctx<R>(f: impl FnOnce(&mut TemplateContext<'_>) -> R) -> R {
        let env = env();
        let tokens = tokens();
        let resp = response();
        let mut ctx = TemplateContext {
            env: &env,
            token: Some(&tokens),
            resp: Some(&resp),
            now: clock(),
            entropy: Entropy::seeded(1),
        };
        f(&mut ctx)
    }

    fn spec() -> RequestSpec {
        RequestSpec {
            method: "POST".into(),
            url: "{{env.baseUrl}}/orders".into(),
            params: vec![KeyValueItem::new("page", "1")],
            headers: vec![KeyValueItem::new(
                "Authorization",
                "Bearer {{token.access}}",
            )],
            body_type: BodyType::Json,
            body: r#"{"ts": {{$timestamp}}}"#.into(),
            settings: RequestSettings::default(),
            request_id: Some(3),
            form_fields: Vec::new(),
        }
    }

    // ============================================================
    // scan
    // ============================================================

    #[test]
    fn scan_finds_every_namespace() {
        let text = "{{env.baseUrl}}|{{token.access}}|{{resp.$.data.id}}|{{$uuid}}|{{prev.$.a}}|{{folder.x.y}}|{{oops}}";
        let hits = scan(text);
        let namespaces: Vec<&str> = hits.iter().map(|h| h.namespace.as_str()).collect();
        assert_eq!(
            namespaces,
            vec!["env", "token", "resp", "func", "prev", "folder", "unknown"]
        );
        assert_eq!(hits[0].key, "baseUrl");
        assert_eq!(hits[0].raw, "{{env.baseUrl}}");
        assert_eq!(hits[2].key, "$.data.id", "resp 之后整段都是路径");
        assert_eq!(hits[3].key, "$uuid");
        assert_eq!(hits[6].key, "oops");
    }

    #[test]
    fn scan_reports_byte_spans_that_slice_the_original() {
        let text = "a {{env.k}} b";
        let hits = scan(text);
        assert_eq!(hits.len(), 1);
        assert_eq!(&text[hits[0].span.clone()], "{{env.k}}");
    }

    /// 没闭合的 `{{` 不该被当成占位符 —— 用户可能正在输入
    #[test]
    fn scanner_ignores_unclosed_braces() {
        assert!(scan("a {{env.k b").is_empty());
        assert!(scan("a {env.k} b").is_empty());
        assert!(scan("{{}}").len() == 1);
    }

    #[test]
    fn scan_ignores_whitespace_inside_braces() {
        let hits = scan("{{ env.baseUrl }}");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].namespace, Namespace::Env);
        assert_eq!(hits[0].key, "baseUrl");
        // raw 保留原样，所以未命中时回填的也是原样
        assert_eq!(hits[0].raw, "{{ env.baseUrl }}");
    }

    // ============================================================
    // resolve：命中
    // ============================================================

    #[test]
    fn resolves_env_token_resp_and_functions_in_one_pass() {
        with_ctx(|ctx| {
            let out = resolve(
                "{{env.baseUrl}}/u?t={{token.access}}&id={{resp.$.data.id}}&n={{$randomInt}}",
                ctx,
            );
            assert!(out
                .text
                .starts_with("https://api.example.com/u?t=t-abc&id=7&n="));
            assert!(out.unresolved_count() == 0);
            assert!(out.placeholders.iter().all(|p| p.value.is_some()));
        });
    }

    #[test]
    fn token_namespace_supports_all_three_keys() {
        with_ctx(|ctx| {
            assert_eq!(resolve("{{token.access}}", ctx).text, "t-abc");
            assert_eq!(resolve("{{token.refresh}}", ctx).text, "r-def");
            assert_eq!(
                resolve("{{token.expiresAt}}", ctx).text,
                "2026-09-11T18:00:00.000Z"
            );
        });
    }

    #[test]
    fn resp_scalars_are_inlined_and_collections_stay_json() {
        with_ctx(|ctx| {
            // 字符串去掉引号
            assert_eq!(resolve("{{resp.$.data.token}}", ctx).text, "from-resp");
            assert_eq!(resolve("{{resp.$.data.id}}", ctx).text, "7");
            // 数组保持紧凑 JSON，这样 ?ids={{resp.$.data.ids}} 能直接用
            assert_eq!(resolve("{{resp.$.data.ids}}", ctx).text, "[1,2]");
        });
    }

    /// 关键语义：解析出来的值里再出现 `{{ }}` 不二次展开
    #[test]
    fn resolution_is_not_recursive() {
        with_ctx(|ctx| {
            let out = resolve("{{env.tricky}}", ctx);
            assert_eq!(out.text, "{{env.baseUrl}}/x");
            assert_eq!(out.placeholders.len(), 1, "只应记录最外层那一个");
        });
    }

    #[test]
    fn same_timestamp_is_reused_within_one_request() {
        with_ctx(|ctx| {
            let out = resolve("{{$timestamp}}/{{$timestamp}}", ctx);
            let (a, b) = out.text.split_once('/').unwrap();
            assert_eq!(a, b, "同一份请求里两处 $timestamp 必须一致");
        });
    }

    // ============================================================
    // resolve：未命中（验收第 4 条：绝不静默变空串）
    // ============================================================

    #[test]
    fn missing_env_var_is_kept_verbatim_with_reason() {
        with_ctx(|ctx| {
            let out = resolve("{{env.nope}}", ctx);
            assert_eq!(out.text, "{{env.nope}}", "未命中必须原样保留");
            assert_eq!(
                out.placeholders[0].reason,
                Some(UnresolvedReason::EnvNotFound)
            );
        });
    }

    #[test]
    fn unknown_namespace_and_bad_function_are_distinguished() {
        with_ctx(|ctx| {
            let out = resolve("{{foo.bar}} {{$datetim}}", ctx);
            assert_eq!(out.text, "{{foo.bar}} {{$datetim}}");
            assert_eq!(
                out.placeholders[0].reason,
                Some(UnresolvedReason::UnknownNamespace)
            );
            assert_eq!(
                out.placeholders[1].reason,
                Some(UnresolvedReason::BadFunction)
            );
        });
    }

    #[test]
    fn missing_token_source_and_missing_token_key_are_reported() {
        let env = env();
        let mut ctx = TemplateContext {
            env: &env,
            token: None,
            resp: None,
            now: clock(),
            entropy: Entropy::seeded(1),
        };
        assert_eq!(
            resolve("{{token.access}}", &mut ctx).placeholders[0].reason,
            Some(UnresolvedReason::NoTokenSource)
        );

        with_ctx(|ctx| {
            assert_eq!(
                resolve("{{token.nope}}", ctx).placeholders[0].reason,
                Some(UnresolvedReason::TokenUnavailable)
            );
        });
    }

    #[test]
    fn response_paths_report_no_response_vs_miss() {
        let env = env();
        let tokens = tokens();
        let mut ctx = TemplateContext {
            env: &env,
            token: Some(&tokens),
            resp: None,
            now: clock(),
            entropy: Entropy::seeded(1),
        };
        assert_eq!(
            resolve("{{resp.$.a}}", &mut ctx).placeholders[0].reason,
            Some(UnresolvedReason::NoResponse)
        );

        with_ctx(|ctx| {
            let out = resolve("{{resp.$.data.nope}}", ctx);
            assert_eq!(out.text, "{{resp.$.data.nope}}");
            assert_eq!(
                out.placeholders[0].reason,
                Some(UnresolvedReason::RespPathMiss)
            );

            // 取到 null 也当没值
            let nul = resolve("{{resp.$.nothing}}", ctx);
            assert_eq!(nul.text, "{{resp.$.nothing}}");
        });
    }

    #[test]
    fn prev_and_folder_namespaces_are_explicitly_unsupported() {
        with_ctx(|ctx| {
            let out = resolve("{{prev.$.a}} {{folder.x.y}}", ctx);
            assert_eq!(out.text, "{{prev.$.a}} {{folder.x.y}}");
            assert_eq!(
                out.placeholders[0].reason,
                Some(UnresolvedReason::NotSupported)
            );
            assert_eq!(
                out.placeholders[1].reason,
                Some(UnresolvedReason::NotSupported)
            );
        });
    }

    #[test]
    fn mixed_hits_and_misses_keep_positions() {
        with_ctx(|ctx| {
            let out = resolve("{{env.baseUrl}}/{{env.nope}}/{{token.access}}", ctx);
            assert_eq!(
                out.text, "https://api.example.com/{{env.nope}}/t-abc",
                "命中与未命中混在一起时，顺序与位置都不能错"
            );
        });
    }

    // ============================================================
    // resolve_spec
    // ============================================================

    #[test]
    fn resolve_spec_touches_every_request_field() {
        // 期望值由注入的时钟推出来 —— 这正是「时钟注入」换到的东西：
        // 断言能落到具体秒数上，而不只是「是个数字」
        let expected_body = "{\"ts\": ".to_string() + &clock().timestamp().to_string() + "}";

        with_ctx(|ctx| {
            let out = resolve_spec(&spec(), ctx);
            assert_eq!(out.spec.url, "https://api.example.com/orders");
            assert_eq!(out.spec.headers[0].value, "Bearer t-abc");
            assert_eq!(out.spec.body, expected_body);
            assert_eq!(out.spec.params[0].value, "1", "没占位符的字段原样保留");
            assert_eq!(out.spec.request_id, Some(3), "上下文要跟着走");
            assert!(out.unresolved.is_empty());
        });
    }

    #[test]
    fn resolve_spec_dedupes_repeated_unresolved_placeholders() {
        let mut s = spec();
        s.url = "{{env.nope}}/a".into();
        s.headers = vec![KeyValueItem::new("X-A", "{{env.nope}}")];
        s.body = "{{env.nope}}".into();
        s.params = Vec::new();

        with_ctx(|ctx| {
            let out = resolve_spec(&s, ctx);
            assert_eq!(out.unresolved.len(), 1, "同一处占位符只报一次");
            assert_eq!(out.unresolved[0].raw, "{{env.nope}}");
            assert_eq!(out.unresolved[0].namespace, "env");
            assert_eq!(out.unresolved[0].reason, "envNotFound");
        });
    }

    /// 扫描与解析必须覆盖**同一批**字段，否则单飞锁会漏判。
    ///
    /// 007 迁移之后「凭据」不再是独立字段，而是 `headers` 里的一行 ——
    /// 这条断言就是守那个挪动的：一条只靠 `Authorization: Bearer {{token.access}}`
    /// 带 token 的请求，必须被认成「要用 token」。
    #[test]
    fn scan_spec_covers_the_same_fields_as_resolve_spec() {
        let mut s = spec();
        s.headers = vec![KeyValueItem::new(
            "Authorization",
            "Bearer {{token.access}}",
        )];
        s.body = String::new();

        let hits = scan_spec(&s);
        let env_hits = hits
            .iter()
            .filter(|h| h.namespace == Namespace::Env)
            .count();
        let token_hits = hits
            .iter()
            .filter(|h| h.namespace == Namespace::Token)
            .count();
        assert_eq!(env_hits, 1, "url 里的 env 要扫到");
        assert_eq!(token_hits, 1, "请求头里的 token 要扫到");
    }

    /// 凭据头走的就是普通请求头那条解析路径 —— 没有第二条通道，
    /// 所以这里断言的是「`Authorization` 的模板值被换成了真 token」，
    /// 而不是某个 `auth` 字段发生了变化。
    #[test]
    fn resolve_spec_expands_a_credential_header() {
        let mut s = spec();
        s.headers = vec![
            KeyValueItem::new("Authorization", "Bearer {{token.access}}"),
            KeyValueItem::new("X-Access-Token", "{{token.access}}"),
        ];

        with_ctx(|ctx| {
            let out = resolve_spec(&s, ctx);
            let by_key = |k: &str| {
                out.spec
                    .headers
                    .iter()
                    .find(|h| h.key == k)
                    .unwrap_or_else(|| panic!("{k} 必须还在"))
                    .value
                    .clone()
            };
            assert_eq!(by_key("Authorization"), "Bearer t-abc");
            assert_eq!(by_key("X-Access-Token"), "t-abc");
            assert!(out.unresolved.is_empty(), "两道头都能解析，不该有未解析项");
        });
    }

    /// 「这次要不要用 token」在两个地方各扫一遍：这里的 `scan_spec`，
    /// 与前端 `RequestPane.tsx` 的 `draftWantsToken`。
    ///
    /// ⚠️ 漏改任意一处都**不是**编译错误。表现是：请求里写着的
    /// `{{token.access}}` 不再触发刷新、发送键旁的 Token 状态条也不出现 ——
    /// 也就是「发出去还是旧 token」，而界面上没有任何提示。
    /// 007 之前最要命的那一处是 `auth`（它曾是 `{{token.access}}` 最常见的落点），
    /// 实测当时有 29 条请求靠那次扫描才会去刷 token。
    ///
    /// 所以直接把两份字段清单对一遍。用 `CARGO_MANIFEST_DIR` 定位而不是相对
    /// 路径：测试的工作目录不保证是 crate 根（前端那份的写法见
    /// `src/external_changes.rs` 的事件名测试）。
    #[test]
    fn the_frontend_token_scan_covers_the_same_fields() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../src/features/request/RequestPane.tsx");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("读不到 {}: {e}", path.display()));

        let body = extract_function(&source, "function draftWantsToken")
            .unwrap_or_else(|| panic!("{} 里找不到 draftWantsToken", path.display()));

        // 每一项都对应 `spec_text_fields` 里的一处。`bodyRows` 前端多扫了一个
        // —— urlencoded 的行最终会拼进 `body`，而 `body` 本来就被扫，
        // 所以多扫是冗余而**无害**的；漏扫才是有害的。
        for field in [
            "draft.url",
            "draft.body",
            "draft.params",
            "draft.headers",
            "draft.formFields",
        ] {
            assert!(
                body.contains(field),
                "前端的 draftWantsToken 里没有 {field} —— 它与 Rust 的 scan_spec \
                 已经不一致，这类请求不会再触发 Token 刷新，而界面上看不出来"
            );
        }

        assert!(
            !body.contains("draft.auth"),
            "draftWantsToken 里还在扫 `draft.auth` —— 007 已经删掉了那个字段，\
             这里读到的是 undefined，等于白扫一遍还占着一行"
        );
    }

    /// 从源码里截出一个函数体（从签名那一行到第一个顶格的 `}`）。
    ///
    /// 够用就行的实现：被测函数的内部没有顶格的 `}` —— JS 的箭头函数与
    /// 对象字面量都带缩进。真要有那么一天，断言的报错会直接指出来。
    fn extract_function<'a>(source: &'a str, signature: &str) -> Option<&'a str> {
        let from = source.find(signature)?;
        let rest = &source[from..];
        let end = rest.find("\n}")?;
        Some(&rest[..end])
    }
}
