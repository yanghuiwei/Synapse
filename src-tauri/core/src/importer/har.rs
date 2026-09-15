//! HAR 1.2 —— 浏览器 DevTools / Charles / Fiddler 的导出格式。
//!
//! ## 一份 HAR 里大多数不是接口
//!
//! 浏览器记的是**一个页面加载过的一切**：document、stylesheet、script、image、font……
//! 真正是接口的（xhr / fetch）往往只有几条。全导进来就是把工作区塞满
//! `analytics.js` 和 favicon，所以按 `_resourceType`（没有这个私有字段的导出工具就退到
//! 响应内容类型）滤掉静态资源，并且**用一条汇总警告说明跳过了多少** ——
//! 逐条警告会刷爆预览界面，一条不说又等于静默丢弃，两个都不行。
//!
//! ## 名字和分组都是「没有的东西只能生成」
//!
//! HAR 的 `request` 里**没有名字字段**（那是 Postman 才有的概念）。
//! 所以：顶层文件夹 = 域名，请求名 = URL 的路径段（`v1/users`）。
//! 不写成「方法 + 路径」，是因为界面上方法本来就单独显示，重复一遍没意义。
//!
//! ## 请求体按 `postData.mimeType` 分档
//!
//! 口径与 curl / postman 两个导入器一致：json 只在**真能解析**时才算 json
//! （那一档发送前会本地校验，归错会把一条本来能发的请求变成 `E_BODY_NOT_JSON`）；
//! urlencoded 走 `params` 拼 `k=v&k=v`；multipart **拼不回原样**（HAR 不记文件正文），
//! 所以留成 `key=value` 行并说明现在发不出去。

use serde_json::Value;

use super::{
    urlencode, value_to_string, FolderNode, ImportBundle, ImportWarning, RequestDraft, WarningKind,
};
use crate::error::{codes, AppError, AppResult};
use crate::storage::models::KeyValueItem;

/// 静态资源的 `_resourceType`。这是黑名单而不是白名单：
/// 白名单（只认 xhr / fetch）会把不写这个字段的工具**整份导入变成 0 条**。
const STATIC_RESOURCE_TYPES: [&str; 6] =
    ["stylesheet", "script", "image", "font", "media", "manifest"];

pub fn parse(text: &str) -> AppResult<ImportBundle> {
    let root: Value = serde_json::from_str(text).map_err(|err| {
        AppError::http_with(
            codes::IMPORT_JSON,
            format!("invalid json: {err}"),
            serde_json::json!({ "line": err.line(), "column": err.column() }),
        )
    })?;

    let entries = root
        .pointer("/log/entries")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::http_with(
                codes::IMPORT_JSON,
                "not a har: `log.entries` is missing".to_string(),
                serde_json::json!({ "reason": "shape" }),
            )
        })?;

    let mut warnings = Vec::new();
    let mut hosts: Vec<(String, FolderNode)> = Vec::new();
    let mut skipped_static = 0usize;
    let mut skipped_entry = 0usize;
    let mut dropped_pseudo = 0usize;

    for entry in entries {
        let Some(request) = entry.get("request") else {
            skipped_entry += 1;
            continue;
        };
        if is_static(entry) {
            skipped_static += 1;
            continue;
        }
        let url = request
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if url.is_empty() {
            skipped_entry += 1;
            continue;
        }

        let draft = draft_of(request, &url, &mut warnings, &mut dropped_pseudo);
        let host = host_of(&url);
        match hosts.iter_mut().find(|(known, _)| *known == host) {
            Some((_, folder)) => folder.requests.push(draft),
            None => {
                let mut folder = FolderNode::new(host.clone());
                folder.requests.push(draft);
                hosts.push((host, folder));
            }
        }
    }

    let mut bundle = ImportBundle {
        folders: hosts.into_iter().map(|(_, node)| node).collect(),
        ..ImportBundle::default()
    };

    // 汇总成一条，而不是每条资源一条 —— 一份页面 HAR 里静态资源动辄几百条
    if skipped_static > 0 {
        warnings.push(ImportWarning::simple(
            WarningKind::SkippedItem,
            format!("skipped {skipped_static} static asset entries (script/stylesheet/image/font/media/manifest)"),
        ));
    }
    if skipped_entry > 0 {
        warnings.push(ImportWarning::simple(
            WarningKind::SkippedItem,
            format!("skipped {skipped_entry} entries without a usable request url"),
        ));
    }
    if dropped_pseudo > 0 {
        warnings.push(ImportWarning::simple(
            WarningKind::UnsupportedField,
            format!("dropped {dropped_pseudo} http/2 pseudo headers (names starting with `:`)"),
        ));
    }
    bundle.warnings = warnings;

    if bundle.request_count() == 0 && bundle.warnings.is_empty() {
        // 认出来了但一条都没有：不是错误（可能真是个空 HAR），但必须说出来，
        // 否则用户看到的是「导入成功、什么都没发生」
        bundle.push_warning(ImportWarning::simple(
            WarningKind::SkippedItem,
            "har contains no request".to_string(),
        ));
    }
    super::dedupe_names(&mut bundle);
    Ok(bundle)
}

fn draft_of(
    request: &Value,
    url: &str,
    warnings: &mut Vec<ImportWarning>,
    dropped_pseudo: &mut usize,
) -> RequestDraft {
    let (url, params) = har_url(request, url);
    let (body_type, body) = har_body(request, warnings);
    RequestDraft {
        name: name_of(request_url(request)),
        method: request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_ascii_uppercase(),
        url,
        params,
        headers: har_headers(request, dropped_pseudo),
        body_type,
        body,
        ..RequestDraft::default()
    }
}

fn request_url(request: &Value) -> &str {
    request.get("url").and_then(Value::as_str).unwrap_or("")
}

/// `queryString[]` → `params[]`，真拿到了才把 query 从 URL 里切掉。
///
/// 「切了却没抓到对应参数」= 把参数整段丢掉，所以**空数组不切**：
/// 有些导出工具会把 query 留在 URL 里、`queryString` 留空。
fn har_url(request: &Value, url: &str) -> (String, Vec<KeyValueItem>) {
    let params: Vec<KeyValueItem> = request
        .get("queryString")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(har_kv).collect())
        .unwrap_or_default();
    if params.is_empty() {
        return (url.to_string(), Vec::new());
    }
    let without_anchor = url.split('#').next().unwrap_or(url);
    let base = without_anchor
        .split_once('?')
        .map(|(head, _)| head.to_string())
        .unwrap_or_else(|| without_anchor.to_string());
    (base, params)
}

fn har_headers(request: &Value, dropped_pseudo: &mut usize) -> Vec<KeyValueItem> {
    request
        .get("headers")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let key = item.get("name").and_then(Value::as_str)?.trim().to_string();
                    if key.is_empty() {
                        return None;
                    }
                    // HTTP/2 的伪头（`:authority` / `:method`）描述的是连接本身，不能照抄成字段
                    if key.starts_with(':') {
                        *dropped_pseudo += 1;
                        return None;
                    }
                    let value = item.get("value").map(value_to_string).unwrap_or_default();
                    Some(KeyValueItem::new(key, value))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn har_body(request: &Value, warnings: &mut Vec<ImportWarning>) -> (String, String) {
    let Some(post) = request.get("postData") else {
        return ("none".to_string(), String::new());
    };
    if post.get("encoding").and_then(Value::as_str) == Some("base64") {
        warnings.push(ImportWarning::simple(
            WarningKind::UnsupportedField,
            "request body is base64 encoded and was dropped".to_string(),
        ));
        return ("none".to_string(), String::new());
    }

    let mime = post
        .get("mimeType")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let text = post.get("text").map(value_to_string).unwrap_or_default();

    if mime.contains("multipart/form-data") {
        // HAR 只记字段名与文件名，**不记文件正文** —— 拼不回原样。
        // 与 curl / postman 同一口径：留成 `key=value` 行并说明现在还发不出去。
        let lines: Vec<String> = post
            .get("params")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(multipart_line).collect())
            .unwrap_or_default();
        if lines.is_empty() {
            return ("none".to_string(), String::new());
        }
        warnings.push(ImportWarning::simple(
            WarningKind::UnsupportedField,
            "multipart body is kept as `key=value` lines but cannot be sent yet".to_string(),
        ));
        return ("form".to_string(), lines.join("\n"));
    }

    if mime.contains("urlencoded") {
        let pairs: Vec<String> = post
            .get("params")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(har_kv)
                    .map(|item| format!("{}={}", item.key, urlencode(&item.value)))
                    .collect()
            })
            .unwrap_or_default();
        // `params` 缺失就退回 `text`：urlencoded 的 text 本来就是拼好的 `k=v&k=v`
        let body = if pairs.is_empty() {
            text
        } else {
            pairs.join("&")
        };
        return if body.trim().is_empty() {
            ("none".to_string(), String::new())
        } else {
            ("urlencoded".to_string(), body)
        };
    }

    if text.trim().is_empty() {
        return ("none".to_string(), String::new());
    }
    // json 只在真能解析时才算 —— curl 导入器同口径，理由见文件头
    if mime.contains("json") && serde_json::from_str::<Value>(&text).is_ok() {
        return ("json".to_string(), text);
    }
    ("raw".to_string(), text)
}

fn multipart_line(item: &Value) -> Option<String> {
    let key = item.get("name").and_then(Value::as_str)?.trim().to_string();
    if key.is_empty() {
        return None;
    }
    // 文件部件在 HAR 里只有 fileName（正文不在），用 Postman 那套 `@` 记法标出来
    match item.get("fileName").and_then(Value::as_str) {
        Some(file) => Some(format!("{key}=@{file}")),
        None => Some(format!(
            "{key}={}",
            item.get("value").map(value_to_string).unwrap_or_default()
        )),
    }
}

fn har_kv(item: &Value) -> Option<KeyValueItem> {
    let key = item.get("name").and_then(Value::as_str)?.trim().to_string();
    if key.is_empty() {
        return None;
    }
    let value = item.get("value").map(value_to_string).unwrap_or_default();
    Some(KeyValueItem::new(key, value))
}

/// 是不是「静态资源」（不是接口）。
///
/// 先看 `_resourceType`（Chrome / Firefox 会写这个私有字段）；没有的（Charles / Fiddler 等）
/// 才退到响应内容类型。**两个都没有就留下** ——
/// 多导一条用户可以删掉，少导一条用户不会知道。
fn is_static(entry: &Value) -> bool {
    if let Some(kind) = entry.get("_resourceType").and_then(Value::as_str) {
        return STATIC_RESOURCE_TYPES.contains(&kind.to_ascii_lowercase().as_str());
    }
    let mime = entry
        .pointer("/response/content/mimeType")
        .and_then(Value::as_str)
        .unwrap_or("");
    is_static_mime(mime)
}

fn is_static_mime(mime: &str) -> bool {
    let bare = mime
        .split(';')
        .next()
        .unwrap_or(mime)
        .trim()
        .to_ascii_lowercase();
    bare.starts_with("image/")
        || bare.starts_with("font/")
        || bare.starts_with("video/")
        || bare.starts_with("audio/")
        || matches!(
            bare.as_str(),
            "text/css" | "text/javascript" | "application/javascript" | "application/x-javascript"
        )
}

/// `https://api.x.com:8443/v1/users?q=1` → `api.x.com:8443`
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    if authority.is_empty() {
        "unknown".to_string()
    } else {
        authority.to_string()
    }
}

/// HAR 里没有名字，只能从 URL 的路径段生成：`…/v1/users?q=1` → `v1/users`。
/// 只有域名（或路径全是斜杠）时退回域名。
fn name_of(url: &str) -> String {
    let without_anchor = url.split(['#', '?']).next().unwrap_or(url);
    let rest = without_anchor
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(without_anchor);
    let path = rest.split_once('/').map(|(_, path)| path).unwrap_or("");
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        host_of(url)
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn har(text: &str) -> ImportBundle {
        parse(text).expect("应该能解析")
    }

    /// 一条 xhr + 一条图片 + 一条 css —— 页面 HAR 的常态
    const PAGE_HAR: &str = r#"{
      "log": {
        "version": "1.2",
        "creator": { "name": "Chrome", "version": "128" },
        "entries": [
          {
            "_resourceType": "xhr",
            "startedDateTime": "2026-09-15T02:00:00.000Z",
            "request": {
              "method": "post",
              "url": "https://api.example.com/v1/users?page=2&q=%E5%BC%A0",
              "headers": [
                { "name": "accept", "value": "application/json" },
                { "name": ":authority", "value": "api.example.com" },
                { "name": "content-type", "value": "application/json" }
              ],
              "queryString": [
                { "name": "page", "value": "2" },
                { "name": "q", "value": "张" }
              ],
              "postData": {
                "mimeType": "application/json",
                "text": "{\"name\":\"张\"}"
              }
            },
            "response": { "status": 200, "content": { "mimeType": "application/json" } }
          },
          {
            "_resourceType": "image",
            "request": { "method": "GET", "url": "https://cdn.example.com/logo.png", "headers": [] },
            "response": { "status": 200, "content": { "mimeType": "image/png" } }
          },
          {
            "_resourceType": "stylesheet",
            "request": { "method": "GET", "url": "https://cdn.example.com/app.css", "headers": [] },
            "response": { "status": 200, "content": { "mimeType": "text/css" } }
          }
        ]
      }
    }"#;

    #[test]
    fn keeps_api_calls_and_drops_static_assets() {
        let bundle = har(PAGE_HAR);
        assert_eq!(bundle.request_count(), 1, "{:?}", bundle.folders);
        assert_eq!(bundle.folders.len(), 1);
        // 顶层文件夹是域名
        assert_eq!(bundle.folders[0].name, "api.example.com");

        let draft = &bundle.folders[0].requests[0];
        assert_eq!(draft.name, "v1/users");
        assert_eq!(draft.method, "POST");
        // query 拆进了 params，URL 上就没有 query 了
        assert_eq!(draft.url, "https://api.example.com/v1/users");
        assert_eq!(draft.params.len(), 2);
        assert_eq!(draft.params[0].key, "page");
        assert_eq!(draft.params[1].value, "张");
        assert_eq!(draft.body_type, "json");
        assert_eq!(draft.body, r#"{"name":"张"}"#);
        assert_eq!(draft.headers.len(), 2, "伪头不该进来");
        assert_eq!(draft.headers[0].key, "accept");
    }

    #[test]
    fn statics_and_pseudo_headers_are_reported_not_silently_dropped() {
        let bundle = har(PAGE_HAR);
        let details: Vec<&str> = bundle
            .warnings
            .iter()
            .map(|warning| warning.detail.as_str())
            .collect();
        assert!(
            details.iter().any(|d| d.contains("2 static asset entries")),
            "{details:?}"
        );
        assert!(
            details
                .iter()
                .any(|d| d.contains("1 http/2 pseudo headers")),
            "{details:?}"
        );
    }

    #[test]
    fn two_hosts_become_two_folders() {
        let bundle = har(r#"{"log":{"entries":[
                {"_resourceType":"xhr","request":{"url":"https://a.example.com/x","headers":[]}},
                {"_resourceType":"fetch","request":{"url":"https://b.example.com/y","headers":[]}}
            ]}}"#);
        let names: Vec<&str> = bundle.folders.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["a.example.com", "b.example.com"]);
        assert_eq!(bundle.request_count(), 2);
    }

    #[test]
    fn query_stays_in_the_url_when_query_string_is_empty() {
        // 有些导出工具只填 URL、`queryString` 留空 —— 这时切掉 query 等于把参数丢掉
        let bundle = har(r#"{"log":{"entries":[{"_resourceType":"xhr","request":{
                "method":"GET","url":"https://a.example.com/x?p=1","queryString":[],"headers":[]}}]}}"#);
        let draft = &bundle.folders[0].requests[0];
        assert_eq!(draft.url, "https://a.example.com/x?p=1");
        assert!(draft.params.is_empty());
    }

    #[test]
    fn urlencoded_body_is_rebuilt_from_params() {
        let bundle = har(r#"{"log":{"entries":[{"_resourceType":"xhr","request":{
                "method":"POST","url":"https://a.example.com/login","headers":[],
                "postData":{"mimeType":"application/x-www-form-urlencoded",
                  "params":[{"name":"u","value":"a b"},{"name":"p","value":"1&2"}]}}}]}}"#);
        let draft = &bundle.folders[0].requests[0];
        assert_eq!(draft.body_type, "urlencoded");
        assert_eq!(draft.body, "u=a%20b&p=1%262");
    }

    #[test]
    fn urlencoded_body_falls_back_to_text() {
        let bundle = har(r#"{"log":{"entries":[{"_resourceType":"xhr","request":{
                "method":"POST","url":"https://a.example.com/login","headers":[],
                "postData":{"mimeType":"application/x-www-form-urlencoded","text":"a=1&b=2"}}}]}}"#);
        let draft = &bundle.folders[0].requests[0];
        assert_eq!(draft.body_type, "urlencoded");
        assert_eq!(draft.body, "a=1&b=2");
    }

    #[test]
    fn multipart_is_kept_as_lines_with_a_warning() {
        let bundle = har(r#"{"log":{"entries":[{"_resourceType":"xhr","request":{
                "method":"POST","url":"https://a.example.com/upload","headers":[],
                "postData":{"mimeType":"multipart/form-data","params":[
                  {"name":"note","value":"hi"},{"name":"file","fileName":"a.png"}]}}}]}}"#);
        let draft = &bundle.folders[0].requests[0];
        assert_eq!(draft.body_type, "form");
        assert_eq!(draft.body, "note=hi\nfile=@a.png");
        assert!(bundle
            .warnings
            .iter()
            .any(|w| w.detail.contains("cannot be sent yet")));
    }

    #[test]
    fn base64_body_is_dropped_with_a_warning() {
        let bundle = har(r#"{"log":{"entries":[{"_resourceType":"xhr","request":{
                "method":"POST","url":"https://a.example.com/up","headers":[],
                "postData":{"mimeType":"application/octet-stream","encoding":"base64","text":"AAAA"}}}]}}"#);
        let draft = &bundle.folders[0].requests[0];
        assert_eq!(draft.body_type, "none");
        assert_eq!(draft.body, "");
        assert!(bundle.warnings.iter().any(|w| w.detail.contains("base64")));
    }

    #[test]
    fn json_that_does_not_parse_stays_raw() {
        // 归到 json 会在发送前被本地校验拦下，把一条本来能发的请求变成 E_BODY_NOT_JSON
        let bundle = har(r#"{"log":{"entries":[{"_resourceType":"xhr","request":{
                "method":"POST","url":"https://a.example.com/x","headers":[],
                "postData":{"mimeType":"application/json","text":"not json at all"}}}]}}"#);
        assert_eq!(bundle.folders[0].requests[0].body_type, "raw");
    }

    #[test]
    fn without_resource_type_static_mime_is_still_filtered() {
        let bundle = har(r#"{"log":{"entries":[
                {"request":{"url":"https://a.example.com/api","headers":[]},
                 "response":{"content":{"mimeType":"application/json; charset=utf-8"}}},
                {"request":{"url":"https://a.example.com/app.js","headers":[]},
                 "response":{"content":{"mimeType":"application/javascript"}}}
            ]}}"#);
        assert_eq!(bundle.request_count(), 1);
        assert_eq!(bundle.folders[0].requests[0].name, "api");
    }

    #[test]
    fn without_any_signal_the_entry_is_kept() {
        // 两个字段都没有 → 宁可多导一条，也不要少导一条
        let bundle = har(
            r#"{"log":{"entries":[{"request":{"url":"https://a.example.com/x","headers":[]}}]}}"#,
        );
        assert_eq!(bundle.request_count(), 1);
    }

    #[test]
    fn entries_without_request_are_counted_and_reported() {
        let bundle = har(
            r#"{"log":{"entries":[{"comment":"a"},{"request":{"url":"https://a/x","headers":[]}}]}}"#,
        );
        assert_eq!(bundle.request_count(), 1);
        assert!(bundle
            .warnings
            .iter()
            .any(|w| w.detail.contains("1 entries without a usable request url")));
    }

    #[test]
    fn same_path_twice_gets_a_suffix() {
        let bundle = har(r#"{"log":{"entries":[
                {"request":{"url":"https://a.example.com/x/1","headers":[]}},
                {"request":{"url":"https://a.example.com/x/1","headers":[]}}
            ]}}"#);
        let names: Vec<&str> = bundle.folders[0]
            .requests
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(names, vec!["x/1", "x/1 (2)"]);
    }

    #[test]
    fn empty_har_warns_instead_of_looking_like_success() {
        let bundle = har(r#"{"log":{"version":"1.2","entries":[]}}"#);
        assert_eq!(bundle.request_count(), 0);
        assert_eq!(bundle.warnings[0].kind, WarningKind::SkippedItem);
    }

    #[test]
    fn missing_log_entries_reports_json_error() {
        let err = parse(r#"{"log":{"version":"1.2"}}"#).unwrap_err();
        assert_eq!(err.code(), codes::IMPORT_JSON);
    }

    #[test]
    fn broken_json_reports_json_error() {
        let err = parse("{ nope").unwrap_err();
        assert_eq!(err.code(), codes::IMPORT_JSON);
    }

    #[test]
    fn host_and_name_helpers() {
        assert_eq!(
            host_of("https://api.x.com:8443/v1/users?q=1"),
            "api.x.com:8443"
        );
        assert_eq!(host_of("https://api.x.com/"), "api.x.com");
        assert_eq!(name_of("https://api.x.com/v1/users?q=1"), "v1/users");
        assert_eq!(name_of("https://api.x.com/"), "api.x.com");
    }
}
