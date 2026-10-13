//! curl 命令 → 请求草稿。
//!
//! ## 支持范围
//!
//! 参数表以 `design.md` §5.1 为准，实现时按「**用户的下一个动作**」分了三档：
//!
//! | 档 | 处理 | 例子 |
//! |---|---|---|
//! | 影响请求语义 | 映射到具体字段 | `-X` `-H` `-d` `-F` `-u` `-b` `-A` `--url` `-k` `-L` `--max-time` `-x` |
//! | 纯输出/交互开关 | **静默忽略**（改了它请求也不变） | `-s` `-v` `-i` `--no-buffer` `-#` `-f` |
//! | 我们不支持的能力 | 忽略 + 进警告列表 | `--cacert` `--resolve` `-T` `@文件` |
//!
//! 第二档刻意**不报警告**：Chrome 的「Copy as cURL」经常带 `--compressed`，
//! 每次导入都弹一条无害警告，警告就不值钱了。
//! `--compressed` 连语义都不缺 —— reqwest 默认就接受并自动解压 gzip/br/deflate。
//!
//! ## 两个「忠实优先」的决定
//!
//! 1. **query 不拆进 `params[]`**：拆开再拼回去要保证编码逐字节一致
//!    （`%2F` 不能被解码、`+` 不能被当成空格、重复 key 的顺序不能变），
//!    拆错一个字请求就变了。验收标准是「Headers 与 body 100% 还原」，URL 更是如此 ——
//!    所以原样留在 `url` 里。发送侧在 `params` 为空时不会碰 URL（`apply_query_params` 提前返回）。
//! 2. **不跟随重定向（curl 的真实默认）**：curl 只有给了 `-L` 才跟随，
//!    而 reqly 的 `RequestSettings` 默认是跟随。这里按 curl 的语义写回 `false` ——
//!    粘一条 curl 进来却得到跟 curl 不一样的结果，是最难查的那种「不一致」。

use serde_json::Value;

use super::tokenizer::{self, Token};
use super::{
    basic_auth_value, merge_auth_rows, AuthRows, ImportBundle, ImportWarning, RequestDraft,
    WarningKind,
};
use crate::error::{codes, AppError, AppResult};
use crate::storage::models::KeyValueItem;

pub fn parse(text: &str) -> AppResult<ImportBundle> {
    let commands = tokenizer::tokenize_commands(text).map_err(|err| {
        AppError::http_with(
            err.code(),
            err.debug(),
            serde_json::json!({ "reason": "lex" }),
        )
    })?;

    let commands: Vec<Vec<Token>> = commands.into_iter().filter(|c| !c.is_empty()).collect();
    if commands.is_empty() {
        return Err(AppError::http_with(
            codes::IMPORT_DETECT,
            "no curl command found".to_string(),
            serde_json::json!({ "reason": "empty" }),
        ));
    }

    let mut bundle = ImportBundle::default();
    for command in &commands {
        match parse_one(command) {
            Ok((draft, warnings)) => {
                bundle.requests.push(draft);
                bundle.warnings.extend(warnings);
            }
            Err(err) => {
                // 一条坏了不该让整批都进不来：记一条警告，继续解下一条
                bundle.push_warning(ImportWarning::simple(
                    WarningKind::SkippedItem,
                    err.to_string(),
                ));
            }
        }
    }

    super::dedupe_names(&mut bundle);
    Ok(bundle)
}

/// 解析一条命令。
fn parse_one(tokens: &[Token]) -> AppResult<(RequestDraft, Vec<ImportWarning>)> {
    let mut warnings = Vec::new();
    let mut state = Args::default();

    // 从终端复制时行首会带 `$ ` / `> ` 这类提示符 —— 只在「开头那串非选项的词」里找 curl
    let start = tokens
        .iter()
        .take_while(|t| !t.text.starts_with('-'))
        .position(Token::is_curl_invocation)
        .map(|index| index + 1)
        .unwrap_or(0);

    let args = &tokens[start..];
    let mut i = 0;
    while i < args.len() {
        let token = &args[i];
        let (flag, inline) = split_flag(&token.text);
        i += 1;

        // ---- 位置参数：URL ----
        if flag.is_empty() {
            state.urls.push(token.text.clone());
            continue;
        }

        let spec = SPECS.iter().find(|s| s.matches(&flag));
        let Some(spec) = spec else {
            // 单个短横线开头的可能是组合布尔项（`-sS`），逐个字符再看一遍
            if is_stacked_short_flags(&flag) {
                continue;
            }
            warnings.push(ImportWarning::simple(
                WarningKind::UnknownArgument,
                format!("unknown argument `{flag}`"),
            ));
            continue;
        };

        let value = if spec.takes_value {
            match inline {
                Some(value) => Some(value),
                None => match args.get(i) {
                    Some(next) => {
                        i += 1;
                        Some(next.text.clone())
                    }
                    None => None,
                },
            }
        } else {
            None
        };

        if spec.takes_value && value.is_none() {
            warnings.push(ImportWarning::simple(
                WarningKind::MissingArgumentValue,
                format!("`{flag}` needs a value"),
            ));
            continue;
        }

        state.apply(spec, value, &mut warnings);
    }

    let draft = state.finish(&mut warnings)?;
    Ok((draft, warnings))
}

/// 把 `--flag=value` 拆开；短选项的粘连值（`-d{...}`）也认。
fn split_flag(text: &str) -> (String, Option<String>) {
    if let Some(rest) = text.strip_prefix("--") {
        return match rest.split_once('=') {
            Some((name, value)) => (format!("--{name}"), Some(value.to_string())),
            None => (text.to_string(), None),
        };
    }
    if let Some(rest) = text.strip_prefix('-') {
        // `-d{...}` / `-XPOST`：前两个字符是选项名
        let mut chars = rest.chars();
        if let (Some(name), Some(_)) = (chars.next(), chars.next()) {
            let attached: String = rest.chars().skip(1).collect();
            if !attached.is_empty() && !rest.starts_with('-') {
                return (format!("-{name}"), Some(attached));
            }
        }
        return (text.to_string(), None);
    }
    (String::new(), None)
}

fn is_stacked_short_flags(flag: &str) -> bool {
    flag.starts_with('-')
        && !flag.starts_with("--")
        && flag.len() > 2
        && flag[1..].chars().all(|c| BOOLEAN_SHORT_FLAGS.contains(c))
}

/// 不吃值的短选项（组合写法的合法性判断用这个集合）
const BOOLEAN_SHORT_FLAGS: &str = "sSviNfkgL#";

/// 一个 curl 参数的规格。
struct Spec {
    long: &'static str,
    shorts: &'static [&'static str],
    takes_value: bool,
    /// 怎么落地
    action: Action,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Request,
    Header,
    Data,
    DataUrlencode,
    Form,
    User,
    Cookie,
    UserAgent,
    Referer,
    Url,
    NoVerifyTls,
    Location,
    MaxTime,
    Proxy,
    /// 纯输出开关：忽略且不报警告
    Ignore,
    /// 我们还不支持：忽略但要说出来
    Unsupported,
}

impl Spec {
    fn matches(&self, flag: &str) -> bool {
        flag == self.long || self.shorts.contains(&flag)
    }
}

macro_rules! spec {
    ($long:literal, [$($short:literal),*], $takes:literal, $action:expr) => {
        Spec { long: $long, shorts: &[$($short),*], takes_value: $takes, action: $action }
    };
}

/// 参数表。加参数只改这里 —— 与 design.md §5.1 一一对应。
static SPECS: &[Spec] = &[
    spec!("--request", ["-X"], true, Action::Request),
    spec!("--header", ["-H"], true, Action::Header),
    spec!("--data", ["-d"], true, Action::Data),
    spec!("--data-raw", [], true, Action::Data),
    spec!("--data-binary", [], true, Action::Data),
    spec!("--data-ascii", [], true, Action::Data),
    spec!("--data-urlencode", [], true, Action::DataUrlencode),
    spec!("--form", ["-F"], true, Action::Form),
    spec!("--form-string", [], true, Action::Form),
    spec!("--user", ["-u"], true, Action::User),
    spec!("--cookie", ["-b"], true, Action::Cookie),
    spec!("--user-agent", ["-A"], true, Action::UserAgent),
    spec!("--referer", ["-e"], true, Action::Referer),
    spec!("--url", [], true, Action::Url),
    spec!("--max-time", ["-m"], true, Action::MaxTime),
    spec!("--proxy", ["-x"], true, Action::Proxy),
    spec!("--location", ["-L"], false, Action::Location),
    spec!("--insecure", ["-k"], false, Action::NoVerifyTls),
    // ---- 纯输出 / 交互开关：忽略，不报警告 ----
    spec!("--silent", ["-s"], false, Action::Ignore),
    spec!("--show-error", ["-S"], false, Action::Ignore),
    spec!("--verbose", ["-v"], false, Action::Ignore),
    spec!("--include", ["-i"], false, Action::Ignore),
    spec!("--head", ["-I"], false, Action::Ignore),
    spec!("--fail", ["-f"], false, Action::Ignore),
    spec!("--compressed", [], false, Action::Ignore),
    spec!("--no-buffer", ["-N"], false, Action::Ignore),
    spec!("--progress-bar", ["-#"], false, Action::Ignore),
    spec!("--globoff", ["-g"], false, Action::Ignore),
    spec!("--no-keepalive", [], false, Action::Ignore),
    spec!("--tlsv1.2", [], false, Action::Ignore),
    spec!("--http1.1", [], false, Action::Ignore),
    spec!("--http2", [], false, Action::Ignore),
    spec!("--path-as-is", [], false, Action::Ignore),
    spec!("--raw", [], false, Action::Ignore),
    // ---- 吃值但我们用不上 ----
    spec!("--output", ["-o"], true, Action::Ignore),
    spec!("--write-out", ["-w"], true, Action::Ignore),
    spec!("--dump-header", ["-D"], true, Action::Ignore),
    spec!("--cookie-jar", ["-c"], true, Action::Ignore),
    spec!("--retry", [], true, Action::Ignore),
    spec!("--limit-rate", [], true, Action::Ignore),
    spec!("--interface", [], true, Action::Ignore),
    // ---- 我们还不支持的能力 ----
    spec!("--connect-timeout", [], true, Action::Unsupported),
    spec!("--cacert", [], true, Action::Unsupported),
    spec!("--capath", [], true, Action::Unsupported),
    spec!("--cert", ["-E"], true, Action::Unsupported),
    spec!("--key", [], true, Action::Unsupported),
    spec!("--resolve", [], true, Action::Unsupported),
    spec!("--upload-file", ["-T"], true, Action::Unsupported),
    spec!("--ntlm", [], false, Action::Unsupported),
    spec!("--digest", [], false, Action::Unsupported),
    spec!("--negotiate", [], false, Action::Unsupported),
    spec!("--cert-type", [], true, Action::Unsupported),
];

#[derive(Default)]
struct Args {
    method: Option<String>,
    urls: Vec<String>,
    headers: Vec<KeyValueItem>,
    user: Option<String>,
    /// `-d*` 系列，curl 会用 `&` 把它们接起来
    data: Vec<String>,
    /// `-F*` 系列
    form: Vec<String>,
    follow_redirects: Option<bool>,
    verify_tls: Option<bool>,
    timeout_ms: Option<u64>,
    proxy: Option<String>,
}

impl Args {
    fn apply(&mut self, spec: &Spec, value: Option<String>, warnings: &mut Vec<ImportWarning>) {
        let value = value.unwrap_or_default();
        match spec.action {
            Action::Request => self.method = Some(value.to_ascii_uppercase()),
            Action::Header => push_header(&mut self.headers, &value, warnings),
            Action::Data => self.data.push(value),
            Action::DataUrlencode => self.data.push(data_urlencode(&value, warnings)),
            Action::Form => self.form.push(value),
            Action::User => self.user = Some(value),
            Action::Cookie => {
                // `-b` 有两种形态：cookie 串（`k=v; k2=v2`）与 Netscape cookie 文件
                if value.contains('=') {
                    push_header(&mut self.headers, &format!("Cookie: {value}"), warnings);
                } else {
                    warnings.push(ImportWarning::simple(
                        WarningKind::UnsupportedField,
                        format!("cookie file `{value}` is not supported"),
                    ));
                }
            }
            Action::UserAgent => {
                push_header(&mut self.headers, &format!("User-Agent: {value}"), warnings)
            }
            Action::Referer => {
                push_header(&mut self.headers, &format!("Referer: {value}"), warnings)
            }
            Action::Url => self.urls.push(value),
            Action::NoVerifyTls => self.verify_tls = Some(false),
            Action::Location => self.follow_redirects = Some(true),
            Action::MaxTime => match value.parse::<f64>() {
                Ok(seconds) if seconds >= 0.0 => {
                    self.timeout_ms = Some((seconds * 1000.0).round() as u64)
                }
                _ => warnings.push(ImportWarning::simple(
                    WarningKind::MissingArgumentValue,
                    format!("`--max-time {value}` is not a number"),
                )),
            },
            Action::Proxy => self.proxy = Some(value),
            Action::Ignore => {}
            Action::Unsupported => warnings.push(ImportWarning::simple(
                WarningKind::UnsupportedField,
                format!("`{}` is not supported yet", spec.long),
            )),
        }
    }

    fn finish(self, warnings: &mut Vec<ImportWarning>) -> AppResult<RequestDraft> {
        let url = self.urls.first().cloned().ok_or_else(|| {
            AppError::http(codes::IMPORT_PARSE, "curl command has no url".to_string())
        })?;
        if self.urls.len() > 1 {
            warnings.push(ImportWarning::simple(
                WarningKind::UnsupportedField,
                format!("extra urls dropped: {}", self.urls[1..].join(", ")),
            ));
        }

        let name = describe(&url);

        // ---- 方法推导 ----
        //
        // 必须照抄 curl 自己的规则，不能一律 GET：
        // 浏览器的「Copy as cURL」在 POST 时**不给 `-X`**，只留一个 `--data-raw`，
        // 靠 curl 的默认把它变成 POST。我们要是落回 GET，
        // 粘进来的一条 POST 就静默变成 GET —— body 还在，方法变了，
        // 打出去是完全不同的接口行为，而且界面上一眼看不出哪里不对。
        //
        // 规则（curl 手册）：`-X` 最高优先；否则给了 `--data*` / `-F`
        // 就是 POST（`-G` 会把 data 挪进 query 并强制 GET，但我们不认 `-G`，
        // 它会落到「未知参数」警告里，不会被静默当成 POST）；
        // 都没有才是 GET。`-I`（HEAD）目前也在忽略列表里，同样是 GET。
        let method = match self.method.clone() {
            Some(explicit) => explicit,
            None if !self.data.is_empty() || !self.form.is_empty() => "POST".to_string(),
            None => "GET".to_string(),
        };

        // curl 的默认是「不跟随重定向」，这里必须写回 false 才与粘贴进来的命令一致；
        // 其余字段只有命令行真的给了才覆盖 —— 用结构体更新语法让「没给」自然落回默认值。
        let defaults = crate::storage::models::RequestSettings::default();
        let settings = crate::storage::models::RequestSettings {
            follow_redirects: self.follow_redirects.unwrap_or(false),
            verify_tls: self.verify_tls.unwrap_or(defaults.verify_tls),
            timeout_ms: self.timeout_ms.unwrap_or(defaults.timeout_ms),
            proxy: self.proxy.clone(),
            ..defaults
        };

        // `-u user:pass` → 一条 `Authorization: Basic …` 请求头。
        //
        // 存的是**模板**而不是算好的 base64：展开发生在发送前一刻
        // （见 `template::functions::basic`），所以界面上看得见用的是哪个账号，
        // 而发出去的值与 curl 自己算的逐字节一致。
        // 落点规则与 007 迁移、Postman 导入器共用同一份拼法（`basic_auth_value`）。
        let auth_rows = match self.user.as_deref() {
            Some(user) => {
                let (username, password) = match user.split_once(':') {
                    // curl 的 `-u user` 等价于空密码（会往服务端发 `user:`）
                    Some((u, p)) => (u.to_string(), p.to_string()),
                    None => (user.to_string(), String::new()),
                };
                AuthRows {
                    headers: vec![KeyValueItem::new(
                        "Authorization",
                        basic_auth_value(&username, &password),
                    )],
                    params: Vec::new(),
                }
            }
            None => AuthRows::default(),
        };

        let (body_type, body) = assemble_body(&self, warnings);

        let mut draft = RequestDraft {
            name,
            method,
            url,
            params: Vec::new(),
            headers: self.headers,
            body_type,
            body,
            settings,
        };
        // 手写的同名列优先：`-u` 与 `-H 'Authorization: …'` 同时给的时候，
        // 用户显式写的那条赢（判据见 `merge_auth_rows`，与 Postman 那边同一份）
        merge_auth_rows(&mut draft, &auth_rows);
        Ok(draft)
    }
}

/// 由 URL 生成请求名。
///
/// 只负责取名字 —— 方法**不从这里推断**：名字是路径末段，
/// 与「这条命令会发什么方法」没有任何关系（`POST /users` 与
/// `GET /users` 的末段都是 `users`）。
fn describe(url: &str) -> String {
    // 去掉 query 与锚点之后再取路径末段
    let without_anchor = url.split('#').next().unwrap_or(url);
    let (path, _query) = without_anchor
        .split_once('?')
        .unwrap_or((without_anchor, ""));

    let trimmed = path.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or("");
    if last.is_empty() {
        // `https://api.example.com/` 这种只有域名的情况
        trimmed
            .split("//")
            .nth(1)
            .unwrap_or(trimmed)
            .split('/')
            .next()
            .unwrap_or("")
            .to_string()
    } else {
        // 路径末段可能带扩展名，留着更认得出（`users.json`）
        last.to_string()
    }
}

fn assemble_body(args: &Args, warnings: &mut Vec<ImportWarning>) -> (String, String) {
    // ---- multipart ----
    if !args.form.is_empty() {
        if !args.data.is_empty() {
            warnings.push(ImportWarning::simple(
                WarningKind::UnsupportedField,
                "both --data and --form were given; --data dropped".to_string(),
            ));
        }
        // 逐行存 `key=value`（`@file` 原样保留）—— P8 做 multipart 时数据已经在对的位置
        warnings.push(ImportWarning::simple(
            WarningKind::UnsupportedField,
            "multipart body is kept as `key=value` lines but cannot be sent yet".to_string(),
        ));
        return ("form".to_string(), args.form.join("\n"));
    }

    // ---- 普通 body ----
    if args.data.is_empty() {
        return ("none".to_string(), String::new());
    }
    let body = args.data.join("&");

    let content_type = args
        .headers
        .iter()
        .find(|h| h.key.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.to_ascii_lowercase());

    let body_type = match content_type.as_deref() {
        Some(ct) if ct.contains("json") => match serde_json::from_str::<Value>(&body) {
            // 只有真能解析成 JSON 才归到 `json`：那一档发送前会做本地校验，
            // 归错了会把一条本来能发出去的请求变成 E_BODY_NOT_JSON
            Ok(_) => "json",
            Err(_) => "raw",
        },
        Some(ct) if ct.contains("xml") || ct.contains("graphql") => "raw",
        Some(ct) if ct.contains("urlencoded") => "urlencoded",
        Some(_) => "raw",
        // curl 对 `-d` 的默认就是 urlencoded —— 我们发送时补的默认头与它一致
        None => "urlencoded",
    };

    (body_type.to_string(), body)
}

fn push_header(headers: &mut Vec<KeyValueItem>, raw: &str, warnings: &mut Vec<ImportWarning>) {
    if let Some(rest) = raw.strip_prefix('@') {
        warnings.push(ImportWarning::simple(
            WarningKind::UnsupportedField,
            format!("header file `{rest}` is not supported"),
        ));
        return;
    }
    let Some((key, value)) = raw.split_once(':') else {
        warnings.push(ImportWarning::simple(
            WarningKind::UnsupportedField,
            format!("header `{raw}` has no `:`"),
        ));
        return;
    };
    let key = key.trim();
    if key.is_empty() {
        warnings.push(ImportWarning::simple(
            WarningKind::UnsupportedField,
            format!("header `{raw}` has an empty name"),
        ));
        return;
    }
    // `name: value` 里的那个空格是分隔符，不是值的一部分（curl 自己也会 trim 前导空格）
    headers.push(KeyValueItem::new(key, value.trim_start()));
}

/// `--data-urlencode` 的几种写法（curl 手册）：
///
/// * `content` → 整段编码
/// * `=content` → 编码后前面补一个 `=`（无名字的字段）
/// * `name=content` → **名字原样**，只编码内容
/// * `@file` / `name@file` → 要读磁盘
///
/// 最后一种在导入阶段读不到那个文件（还不一定在本机），所以原样保留 + 警告，
/// 而不是猜一个空值 —— 猜错了是「请求发出去了但内容是错的」，最难查。
fn data_urlencode(raw: &str, warnings: &mut Vec<ImportWarning>) -> String {
    let (prefix, content) = match raw.split_once('=') {
        Some((name, content)) => (format!("{name}="), content),
        None => (String::new(), raw),
    };

    let file = if let Some(path) = content.strip_prefix('@') {
        Some(path)
    } else if prefix.is_empty() {
        // `name@file`：没有 `=` 的情况下，`@` 后面才是文件名
        raw.split_once('@').map(|(_, path)| path)
    } else {
        None
    };
    if let Some(path) = file {
        warnings.push(ImportWarning::simple(
            WarningKind::UnsupportedField,
            format!("`--data-urlencode {raw}` reads `{path}` from disk; kept as literal text"),
        ));
        return raw.to_string();
    }

    format!("{prefix}{}", urlencode_form(content))
}

/// `--data-urlencode` 的编码规则：非保留字符原样，其余 `%XX`（空格用 `%20`，不是 `+`）
fn urlencode_form(raw: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn one(input: &str) -> RequestDraft {
        let bundle = parse(input).expect("应该能解析");
        assert_eq!(bundle.requests.len(), 1, "这个用例只该出一条请求");
        bundle.requests[0].clone()
    }

    fn header<'a>(draft: &'a RequestDraft, key: &str) -> Option<&'a str> {
        draft
            .headers
            .iter()
            .find(|h| h.key.eq_ignore_ascii_case(key))
            .map(|h| h.value.as_str())
    }

    // ============================================================
    // 验收标准：三种浏览器的「Copy as cURL」，Headers 与 body 100% 还原
    // ============================================================

    #[test]
    fn chrome_bash_copy_is_restored_exactly() {
        let input = "curl 'https://api.example.com/v1/users?page=1&size=20' \\\n  \
             -H 'accept: application/json' \\\n  \
             -H 'content-type: application/json' \\\n  \
             -H 'authorization: Bearer eyJhbGciOiJIUzI1NiJ9.abc' \\\n  \
             --data-raw '{\"name\":\"张三\",\"tags\":[\"a\",\"b\"]}'";

        let draft = one(input);
        assert_eq!(draft.url, "https://api.example.com/v1/users?page=1&size=20");
        // 浏览器没给 `-X`，但它给了 `--data-raw` —— curl 的默认把这条变成 POST。
        // 落回 GET 就会把一条 POST 静默当成 GET 发出去
        assert_eq!(draft.method, "POST");
        assert_eq!(header(&draft, "accept"), Some("application/json"));
        assert_eq!(header(&draft, "content-type"), Some("application/json"));
        assert_eq!(
            header(&draft, "authorization"),
            Some("Bearer eyJhbGciOiJIUzI1NiJ9.abc")
        );
        assert_eq!(draft.body, r#"{"name":"张三","tags":["a","b"]}"#);
        assert_eq!(draft.body_type, "json");
        assert_eq!(draft.name, "users");
    }

    #[test]
    fn firefox_bash_copy_is_restored_exactly() {
        let input = "curl 'https://httpbin.org/post' --compressed \\\n  \
             -X POST \\\n  \
             -H 'User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64)' \\\n  \
             -H 'Accept: */*' \\\n  \
             -H 'Content-Type: application/x-www-form-urlencoded' \\\n  \
             -H 'Cookie: sid=abc123; theme=dark' \\\n  \
             --data-raw 'x=1&y=%E4%B8%AD'";

        let draft = one(input);
        assert_eq!(draft.method, "POST");
        assert_eq!(draft.body_type, "urlencoded");
        assert_eq!(draft.body, "x=1&y=%E4%B8%AD");
        // `%E4%B8%AD` 必须逐字节保留 —— 解码再编码就会变成别的字节序列
        assert_eq!(header(&draft, "cookie"), Some("sid=abc123; theme=dark"));
        assert_eq!(
            header(&draft, "user-agent"),
            Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        );
    }

    #[test]
    fn cmd_copy_is_restored_exactly() {
        let input = "curl \"https://api.example.com/v1/orders\" ^\n  \
             -H \"accept: application/json\" ^\n  \
             -H \"content-type: application/json\" ^\n  \
             --data-raw \"{\\\"amount\\\":12.5}\" ^\n  \
             --compressed";
        let draft = one(input);
        assert_eq!(draft.url, "https://api.example.com/v1/orders");
        assert_eq!(draft.body, r#"{"amount":12.5}"#);
        assert_eq!(draft.body_type, "json");
        assert_eq!(header(&draft, "accept"), Some("application/json"));
    }

    #[test]
    fn powershell_copy_is_restored_exactly() {
        let input = "curl.exe \"https://api.example.com/v1/users\" `\n  \
             -H \"accept: application/json\" `\n  \
             -H \"authorization: Bearer abc\"";
        let draft = one(input);
        assert_eq!(draft.url, "https://api.example.com/v1/users");
        assert_eq!(header(&draft, "authorization"), Some("Bearer abc"));
    }

    // ============================================================
    // 验收标准：三种浏览器的「Copy as cURL」，Headers 与 body 100% 还原
    // ============================================================

    /// Edge（Windows）。
    ///
    /// Edge 与 Chrome 是同一个 Chromium 转义生成器，所以形状一致 ——
    /// 但这条用例走的是 `^"` **把引号也转义掉**的那个形态，
    /// 此前只有 tokenizer 层的词法覆盖（`caret_escaped_quotes_can_open_and_close`），
    /// curl 层端到端没有。同时它带着 Edge 自己的 `Edg/` 尾巴与 `sec-ch-ua`，
    /// 顺带验证 UA 里的括号 / 斜杠 / 空格不被撕。
    #[test]
    fn edge_cmd_copy_is_restored_exactly() {
        let input = "curl ^\"https://httpbin.org/post^\" ^\n  \
             -H ^\"accept: application/json^\" ^\n  \
             -H ^\"content-type: application/json^\" ^\n  \
             -H ^\"sec-ch-ua: \\\"Chromium\\\";v=\\\"126\\\", \\\"Microsoft Edge\\\";v=\\\"126\\\", \\\"Not.A/Brand\\\";v=\\\"24\\\"^\" ^\n  \
             -H ^\"user-agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0^\" ^\n  \
             --data-raw ^\"{\\\"orderId\\\":7,\\\"tags\\\":[\\\"a\\\",\\\"b\\\"]}^\" ^\n  \
             --compressed";
        let draft = one(input);

        assert_eq!(draft.url, "https://httpbin.org/post");
        assert_eq!(draft.method, "POST");
        assert_eq!(draft.name, "post");
        assert_eq!(draft.body_type, "json");
        assert_eq!(draft.body, r#"{"orderId":7,"tags":["a","b"]}"#);
        assert_eq!(header(&draft, "accept"), Some("application/json"));
        assert_eq!(
            header(&draft, "sec-ch-ua"),
            Some(r#""Chromium";v="126", "Microsoft Edge";v="126", "Not.A/Brand";v="24""#)
        );
        assert_eq!(
            header(&draft, "user-agent"),
            Some(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36 Edg/126.0.0.0"
            )
        );
        // `--compressed` 属于「静默忽略」，不该冒出警告
        let bundle = parse(input).unwrap();
        assert!(bundle.warnings.is_empty(), "{:?}", bundle.warnings);
    }

    // ============================================================
    // 方法推导（curl 的规则，不照抄就会把 POST 静默变成 GET）
    // ============================================================

    /// 有 body、没有 `-X` → POST。这是浏览器「Copy as cURL」的默认形态
    #[test]
    fn body_without_an_explicit_method_becomes_post() {
        assert_eq!(one("curl 'https://x/y' -d a=1").method, "POST");
        assert_eq!(one("curl 'https://x/y' --data-raw '{}'").method, "POST");
        assert_eq!(
            one("curl 'https://x/y' --data-urlencode 'a=1'").method,
            "POST"
        );
    }

    #[test]
    fn form_without_an_explicit_method_becomes_post() {
        assert_eq!(one("curl 'https://x/y' -F a=1").method, "POST");
    }

    #[test]
    fn no_body_without_an_explicit_method_stays_get() {
        assert_eq!(one("curl 'https://x/y'").method, "GET");
        assert_eq!(one("curl 'https://x/y' -H 'accept: */*'").method, "GET");
    }

    /// `-X` 最高优先：给了就照给的走，body 不参与决策
    #[test]
    fn explicit_method_wins_over_the_body() {
        assert_eq!(one("curl -X DELETE 'https://x/y'").method, "DELETE");
        assert_eq!(one("curl -X PUT 'https://x/y' -d a=1").method, "PUT");
        assert_eq!(one("curl -X GET 'https://x/y' -d a=1").method, "GET");
        // 小写也要归一
        assert_eq!(one("curl -X patch 'https://x/y'").method, "PATCH");
    }

    // ============================================================
    // 参数映射
    // ============================================================

    /// `-u user:pass` 落成一条 `Authorization` 请求头 —— 凭据不再是一个独立字段。
    ///
    /// 值存成 `{{$basic …}}` 模板：展开发生在发送前一刻，算出来的
    /// `Basic <base64>` 与 curl 自己发的逐字节一致；而界面上看得见账号。
    /// 与 007 迁移、Postman 导入器同一条落点规则。
    #[test]
    fn basic_auth_becomes_an_authorization_header() {
        let draft = one("curl -u alice:s3cret 'https://x/y'");
        assert_eq!(
            draft.headers.len(),
            1,
            "应当只有这一条头：{:?}",
            draft.headers
        );
        assert_eq!(draft.headers[0].key, "Authorization");
        assert_eq!(draft.headers[0].value, r#"{{$basic "alice" "s3cret"}}"#);
    }

    /// 手写的 `Authorization` 优先于 `-u` —— 两条都发出去等于让服务端替我们挑
    #[test]
    fn a_handwritten_authorization_header_wins_over_the_user_flag() {
        let draft = one("curl -u alice:s3cret -H 'Authorization: Bearer x' 'https://x/y'");
        assert_eq!(draft.headers.len(), 1, "{:?}", draft.headers);
        assert_eq!(draft.headers[0].value, "Bearer x");
    }

    #[test]
    fn insecure_and_location_and_max_time_map_to_settings() {
        let draft = one("curl -k -L --max-time 2.5 'https://x/y'");
        assert!(!draft.settings.verify_tls);
        assert!(draft.settings.follow_redirects);
        assert_eq!(draft.settings.timeout_ms, 2500);
    }

    /// curl 的默认是**不跟随**重定向 —— 没给 `-L` 就必须写回 false
    #[test]
    fn redirects_default_to_off_like_curl() {
        assert!(!one("curl 'https://x/y'").settings.follow_redirects);
    }

    #[test]
    fn proxy_maps_to_settings() {
        let draft = one("curl -x http://127.0.0.1:7890 'https://x/y'");
        assert_eq!(
            draft.settings.proxy.as_deref(),
            Some("http://127.0.0.1:7890")
        );
    }

    #[test]
    fn multiple_data_parts_are_joined_with_ampersand() {
        let draft = one("curl 'https://x/y' -d a=1 -d b=2");
        assert_eq!(draft.body, "a=1&b=2");
    }

    #[test]
    fn data_urlencode_percent_encodes() {
        let draft = one("curl 'https://x/y' --data-urlencode 'name=张 三&x'");
        assert_eq!(draft.body, "name=%E5%BC%A0%20%E4%B8%89%26x");
    }

    /// `=content` 形式（无名字段）：名字是空的，但那个 `=` 要留下
    #[test]
    fn data_urlencode_without_a_name() {
        let draft = one("curl 'https://x/y' --data-urlencode '=a b'");
        assert_eq!(draft.body, "=a%20b");
    }

    #[test]
    fn data_urlencode_from_a_file_is_kept_as_text_and_warned() {
        let bundle = parse("curl 'https://x/y' --data-urlencode 'body@payload.json'").unwrap();
        assert_eq!(bundle.requests[0].body, "body@payload.json");
        assert_eq!(bundle.warnings[0].kind, WarningKind::UnsupportedField);
    }

    #[test]
    fn short_flags_can_be_stacked() {
        // `-sS` 是两个布尔项，不是「选项 s 的值是 S」
        let draft = one("curl -sS -L 'https://x/y'");
        assert_eq!(draft.url, "https://x/y");
        assert!(draft.settings.follow_redirects);
    }

    #[test]
    fn url_can_come_from_the_url_flag() {
        let draft = one("curl --url 'https://x/y' -s");
        assert_eq!(draft.url, "https://x/y");
    }

    #[test]
    fn user_agent_and_referer_become_headers() {
        let draft = one("curl -A 'reqly/0.1' -e 'https://from.example' 'https://x/y'");
        assert_eq!(header(&draft, "user-agent"), Some("reqly/0.1"));
        assert_eq!(header(&draft, "referer"), Some("https://from.example"));
    }

    #[test]
    fn cookie_string_becomes_a_header() {
        let draft = one("curl -b 'a=1; b=2' 'https://x/y'");
        assert_eq!(header(&draft, "cookie"), Some("a=1; b=2"));
    }

    #[test]
    fn empty_body_is_still_given_a_type() {
        let draft = one("curl 'https://x/y' -d ''");
        assert_eq!(draft.body, "");
        assert_eq!(draft.body_type, "urlencoded");
    }

    #[test]
    fn multipart_becomes_form_and_warns() {
        let bundle = parse("curl 'https://x/upload' -F 'file=@a.txt' -F 'note=hi'").unwrap();
        let draft = &bundle.requests[0];
        assert_eq!(draft.body_type, "form");
        assert_eq!(draft.body, "file=@a.txt\nnote=hi");
        assert!(
            bundle
                .warnings
                .iter()
                .any(|w| w.kind == WarningKind::UnsupportedField),
            "multipart 还不能发送，必须说出来"
        );
    }

    #[test]
    fn json_without_content_type_stays_urlencoded() {
        // curl 自己会把 `-d` 当 urlencoded 发 —— 我们也一样，不能自作聪明改成 json
        let draft = one(r#"curl 'https://x/y' -d '{"a":1}'"#);
        assert_eq!(draft.body_type, "urlencoded");
    }

    #[test]
    fn declared_json_but_broken_is_downgraded_to_raw() {
        // 声明了 json 但内容不是合法 JSON：归到 json 会在发送前被本地校验拦下来，
        // 那是「导进来就发不出去」，比归成 raw 糟得多
        let draft =
            one(r#"curl 'https://x/y' -H 'content-type: application/json' -d '{"a":1}{"b":2}'"#);
        assert_eq!(draft.body_type, "raw");
    }

    // ============================================================
    // 警告 / 边界
    // ============================================================

    #[test]
    fn silent_flags_do_not_produce_warnings() {
        let bundle = parse("curl -sS -v --compressed -i 'https://x/y'").unwrap();
        assert!(
            bundle.warnings.is_empty(),
            "纯输出开关不该报警告：{:?}",
            bundle.warnings
        );
    }

    #[test]
    fn unknown_argument_is_reported_but_request_still_imports() {
        let bundle = parse("curl --totally-made-up 'https://x/y'").unwrap();
        assert_eq!(bundle.requests.len(), 1, "认不出的参数不该让整条失败");
        assert_eq!(bundle.warnings[0].kind, WarningKind::UnknownArgument);
    }

    #[test]
    fn unsupported_features_are_reported() {
        let bundle = parse("curl --cacert ca.pem 'https://x/y'").unwrap();
        assert_eq!(bundle.warnings[0].kind, WarningKind::UnsupportedField);
    }

    #[test]
    fn duplicate_names_get_a_suffix() {
        let bundle =
            parse("curl 'https://a.example.com/users' -s\ncurl 'https://b.example.com/users' -s\n")
                .unwrap();
        assert_eq!(bundle.requests[0].name, "users");
        assert_eq!(bundle.requests[1].name, "users (2)");
        assert!(bundle
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::RenamedDuplicate));
    }

    #[test]
    fn url_without_path_falls_back_to_host() {
        assert_eq!(
            one("curl 'https://api.example.com/'").name,
            "api.example.com"
        );
        assert_eq!(
            one("curl 'https://api.example.com'").name,
            "api.example.com"
        );
    }

    #[test]
    fn command_without_url_is_skipped_with_a_warning() {
        let bundle = parse("curl -s -H 'a: b'").unwrap();
        assert!(bundle.requests.is_empty());
        assert_eq!(bundle.warnings[0].kind, WarningKind::SkippedItem);
    }

    #[test]
    fn multiple_commands_are_all_imported() {
        let bundle =
            parse("curl 'https://a.example.com/1' -s\ncurl 'https://a.example.com/2' -s\n")
                .unwrap();
        assert_eq!(bundle.requests.len(), 2);
        assert_eq!(bundle.requests[1].url, "https://a.example.com/2");
    }

    #[test]
    fn header_without_colon_is_dropped_with_a_warning() {
        let bundle = parse("curl 'https://x/y' -H 'not-a-header'").unwrap();
        assert!(bundle.requests[0].headers.is_empty());
        assert_eq!(bundle.warnings[0].kind, WarningKind::UnsupportedField);
    }

    #[test]
    fn header_value_keeps_inner_colons() {
        let draft = one("curl 'https://x/y' -H 'authorization: Bearer a:b:c'");
        assert_eq!(header(&draft, "authorization"), Some("Bearer a:b:c"));
    }

    #[test]
    fn terminal_prompt_prefix_is_ignored() {
        let draft = one("$ curl 'https://x/y' -H 'a: b'");
        assert_eq!(draft.url, "https://x/y");
        assert_eq!(header(&draft, "a"), Some("b"));
    }

    #[test]
    fn placeholder_survives_import() {
        // 导入的 `{{...}}` 是资产，不是要展开的宏
        let draft = one("curl '{{env.baseUrl}}/users' -H 'x-token: {{token.access}}'");
        assert_eq!(draft.url, "{{env.baseUrl}}/users");
        assert_eq!(header(&draft, "x-token"), Some("{{token.access}}"));
    }
}
