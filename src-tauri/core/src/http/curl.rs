//! 已解析的请求 → curl 命令。
//!
//! ## 为什么生成必须在后端
//!
//! 前端手上只有**编辑态草稿**：`{{env.baseUrl}}` / `{{token.access}}` 都还是原文。
//! 用它拼出来的 curl 恰好答不了「为什么 401」这个问题 —— 要看的是真实发出去的那串值。
//! 而「解析后」的形态只有 `Pipeline::prepare` 之后才存在，所以生成只能在这一侧。
//!
//! ## 与导入器互为逆
//!
//! 只用 `importer::curl` 认得的那批参数（`-X` `-H` `--data-raw` `-F` `-u` `-L`
//! `-k` `--max-time` `-x`），让「导出 → 粘回导入器」闭环。
//! 这条性质由 `round_trip_through_the_importer_is_stable` 钉着 —— 它比逐字符的
//! 期望值更能反映真实约束：界面里显示的形态，用户是真的会粘回去的。
//!
//! ## 三处刻意的不一致
//!
//! 括号里是「另一条路会怎样」，都是互斥的取舍：
//!
//! * **不写 `--http1.1` / `--http2`**：导入器把它们当纯输出开关忽略，写出去就丢，
//!   往返立刻不等（写了更忠实，但那份忠实只能维持一次）。
//! * **单引号里出现 `'` 时改用双引号**：bash 的 `'\''` 写法 tokenizer 解不出来
//!   （它的 `Single` 状态里 `\` 是字面量），往返会多一个反斜杠。
//! * **multipart 不带 `Content-Type`**：执行层对 multipart 会**删掉**用户手写的
//!   `Content-Type`（boundary 必须由发送方现生成，手写的那条注定对不上），
//!   这里跟着删才与真实请求一致。
//!
//! ## 但**判据**上不许有分歧
//!
//! 上面三处都是在「多写一点还是少写一点」上做取舍。授权那边不是同一类事：
//! `token` 是空串时执行层照发不误（`bearer_auth("")` → 值为 `"Bearer "` 的合法头），
//! 生成端却曾经判空把它整个丢掉 —— 于是面板上少掉唯一能解释那次 401 的一行，
//! 而用户是拿着这份面板去找原因的（2026-09-17 真实踩到，见
//! `an_empty_bearer_is_written_out_rather_than_dropped`）。
//! **同一件事的两条路，判据必须相同**：宁可多写一条值为空的头，
//! 也不能让面板比真实请求更干净。

use super::execute::{apply_query_params, form_fields_from_body, parse_url, query_auth_pair};
use super::RequestSpec;
use crate::redact;
use crate::storage::models::{ApiKeyLocation, AuthConfig, BodyType, KeyValueItem};

/// 把一份请求拼成可执行的 curl 命令。
///
/// `mask_secrets = true` 时凭据类的值换成 `••••`（名单见 [`crate::redact`]）——
/// 用于「要截图 / 要贴给别人看」的场合。默认应当传 `false`：
/// 核对 401 时看不到 token 本身就等于没看。
pub fn to_curl(spec: &RequestSpec, mask_secrets: bool) -> String {
    let mut parts: Vec<String> = vec!["curl".to_string()];

    // 方法总是显式写出来。curl 只在「有 body」时自己变成 POST，
    // 依赖那条规则等于让两条路径各推一次方法，而一旦推得不一样，
    // 表现是「粘出去的 curl 打的是另一个接口」—— 方法错了，401 与 200 都无法解释。
    parts.push(format!("-X {}", spec.method.trim().to_ascii_uppercase()));

    parts.push(quote(&build_url(spec, mask_secrets)));

    for line in header_lines(spec, mask_secrets) {
        parts.push(format!("-H {}", quote(&line)));
    }

    if let AuthConfig::Basic { username, password } = &spec.auth {
        let password = if mask_secrets {
            redact::MASK
        } else {
            password.as_str()
        };
        // 用户名不打码：核对「是不是发错了账号」时它是有用信息，而它本身不是凭据
        parts.push(format!("-u {}", quote(&format!("{username}:{password}"))));
    }

    parts.extend(body_args(spec));

    let settings = &spec.settings;
    // curl 的真实默认是**不跟随**重定向，所以只有开启才写 `-L`
    if settings.follow_redirects {
        parts.push("-L".to_string());
    }
    if !settings.verify_tls {
        parts.push("-k".to_string());
    }
    // 总是写出来：它改变行为（reqly 默认 30s，curl 默认不超时），
    // 漏掉它「粘出去跑」的结果就与界面上的那次不是一回事。
    // 向上取整 —— 1.2s 不能写成 `1`，那比界面里配的超时更短。
    if settings.timeout_ms > 0 {
        parts.push(format!("--max-time {}", settings.timeout_ms.div_ceil(1000)));
    }
    if let Some(proxy) = settings
        .proxy
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        parts.push(format!("-x {}", quote(proxy)));
    }

    parts.join(" \\\n  ")
}

/// 拼最终 URL（params + query 形态的 API Key）。
///
/// 复用执行层的两个函数而不是自己拼字符串：`?`、`#`、中文、重复 key 的编码差异
/// 会让这条命令与真实请求指向不同的地址，而这个面板的全部价值就是「一模一样」。
fn build_url(spec: &RequestSpec, mask_secrets: bool) -> String {
    let Ok(mut url) = parse_url(spec.url.trim()) else {
        // 解析不了（占位符没解析出来、或用户还没填完）→ 原样给出。
        // 这时显示原文比显示一个我们改写过「更像样」的版本有用。
        return spec.url.trim().to_string();
    };

    apply_query_params(&mut url, &spec.params);
    if let Some((key, value)) = query_auth_pair(&spec.auth) {
        let value = if mask_secrets { redact::MASK } else { value };
        url.query_pairs_mut().append_pair(key, value);
    }
    url.to_string()
}

/// 请求头行（`name: value`），含授权与缺省 `Content-Type`。
fn header_lines(spec: &RequestSpec, mask_secrets: bool) -> Vec<String> {
    let multipart = matches!(spec.body_type, BodyType::Form);

    let mut out: Vec<String> = Vec::new();
    let mut has_content_type = false;

    for item in &spec.headers {
        if !item.enabled || item.key.trim().is_empty() {
            continue;
        }
        let name = item.key.trim();

        if name.eq_ignore_ascii_case("content-type") {
            // multipart 的这条会被执行层删掉（见模块注释），这里跟着删
            if multipart {
                continue;
            }
            has_content_type = true;
        }

        let value = if mask_secrets {
            redact::mask_value(name, &item.value)
        } else {
            item.value.clone()
        };
        out.push(format!("{name}: {value}"));
    }

    match &spec.auth {
        // 🔴 **不判空**，空的也照实写出来。执行层（`execute::execute_inner`）把
        // `Bearer` 无条件交给 reqwest，而 `bearer_auth("")` 生成的是值为
        // `"Bearer "` 的**合法头** —— 真发出去的请求带的就是这么一条空凭据，
        // 服务端据此回 401。这里要是判空丢掉，面板上就少掉唯一能解释这次 401 的
        // 那一行，而这个面板存在的全部理由就是「显示真实发出去的那串值」。
        // 判据一旦与执行层不同，用户会拿着一份不完整的事实去怀疑一个不存在的 bug。
        AuthConfig::Bearer { token } => {
            let value = if mask_secrets {
                redact::MASK.to_string()
            } else {
                token.clone()
            };
            out.push(format!("Authorization: Bearer {value}"));
        }
        AuthConfig::ApiKey {
            key,
            value,
            location: ApiKeyLocation::Header,
        } if !key.trim().is_empty() => {
            let value = if mask_secrets {
                redact::mask_value(key, value)
            } else {
                value.clone()
            };
            out.push(format!("{}: {value}", key.trim()));
        }
        // `inherit` 到这一步还没解析开是不该发生的（流水线负责解开）；
        // 真出现了就什么都不加，绝不猜一个出来。`None` 同理 —— 它表示「不注入」。
        _ => {}
    }

    if !has_content_type && !multipart {
        if let Some(ct) = default_content_type(spec) {
            out.push(format!("Content-Type: {ct}"));
        }
    }

    out
}

/// 执行层会给这类 body 补的缺省 `Content-Type`。**两边必须是同一张表** ——
/// 缺了它，粘出去的 curl 少了头，服务端的行为就与界面上那次不一样。
fn default_content_type(spec: &RequestSpec) -> Option<&'static str> {
    match spec.body_type {
        BodyType::Json => (!spec.body.trim().is_empty()).then_some("application/json"),
        BodyType::Graphql => Some("application/json"),
        BodyType::Urlencoded => Some("application/x-www-form-urlencoded"),
        BodyType::Raw => Some("text/plain; charset=utf-8"),
        // multipart 的 boundary 由发送方现生成，没有可写的常量；
        // `none` / `binary` 本来就没有缺省类型
        BodyType::Form | BodyType::None | BodyType::Binary => None,
    }
}

/// 请求体对应的参数。
fn body_args(spec: &RequestSpec) -> Vec<String> {
    match spec.body_type {
        // 空 JSON 体在执行层被当成「没有 body」，这里也得一致 ——
        // 否则 curl 会发出一个界面里根本没发的 body
        BodyType::Json if spec.body.trim().is_empty() => Vec::new(),
        BodyType::Json | BodyType::Graphql | BodyType::Urlencoded | BodyType::Raw => {
            vec![format!("--data-raw {}", quote(&spec.body))]
        }
        // binary 的请求体至今没实现（执行层直接报 `NotImplemented`），
        // 但用户可能已经把路径填在那儿了 —— 照实写出来，比静默丢掉强
        BodyType::Binary => {
            if spec.body.trim().is_empty() {
                Vec::new()
            } else {
                vec![format!("--data-binary {}", quote(&spec.body))]
            }
        }
        BodyType::Form => form_args(spec),
        BodyType::None => Vec::new(),
    }
}

/// multipart 的 `-F` 行。
///
/// `form_fields` 为空而 `body` 非空时退回解析 `body`（`k=v&k=@路径`）——
/// 与执行层 [`form_fields_from_body`] 是同一个口径：那份内容来自只认 `body` 的
/// 写入方（MCP / CLI / 导入器），漏了这一步，导出的是个空表单。
fn form_args(spec: &RequestSpec) -> Vec<String> {
    let parsed;
    let fields: &[KeyValueItem] = if spec.form_fields.is_empty() && !spec.body.is_empty() {
        parsed = form_fields_from_body(&spec.body);
        &parsed
    } else {
        &spec.form_fields
    };

    fields
        .iter()
        .filter(|f| f.enabled && !f.key.trim().is_empty())
        .map(|f| {
            let name = f.key.trim();
            let pair = match f.file.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
                // `@` 前缀是 curl 的「这是个文件」，导入器也按它还原成文件字段
                Some(path) => format!("{name}=@{path}"),
                None => format!("{name}={}", f.value),
            };
            format!("-F {}", quote(&pair))
        })
        .collect()
}

/// 把一个值包成 shell 词。
///
/// 默认单引号（bash 里引号内**什么都不是特殊字符**，对值是绝对忠实的）。
/// 值里出现单引号时才改用双引号 —— **不能**用 bash 那套 `'\''`：
/// tokenizer 的 `Single` 状态里 `\` 是字面量，那串会被解成「反斜杠 + 空」，
/// 于是「导出 → 导入 → 再导出」平白多出一个反斜杠，往返立刻不等。
fn quote(value: &str) -> String {
    if !value.contains('\'') {
        return format!("'{value}'");
    }

    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        // 这四个是双引号里唯一需要转义的（tokenizer 的 `Double` 状态与 bash 一致）
        if matches!(ch, '"' | '\\' | '$' | '`') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importer::curl as importer;
    use crate::storage::models::{KeyValueItem, RequestSettings};

    fn spec() -> RequestSpec {
        RequestSpec {
            method: "GET".into(),
            url: "https://api.example.com/orders".into(),
            params: Vec::new(),
            headers: Vec::new(),
            body_type: BodyType::None,
            body: String::new(),
            auth: AuthConfig::None,
            settings: RequestSettings {
                // 测试大多不关心重定向与超时，钉住它们才能让断言只反映被测的那一项
                follow_redirects: false,
                timeout_ms: 30_000,
                ..RequestSettings::default()
            },
            folder_id: None,
            form_fields: Vec::new(),
        }
    }

    fn header_of<'a>(curl: &'a str, name: &str) -> Option<&'a str> {
        curl.lines().map(str::trim).find(|line| {
            line.to_ascii_lowercase()
                .starts_with(&format!("-h '{}", name.to_ascii_lowercase()))
        })
    }

    #[test]
    fn method_is_always_explicit() {
        let curl = to_curl(&spec(), false);
        assert!(
            curl.starts_with("curl \\\n  -X GET \\\n"),
            "方法必须显式：{curl}"
        );
    }

    #[test]
    fn params_are_folded_into_the_url() {
        let mut s = spec();
        s.params = vec![
            KeyValueItem::new("page", "1"),
            KeyValueItem::new("keyword", "武汉 天 气"), // 空格与中文都要按 URL 规则编码
            KeyValueItem {
                enabled: false,
                ..KeyValueItem::new("dropped", "x")
            },
        ];

        let curl = to_curl(&s, false);
        assert!(curl.contains("page=1"), "{curl}");
        assert!(
            curl.contains("keyword=%E6%AD%A6%E6%B1%89+%E5%A4%A9+%E6%B0%94"),
            "中文与空格必须编码成与真实请求一致的形式：{curl}"
        );
        assert!(!curl.contains("dropped"), "未勾选的参数不该出现：{curl}");
    }

    #[test]
    fn bearer_token_becomes_an_authorization_header() {
        let mut s = spec();
        s.auth = AuthConfig::Bearer {
            token: "eyJhbGciOi.abc".into(),
        };

        let curl = to_curl(&s, false);
        assert!(
            curl.contains("'Authorization: Bearer eyJhbGciOi.abc'"),
            "{curl}"
        );
    }

    /// 空的 Bearer **照实写出来**，绝不静默丢掉。
    ///
    /// 执行层不判空（`bearer_auth("")` 生成的是值为 `"Bearer "` 的合法头），
    /// 真发出去的请求确实带着一条空凭据 —— 实测过，原始请求头就是
    /// `authorization: Bearer `（尾部一个空格）。这里判空丢掉的话，
    /// 「面板显示的」和「真实发出的」就说不到一起，而用户打开这个面板的
    /// 动机恰恰是解释一个已经发生的 401：少掉那一行，等于让他去找一个
    /// 不存在的 bug（2026-09-17 真实踩到）。
    ///
    /// 这条测试钉的是**两边判据必须一致**，不是「空值好看不好看」。
    #[test]
    fn an_empty_bearer_is_written_out_rather_than_dropped() {
        let mut s = spec();
        s.auth = AuthConfig::Bearer {
            token: String::new(),
        };

        let curl = to_curl(&s, false);
        assert!(
            curl.contains("Authorization: Bearer '"),
            "空 token 也必须照实写出来（尾部是那个空值），否则面板会撒谎：{curl}"
        );
    }

    #[test]
    fn basic_auth_uses_the_user_flag() {
        let mut s = spec();
        s.auth = AuthConfig::Basic {
            username: "alice".into(),
            password: "s3cr3t".into(),
        };

        let curl = to_curl(&s, false);
        assert!(curl.contains("-u 'alice:s3cr3t'"), "{curl}");
    }

    #[test]
    fn api_key_in_query_lands_in_the_url_not_in_headers() {
        let mut s = spec();
        s.auth = AuthConfig::ApiKey {
            key: "api_key".into(),
            value: "v1".into(),
            location: ApiKeyLocation::Query,
        };

        let curl = to_curl(&s, false);
        assert!(curl.contains("api_key=v1"), "{curl}");
        assert!(
            !curl.contains("-H 'api_key"),
            "query 形态不该变成请求头：{curl}"
        );
    }

    #[test]
    fn json_body_brings_its_own_content_type() {
        let mut s = spec();
        s.method = "POST".into();
        s.body_type = BodyType::Json;
        s.body = r#"{"a":1}"#.into();

        let curl = to_curl(&s, false);
        assert!(curl.contains("'Content-Type: application/json'"), "{curl}");
        assert!(curl.contains("--data-raw '{\"a\":1}'"), "{curl}");
    }

    /// 空 JSON body 在执行层等于「没有 body」，导出时也必须一致
    #[test]
    fn empty_json_body_adds_neither_content_type_nor_data() {
        let mut s = spec();
        s.body_type = BodyType::Json;
        s.body = "   \n".into();

        let curl = to_curl(&s, false);
        assert!(!curl.contains("Content-Type"), "{curl}");
        assert!(!curl.contains("--data-raw"), "{curl}");
    }

    #[test]
    fn user_content_type_wins_over_the_default() {
        let mut s = spec();
        s.method = "POST".into();
        s.body_type = BodyType::Json;
        s.body = r#"{"a":1}"#.into();
        s.headers = vec![KeyValueItem::new(
            "Content-Type",
            "application/vnd.api+json",
        )];

        let curl = to_curl(&s, false);
        assert!(
            curl.contains("'Content-Type: application/vnd.api+json'"),
            "{curl}"
        );
        assert!(
            !curl.contains("application/json'"),
            "不该再补一条默认值：{curl}"
        );
    }

    #[test]
    fn multipart_uses_form_flags_including_uploaded_files() {
        let mut s = spec();
        s.method = "POST".into();
        s.body_type = BodyType::Form;
        s.form_fields = vec![
            KeyValueItem::new("grant_type", "client_credentials"),
            KeyValueItem {
                key: "file".into(),
                value: String::new(),
                enabled: true,
                desc: None,
                file: Some(r"C:\tmp\a.xlsx".into()),
            },
        ];

        let curl = to_curl(&s, false);
        assert!(
            curl.contains("-F 'grant_type=client_credentials'"),
            "{curl}"
        );
        assert!(curl.contains(r"-F 'file=@C:\tmp\a.xlsx'"), "{curl}");
        assert!(
            !curl.contains("Content-Type"),
            "boundary 由发送方现生成，写死一条反而是错的：{curl}"
        );
    }

    /// multipart 的字段只存在 `body` 里（MCP / CLI / 导入器写进来的形态）
    #[test]
    fn multipart_falls_back_to_parsing_the_body_column() {
        let mut s = spec();
        s.method = "POST".into();
        s.body_type = BodyType::Form;
        s.body = "a=1\nb=2".into();

        let curl = to_curl(&s, false);
        assert!(curl.contains("-F 'a=1'"), "{curl}");
        assert!(curl.contains("-F 'b=2'"), "换行分隔的字段必须拆开：{curl}");
    }

    #[test]
    fn settings_become_their_curl_flags() {
        let mut s = spec();
        s.settings = RequestSettings {
            follow_redirects: true,
            verify_tls: false,
            timeout_ms: 1500,
            proxy: Some("http://127.0.0.1:7890".into()),
            ..RequestSettings::default()
        };

        let curl = to_curl(&s, false);
        assert!(curl.contains(" -L"), "{curl}");
        assert!(curl.contains(" -k"), "{curl}");
        assert!(
            curl.contains("--max-time 2"),
            "向上取整，不能比配置的超时更短：{curl}"
        );
        assert!(curl.contains("-x 'http://127.0.0.1:7890'"), "{curl}");
    }

    #[test]
    fn masking_hides_credentials_but_keeps_ordinary_headers() {
        let mut s = spec();
        s.headers = vec![KeyValueItem::new("X-Request-Id", "trace-1")];
        s.auth = AuthConfig::Bearer {
            token: "super-secret".into(),
        };

        let curl = to_curl(&s, true);
        assert!(!curl.contains("super-secret"), "凭据不能出现：{curl}");
        assert!(curl.contains("'Authorization: Bearer ••••'"), "{curl}");
        assert!(
            curl.contains("'X-Request-Id: trace-1'"),
            "普通头不该被打码：{curl}"
        );
    }

    #[test]
    fn a_value_containing_a_single_quote_falls_back_to_double_quotes() {
        let mut s = spec();
        s.headers = vec![KeyValueItem::new("X-Note", "it's fine")];

        let curl = to_curl(&s, false);
        assert!(curl.contains(r#""X-Note: it's fine""#), "{curl}");
    }

    /// 值里有单引号**又**有双引号 —— 走双引号那一支时的转义必须与 bash 一致
    #[test]
    fn double_quote_fallback_escapes_bash_specials() {
        let mut s = spec();
        s.headers = vec![KeyValueItem::new("X-Note", r#"it's "quoted" $HOME `id`"#)];

        let curl = to_curl(&s, false);
        assert!(
            curl.contains(r#"\$HOME"#),
            "$ 必须转义，否则 bash 会展开成空串：{curl}"
        );
        assert!(
            curl.contains(r#"\`id\`"#),
            "反引号在 bash 里是命令替换：{curl}"
        );
        assert!(curl.contains(r#"\""#), "{curl}");
    }

    // ---------------------------------------------------------------
    // 往返：导出 → 导入 → 再导出，两次文本必须逐字相同
    // ---------------------------------------------------------------

    fn draft_to_spec(draft: crate::importer::RequestDraft) -> RequestSpec {
        RequestSpec {
            method: draft.method,
            url: draft.url,
            params: draft.params,
            headers: draft.headers,
            body_type: BodyType::parse(&draft.body_type),
            body: draft.body,
            auth: draft.auth,
            settings: draft.settings,
            folder_id: None,
            form_fields: Vec::new(),
        }
    }

    fn assert_round_trip(spec: &RequestSpec, label: &str) {
        let first = to_curl(spec, false);
        let bundle = importer::parse(&first)
            .unwrap_or_else(|e| panic!("{label}: 导出的 curl 自己读不回来：{e}"));
        assert_eq!(bundle.requests.len(), 1, "{label}: 应当解析出一条命令");

        let second = to_curl(&draft_to_spec(bundle.requests[0].clone()), false);
        assert_eq!(first, second, "{label}: 往返之后形态变了");
    }

    /// 这条测试是这个模块的主要保障：界面里显示的形态，用户是真的会粘回导入器的。
    /// 任何「导出得出来、导入不回去」的写法都会在这里现形。
    #[test]
    fn round_trip_through_the_importer_is_stable() {
        // ① 带参数与请求头的 GET
        let mut get = spec();
        get.params = vec![
            KeyValueItem::new("page", "2"),
            KeyValueItem::new("q", "武汉"),
        ];
        get.headers = vec![
            KeyValueItem::new("Accept", "application/json"),
            KeyValueItem::new("X-Note", "张三's note"),
        ];
        assert_round_trip(&get, "带中文与单引号的 GET");

        // ② Bearer + JSON body
        let mut post = spec();
        post.method = "POST".into();
        post.headers = vec![KeyValueItem::new("Accept", "application/json")];
        post.body_type = BodyType::Json;
        post.body = r#"{"name":"张三","tags":["a","b"]}"#.into();
        post.auth = AuthConfig::Bearer {
            token: "eyJhbGciOi.abc".into(),
        };
        assert_round_trip(&post, "Bearer + JSON");

        // ③ Basic 认证走 `-u`
        let mut basic = spec();
        basic.method = "POST".into();
        basic.auth = AuthConfig::Basic {
            username: "alice".into(),
            password: "s3cr3t".into(),
        };
        basic.body_type = BodyType::Urlencoded;
        basic.body = "grant_type=password&scope=all".into();
        assert_round_trip(&basic, "Basic + urlencoded");

        // ④ multipart（含文件字段）
        let mut form = spec();
        form.method = "POST".into();
        form.body_type = BodyType::Form;
        form.form_fields = vec![
            KeyValueItem::new("a", "1"),
            KeyValueItem {
                key: "file".into(),
                value: String::new(),
                enabled: true,
                desc: None,
                file: Some(r"C:\tmp\a b.xlsx".into()),
            },
        ];
        assert_round_trip(&form, "multipart");

        // ⑤ Raw 体里带换行与各种引号 —— 最容易在引号处理上翻车的一条
        let mut raw = spec();
        raw.method = "PUT".into();
        raw.body_type = BodyType::Raw;
        raw.body = "line1: it's \"quoted\" $HOME\nline2\n".into();
        assert_round_trip(&raw, "raw 多行 + 引号");

        // ⑥ query 形态的 API Key
        let mut api_key = spec();
        api_key.auth = AuthConfig::ApiKey {
            key: "api_key".into(),
            value: "v1".into(),
            location: ApiKeyLocation::Query,
        };
        api_key.params = vec![KeyValueItem::new("page", "1")];
        assert_round_trip(&api_key, "query API Key");

        // ⑦ 全部设置项都开
        let mut tuned = spec();
        tuned.settings = RequestSettings {
            follow_redirects: true,
            verify_tls: false,
            timeout_ms: 5000,
            proxy: Some("http://127.0.0.1:7890".into()),
            ..RequestSettings::default()
        };
        assert_round_trip(&tuned, "带 -L / -k / --max-time / -x");
    }

    /// 打码只看**值**：命令骨架必须与明文版逐字相同，
    /// 否则「切一下打码开关」会让人以为请求本身变了
    #[test]
    fn masking_only_swaps_the_values() {
        let mut s = spec();
        s.method = "POST".into();
        s.body_type = BodyType::Json;
        s.body = r#"{"a":1}"#.into();
        s.headers = vec![KeyValueItem::new("X-Request-Id", "trace-1")];
        s.auth = AuthConfig::Bearer { token: "t".into() };

        let plain = to_curl(&s, false);
        let masked = to_curl(&s, true);

        assert_eq!(
            plain.lines().count(),
            masked.lines().count(),
            "行数都不该变：\n{plain}\n---\n{masked}"
        );
        assert_eq!(
            header_of(&plain, "x-request-id"),
            header_of(&masked, "x-request-id"),
            "普通头必须原样"
        );
    }
}
