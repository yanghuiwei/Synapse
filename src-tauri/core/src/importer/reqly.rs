//! reqly 自有 JSON —— 导入导出一体。
//!
//! ## 形状
//!
//! ```json
//! {
//!   "reqly": 1,
//!   "name": "内部接口",
//!   "folders": [
//!     { "name": "用户中心", "requests": [ { "name": "列表", "method": "GET",
//!       "url": "{{env.baseUrl}}/users", "headers": [ { "key": "accept", "value": "*/*" } ] } ] }
//!   ],
//!   "requests": [ { "name": "健康检查", "method": "GET", "url": "/health" } ],
//!   "environments": [ { "name": "本地", "vars": [ { "key": "baseUrl", "value": "http://127.0.0.1:8080" } ] } ]
//! }
//! ```
//!
//! 请求对象**就是** [`RequestDraft`] 本身，没有第二套映射：
//! 导出的东西必须能原样导回来，中间多一层转换表就等于多一个会分叉的地方。
//!
//! ## 为什么它值得存在（design.md §5.3 说是「必做」）
//!
//! 它是唯一**无损**的格式 —— Postman 装不下 `{{$uuid}}` 这种动态函数、
//! 装不下 Token 源配置；curl 连文件夹都没有。团队之间传集合、
//! 以及「导出 → 改 → 导回」这条自测路径，都只能靠它。

use super::{EnvDraft, FolderNode, ImportBundle, ImportWarning, RequestDraft, WarningKind};
use crate::error::{codes, AppError, AppResult};

/// 整份文件的形状。除了请求本身，只有集合名与版本号。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundleFile {
    /// 格式版本。先只认 1，将来结构变了才知道要兼容谁
    #[serde(default)]
    reqly: Option<u32>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    folders: Vec<FolderNode>,
    #[serde(default)]
    requests: Vec<RequestDraft>,
    #[serde(default)]
    environments: Vec<EnvDraft>,
}

pub fn parse(text: &str) -> AppResult<ImportBundle> {
    // 顶层也可能是 `[...]`（只导出一批请求时）；统一成对象再走同一条路
    let value: serde_json::Value = serde_json::from_str(text).map_err(|err| {
        AppError::http_with(
            codes::IMPORT_JSON,
            format!("invalid json: {err}"),
            serde_json::json!({ "line": err.line(), "column": err.column() }),
        )
    })?;
    let normalized = match value {
        serde_json::Value::Array(items) => serde_json::json!({ "requests": items }),
        other => other,
    };

    let file: BundleFile = serde_json::from_value(normalized).map_err(|err| {
        AppError::http_with(
            codes::IMPORT_JSON,
            format!("not a reqly bundle: {err}"),
            serde_json::json!({ "reason": "shape" }),
        )
    })?;

    if let Some(version) = file.reqly {
        if version > 1 {
            // 不拦下来。新版本多出来的字段会被忽略，能导的部分照导 ——
            // 报「版本不支持」然后什么都不给，比导进来再告知差得多
            return Ok(finish(file, Some(version)));
        }
    }
    Ok(finish(file, None))
}

fn finish(file: BundleFile, newer_version: Option<u32>) -> ImportBundle {
    let mut bundle = ImportBundle {
        folders: file.folders,
        requests: file.requests,
        environments: file.environments,
        warnings: Vec::new(),
    };

    if let Some(version) = newer_version {
        bundle.push_warning(ImportWarning::simple(
            WarningKind::UnsupportedField,
            format!("bundle version {version} is newer than 1; unknown fields were ignored"),
        ));
    }

    let collection_name = file.name;
    // 顶层没有文件夹、也没有零散请求 → 这份文件里什么都没有
    if bundle.folders.is_empty() && bundle.requests.is_empty() {
        bundle.push_warning(ImportWarning::simple(
            WarningKind::SkippedItem,
            "bundle contains no request".to_string(),
        ));
    }

    // ---- 补名字 / 剔掉没有 URL 的条目 ----
    let mut loose = std::mem::take(&mut bundle.requests);
    filter_requests(&mut loose, &collection_name, &mut bundle.warnings);
    bundle.requests = loose;

    let mut folders = std::mem::take(&mut bundle.folders);
    for folder in &mut folders {
        clean_folder(folder, &mut bundle.warnings);
    }
    bundle.folders = folders;

    // ---- 环境里的空 key 剔掉 ----
    for env in &mut bundle.environments {
        env.vars.retain(|var| !var.key.trim().is_empty());
    }
    bundle.environments.retain(|env| !env.vars.is_empty());

    bundle
}

fn clean_folder(folder: &mut FolderNode, warnings: &mut Vec<ImportWarning>) {
    // 空名留给前端补占位文案（后端不生产界面文案）
    folder.name = folder.name.trim().to_string();
    filter_requests(&mut folder.requests, &None, warnings);
    for child in &mut folder.folders {
        clean_folder(child, warnings);
    }
}

/// 没有 URL 的条目留着也没用（发送时必然报 E_URL），挑出来记一条警告
fn filter_requests(
    requests: &mut Vec<RequestDraft>,
    collection_name: &Option<String>,
    warnings: &mut Vec<ImportWarning>,
) {
    let mut kept = Vec::with_capacity(requests.len());
    for draft in requests.drain(..) {
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
            continue;
        }
        kept.push(draft);
    }

    // 名字是按 URL 生成的
    for draft in &mut kept {
        if draft.name.trim().is_empty() {
            draft.name = generate_name(&draft.url, collection_name);
            warnings.push(ImportWarning::new(
                WarningKind::GeneratedName,
                Some(draft.name.clone()),
                "request had no name; generated from the url".to_string(),
            ));
        }
    }
    *requests = kept;
}

fn generate_name(url: &str, collection_name: &Option<String>) -> String {
    let without_anchor = url.split(['#', '?']).next().unwrap_or(url);
    let trimmed = without_anchor.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or("");
    if !last.is_empty() && !last.contains("{{") {
        return last.to_string();
    }
    collection_name.clone().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::KeyValueItem;

    #[test]
    fn round_trips_a_full_bundle() {
        let text = r#"{
          "reqly": 1,
          "name": "内部接口",
          "folders": [
            { "name": "用户中心", "folders": [
                { "name": "认证", "requests": [
                  { "name": "登录", "method": "POST", "url": "{{env.baseUrl}}/login",
                    "bodyType": "json", "body": "{\"u\":1}",
                    "headers": [ { "key": "accept", "value": "application/json" } ] } ] } ],
              "requests": [ { "name": "列表", "method": "GET", "url": "{{env.baseUrl}}/users" } ] }
          ],
          "requests": [ { "name": "健康检查", "url": "{{env.baseUrl}}/health" } ],
          "environments": [ { "name": "本地", "vars": [ { "key": "baseUrl", "value": "http://127.0.0.1:8080" } ] } ]
        }"#;

        let bundle = parse(text).unwrap();
        assert_eq!(bundle.folders.len(), 1);
        assert_eq!(bundle.folders[0].name, "用户中心");
        assert_eq!(bundle.folders[0].folders[0].name, "认证");
        assert_eq!(bundle.folders[0].folders[0].requests[0].method, "POST");
        assert_eq!(bundle.folders[0].folders[0].requests[0].body_type, "json");
        assert_eq!(
            bundle.folders[0].folders[0].requests[0].headers,
            vec![KeyValueItem::new("accept", "application/json")]
        );
        assert_eq!(bundle.requests.len(), 1);
        assert_eq!(bundle.request_count(), 3);
        assert_eq!(bundle.environments[0].vars[0].key, "baseUrl");
        assert!(bundle.warnings.is_empty(), "{:?}", bundle.warnings);
    }

    #[test]
    fn top_level_array_is_accepted() {
        let bundle = parse(r#"[{"name":"a","url":"https://x/a"}]"#).unwrap();
        assert_eq!(bundle.requests.len(), 1);
    }

    #[test]
    fn request_without_url_is_skipped_with_a_warning() {
        let bundle =
            parse(r#"{"requests":[{"name":"空的"},{"name":"好的","url":"https://x/y"}]}"#).unwrap();
        assert_eq!(bundle.requests.len(), 1);
        assert_eq!(bundle.requests[0].name, "好的");
        assert_eq!(bundle.warnings[0].kind, WarningKind::SkippedItem);
        assert_eq!(bundle.warnings[0].subject.as_deref(), Some("空的"));
    }

    #[test]
    fn missing_name_is_generated_from_url() {
        let bundle =
            parse(r#"{"name":"集合","requests":[{"url":"https://x/v1/users?p=1"}]}"#).unwrap();
        assert_eq!(bundle.requests[0].name, "users");
        assert_eq!(bundle.warnings[0].kind, WarningKind::GeneratedName);
    }

    #[test]
    fn newer_version_still_imports_what_it_can() {
        let bundle = parse(r#"{"reqly":9,"requests":[{"name":"a","url":"https://x/a"}]}"#).unwrap();
        assert_eq!(bundle.requests.len(), 1, "新版本也要尽力导");
        assert_eq!(bundle.warnings[0].kind, WarningKind::UnsupportedField);
    }

    #[test]
    fn empty_bundle_warns_instead_of_silently_doing_nothing() {
        let bundle = parse(r#"{"reqly":1,"name":"空集合"}"#).unwrap();
        assert!(bundle.request_count() == 0);
        assert_eq!(bundle.warnings[0].kind, WarningKind::SkippedItem);
    }

    #[test]
    fn broken_json_reports_json_error() {
        let err = parse("{ nope").unwrap_err();
        assert_eq!(err.code(), codes::IMPORT_JSON);
    }

    #[test]
    fn wrong_shape_reports_json_error() {
        let err = parse(r#"{"folders":"不是数组"}"#).unwrap_err();
        assert_eq!(err.code(), codes::IMPORT_JSON);
    }

    #[test]
    fn empty_vars_are_dropped() {
        let bundle =
            parse(r#"{"requests":[{"url":"https://x/a"}],"environments":[{"name":"e","vars":[{"key":"  ","value":"1"},{"key":"k","value":"1"}]}]}"#)
                .unwrap();
        assert_eq!(bundle.environments[0].vars.len(), 1);
    }
}
