//! 导入器：把外部格式转成 reqly 的内部结构。
//!
//! ## 一条流水线
//!
//! ```text
//! 粘贴的文本 / 选中的文件
//!   → detect()      指纹识别（curl 命令 / Postman v2.1 / apipost 原生 / reqly JSON）
//!   → parse()       各自解析成 ImportBundle
//!   → preview_import 与库里现有数据比对，出「+新增 / ~更新 / =跳过」的差异树
//!   → commit_import  按冲突模式落库
//! ```
//!
//! ## 两条贯穿全部解析器的规矩
//!
//! 1. **能导就导，导不了的那部分要说出来。**
//!    认不出来的命令行参数、解不出的字段、被丢弃的脚本，一律进
//!    [`ImportWarning`]，在预览界面里列给用户看。静默丢弃是最坏的结果 ——
//!    用户会以为导进来了，直到调用接口那天才发现少了东西。
//! 2. **`{{...}}` 原样保留。** 导入的 URL / header 里那些 `{{baseUrl}}` 是**资产**，
//!    不是要展开的宏。pipeline 在发送前会处理它们。

pub mod curl;
pub mod postman;
pub mod reqly;
pub mod tokenizer;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{codes, AppError, AppResult};
use crate::storage::models::{AuthConfig, KeyValueItem, RequestSettings};

/// 支持的来源格式。加一种就在 [`detect`] 里加一条指纹。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    Curl,
    Postman,
    Apipost,
    Reqly,
}

impl Format {
    /// 前端要按格式换提示文案，所以要有稳定的字符串
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Curl => "curl",
            Format::Postman => "postman",
            Format::Apipost => "apipost",
            Format::Reqly => "reqly",
        }
    }
}

/// 一次导入要给用户看的警告。**不是错误** —— 有警告也照样导得进来。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportWarning {
    /// 出问题的对象名（接口名 / 文件夹名），没有就省略
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// 机器可读的原因，前端翻成中文（后端只回错误码，不回来的文案）
    pub kind: WarningKind,
    /// 只给开发者看的细节
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WarningKind {
    /// 命令行参数不认识，已跳过（curl）
    UnknownArgument,
    /// 参数需要值但没给够
    MissingArgumentValue,
    /// 这个字段我们暂时不支持，已丢弃
    UnsupportedField,
    /// 名字重复，导入时被自动加了后缀
    RenamedDuplicate,
    /// 名字太长，落库前被截断了
    TruncatedName,
    /// 请求没有名字，按方法 + 路径生成的
    GeneratedName,
    /// 整条对象解不出来，已跳过
    SkippedItem,
}

impl ImportWarning {
    pub fn new(kind: WarningKind, subject: Option<String>, detail: impl Into<String>) -> Self {
        Self {
            subject,
            kind,
            detail: detail.into(),
        }
    }

    pub fn simple(kind: WarningKind, detail: impl Into<String>) -> Self {
        Self::new(kind, None, detail)
    }
}

/// 解析出来的一个请求。字段与 `request` 表一一对应，落库时直接映射。
///
/// 全部字段都带 `#[serde(default)]`：导入的文件是**外面来的**，
/// 缺字段是常态（手写的 reqly JSON、Postman 里没填 URL 的占位条目）。
/// 缺到不能用的那些由各自的解析器挑出来记警告，而不是让整份文件解析失败。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestDraft {
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub params: Vec<KeyValueItem>,
    #[serde(default)]
    pub headers: Vec<KeyValueItem>,
    #[serde(default = "default_body_type")]
    pub body_type: String,
    #[serde(default)]
    pub body: String,
    /// 宽松解析：认不出的形状退化成 `Inherit`，而不是让整份文件解析失败
    #[serde(default, deserialize_with = "de_lenient_auth")]
    pub auth: AuthConfig,
    #[serde(default)]
    pub settings: RequestSettings,
    #[serde(default)]
    pub pre_script: String,
    #[serde(default)]
    pub test_script: String,
}

fn default_method() -> String {
    "GET".to_string()
}

fn default_body_type() -> String {
    "none".to_string()
}

impl Default for RequestDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            method: "GET".to_string(),
            url: String::new(),
            params: Vec::new(),
            headers: Vec::new(),
            body_type: default_body_type(),
            body: String::new(),
            auth: AuthConfig::Inherit,
            settings: RequestSettings::default(),
            pre_script: String::new(),
            test_script: String::new(),
        }
    }
}

/// 解析出来的一棵文件夹树。**嵌套**而不是拍平 ——
/// 拍平要额外带 parentId，而导入时那些 id 还不存在。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FolderNode {
    #[serde(default)]
    pub name: String,
    /// 文件夹级授权（Postman 的 folder auth）。`None` = 没配，请求靠继承链去找
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_optional_auth"
    )]
    pub auth: Option<AuthConfig>,
    #[serde(default)]
    pub folders: Vec<FolderNode>,
    #[serde(default)]
    pub requests: Vec<RequestDraft>,
}

impl FolderNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            auth: None,
            folders: Vec::new(),
            requests: Vec::new(),
        }
    }

    /// 这棵子树里（含自己）一共有几个接口 —— 预览界面的计数靠它
    pub fn request_count(&self) -> usize {
        self.requests.len()
            + self
                .folders
                .iter()
                .map(FolderNode::request_count)
                .sum::<usize>()
    }

    pub fn folder_count(&self) -> usize {
        1 + self
            .folders
            .iter()
            .map(FolderNode::folder_count)
            .sum::<usize>()
    }
}

/// 解析出来的一个环境（Postman 的 collection 变量、apipost 的 environment）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvDraft {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub vars: Vec<EnvVarDraft>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvVarDraft {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub is_secret: bool,
}

/// 一个解析器的产出。
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportBundle {
    /// 顶层文件夹（不包含「根」—— 根由导入目标决定）
    #[serde(default)]
    pub folders: Vec<FolderNode>,
    /// 不归任何文件夹的接口（curl 导入、单文件接口）
    #[serde(default)]
    pub requests: Vec<RequestDraft>,
    #[serde(default)]
    pub environments: Vec<EnvDraft>,
    #[serde(default)]
    pub warnings: Vec<ImportWarning>,
}

impl ImportBundle {
    pub fn request_count(&self) -> usize {
        self.requests.len()
            + self
                .folders
                .iter()
                .map(FolderNode::request_count)
                .sum::<usize>()
    }

    pub fn push_warning(&mut self, warning: ImportWarning) {
        self.warnings.push(warning);
    }
}

/// 待识别的输入。文本与文件名二选一（文件也读成文本，只是多一个名字做提示）。
#[derive(Debug, Clone)]
pub struct ImportInput {
    pub text: String,
    pub file_name: Option<String>,
}

impl ImportInput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            file_name: None,
        }
    }

    pub fn with_file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    /// 去掉 BOM 与首尾空白后的正文
    fn trimmed(&self) -> &str {
        self.text.trim_start_matches('\u{feff}').trim()
    }
}

/// 指纹识别。
///
/// 判断顺序是有讲究的：**先看 JSON 里的指纹字段，最后才退到「是不是 curl 命令」**。
/// 反过来会出现「一个 JSON 文件里恰好有 curl 字样」这种误判。
pub fn detect(input: &ImportInput) -> AppResult<Format> {
    let text = input.trimmed();
    if text.is_empty() {
        return Err(AppError::http_with(
            codes::IMPORT_DETECT,
            "input is empty".to_string(),
            serde_json::json!({ "reason": "empty" }),
        ));
    }

    if text.starts_with('{') || text.starts_with('[') {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|err| {
            AppError::http_with(
                codes::IMPORT_JSON,
                format!("invalid json: {err}"),
                serde_json::json!({ "line": err.line(), "column": err.column() }),
            )
        })?;
        return Ok(json_format(&value));
    }

    // 不是 JSON 就只能是 curl（本阶段支持的范围里）
    if looks_like_curl(text) {
        return Ok(Format::Curl);
    }

    Err(AppError::http_with(
        codes::IMPORT_DETECT,
        "no known fingerprint".to_string(),
        serde_json::json!({ "reason": "unknown" }),
    ))
}

/// 解析入口：识别 → 分派。
pub fn parse(input: &ImportInput) -> AppResult<(Format, ImportBundle)> {
    let format = detect(input)?;
    let bundle = match format {
        Format::Curl => curl::parse(input.trimmed())?,
        Format::Postman | Format::Apipost => postman::parse(input.trimmed(), format)?,
        Format::Reqly => reqly::parse(input.trimmed())?,
    };
    Ok((format, bundle))
}

/// JSON 根节点的指纹 → 格式。
///
/// 放在 mod.rs 而不是 postman.rs，是因为 `detect` 要它，而 detect 是所有格式共用的。
fn json_format(value: &serde_json::Value) -> Format {
    // apipost 原生导出的标志位。放在最前面：它的结构里也有 `info`，
    // 先按 `apipost` 认出来，免得被 Postman 的分支抢走
    if value.get("apipost").is_some() || value.get("target_type").is_some() {
        return Format::Apipost;
    }
    if let Some(info) = value.get("info") {
        if info.get("schema").is_some() || value.get("_postman_id").is_some() {
            return Format::Postman;
        }
    }
    if value.get("_postman_id").is_some() || value.get("item").is_some() {
        return Format::Postman;
    }
    // 其余当作 reqly 自有格式（结构最松，放最后兜底）
    Format::Reqly
}

/// 宽松的 auth 反序列化。见 [`AuthConfig::from_json`] 的注释 ——
/// 外部文件里一个认不出的 `auth` 形状不该让整份导入失败。
fn de_lenient_auth<'de, D: Deserializer<'de>>(de: D) -> Result<AuthConfig, D::Error> {
    let raw = serde_json::Value::deserialize(de)?;
    Ok(AuthConfig::from_json(&raw))
}

fn de_optional_auth<'de, D: Deserializer<'de>>(de: D) -> Result<Option<AuthConfig>, D::Error> {
    let raw = serde_json::Value::deserialize(de)?;
    if raw.is_null() {
        return Ok(None);
    }
    // 显式写了 `{"type":"inherit"}` 等于没写 —— 落库时不必多出一列配置
    let parsed = AuthConfig::from_json(&raw);
    Ok(match parsed {
        AuthConfig::Inherit => None,
        other => Some(other),
    })
}

/// 同一次导入里重名的请求加后缀，免得落库后靠名字都分不清谁是谁。
/// 放在 mod.rs 是因为**所有解析器都要用**：curl 里连个名字都没有（全靠 URL 生成），
/// Postman 里同名的接口一大把（不同文件夹下各有一个 `列表`）。
///
/// 空名字不参与：那表示「这份文件没给它起名」，占位文案由**前端**在落库前补
/// （后端不生产面向用户的文案），把它当重名去加后缀只会得到一堆 ` (2)`。
pub(crate) fn dedupe_names(bundle: &mut ImportBundle) {
    let mut seen: Vec<String> = Vec::new();
    let mut requests = std::mem::take(&mut bundle.requests);
    for draft in &mut requests {
        dedupe_one(draft, &mut seen, &mut bundle.warnings);
    }
    bundle.requests = requests;

    let mut folders = std::mem::take(&mut bundle.folders);
    for folder in &mut folders {
        walk_dedupe(folder, &mut seen, &mut bundle.warnings);
    }
    bundle.folders = folders;
}

fn walk_dedupe(node: &mut FolderNode, seen: &mut Vec<String>, warnings: &mut Vec<ImportWarning>) {
    for draft in &mut node.requests {
        dedupe_one(draft, seen, warnings);
    }
    for child in &mut node.folders {
        walk_dedupe(child, seen, warnings);
    }
}

fn dedupe_one(draft: &mut RequestDraft, seen: &mut Vec<String>, warnings: &mut Vec<ImportWarning>) {
    // 无名 = 前端落库前才补的占位，这里不该参与重名判断
    if draft.name.is_empty() {
        return;
    }
    if !seen.contains(&draft.name) {
        seen.push(draft.name.clone());
        return;
    }
    let original = draft.name.clone();
    let mut index = 2;
    while seen.contains(&format!("{original} ({index})")) {
        index += 1;
    }
    draft.name = format!("{original} ({index})");
    seen.push(draft.name.clone());
    warnings.push(ImportWarning::new(
        WarningKind::RenamedDuplicate,
        Some(original),
        format!("duplicate name, renamed to `{}`", draft.name),
    ));
}

/// 「看起来像 curl 命令」：第一个非空行以 curl 开头，或者第一个词就是 curl
fn looks_like_curl(text: &str) -> bool {
    let head = text.lines().find(|line| !line.trim().is_empty());
    let Some(head) = head else {
        return false;
    };
    let head = head.trim_start();
    // 允许 `$ ` / `> ` 这种从终端复制时带上的提示符
    let head = head
        .strip_prefix("$ ")
        .or_else(|| head.strip_prefix("> "))
        .unwrap_or(head);
    let first = head.split_whitespace().next().unwrap_or("");
    let name = first
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(first)
        .trim_matches(|c| c == '\'' || c == '"')
        .to_ascii_lowercase();
    name == "curl" || name == "curl.exe"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_curl_with_a_terminal_prompt() {
        let input = ImportInput::text("$ curl 'https://x' -H 'a: b'");
        assert_eq!(detect(&input).unwrap(), Format::Curl);
    }

    #[test]
    fn detects_postman_by_schema() {
        let input = ImportInput::text(
            r#"{"info":{"name":"c","schema":"https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},"item":[]}"#,
        );
        assert_eq!(detect(&input).unwrap(), Format::Postman);
    }

    #[test]
    fn detects_apipost_before_postman() {
        // apipost 的结构里同时有 info 与 apipost 标志 → 必须优先认出 apipost
        let input = ImportInput::text(r#"{"apipost":"1.0","info":{"name":"c"},"item":[]}"#);
        assert_eq!(detect(&input).unwrap(), Format::Apipost);
    }

    #[test]
    fn empty_input_reports_detect_failure() {
        let err = detect(&ImportInput::text("   \n")).unwrap_err();
        assert_eq!(err.code(), codes::IMPORT_DETECT);
    }

    #[test]
    fn broken_json_reports_json_error() {
        let err = detect(&ImportInput::text("{ not json")).unwrap_err();
        assert_eq!(err.code(), codes::IMPORT_JSON);
    }

    #[test]
    fn random_text_reports_detect_failure() {
        let err = detect(&ImportInput::text("hello world")).unwrap_err();
        assert_eq!(err.code(), codes::IMPORT_DETECT);
    }

    #[test]
    fn folder_counts_include_descendants() {
        let mut root = FolderNode::new("a");
        root.requests.push(RequestDraft::default());
        let mut child = FolderNode::new("b");
        child.requests.push(RequestDraft::default());
        child.requests.push(RequestDraft::default());
        root.folders.push(child);
        assert_eq!(root.request_count(), 3);
        assert_eq!(root.folder_count(), 2);
    }
}
