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
pub mod har;
pub mod postman;
pub mod reqly;
pub mod tokenizer;

use serde::{Deserialize, Serialize};

use crate::error::{codes, AppError, AppResult};
use crate::storage::models::{KeyValueItem, RequestSettings};

/// 支持的来源格式。加一种就在 [`detect`] 里加一条指纹。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    Curl,
    Postman,
    Apipost,
    Reqly,
    Har,
}

impl Format {
    /// 前端要按格式换提示文案，所以要有稳定的字符串
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Curl => "curl",
            Format::Postman => "postman",
            Format::Apipost => "apipost",
            Format::Reqly => "reqly",
            Format::Har => "har",
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
            settings: RequestSettings::default(),
            pre_script: String::new(),
            test_script: String::new(),
        }
    }
}

/// 解析出来的一棵文件夹树。**嵌套**而不是拍平 ——
/// 拍平要额外带 parentId，而导入时那些 id 还不存在。
///
/// 🔴 **文件夹上没有授权字段**。外部格式（Postman / ApiPost）里那些
/// folder / collection 级 auth 在**解析时就被物化成请求行**了 ——
/// 见 [`AuthRows`] 与 [`merge_auth_rows`] 的说明。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FolderNode {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub folders: Vec<FolderNode>,
    #[serde(default)]
    pub requests: Vec<RequestDraft>,
}

impl FolderNode {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
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
        return json_format(&value).ok_or_else(|| {
            AppError::http_with(
                codes::IMPORT_DETECT,
                "no known fingerprint".to_string(),
                serde_json::json!({ "reason": "unknown" }),
            )
        });
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
        Format::Har => har::parse(input.trimmed())?,
    };
    Ok((format, bundle))
}

/// JSON 根节点的指纹 → 格式。认不出返回 `None`（由 [`detect`] 翻成 `E_IMPORT_DETECT`）。
///
/// 放在 mod.rs 而不是 postman.rs，是因为 `detect` 要它，而 detect 是所有格式共用的。
///
/// **判据顺序是硬契约**：越具体的形状越靠前。一条新指纹如果排在靠后的分支后面，
/// 就会被那个分支抢走，而抢走的后果通常不是报错，是「导进来 0 条」。
fn json_format(value: &serde_json::Value) -> Option<Format> {
    // apipost 原生导出的标志位。放在最前面：它的结构里也有 `info`，
    // 先按 `apipost` 认出来，免得被 Postman 的分支抢走
    if value.get("apipost").is_some() || value.get("target_type").is_some() {
        return Some(Format::Apipost);
    }
    // ApiPost 桌面端「导出项目」的固定搭配：顶层 `project` / `apis` / `envs` / `models`，
    // 三个 `target_type` 都在 `apis[]` 里面 —— 上面那条只认顶层的，正好漏掉这一种。
    // 漏掉的后果不是报错，是掉进 reqly 兜底 → 静默 0 条（2026-09-15 实际踩到）
    if is_apipost_export(value) {
        return Some(Format::Apipost);
    }
    // HAR 1.2 靠 `log.entries` 认。它和下面几家的顶层键都不撞，
    // 但顺序还是放在 Postman 之前 —— 以后加 OpenAPI 时那个 `info` 是要抢的，
    // 这里先把「更具体的形状优先」立住
    if value
        .pointer("/log/entries")
        .and_then(serde_json::Value::as_array)
        .is_some()
    {
        return Some(Format::Har);
    }
    if let Some(info) = value.get("info") {
        if info.get("schema").is_some() || value.get("_postman_id").is_some() {
            return Some(Format::Postman);
        }
    }
    if value.get("_postman_id").is_some() || value.get("item").is_some() {
        return Some(Format::Postman);
    }
    // reqly 自有格式：带版本号，或三个结构键之一，或顶层直接就是数组。
    //
    // 这里**不再无条件兜底**。认不出的 JSON（ApiPost 桌面端导出、OpenAPI、
    // Insomnia…）以前会落到这个分支，然后 `reqly.rs` 因为字段全带 `#[serde(default)]`
    // 「解析成功但内容为空」→ 界面显示「已导入 0 个接口」。
    // 用户看到的是「成功」，真相是「这个格式我们不认」—— 他会去翻自己的文件语法。
    // 三个错误码的语义边界本来就划好了（见 `error.rs`），只是这一处兑现错了
    if value.is_array()
        || value.get("reqly").is_some()
        || value.get("folders").is_some()
        || value.get("requests").is_some()
        || value.get("environments").is_some()
    {
        return Some(Format::Reqly);
    }
    None
}

/// 是不是 ApiPost 桌面端「导出项目」的原生结构。
///
/// 判据得同时落在**嵌套层**上：这份结构顶层只有 `project` / `apis` / `envs` / `models`
/// 这四个键，`target_type` 藏在 `apis[]` 的元素里。放在 mod.rs 是因为
/// [`detect`] 与 `postman::parse` 两边都要用它 —— 各写一份迟早会分叉成两套判据。
pub(crate) fn is_apipost_export(value: &serde_json::Value) -> bool {
    let Some(apis) = value.get("apis").and_then(serde_json::Value::as_array) else {
        return false;
    };
    // `apis` + `envs`/`project` 是官方导出的固定搭配；
    // 再退一步，只要有一个节点带着 `target_type` 也认
    value.get("envs").is_some()
        || value.get("project").is_some()
        || apis.iter().any(|item| item.get("target_type").is_some())
}

/// 外部文件里的一份授权配置，翻译成 reqly 的**行**。
///
/// 这就是「授权不是独立概念」在导入器里的落点：外部格式仍然有独立的 auth 节点，
/// 但它在解析阶段就被翻译成请求头 / 查询参数，落库时和用户手写的行没有区别。
///
/// ## 为什么分成两个数组
///
/// 因为落点确实有两处：`Authorization` 是请求头，`?api_key=x` 是查询参数。
/// 若只给一个 `Vec<KeyValueItem>`，就得再加一个「这行该放哪儿」的标记 ——
/// 那等于把刚拆掉的枚举换个名字装回来。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuthRows {
    pub headers: Vec<KeyValueItem>,
    pub params: Vec<KeyValueItem>,
}

/// Basic 凭据的**模板值**：`{{$basic "U" "P"}}`。
///
/// **只在拼这一处**。这个拼法是模板函数 `$basic` 的输入契约
/// （见 `template::functions::split_two_quoted`：两个成对的同种引号、中间可空白、
/// 后面不许有别的东西），散在多处写，某处少一个引号就变成「值原样发出去」——
/// 而那种错误在界面上看不出来，服务端只会回 401。
///
/// 为什么不在这里直接算出 base64：模板值里留着账号原文，用户在主面板上
/// 能看清这条请求用的是哪个账号；展开发生在发送前一刻，算出来的
/// `Basic <base64>` 与 curl / 浏览器自己算的逐字节一致。
pub(crate) fn basic_auth_value(username: &str, password: &str) -> String {
    format!("{{{{$basic \"{username}\" \"{password}\"}}}}")
}

/// 把「继承来的授权行」并进请求草稿。
///
/// **目标已有同名的行就跳过**（头名 / 参数键不分大小写）。判据与 007 迁移
/// 逐字相同：用户手写的优先，自动展开的不许覆盖它 ——
/// 反过来写的代价是「导入一次，用户的 `Authorization` 被文件里的旧 token 静默换掉」。
///
/// 追加而不是覆盖也意味着：反复导入同一份文件不会让行数越滚越多。
pub(crate) fn merge_auth_rows(draft: &mut RequestDraft, rows: &AuthRows) {
    for row in &rows.params {
        if !draft
            .params
            .iter()
            .any(|existing| existing.key.eq_ignore_ascii_case(&row.key))
        {
            draft.params.push(row.clone());
        }
    }
    for row in &rows.headers {
        if !draft
            .headers
            .iter()
            .any(|existing| existing.key.eq_ignore_ascii_case(&row.key))
        {
            draft.headers.push(row.clone());
        }
    }
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

/// 百分号编码。postman 与 har 都要把键值对拼成 urlencoded 串，
/// 所以规则只留这一份 —— 这种函数一旦有两份，迟早有一份把空格编成 `+`，
/// 而那种错在导入时看不出来。
pub(crate) fn urlencode(raw: &str) -> String {
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

/// JSON 里的值 → 字符串。数字 / 布尔照字面量转，`null` 当空串。
/// 导入的文件是外面来的，`"value": 1` 这种地方不该让整条解不出来。
pub(crate) fn value_to_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
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

    /// ApiPost 桌面端「导出项目」：`target_type` 全在 `apis[]` 里，顶层一个都没有。
    /// 早先只认顶层 `apipost` / `target_type`，这一种会掉进 reqly 兜底 → 静默 0 条
    #[test]
    fn detects_apipost_desktop_export() {
        let input = ImportInput::text(
            r#"{"project":{"name":"默认离线项目"},"apis":[{"target_type":"folder","name":"f","children":[]}],"envs":[],"models":[]}"#,
        );
        assert_eq!(detect(&input).unwrap(), Format::Apipost);
    }

    #[test]
    fn detects_apipost_export_without_the_project_key() {
        // 只有 `apis` + 带 `target_type` 的元素也认（`envs` / `project` 都可能被裁剪掉）
        let input = ImportInput::text(r#"{"apis":[{"target_type":"api","name":"a"}],"envs":[]}"#);
        assert_eq!(detect(&input).unwrap(), Format::Apipost);
    }

    #[test]
    fn apis_key_alone_is_not_enough_to_claim_apipost() {
        // 只有 `apis`，既没有 `envs`/`project`，元素也不带 `target_type`
        // → 不误判成 apipost
        let input = ImportInput::text(r#"{"apis":[{"name":"a","url":"https://x"}]}"#);
        assert_eq!(detect(&input).unwrap_err().code(), codes::IMPORT_DETECT);
    }

    /// 认不出的 JSON 必须**报错**，不能落到 reqly 兜底。
    ///
    /// 兜底的后果是 `reqly.rs` 因为字段全带 `#[serde(default)]` 而「解析成功、内容为空」，
    /// 界面显示「已导入 0 个接口」—— 看着像文件坏了，真相是这个格式还不支持。
    #[test]
    fn unknown_json_reports_detect_failure_instead_of_falling_back() {
        let input = ImportInput::text(r#"{"swagger":"2.0","paths":{}}"#);
        assert_eq!(detect(&input).unwrap_err().code(), codes::IMPORT_DETECT);
    }

    #[test]
    fn reqly_shapes_are_still_recognised() {
        for text in [
            r#"{"reqly":1,"name":"c"}"#,
            r#"{"name":"c","folders":[]}"#,
            r#"{"requests":[{"name":"a","url":"https://x"}]}"#,
            r#"{"environments":[]}"#,
            r#"[{"name":"a","url":"https://x"}]"#,
        ] {
            assert_eq!(
                detect(&ImportInput::text(text)).unwrap(),
                Format::Reqly,
                "{text} 应该还认得出来"
            );
        }
    }

    #[test]
    fn detects_har_by_log_entries() {
        let input = ImportInput::text(r#"{"log":{"version":"1.2","entries":[]}}"#);
        assert_eq!(detect(&input).unwrap(), Format::Har);
    }

    #[test]
    fn har_format_string_matches_the_frontend_contract() {
        // 前端按这个字符串切文案（src/lib/schemas.ts 的 importFormatSchema）
        assert_eq!(Format::Har.as_str(), "har");
        assert_eq!(
            serde_json::to_value(Format::Har).unwrap(),
            serde_json::json!("har")
        );
    }

    #[test]
    fn urlencode_keeps_the_unreserved_set_only() {
        assert_eq!(urlencode("a b"), "a%20b");
        assert_eq!(urlencode("-_.~"), "-_.~");
        assert_eq!(urlencode("张"), "%E5%BC%A0");
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
