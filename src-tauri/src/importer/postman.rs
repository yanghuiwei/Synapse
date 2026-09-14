//! Postman Collection v2.1 与 apipost。
//!
//! ## 为什么两个格式挤在一个文件里
//!
//! design.md §5.2 写得很清楚：**apipost 的主路径就是「导出成 Postman v2.1」**，
//! 原生格式只是副路径。两者的树、请求、变量、脚本概念一一对应，
//! 拆成两个文件会得到两份几乎相同的递归 —— 然后两边各自长歪。
//! 所以这里是一个文件、两套「字段名映射」，共用一个树遍历器。
//!
//! ## apipost 原生格式的诚实说明
//!
//! 原生格式的字段名取自 design.md §5.2 与 apipost 常见导出的结构，
//! **没有拿到真实导出文件逐字段对过**。因此这里一律用
//! 「多个候选字段名 + 认不出就记警告」的宽松策略：
//! 宁可少导一个字段并说出来，也不要按猜的名字导进来一个错的值。
//! （主路径 Postman v2.1 是有真实形状可依的，那一侧不宽松。）

use serde_json::{json, Value};

use super::Format;
use super::{
    EnvDraft, EnvVarDraft, FolderNode, ImportBundle, ImportWarning, RequestDraft, WarningKind,
};
use crate::error::{codes, AppError, AppResult};
use crate::storage::models::{ApiKeyLocation, AuthConfig, KeyValueItem};

pub fn parse(text: &str, format: Format) -> AppResult<ImportBundle> {
    let root: Value = serde_json::from_str(text).map_err(|err| {
        AppError::http_with(
            codes::IMPORT_JSON,
            format!("invalid json: {err}"),
            json!({ "line": err.line(), "column": err.column() }),
        )
    })?;

    let mut warnings = Vec::new();
    let mut bundle = match format {
        Format::Apipost => parse_apipost(&root, &mut warnings),
        _ => parse_postman(&root, &mut warnings),
    };
    bundle.warnings = warnings;
    if bundle.request_count() == 0 && bundle.warnings.is_empty() {
        // 结构认出来了但一条接口都没有：不是错误（可能真是个空集合），
        // 但必须说出来，否则用户看到的是「导入成功、什么都没发生」
        bundle.push_warning(ImportWarning::simple(
            WarningKind::SkippedItem,
            "collection contains no request".to_string(),
        ));
    }
    super::dedupe_names(&mut bundle);
    Ok(bundle)
}

// ============================================================
// Postman Collection v2.1
// ============================================================

fn parse_postman(root: &Value, warnings: &mut Vec<ImportWarning>) -> ImportBundle {
    // 集合名同时也是环境名。没写就留空 —— 占位文案属于界面文案，
    // 由前端在落库前补（后端只生产数据，不生产给人看的句子）。
    let collection_name = root
        .pointer("/info/name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    let mut bundle = ImportBundle::default();
    let mut vars = Vec::new();
    collect_postman_vars(root, &mut vars);
    if let Some(events) = root.get("event").and_then(Value::as_array) {
        if !events.is_empty() {
            warnings.push(ImportWarning::simple(
                WarningKind::UnsupportedField,
                format!(
                    "collection-level script ({} event) cannot be attached to a single request",
                    events.len()
                ),
            ));
        }
    }

    // 集合级授权：reqly 的「根」没有 auth 这一列，所以要么物化到顶层文件夹、
    // 要么 inline 进根级请求（见 walk 里的 inside_folder 分支）
    let collection_auth = postman_auth(root.get("auth"), warnings);

    if let Some(items) = root.get("item").and_then(Value::as_array) {
        for item in items {
            push_walked(
                &mut bundle,
                walk_postman(item, &mut vars, &collection_auth, false, warnings),
            );
        }
    }

    if !vars.is_empty() {
        bundle.environments.push(EnvDraft {
            name: collection_name,
            vars,
        });
    }
    bundle
}

enum Walked {
    Folder(FolderNode),
    Request(RequestDraft),
    Skipped,
}

fn push_walked(bundle: &mut ImportBundle, walked: Walked) {
    match walked {
        Walked::Folder(node) => bundle.folders.push(node),
        Walked::Request(draft) => bundle.requests.push(draft),
        Walked::Skipped => {}
    }
}

/// `inside_folder` 决定「没有显式 auth 的请求」怎么处理：
/// 在文件夹里 → 留 `Inherit`（由文件夹链解析）；在根上 → 把集合级授权 inline 进来。
fn walk_postman(
    item: &Value,
    vars: &mut Vec<EnvVarDraft>,
    inherited: &Option<AuthConfig>,
    inside_folder: bool,
    warnings: &mut Vec<ImportWarning>,
) -> Walked {
    // ---- 文件夹 ----
    if let Some(children) = item.get("item").and_then(Value::as_array) {
        let name = str_at(item, "name").unwrap_or_default();
        let mut node = FolderNode::new(name);
        collect_postman_vars(item, vars);
        // 文件夹自己没配就沿用上层 —— 物化下来，让继承链在落库后依然成立
        node.auth = postman_auth(item.get("auth"), warnings).or_else(|| inherited.clone());
        let child_inherited = node.auth.clone();
        for child in children {
            match walk_postman(child, vars, &child_inherited, true, warnings) {
                Walked::Folder(child_node) => node.folders.push(child_node),
                Walked::Request(draft) => node.requests.push(draft),
                Walked::Skipped => {}
            }
        }
        return Walked::Folder(node);
    }

    // ---- 请求 ----
    let Some(request) = item.get("request") else {
        warnings.push(ImportWarning::new(
            WarningKind::SkippedItem,
            str_at(item, "name"),
            "item has neither `item` nor `request`".to_string(),
        ));
        return Walked::Skipped;
    };

    // v1 风格的 `"request": "https://x/y"`：只是个 URL
    let request = if let Some(url) = request.as_str() {
        json!({ "method": "GET", "url": url })
    } else {
        request.clone()
    };

    let (url, params) = postman_url(request.get("url"));
    let (body_type, body) = postman_body(request.get("body"), warnings);
    let (pre_script, test_script) = postman_scripts(item);
    let auth = postman_auth(request.get("auth"), warnings).unwrap_or_else(|| {
        if inside_folder {
            AuthConfig::Inherit
        } else {
            inherited.clone().unwrap_or(AuthConfig::Inherit)
        }
    });

    let mut draft = RequestDraft {
        name: str_at(item, "name").unwrap_or_default(),
        method: request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_ascii_uppercase(),
        url,
        params,
        headers: postman_headers(request.get("header"), warnings),
        body_type,
        body,
        auth,
        settings: Default::default(),
        pre_script,
        test_script,
    };
    finalize_draft(&mut draft, warnings);
    Walked::Request(draft)
}

/// Postman 的 `url` 有两种形状：字符串、或结构化的对象。
///
/// 结构化那种才拆 `query[]` → `params[]`（design.md §5.2 的映射）；
/// **字符串形状的 query 原样留在 URL 里** —— 拆了再拼回去要保证编码逐字节一致，
/// 而 `raw` 里可能已经有 `%2F`、`+`、重复 key 这些一碰就变的东西。
fn postman_url(value: Option<&Value>) -> (String, Vec<KeyValueItem>) {
    match value {
        Some(Value::String(raw)) => (raw.clone(), Vec::new()),
        Some(Value::Object(_)) => {
            let raw = match value.and_then(|v| v.get("raw")).and_then(Value::as_str) {
                Some(raw) => raw.to_string(),
                None => build_url_from_parts(value.expect("上面已判断是对象")),
            };
            // 只有真给了 `query[]` 才从 raw 里切掉 query，否则会把它整段丢掉
            let has_query_array = value
                .and_then(|v| v.get("query"))
                .and_then(Value::as_array)
                .is_some();
            let url = if has_query_array {
                let without_anchor = raw.split('#').next().unwrap_or(&raw);
                without_anchor
                    .split_once('?')
                    .map(|(base, _)| base.to_string())
                    .unwrap_or_else(|| without_anchor.to_string())
            } else {
                raw
            };
            let params = value
                .and_then(|v| v.get("query"))
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(postman_kv).collect())
                .unwrap_or_default();
            (url, params)
        }
        _ => (String::new(), Vec::new()),
    }
}

fn build_url_from_parts(url: &Value) -> String {
    let protocol = url
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or("https");
    let host = url.get("host").map(join_segments).unwrap_or_default();
    let path = url.get("path").map(join_segments).unwrap_or_default();
    if host.is_empty() {
        return String::new();
    }
    if path.is_empty() {
        format!("{protocol}://{host}")
    } else {
        format!("{protocol}://{host}/{path}")
    }
}

/// Postman 的 host / path 既可以是字符串，也可以是分段数组
fn join_segments(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(value_to_string)
            .collect::<Vec<_>>()
            .join("/"),
        other => value_to_string(other),
    }
}

fn postman_headers(value: Option<&Value>, warnings: &mut Vec<ImportWarning>) -> Vec<KeyValueItem> {
    match value {
        Some(Value::Array(items)) => items.iter().filter_map(postman_kv).collect(),
        // v1 的 header 是一整段文本
        Some(Value::String(text)) => {
            warnings.push(ImportWarning::simple(
                WarningKind::UnsupportedField,
                "headers were given as raw text; parsed line by line".to_string(),
            ));
            text.lines().filter_map(parse_header_line).collect()
        }
        _ => Vec::new(),
    }
}

fn parse_header_line(line: &str) -> Option<KeyValueItem> {
    let (key, value) = line.split_once(':')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    Some(KeyValueItem::new(key, value.trim_start()))
}

/// 一个 Postman 的 key/value 条目。`disabled: true` → 保留但 `enabled: false`
/// （保留是有意的：禁用状态也是用户配置的一部分，丢掉他就要重新勾一遍）
fn postman_kv(entry: &Value) -> Option<KeyValueItem> {
    let key = entry.get("key").and_then(Value::as_str)?.trim().to_string();
    if key.is_empty() {
        return None;
    }
    let value = entry.get("value").map(value_to_string).unwrap_or_default();
    let enabled = !entry
        .get("disabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let desc = entry
        .get("description")
        .and_then(|d| match d {
            // 有的版本 description 是 `{content, type}` 对象
            Value::String(s) => Some(s.clone()),
            Value::Object(map) => map
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string),
            _ => None,
        })
        .filter(|s| !s.is_empty());

    let mut item = KeyValueItem::new(key, value);
    item.enabled = enabled;
    item.desc = desc;
    Some(item)
}

/// 返回 `(bodyType, body)`
fn postman_body(value: Option<&Value>, warnings: &mut Vec<ImportWarning>) -> (String, String) {
    let Some(body) = value else {
        return ("none".to_string(), String::new());
    };
    let mode = body.get("mode").and_then(Value::as_str).unwrap_or("none");

    match mode {
        "raw" => {
            let raw = body.get("raw").map(value_to_string).unwrap_or_default();
            if raw.trim().is_empty() {
                return ("none".to_string(), String::new());
            }
            let language = body
                .pointer("/options/raw/language")
                .and_then(Value::as_str)
                .unwrap_or("");
            let body_type = match language {
                "json" => "json",
                // xml / html / text 都当纯文本发（我们的 `raw` 语义就是「原样发」）
                "xml" | "html" | "text" | "javascript" => "raw",
                _ => {
                    if serde_json::from_str::<Value>(&raw).is_ok() {
                        "json"
                    } else {
                        "raw"
                    }
                }
            };
            (body_type.to_string(), raw)
        }
        "urlencoded" => {
            let pairs: Vec<String> = body
                .get("urlencoded")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(postman_kv)
                        .filter(|item| item.enabled)
                        .map(|item| format!("{}={}", item.key, urlencode(&item.value)))
                        .collect()
                })
                .unwrap_or_default();
            ("urlencoded".to_string(), pairs.join("&"))
        }
        "formdata" => {
            let lines: Vec<String> = body
                .get("formdata")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter(|entry| {
                            !entry
                                .get("disabled")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                        })
                        .filter_map(|entry| {
                            let key = entry.get("key").and_then(Value::as_str)?;
                            let is_file = entry.get("type").and_then(Value::as_str) == Some("file");
                            let value = if is_file {
                                entry.get("src").map(value_to_string).unwrap_or_default()
                            } else {
                                entry.get("value").map(value_to_string).unwrap_or_default()
                            };
                            Some(format!("{key}={}{value}", if is_file { "@" } else { "" }))
                        })
                        .collect()
                })
                .unwrap_or_default();
            warnings.push(ImportWarning::simple(
                WarningKind::UnsupportedField,
                "multipart body is kept as `key=value` lines but cannot be sent yet".to_string(),
            ));
            ("form".to_string(), lines.join("\n"))
        }
        "graphql" => {
            let query = body
                .pointer("/graphql/query")
                .and_then(Value::as_str)
                .unwrap_or("");
            let variables = body
                .pointer("/graphql/variables")
                .and_then(Value::as_str)
                .unwrap_or("");
            // 我们这条链路上 GraphQL 在**线上**就是
            // `{"query":…,"variables":…}` + `application/json`，
            // 而 UI 的请求体类型里没有 graphql 这个入口 ——
            // 落成 `json` 线上字节完全一致，而且用户能直接编辑
            let flattened = json!({
                "query": query,
                "variables": serde_json::from_str::<Value>(variables).unwrap_or(Value::Null),
            })
            .to_string();
            warnings.push(ImportWarning::simple(
                WarningKind::UnsupportedField,
                "graphql body was flattened into its json wire form".to_string(),
            ));
            ("json".to_string(), flattened)
        }
        "file" => {
            let src = body
                .pointer("/file/src")
                .map(value_to_string)
                .unwrap_or_default();
            warnings.push(ImportWarning::simple(
                WarningKind::UnsupportedField,
                "binary file body cannot be sent yet".to_string(),
            ));
            ("binary".to_string(), src)
        }
        other => {
            warnings.push(ImportWarning::simple(
                WarningKind::UnsupportedField,
                format!("unknown body mode `{other}`"),
            ));
            ("none".to_string(), String::new())
        }
    }
}

/// Postman 的 `event[]` → `pre_script` / `test_script`
fn postman_scripts(item: &Value) -> (String, String) {
    let mut pre = String::new();
    let mut test = String::new();
    let Some(events) = item.get("event").and_then(Value::as_array) else {
        return (pre, test);
    };
    for event in events {
        let listen = event.get("listen").and_then(Value::as_str).unwrap_or("");
        let exec = event
            .pointer("/script/exec")
            .map(|value| match value {
                Value::Array(lines) => lines
                    .iter()
                    .map(value_to_string)
                    .collect::<Vec<_>>()
                    .join("\n"),
                other => value_to_string(other),
            })
            .unwrap_or_default();
        if exec.is_empty() {
            continue;
        }
        match listen {
            "prerequest" => append_script(&mut pre, &exec),
            "test" => append_script(&mut test, &exec),
            _ => {}
        }
    }
    (pre, test)
}

fn append_script(target: &mut String, chunk: &str) {
    if !target.is_empty() {
        target.push('\n');
    }
    target.push_str(chunk);
}

/// collection / folder 级的 `variable[]` → 环境变量
fn collect_postman_vars(node: &Value, out: &mut Vec<EnvVarDraft>) {
    let Some(items) = node.get("variable").and_then(Value::as_array) else {
        return;
    };
    for entry in items {
        let key = entry
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if key.is_empty() || out.iter().any(|v| v.key == key) {
            // 外层同名变量先出现就先赢（与 Postman 的作用域顺序一致）
            continue;
        }
        out.push(EnvVarDraft {
            key,
            value: entry.get("value").map(value_to_string).unwrap_or_default(),
            is_secret: entry.get("type").and_then(Value::as_str) == Some("secret"),
        });
    }
}

// ============================================================
// 授权
// ============================================================

/// Postman 的 auth 与我们 `AuthConfig` 的差异只在**形状**：
/// Postman 是 `{"type":"bearer","bearer":[{"key":"token","value":"x"}]}`，
/// 我们是 `{"type":"bearer","token":"x"}`。所以必须翻译一遍，
/// 不能指望 `from_json` 直接吃下去（吃不下会退化成 Inherit = 授权静默丢失）。
fn postman_auth(value: Option<&Value>, warnings: &mut Vec<ImportWarning>) -> Option<AuthConfig> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    // 有些工具会直接写成我们的形状
    let direct = AuthConfig::from_json(value);
    if direct != AuthConfig::Inherit {
        return Some(direct);
    }

    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "noauth" => Some(AuthConfig::None),
        "bearer" => Some(AuthConfig::Bearer {
            token: auth_param(value, "bearer", "token").unwrap_or_default(),
        }),
        "basic" => Some(AuthConfig::Basic {
            username: auth_param(value, "basic", "username").unwrap_or_default(),
            password: auth_param(value, "basic", "password").unwrap_or_default(),
        }),
        "apikey" => Some(AuthConfig::ApiKey {
            key: auth_param(value, "apikey", "key").unwrap_or_default(),
            value: auth_param(value, "apikey", "value").unwrap_or_default(),
            location: match auth_param(value, "apikey", "in").as_deref() {
                Some("query") => ApiKeyLocation::Query,
                _ => ApiKeyLocation::Header,
            },
        }),
        "" | "inherit" => None,
        other => {
            // digest / oauth2 / hawk / aws… 我们没有对应实现。
            // 退化成「不注入」并说出来 —— 静默丢掉授权会表现为 401，最难查
            warnings.push(ImportWarning::simple(
                WarningKind::UnsupportedField,
                format!("auth type `{other}` is not supported; dropped"),
            ));
            None
        }
    }
}

/// Postman 的 auth 参数有两种形状：对象 `{"token":"x"}` 或数组 `[{"key":"token","value":"x"}]`
fn auth_param(value: &Value, group: &str, key: &str) -> Option<String> {
    let node = value.get(group)?;
    if let Some(map) = node.as_object() {
        return map.get(key).map(value_to_string);
    }
    for entry in node.as_array()? {
        if entry.get("key").and_then(Value::as_str) == Some(key) {
            return entry.get("value").map(value_to_string);
        }
    }
    None
}

// ============================================================
// apipost 原生格式（宽松）
// ============================================================

fn parse_apipost(root: &Value, warnings: &mut Vec<ImportWarning>) -> ImportBundle {
    let mut bundle = ImportBundle::default();
    let collection_name = str_any(root, &["name", "title", "target_name"]).unwrap_or_default();

    let mut vars = Vec::new();
    collect_apipost_vars(root, &mut vars);

    match first_array(root, &["children", "item", "apis", "api_list"]) {
        Some(items) => {
            for item in items {
                match walk_apipost(item, &mut vars, warnings) {
                    Walked::Folder(node) => bundle.folders.push(node),
                    Walked::Request(draft) => bundle.requests.push(draft),
                    Walked::Skipped => {}
                }
            }
        }
        None => {
            // 没有子节点，看看本身是不是一条接口
            if let Some(draft) = apipost_request(root, warnings) {
                bundle.requests.push(draft);
            }
        }
    }

    if !vars.is_empty() {
        bundle.environments.push(EnvDraft {
            name: collection_name,
            vars,
        });
    }
    bundle
}

fn walk_apipost(
    item: &Value,
    vars: &mut Vec<EnvVarDraft>,
    warnings: &mut Vec<ImportWarning>,
) -> Walked {
    if let Some(children) = first_array(item, &["children", "item", "apis", "api_list"]) {
        let name = str_any(item, &["name", "target_name", "title"]).unwrap_or_default();
        let mut node = FolderNode::new(name);
        collect_apipost_vars(item, vars);
        for child in children {
            match walk_apipost(child, vars, warnings) {
                Walked::Folder(child_node) => node.folders.push(child_node),
                Walked::Request(draft) => node.requests.push(draft),
                Walked::Skipped => {}
            }
        }
        return Walked::Folder(node);
    }
    match apipost_request(item, warnings) {
        Some(draft) => Walked::Request(draft),
        None => Walked::Skipped,
    }
}

fn apipost_request(item: &Value, warnings: &mut Vec<ImportWarning>) -> Option<RequestDraft> {
    let url = str_any(item, &["url", "uri", "path"]).unwrap_or_default();
    let method = str_any(item, &["method"]).unwrap_or_else(|| "GET".into());
    // 既没有 url 也没有 method → 不是接口（可能是别的节点类型）
    if url.is_empty() && item.get("method").is_none() {
        return None;
    }
    let (body_type, body) = apipost_body(item.get("body"), warnings);
    let (pre_script, test_script) = apipost_scripts(item);

    let mut draft = RequestDraft {
        name: str_any(item, &["name", "target_name", "title"]).unwrap_or_default(),
        method,
        url,
        params: Vec::new(),
        headers: apipost_headers(item.get("headers").or_else(|| item.get("header"))),
        body_type,
        body,
        auth: apipost_auth(item.get("auth"), warnings).unwrap_or(AuthConfig::Inherit),
        settings: Default::default(),
        pre_script,
        test_script,
    };
    finalize_draft(&mut draft, warnings);
    Some(draft)
}

fn apipost_headers(value: Option<&Value>) -> Vec<KeyValueItem> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|entry| {
                let key = entry.get("key").and_then(Value::as_str)?.trim().to_string();
                if key.is_empty() {
                    return None;
                }
                let value = entry.get("value").map(value_to_string).unwrap_or_default();
                let enabled = entry
                    .get("enable")
                    .or_else(|| entry.get("enabled"))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let mut item = KeyValueItem::new(key, value);
                item.enabled = enabled;
                Some(item)
            })
            .collect(),
        // `{"Accept": "*/*"}` 这种对象映射
        Some(Value::Object(map)) => map
            .iter()
            .filter(|(key, _)| !key.is_empty())
            .map(|(key, value)| KeyValueItem::new(key.clone(), value_to_string(value)))
            .collect(),
        _ => Vec::new(),
    }
}

fn apipost_body(value: Option<&Value>, warnings: &mut Vec<ImportWarning>) -> (String, String) {
    let Some(body) = value else {
        return ("none".to_string(), String::new());
    };
    // 直接就是一段文本
    if let Some(text) = body.as_str() {
        return (guess_body_type(text), text.to_string());
    }

    let mode = str_any(body, &["mode", "type", "body_type"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let raw = str_any(body, &["raw", "content", "text"]).unwrap_or_default();
    let parameters = first_array(body, &["parameter", "parameters", "params"]);

    match mode.as_str() {
        "form-data" | "formdata" | "multipart" => {
            let lines: Vec<String> = parameters
                .map(|items| {
                    items
                        .iter()
                        .filter_map(apipost_kv)
                        .map(|(key, value)| format!("{key}={value}"))
                        .collect()
                })
                .unwrap_or_default();
            warnings.push(ImportWarning::simple(
                WarningKind::UnsupportedField,
                "multipart body is kept as `key=value` lines but cannot be sent yet".to_string(),
            ));
            ("form".to_string(), lines.join("\n"))
        }
        "urlencoded" | "x-www-form-urlencoded" => (
            "urlencoded".to_string(),
            apipost_params_to_form(parameters.map(Vec::as_slice)),
        ),
        "json" => {
            let raw = if raw.is_empty() {
                apipost_params_to_json(parameters.map(Vec::as_slice))
            } else {
                raw
            };
            ("json".to_string(), raw)
        }
        // 没写 mode 但给了参数表 → 按 urlencoded 处理（apipost 界面上的默认）
        "" if parameters.is_some() => (
            "urlencoded".to_string(),
            apipost_params_to_form(parameters.map(Vec::as_slice)),
        ),
        "raw" | "text" | "file" | "" => (guess_body_type(&raw), raw),
        other => {
            warnings.push(ImportWarning::simple(
                WarningKind::UnsupportedField,
                format!("unknown apipost body mode `{other}`; body dropped"),
            ));
            ("none".to_string(), String::new())
        }
    }
}

/// apipost 的参数表在 `key`/`name` 与 `value`/`content` 之间摇摆，统一抽出来。
fn apipost_kv(entry: &Value) -> Option<(String, String)> {
    let key = str_any(entry, &["key", "name"])?;
    let value = str_any(entry, &["value", "content"]).unwrap_or_default();
    Some((key, value))
}

fn apipost_params_to_form(parameters: Option<&[Value]>) -> String {
    let Some(items) = parameters else {
        return String::new();
    };
    items
        .iter()
        .filter_map(apipost_kv)
        .map(|(key, value)| format!("{key}={}", urlencode(&value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn apipost_params_to_json(parameters: Option<&[Value]>) -> String {
    let Some(items) = parameters else {
        return String::new();
    };
    let map: serde_json::Map<String, Value> = items
        .iter()
        .filter_map(apipost_kv)
        .map(|(key, value)| (key, json!(value)))
        .collect();
    Value::Object(map).to_string()
}

fn apipost_auth(value: Option<&Value>, warnings: &mut Vec<ImportWarning>) -> Option<AuthConfig> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    let direct = AuthConfig::from_json(value);
    if direct != AuthConfig::Inherit {
        return Some(direct);
    }
    // apipost 把参数放在 `auth` 下的分组名里，分组名的拼写有几种
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    match kind.as_str() {
        "noauth" | "none" => Some(AuthConfig::None),
        "bearer" | "bearer_token" => Some(AuthConfig::Bearer {
            token: auth_param_any(value, &["bearer", "bearer_token"], &["token", "value"])
                .unwrap_or_default(),
        }),
        "basic" => Some(AuthConfig::Basic {
            username: auth_param_any(value, &["basic"], &["username", "user"]).unwrap_or_default(),
            password: auth_param_any(value, &["basic"], &["password", "pass"]).unwrap_or_default(),
        }),
        "apikey" | "api_key" => Some(AuthConfig::ApiKey {
            key: auth_param_any(value, &["apikey", "api_key"], &["key", "name"])
                .unwrap_or_default(),
            value: auth_param_any(value, &["apikey", "api_key"], &["value"]).unwrap_or_default(),
            location: match auth_param_any(value, &["apikey", "api_key"], &["in", "addTo"])
                .as_deref()
            {
                Some("query") => ApiKeyLocation::Query,
                _ => ApiKeyLocation::Header,
            },
        }),
        "" | "inherit" => None,
        other => {
            warnings.push(ImportWarning::simple(
                WarningKind::UnsupportedField,
                format!("auth type `{other}` is not supported; dropped"),
            ));
            None
        }
    }
}

fn auth_param_any(value: &Value, groups: &[&str], keys: &[&str]) -> Option<String> {
    for group in groups {
        for key in keys {
            if let Some(found) = auth_param(value, group, key) {
                return Some(found);
            }
        }
    }
    None
}

fn apipost_scripts(item: &Value) -> (String, String) {
    let pre = str_any(item, &["pre_script", "prerequest_script", "preScript"]).unwrap_or_default();
    let test = str_any(item, &["test_script", "testScript", "after_script"]).unwrap_or_default();
    (pre, test)
}

fn collect_apipost_vars(node: &Value, out: &mut Vec<EnvVarDraft>) {
    let Some(items) = first_array(node, &["variable", "variables", "environment"]) else {
        return;
    };
    for entry in items {
        let key = str_any(entry, &["key", "name"]).unwrap_or_default();
        let key = key.trim().to_string();
        if key.is_empty() || out.iter().any(|v| v.key == key) {
            continue;
        }
        out.push(EnvVarDraft {
            key,
            value: str_any(entry, &["value", "content"]).unwrap_or_default(),
            is_secret: entry
                .get("is_secret")
                .or_else(|| entry.get("isSecret"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
}

// ============================================================
// 两个格式共用的尾部
// ============================================================

/// 拼完之后的两处兜底：名字缺了按 URL 生成、URL 缺了记警告。
///
/// URL 缺了**不丢** —— 留着让用户在界面上补，比「导入时静默少一条」好查得多。
fn finalize_draft(draft: &mut RequestDraft, warnings: &mut Vec<ImportWarning>) {
    if draft.url.trim().is_empty() {
        warnings.push(ImportWarning::new(
            WarningKind::SkippedItem,
            Some(if draft.name.trim().is_empty() {
                "<unnamed>".to_string()
            } else {
                draft.name.clone()
            }),
            "request has no url".to_string(),
        ));
    }
    if draft.name.trim().is_empty() {
        let generated = generate_name(&draft.url);
        warnings.push(ImportWarning::new(
            WarningKind::GeneratedName,
            Some(generated.clone()),
            "request had no name; generated from the url".to_string(),
        ));
        draft.name = generated;
    } else {
        draft.name = draft.name.trim().to_string();
    }
    draft.method = draft.method.to_ascii_uppercase();
}

fn generate_name(url: &str) -> String {
    let without_anchor = url.split(['#', '?']).next().unwrap_or(url);
    let trimmed = without_anchor.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or("");
    if last.is_empty() || last.contains("{{") {
        // 只有域名、或者路径里全是 `{{var}}` → 生成不出像样的名字，
        // 留空交给前端补占位文案
        trimmed
            .split("//")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .unwrap_or("")
            .to_string()
    } else {
        last.to_string()
    }
}

/// 没有声明 language 时猜 body 类型：能解析成 JSON 就是 json
fn guess_body_type(raw: &str) -> String {
    if serde_json::from_str::<Value>(raw).is_ok() {
        "json".to_string()
    } else {
        "raw".to_string()
    }
}

fn urlencode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

// ---- 小而多的取值助手 ----

fn str_at(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// 依次试几个候选字段名，取第一个非空的
fn str_any(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| str_at(value, key))
        .find(|found| !found.is_empty())
}

fn first_array<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Vec<Value>> {
    keys.iter()
        .filter_map(|key| value.get(*key).and_then(Value::as_array))
        .find(|found| !found.is_empty())
}

/// JSON 里的值 → 字符串。数字/布尔照字面量转，`null` 当空串
fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn postman(text: &str) -> ImportBundle {
        parse(text, Format::Postman).expect("应该能解析")
    }

    fn apipost(text: &str) -> ImportBundle {
        parse(text, Format::Apipost).expect("应该能解析")
    }

    /// 一个贴近真实导出的 Postman v2.1 集合
    const COLLECTION: &str = r#"{
      "info": {
        "_postman_id": "0e1b1d2a-1111-2222-3333-444455556666",
        "name": "内部接口",
        "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
      },
      "item": [
        {
          "name": "用户中心",
          "item": [
            {
              "name": "用户列表",
              "request": {
                "method": "GET",
                "header": [
                  { "key": "accept", "value": "application/json", "type": "text" },
                  { "key": "x-debug", "value": "1", "type": "text", "disabled": true }
                ],
                "url": {
                  "raw": "{{baseUrl}}/v1/users?page=1&size=20",
                  "host": ["{{baseUrl}}"],
                  "path": ["v1", "users"],
                  "query": [
                    { "key": "page", "value": "1" },
                    { "key": "size", "value": "20" },
                    { "key": "debug", "value": "1", "disabled": true }
                  ]
                }
              },
              "response": []
            },
            {
              "name": "创建用户",
              "request": {
                "method": "POST",
                "header": [ { "key": "content-type", "value": "application/json" } ],
                "body": {
                  "mode": "raw",
                  "raw": "{\"name\":\"张三\"}",
                  "options": { "raw": { "language": "json" } }
                },
                "url": { "raw": "{{baseUrl}}/v1/users", "host": ["{{baseUrl}}"], "path": ["v1", "users"] },
                "auth": {
                  "type": "bearer",
                  "bearer": [ { "key": "token", "value": "{{token.access}}" } ]
                },
                "description": "建一个用户"
              },
              "event": [
                { "listen": "prerequest", "script": { "exec": ["const t = Date.now();", "console.log(t);"] } },
                { "listen": "test", "script": { "exec": ["pm.test('ok', () => {});"] } }
              ]
            }
          ]
        },
        {
          "name": "健康检查",
          "request": { "method": "GET", "url": "{{baseUrl}}/health" }
        }
      ],
      "variable": [
        { "key": "baseUrl", "value": "https://api.example.com", "type": "string" },
        { "key": "appSecret", "value": "s3cret", "type": "secret" }
      ]
    }"#;

    #[test]
    fn postman_tree_becomes_folders_and_requests() {
        let bundle = postman(COLLECTION);
        assert_eq!(bundle.folders.len(), 1, "一个文件夹 + 一条根级请求");
        assert_eq!(bundle.requests.len(), 1);
        assert_eq!(bundle.folders[0].name, "用户中心");
        assert_eq!(bundle.folders[0].requests.len(), 2);
        assert_eq!(bundle.request_count(), 3);
    }

    #[test]
    fn postman_query_array_becomes_params() {
        let bundle = postman(COLLECTION);
        let list = &bundle.folders[0].requests[0];
        assert_eq!(list.url, "{{baseUrl}}/v1/users", "query 从 raw 里切掉了");
        assert_eq!(list.params.len(), 3);
        assert_eq!(list.params[0].key, "page");
        assert_eq!(list.params[0].value, "1");
        assert!(!list.params[2].enabled, "disabled 的 query 要保留但置灰");
    }

    #[test]
    fn postman_disabled_header_is_kept_but_disabled() {
        let bundle = postman(COLLECTION);
        let headers = &bundle.folders[0].requests[0].headers;
        assert_eq!(headers.len(), 2);
        assert!(!headers[1].enabled);
    }

    #[test]
    fn postman_raw_json_body_is_typed() {
        let bundle = postman(COLLECTION);
        let create = &bundle.folders[0].requests[1];
        assert_eq!(create.body_type, "json");
        assert_eq!(create.body, r#"{"name":"张三"}"#);
        assert_eq!(create.method, "POST");
    }

    #[test]
    fn postman_bearer_auth_is_translated() {
        let bundle = postman(COLLECTION);
        assert_eq!(
            bundle.folders[0].requests[1].auth,
            AuthConfig::Bearer {
                token: "{{token.access}}".to_string()
            },
            "占位符要原样保留，不能展开"
        );
    }

    #[test]
    fn postman_scripts_land_in_the_script_columns() {
        let bundle = postman(COLLECTION);
        let create = &bundle.folders[0].requests[1];
        assert_eq!(create.pre_script, "const t = Date.now();\nconsole.log(t);");
        assert_eq!(create.test_script, "pm.test('ok', () => {});");
    }

    #[test]
    fn postman_collection_variables_become_an_environment() {
        let bundle = postman(COLLECTION);
        assert_eq!(bundle.environments.len(), 1);
        let env = &bundle.environments[0];
        // 环境名就是集合名，不拼后缀 —— 集合级的变量本来就属于这个集合
        assert_eq!(env.name, "内部接口");
        assert_eq!(env.vars.len(), 2);
        assert_eq!(env.vars[0].key, "baseUrl");
        assert!(!env.vars[0].is_secret);
        assert!(env.vars[1].is_secret, "type=secret 要认出来");
    }

    /// 字符串形式的 url：query 不能丢
    #[test]
    fn postman_string_url_keeps_its_query() {
        let bundle = postman(COLLECTION);
        let health = &bundle.requests[0];
        assert_eq!(health.url, "{{baseUrl}}/health");
        assert!(health.params.is_empty());
    }

    #[test]
    fn postman_string_url_with_query_is_not_cut() {
        let text = r#"{"info":{"name":"c"},"item":[{"name":"a","request":{"url":"https://x/y?a=1&b=2"}}]}"#;
        let bundle = postman(text);
        assert_eq!(bundle.requests[0].url, "https://x/y?a=1&b=2");
        assert!(bundle.requests[0].params.is_empty());
    }

    #[test]
    fn postman_v1_string_request_becomes_get() {
        let text = r#"{"info":{"name":"c"},"item":[{"name":"老格式","request":"https://x/y"}]}"#;
        let bundle = postman(text);
        assert_eq!(bundle.requests[0].method, "GET");
        assert_eq!(bundle.requests[0].url, "https://x/y");
    }

    #[test]
    fn postman_folder_auth_is_materialized_down_the_tree() {
        let text = r#"{
          "info":{"name":"c"},
          "auth":{ "type":"bearer", "bearer":[{"key":"token","value":"collection-token"}] },
          "item":[
            { "name":"子目录", "item":[
                { "name":"a","request":{"url":"https://x/a"} },
                { "name":"b","request":{"url":"https://x/b","auth":{"type":"noauth"}} } ] }
          ]
        }"#;
        let bundle = postman(text);
        let folder = &bundle.folders[0];
        assert_eq!(
            folder.auth,
            Some(AuthConfig::Bearer {
                token: "collection-token".to_string()
            }),
            "集合级授权要物化到顶层文件夹，否则落库后就丢了"
        );
        assert_eq!(folder.requests[0].auth, AuthConfig::Inherit);
        assert_eq!(
            folder.requests[1].auth,
            AuthConfig::None,
            "显式 noauth 不能继承"
        );
    }

    /// 没有文件夹时，集合级授权必须 inline 进根级请求
    #[test]
    fn postman_collection_auth_inlines_into_root_requests() {
        let text = r#"{
          "info":{"name":"c"},
          "auth":{ "type":"basic", "basic":[{"key":"username","value":"u"},{"key":"password","value":"p"}] },
          "item":[ { "name":"a","request":{"url":"https://x/a"} } ]
        }"#;
        let bundle = postman(text);
        assert_eq!(
            bundle.requests[0].auth,
            AuthConfig::Basic {
                username: "u".to_string(),
                password: "p".to_string()
            }
        );
    }

    #[test]
    fn postman_unsupported_auth_warns_instead_of_silently_dropping() {
        let text = r#"{"info":{"name":"c"},"item":[{"name":"a","request":{"url":"https://x/a","auth":{"type":"oauth2"}}}]}"#;
        let bundle = postman(text);
        assert_eq!(bundle.requests[0].auth, AuthConfig::Inherit);
        assert!(bundle
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::UnsupportedField));
    }

    #[test]
    fn postman_urlencoded_body_is_built_from_the_array() {
        let text = r#"{"info":{"name":"c"},"item":[{"name":"a","request":{"method":"POST","url":"https://x/a",
            "body":{"mode":"urlencoded","urlencoded":[
              {"key":"a","value":"1 2"},{"key":"b","value":"中"},{"key":"skip","value":"x","disabled":true}]}}}]}"#;
        let bundle = postman(text);
        let draft = &bundle.requests[0];
        assert_eq!(draft.body_type, "urlencoded");
        assert_eq!(draft.body, "a=1%202&b=%E4%B8%AD");
    }

    #[test]
    fn postman_multipart_warns_and_keeps_lines() {
        let text = r#"{"info":{"name":"c"},"item":[{"name":"a","request":{"method":"POST","url":"https://x/a",
            "body":{"mode":"formdata","formdata":[
              {"key":"note","value":"hi","type":"text"},
              {"key":"file","type":"file","src":"a.txt"}]}}}]}"#;
        let bundle = postman(text);
        assert_eq!(bundle.requests[0].body_type, "form");
        assert_eq!(bundle.requests[0].body, "note=hi\nfile=@a.txt");
        assert!(bundle
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::UnsupportedField));
    }

    #[test]
    fn postman_graphql_becomes_json_on_the_wire() {
        let text = r#"{"info":{"name":"c"},"item":[{"name":"a","request":{"method":"POST","url":"https://x/gql",
            "body":{"mode":"graphql","graphql":{"query":"{ me }","variables":"{\"a\":1}"}}}}]}"#;
        let bundle = postman(text);
        let draft = &bundle.requests[0];
        assert_eq!(draft.body_type, "json");
        let parsed: Value = serde_json::from_str(&draft.body).unwrap();
        assert_eq!(parsed["query"], "{ me }");
        assert_eq!(parsed["variables"]["a"], 1);
    }

    #[test]
    fn postman_item_without_request_is_reported() {
        let text = r#"{"info":{"name":"c"},"item":[{"name":"怪东西"}]}"#;
        let bundle = postman(text);
        assert!(bundle.requests.is_empty());
        assert_eq!(bundle.warnings[0].kind, WarningKind::SkippedItem);
    }

    #[test]
    fn postman_request_without_url_is_kept_but_flagged() {
        let text = r#"{"info":{"name":"c"},"item":[{"name":"占位","request":{"method":"GET"}}]}"#;
        let bundle = postman(text);
        assert_eq!(bundle.requests.len(), 1);
        assert!(bundle
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::SkippedItem));
    }

    #[test]
    fn postman_empty_collection_warns() {
        let bundle = postman(r#"{"info":{"name":"空"},"item":[]}"#);
        assert_eq!(bundle.request_count(), 0);
        assert_eq!(bundle.warnings[0].kind, WarningKind::SkippedItem);
    }

    #[test]
    fn postman_broken_json_reports_json_error() {
        let err = parse("{ nope", Format::Postman).unwrap_err();
        assert_eq!(err.code(), codes::IMPORT_JSON);
    }

    // ---- apipost ----

    #[test]
    fn apipost_tree_is_walked() {
        let text = r#"{
          "apipost": "1.0",
          "name": "我的项目",
          "children": [
            { "target_type": "folder", "name": "订单", "children": [
                { "target_type": "api", "name": "下单", "method": "POST",
                  "url": "{{host}}/orders",
                  "headers": [ { "key": "content-type", "value": "application/json", "enable": true } ],
                  "body": { "mode": "json", "raw": "{\"sku\":1}" },
                  "auth": { "type": "bearer", "bearer": { "token": "{{token.access}}" } },
                  "pre_script": "console.log(1)" } ] } ],
          "variables": [ { "key": "host", "value": "https://api.example.com" } ]
        }"#;
        let bundle = apipost(text);
        assert_eq!(bundle.folders[0].name, "订单");
        let draft = &bundle.folders[0].requests[0];
        assert_eq!(draft.name, "下单");
        assert_eq!(draft.method, "POST");
        assert_eq!(draft.url, "{{host}}/orders");
        assert_eq!(draft.body_type, "json");
        assert_eq!(draft.headers[0].key, "content-type");
        assert_eq!(draft.pre_script, "console.log(1)");
        assert_eq!(
            draft.auth,
            AuthConfig::Bearer {
                token: "{{token.access}}".to_string()
            }
        );
        assert_eq!(bundle.environments[0].vars[0].key, "host");
    }

    #[test]
    fn apipost_object_headers_and_plain_text_body() {
        let text = r#"{"apipost":"1","name":"p","children":[
            {"name":"a","method":"GET","url":"https://x/a",
             "headers":{"Accept":"*/*"},
             "body":"{\"a\":1}"}]}"#;
        let bundle = apipost(text);
        let draft = &bundle.requests[0];
        assert_eq!(draft.headers[0].key, "Accept");
        assert_eq!(draft.body_type, "json", "能解析成 JSON 就该归到 json");
    }

    #[test]
    fn apipost_single_request_root() {
        let bundle = apipost(r#"{"apipost":"1","name":"单条","method":"GET","url":"https://x/a"}"#);
        assert_eq!(bundle.requests.len(), 1);
        assert_eq!(bundle.requests[0].name, "单条");
    }

    #[test]
    fn apipost_unknown_body_mode_warns() {
        let text = r#"{"apipost":"1","name":"p","children":[
            {"name":"a","method":"POST","url":"https://x/a","body":{"mode":"weird"}}]}"#;
        let bundle = apipost(text);
        assert_eq!(bundle.requests[0].body_type, "none");
        assert!(bundle
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::UnsupportedField));
    }
}
