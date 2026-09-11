//! 一次请求的执行：组装 → 发送 → 计时 → 读响应体。
//!
//! ## 计时为什么只有三段
//!
//! design.md §6 的时间线画的是 DNS / 连接 / TLS / 首字节 / 下载五段，
//! 但 reqwest 只暴露「发出 → 拿到响应头」（首字节）和「拿到头 → 读完 body」（下载）
//! 这两个可观测点。想拆出 DNS / 连接 / TLS 得自己写 `dns_resolver` 和连接器，
//! 而这两处都是**全局**的（构造时就烘进 Client），并发压测时无法把耗时归给正确的请求。
//! 与其编一个看起来精确的假数字，不如只报能测准的三段，前端把拿不到的显示成「—」。
//!
//! ## 超时为什么自己管
//!
//! reqwest 的 `timeout` 到底覆不覆盖「读完 body」在不同版本里有过变化，
//! 而「读 body 卡住」恰恰是最常见的假死场景。所以这里用
//! `tokio::time::timeout` 把「连接 + 读头 + 读 body」整套箍起来，
//! 语义由我们自己的代码保证，不依赖上游实现的细节。

use std::borrow::Cow;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::{Client, Method, Url};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::{codes, AppError, AppResult};
use crate::storage::models::{BodyType, KeyValueItem, RequestSettings};

/// 内联返回给前端的响应体上限。
///
/// 8MB 的依据：前端在 2MB 以上就退化到纯文本模式，8MB 是「还能塞进 IPC 且不卡死」的边界。
/// 再大就该走流式落盘（design.md §12 把它列为低风险项，留给 P8）。
const MAX_INLINE_BODY: usize = 8 * 1024 * 1024;

/// 前端传过来的请求定义。
///
/// 刻意**不**从数据库读：这样「还没保存的新请求」也能直接发（Postman 的核心体验），
/// 压测（P6）也能对一份改了一半的草稿发起 —— 保存和发送是两件事。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSpec {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub params: Vec<KeyValueItem>,
    #[serde(default)]
    pub headers: Vec<KeyValueItem>,
    #[serde(default)]
    pub body_type: BodyType,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub settings: RequestSettings,
}

impl RequestSpec {
    /// 给日志/历史用的简短描述（不含 query，避免把 token 写进日志）
    pub fn label(&self) -> String {
        format!("{} {}", self.method.trim().to_uppercase(), self.url.trim())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

/// 响应体是怎么解读出来的。
///
/// 为什么要把这件事显式告诉前端：一份二进制响应被 lossy 成字符串之后界面就是乱码，
/// 用户会以为是 Reqly 坏了。有了这个标记，前端可以说
/// 「这是二进制内容，共 N 字节」而不是硬渲染乱码。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BodyEncoding {
    /// 合法 UTF-8 文本
    Text,
    /// 按 content-type 判断是文本，但字节流里有非法序列 → 有损转换
    Lossy,
    /// 二进制内容，`body` 为空串
    Binary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpResponsePayload {
    pub status: u16,
    pub status_text: String,
    /// 实际使用的协议版本（`HTTP/1.1` / `HTTP/2.0`）。是协商结果，不是用户选的那个。
    pub http_version: String,
    pub headers: Vec<HttpHeader>,
    /// 最终生效的 URL。跟随重定向后会与请求 URL 不同 —— 这是排查 302 的关键信息。
    pub url: String,
    pub redirected: bool,
    pub body: String,
    pub body_encoding: BodyEncoding,
    /// 实际收到的字节数（**截断前**）
    pub body_size: u64,
    pub truncated: bool,
    /// 发出 → 收到响应头
    pub ttfb_ms: u64,
    /// 响应头 → 读完 body
    pub download_ms: u64,
    /// 端到端总耗时（含组装与解析；比 ttfb + download 略大）
    pub total_ms: u64,
}

/// 执行一个请求。`client` 由 [`crate::http::ClientCache`] 提供（连接池复用）。
///
/// 不可取消 —— 留给测试和内部复用。命令层用 [`execute_with_cancel`]。
pub async fn execute(client: &Client, spec: &RequestSpec) -> AppResult<HttpResponsePayload> {
    execute_with_limit(client, spec, MAX_INLINE_BODY).await
}

/// 可取消的执行。`cancel` 为 `None` 时与 [`execute`] 行为完全一致。
pub async fn execute_with_cancel(
    client: &Client,
    spec: &RequestSpec,
    cancel: Option<&CancellationToken>,
) -> AppResult<HttpResponsePayload> {
    execute_inner(client, spec, MAX_INLINE_BODY, cancel).await
}

/// 带上限的版本。单独拆出来是为了让截断逻辑可测 ——
/// 否则测试得真造一个 8MB 响应。
pub async fn execute_with_limit(
    client: &Client,
    spec: &RequestSpec,
    max_inline: usize,
) -> AppResult<HttpResponsePayload> {
    execute_inner(client, spec, max_inline, None).await
}

async fn execute_inner(
    client: &Client,
    spec: &RequestSpec,
    max_inline: usize,
    cancel: Option<&CancellationToken>,
) -> AppResult<HttpResponsePayload> {
    let method = parse_method(&spec.method)?;
    let mut url = parse_url(&spec.url)?;
    apply_query_params(&mut url, &spec.params);

    let (body, default_content_type) = build_body(&spec.body_type, &spec.body)?;
    let mut headers = build_headers(&spec.headers)?;
    // 用户显式设了 content-type 就以他为准（Postman 的行为，也是唯一不让人意外的行为）
    if let Some(ct) = default_content_type {
        if !headers.contains_key(CONTENT_TYPE) {
            headers.insert(CONTENT_TYPE, HeaderValue::from_static(ct));
        }
    }

    let mut builder = client.request(method, url.clone()).headers(headers);
    if let Some(body) = body {
        builder = builder.body(body);
    }

    let timeout_ms = spec.settings.timeout_ms.max(1);
    let started = Instant::now();

    let attempt = async {
        let response = builder.send().await.map_err(|e| classify(e, &url))?;
        let ttfb = started.elapsed();

        let status = response.status();
        let version = version_label(response.version());
        let final_url = response.url().to_string();
        let headers_out = collect_headers(response.headers());
        let content_type = content_type_of(&headers_out);

        // 必须先把 headers 收成 owned 再 bytes() —— bytes() 会消费掉 response
        let bytes = response.bytes().await.map_err(|e| classify(e, &url))?;

        Ok::<_, AppError>((
            ttfb,
            status,
            version,
            final_url,
            headers_out,
            content_type,
            bytes,
        ))
    };

    // 超时与取消在这里合流成一个出口：
    // * 超时用 `tokio::time::timeout` 箍住「连接 + 读头 + 读 body」整段，
    //   语义由我们自己的代码保证，不依赖 reqwest 的 timeout 到底覆盖到哪一层。
    // * 取消靠 `select!` 与超时竞争。`biased` 让取消优先 —— 用户点了取消就该立刻停，
    //   没必要等这一拍的超时判定。没被选中的那一支连同 `attempt` 一起被 drop，
    //   reqwest 会因此关掉连接，所以取消是**真打断**，不是「结果丢掉、连接还挂着」。
    let timed = tokio::time::timeout(Duration::from_millis(timeout_ms), attempt);
    let outcome = match cancel {
        Some(token) => tokio::select! {
            biased;
            _ = token.cancelled() => return Err(AppError::Cancelled),
            result = timed => result,
        },
        None => timed.await,
    };

    let (ttfb, status, version, final_url, headers_out, content_type, bytes) = match outcome {
        Ok(Ok(value)) => value,
        Ok(Err(err)) => return Err(err),
        Err(_elapsed) => {
            return Err(AppError::http_with(
                codes::TIMEOUT,
                format!("request to {url} timed out after {timeout_ms}ms"),
                serde_json::json!({ "ms": timeout_ms }),
            ))
        }
    };

    let total = started.elapsed();
    let body_size = bytes.len() as u64;
    let truncated = bytes.len() > max_inline;
    let slice: &[u8] = if truncated {
        &bytes[..max_inline]
    } else {
        &bytes[..]
    };
    let (body, body_encoding) = decode_body(slice, truncated, &content_type);

    Ok(HttpResponsePayload {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or_default().to_string(),
        http_version: version,
        headers: headers_out,
        redirected: final_url != url.as_str(),
        url: final_url,
        body,
        body_encoding,
        body_size,
        truncated,
        ttfb_ms: ttfb.as_millis() as u64,
        download_ms: total.saturating_sub(ttfb).as_millis() as u64,
        total_ms: total.as_millis() as u64,
    })
}

// ---------------------------------------------------------------
// 组装请求
// ---------------------------------------------------------------

fn parse_method(raw: &str) -> AppResult<Method> {
    let upper = raw.trim().to_ascii_uppercase();
    Method::from_bytes(upper.as_bytes()).map_err(|_| AppError::InvalidInput {
        field: "method",
        reason: format!("`{raw}` 不是合法的 HTTP 方法"),
    })
}

/// 解析 URL。**没有 scheme 时补 `http://`** ——
/// 直接粘 `127.0.0.1:8080/health` 是极常见的操作，逼用户先去补 `http://` 纯属找麻烦。
fn parse_url(raw: &str) -> AppResult<Url> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidUrl {
            raw: raw.to_string(),
            reason: "请求地址为空".into(),
        });
    }

    let parse = |s: &str| {
        Url::parse(s).map_err(|e| AppError::InvalidUrl {
            raw: trimmed.to_string(),
            reason: e.to_string(),
        })
    };

    let url = match parse(trimmed) {
        Ok(url) => url,
        Err(original) => {
            // `127.0.0.1:8080/x` 这种「主机:端口」被 URL 解析器当成「相对引用」而失败。
            // 含 `://`（协议已写但写错）或含空格（用户就打错了）的不补，补了还是错。
            if trimmed.contains("://") || trimmed.contains(' ') {
                return Err(original);
            }
            parse(&format!("http://{trimmed}"))?
        }
    };

    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(AppError::InvalidUrl {
            raw: trimmed.to_string(),
            reason: format!("只支持 http/https，收到 `{other}`"),
        }),
    }
}

fn apply_query_params(url: &mut Url, params: &[KeyValueItem]) {
    let active: Vec<&KeyValueItem> = params
        .iter()
        .filter(|p| p.enabled && !p.key.trim().is_empty())
        .collect();
    if active.is_empty() {
        return;
    }

    // 用 URL API 追加而不是字符串拼接：它会正确处理已有的 `?`、锚点 `#`，
    // 以及 key/value 里的特殊字符（`&`、空格、中文）。
    let mut pairs = url.query_pairs_mut();
    for p in active {
        pairs.append_pair(p.key.trim(), &p.value);
    }
}

fn build_headers(items: &[KeyValueItem]) -> AppResult<HeaderMap> {
    let mut map = HeaderMap::new();

    for item in items
        .iter()
        .filter(|i| i.enabled && !i.key.trim().is_empty())
    {
        let name = HeaderName::from_bytes(item.key.trim().as_bytes()).map_err(|e| {
            AppError::InvalidInput {
                field: "headers",
                reason: format!("`{}` 不是合法的 header 名：{e}", item.key),
            }
        })?;
        let value = HeaderValue::from_str(&item.value).map_err(|e| AppError::InvalidInput {
            field: "headers",
            reason: format!("`{}` 的值里有非法字符：{e}", item.key),
        })?;
        // append 而不是 insert：同名 header 合法且常见
        // （多份 `Cookie`、`Accept`、`Cache-Control`…），insert 会把前一条悄悄吃掉。
        map.append(name, value);
    }

    Ok(map)
}

/// 返回 `(请求体, 缺省 content-type)`。
fn build_body(
    body_type: &BodyType,
    body: &str,
) -> AppResult<(Option<String>, Option<&'static str>)> {
    match body_type {
        BodyType::None => Ok((None, None)),
        BodyType::Json => {
            if body.trim().is_empty() {
                return Ok((None, None));
            }
            // 顺手校验：JSON 写错了当场告诉用户，比等服务端回一个语焉不详的 400 强得多
            serde_json::from_str::<serde_json::Value>(body).map_err(|e| {
                AppError::http(
                    codes::BODY_NOT_JSON,
                    format!("request body is not valid json: {e}"),
                )
            })?;
            Ok((Some(body.to_string()), Some("application/json")))
        }
        // GraphQL 的 body 本身就是 JSON 字符串 `{"query":"…","variables":{…}}`
        BodyType::Graphql => Ok((Some(body.to_string()), Some("application/json"))),
        BodyType::Urlencoded => Ok((
            Some(body.to_string()),
            Some("application/x-www-form-urlencoded"),
        )),
        BodyType::Raw => Ok((Some(body.to_string()), Some("text/plain; charset=utf-8"))),
        // 这两个要读磁盘 / 拼 multipart 流，涉及文件选择器（design.md 排在 P8）。
        // 明确报「未实现」，而不是把一段 JSON 当文件内容发出去。
        BodyType::Form => Err(AppError::NotImplemented {
            feature: "multipart",
        }),
        BodyType::Binary => Err(AppError::NotImplemented { feature: "binary" }),
    }
}

// ---------------------------------------------------------------
// 读响应
// ---------------------------------------------------------------

fn collect_headers(headers: &HeaderMap) -> Vec<HttpHeader> {
    headers
        .iter()
        .map(|(name, value)| HttpHeader {
            name: name.as_str().to_string(),
            // header 值允许非 UTF-8 字节（RFC 允许 obs-text），不能直接 unwrap
            value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
        })
        .collect()
}

fn content_type_of(headers: &[HttpHeader]) -> String {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-type"))
        .map(|h| h.value.to_ascii_lowercase())
        .unwrap_or_default()
}

fn decode_body(slice: &[u8], truncated: bool, content_type: &str) -> (String, BodyEncoding) {
    match std::str::from_utf8(slice) {
        Ok(text) => (text.to_string(), BodyEncoding::Text),
        Err(e) if truncated && e.error_len().is_none() => {
            // 唯一的毛病是截断正好切在一个多字节字符中间 —— 内容仍然是文本，
            // 只是末尾那个残缺字符要丢掉。这种情况不该报「二进制」。
            let cut = &slice[..e.valid_up_to()];
            (
                String::from_utf8_lossy(cut).into_owned(),
                BodyEncoding::Text,
            )
        }
        Err(_) => {
            let looks_textual = content_type.starts_with("text/")
                || content_type.contains("json")
                || content_type.contains("xml")
                || content_type.contains("javascript")
                || content_type.contains("urlencoded")
                || content_type.contains("graphql");

            if looks_textual {
                (
                    String::from_utf8_lossy(slice).into_owned(),
                    BodyEncoding::Lossy,
                )
            } else {
                // 不返回乱码：让前端按「二进制内容，共 N 字节」展示
                (String::new(), BodyEncoding::Binary)
            }
        }
    }
}

fn version_label(version: reqwest::Version) -> String {
    use reqwest::Version;
    match version {
        Version::HTTP_09 => "HTTP/0.9",
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_11 => "HTTP/1.1",
        Version::HTTP_2 => "HTTP/2.0",
        Version::HTTP_3 => "HTTP/3.0",
        _ => "HTTP/?",
    }
    .to_string()
}

// ---------------------------------------------------------------
// 错误分类
// ---------------------------------------------------------------

/// 把 reqwest 的错误翻成**能指导下一步动作**的错误码。
///
/// 为什么分这么细：「网络错误」这一句对用户没有任何帮助。
/// 域名打错要去改地址，连接被拒说明服务没起，TLS 失败要去关「校验证书」开关 ——
/// 三种情况的动作完全不同，合成一个码等于让用户自己猜。
fn classify(err: reqwest::Error, url: &Url) -> AppError {
    // debug 保留完整信息（含 URL），给日志和 issue 用
    let debug = err.to_string();

    if err.is_timeout() {
        // 走到这里说明是 reqwest 自己判的超时（我们外层还箍了一层 timeout）
        return AppError::http_with(
            codes::TIMEOUT,
            debug,
            serde_json::json!({ "ms": serde_json::Value::Null }),
        );
    }
    if err.is_redirect() {
        return AppError::http(codes::REDIRECT, debug);
    }

    // 关键字匹配只用 **source 链**，不用顶层 message：
    // 顶层 message 里带着完整 URL（`error sending request for url (https://…)`），
    // 而 URL 里出现 "tls" / "certificate" 这类字眼会立刻造成误判。
    let sources = source_chain(&err);
    // 只在 source 链为空时才退化成看顶层 message，所以借一份而不是搬走 ——
    // 下面 TLS 那条分支还要用 `sources` 提取具体原因。
    let marker: Cow<'_, str> = if sources.is_empty() {
        Cow::Owned(debug.to_lowercase())
    } else {
        Cow::Borrowed(sources.as_str())
    };
    let host = url.host_str().unwrap_or_default().to_string();

    // ---- DNS ----
    // 各平台措辞不统一，只能堆关键字。宁可**先**判 DNS：
    // 判成 DNS 但其实是别的错，用户改域名时会发现域名没错，回头看地址栏；
    // 反过来把 DNS 失败说成「网络错误」，用户就一点线索都没有了。
    const DNS_MARKERS: [&str; 6] = [
        "dns",
        "failed to lookup address information",
        "name or service not known",
        "no such host",
        "nodename nor servname",
        "temporary failure in name resolution",
    ];
    if DNS_MARKERS.iter().any(|m| marker.contains(m)) {
        return AppError::http_with(codes::DNS, debug, serde_json::json!({ "host": host }));
    }

    // ---- TLS ----
    // 必须是**具体**标记，不能只匹配 "tls" —— 那会命中一堆无关文本。
    const TLS_MARKERS: [&str; 7] = [
        "invalid peer certificate",
        "unknownissuer",
        "certificate",
        "handshake",
        "received fatal alert",
        "bad certificate",
        "sslerror",
    ];
    if TLS_MARKERS.iter().any(|m| marker.contains(m)) {
        return AppError::http_with(
            codes::TLS,
            debug,
            serde_json::json!({ "reason": first_source(&sources) }),
        );
    }

    // ---- 连接层 ----
    if let Some(io) = find_io_error(&err) {
        match io.kind() {
            std::io::ErrorKind::ConnectionRefused => {
                return AppError::http_with(
                    codes::CONN_REFUSED,
                    debug,
                    serde_json::json!({ "host": host }),
                );
            }
            std::io::ErrorKind::TimedOut => {
                return AppError::http(codes::TIMEOUT, debug);
            }
            _ => {}
        }
    }
    // is_connect 覆盖 DNS/TLS 之外的连接失败（连接被重置、网络不可达、代理连不上…）。
    // 走到这里说明不是上面那几种，统一按「连不上」报。
    if err.is_connect() {
        return AppError::http_with(
            codes::CONN_REFUSED,
            debug,
            serde_json::json!({ "host": host }),
        );
    }

    if err.is_body() || err.is_decode() {
        return AppError::http(codes::DECODE, debug);
    }
    if err.is_request() {
        // 请求都构造不出来：多半是 header 值非法、URL 里混了控制字符
        return AppError::http(codes::REQUEST, debug);
    }
    AppError::http(codes::NETWORK, debug)
}

/// 把 source 链拼成一行小写文本，供关键字匹配。
fn source_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut out = String::new();
    let mut cursor = err.source();
    while let Some(e) = cursor {
        out.push_str(&e.to_string().to_lowercase());
        out.push_str(" | ");
        cursor = e.source();
    }
    out
}

/// 取 source 链里第一条非空项，作为给用户看的 reason
fn first_source(sources: &str) -> String {
    sources
        .split('|')
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .chars()
        .take(140)
        .collect()
}

/// 在 source 链里找 `std::io::Error`（reqwest 把底层 IO 错误裹了好几层）
fn find_io_error<'a>(err: &'a (dyn std::error::Error + 'static)) -> Option<&'a std::io::Error> {
    let mut cursor: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(current) = cursor {
        if let Some(io) = current.downcast_ref::<std::io::Error>() {
            return Some(io);
        }
        cursor = current.source();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::client::ClientCache;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;

    // ============================================================
    // 被测 client
    // ============================================================
    //
    // 统一走 `ClientCache` —— 也就是线上那条路径 —— 而**不要**用 `Client::new()`：
    //
    // 1. 线上是「settings → ClientCache → Client」。用 `Client::new()` 等于把
    //    「设置到底生效没有」这层整个跳过。重定向那两条测试就是栽在这里：
    //    settings 里写了 `follow_redirects = false`，client 却是默认跟随 10 跳的，
    //    于是断言「应当拿到 301」变成了「重定向超限」。
    // 2. `Client::new()` 会读 `HTTP_PROXY` / `HTTPS_PROXY`。只要当前 shell 里
    //    export 过代理，「连 127.0.0.1:1 应当报 E_CONN_REFUSED」就会变成
    //    「代理回了 502」—— 看上去像分类逻辑写错了，其实是请求压根没发出去。
    //    `ClientCache` 内部显式 `no_proxy()`，不受环境影响。
    fn client() -> Client {
        client_for(&RequestSettings::default())
    }

    fn client_for(settings: &RequestSettings) -> Client {
        ClientCache::new().get(settings).expect("构造 client")
    }

    /// 按 spec 自己的 settings 建 client —— 和命令层 `send_request` 做的事一致
    fn client_for_spec(spec: &RequestSpec) -> Client {
        client_for(&spec.settings)
    }

    // ============================================================
    // 进程内极简 HTTP 服务器
    // ============================================================
    //
    // 为什么不引 wiremock / hyper 测试框架：这里需要的只是「一段 reqwest 能解析的字节流」。
    // 更关键的是，**最容易出问题的几条路径恰恰不能 mock** ——
    // 「域名解析失败」「连接被拒」「读 body 超时」必须让 reqwest 真的去连一个不存在的端口、
    // 真的等一个不响应的 socket。mock 出来的都是假绿。

    struct TestServer {
        port: u16,
        requests: mpsc::Receiver<String>,
    }

    impl TestServer {
        fn url(&self, path: &str) -> String {
            format!("http://127.0.0.1:{}{path}", self.port)
        }

        /// 取出服务器收到的下一个原始请求（含头和体）
        fn request(&self) -> String {
            self.requests
                .recv_timeout(Duration::from_secs(5))
                .expect("服务器没收到请求")
        }
    }

    /// 在**已绑定**的 listener 上起服务：第 i 个连接回 `responses[i % len]`。
    ///
    /// 之所以让调用方先 bind：重定向测试需要把真实端口写进 `Location` 头，
    /// 所以必须先拿到端口才能构造响应。
    fn serve_on(listener: TcpListener, responses: Vec<String>) -> TestServer {
        let port = listener.local_addr().expect("addr").port();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            for (i, stream) in listener.incoming().enumerate() {
                let Ok(mut stream) = stream else { break };
                let raw = read_request(&mut stream);
                let _ = tx.send(raw);
                let response = &responses[i % responses.len()];
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        TestServer { port, requests: rx }
    }

    fn serve(responses: Vec<String>) -> TestServer {
        serve_on(TcpListener::bind("127.0.0.1:0").expect("bind"), responses)
    }

    fn serve_once(status: &str, content_type: &str, body: &str) -> TestServer {
        serve(vec![raw_response(status, content_type, body)])
    }

    fn raw_response(status: &str, content_type: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\n\
             Content-Type: {content_type}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// 读一个完整的请求（头 + 按 content-length 读满 body）
    fn read_request(stream: &mut TcpStream) -> String {
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 1024];

        loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);

            let Some(head_end) = find_subslice(&buf, b"\r\n\r\n") else {
                continue;
            };
            let head = String::from_utf8_lossy(&buf[..head_end]).to_lowercase();
            let want = content_length(&head);
            if buf.len() - (head_end + 4) >= want {
                break;
            }
        }

        String::from_utf8_lossy(&buf).into_owned()
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    fn content_length(head: &str) -> usize {
        head.lines()
            .find_map(|l| l.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0)
    }

    fn spec(method: &str, url: String) -> RequestSpec {
        RequestSpec {
            method: method.to_string(),
            url,
            params: Vec::new(),
            headers: Vec::new(),
            body_type: BodyType::None,
            body: String::new(),
            settings: RequestSettings::default(),
        }
    }

    /// body 部分的文本（头之后的全部内容）
    fn body_of(raw: &str) -> &str {
        raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("")
    }

    // ============================================================
    // 正常路径
    // ============================================================

    #[tokio::test]
    async fn get_reads_status_headers_and_body() {
        let server = serve_once("200 OK", "application/json", r#"{"id":42}"#);
        let client = client();

        let out = execute(&client, &spec("GET", server.url("/users/42")))
            .await
            .expect("请求应当成功");

        assert_eq!(out.status, 200);
        assert_eq!(out.status_text, "OK");
        assert_eq!(out.body, r#"{"id":42}"#);
        assert_eq!(out.body_encoding, BodyEncoding::Text);
        assert_eq!(out.body_size, 9);
        assert!(!out.truncated);
        assert!(!out.redirected);
        assert_eq!(out.http_version, "HTTP/1.1");
        assert!(out
            .headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("content-type")));
        // 三段计时的关系必须自洽（毫秒取整可能差 1，所以留 1ms 余量）
        assert!(out.total_ms >= out.ttfb_ms);
        assert!(out.ttfb_ms + out.download_ms + 1 >= out.total_ms);
    }

    #[tokio::test]
    async fn post_sends_json_body_and_default_content_type() {
        let server = serve_once("200 OK", "application/json", "{}");
        let client = client();

        let mut s = spec("POST", server.url("/post"));
        s.body_type = BodyType::Json;
        s.body = r#"{"name":"reqly"}"#.to_string();

        execute(&client, &s).await.expect("POST 应当成功");

        let raw = server.request();
        assert!(raw.starts_with("POST /post HTTP/1.1"), "实际：{raw}");
        assert!(raw
            .to_lowercase()
            .contains("content-type: application/json"));
        assert_eq!(body_of(&raw), r#"{"name":"reqly"}"#);
    }

    #[tokio::test]
    async fn explicit_content_type_wins_over_the_default() {
        let server = serve_once("200 OK", "text/plain", "ok");
        let client = client();

        let mut s = spec("POST", server.url("/post"));
        s.body_type = BodyType::Json;
        s.body = "{}".into();
        s.headers = vec![KeyValueItem::new(
            "Content-Type",
            "application/vnd.api+json",
        )];

        execute(&client, &s).await.expect("应当成功");

        let raw = server.request().to_lowercase();
        assert!(raw.contains("content-type: application/vnd.api+json"));
        assert_eq!(
            raw.matches("content-type:").count(),
            1,
            "同一个 header 不该发两遍：{raw}"
        );
    }

    #[tokio::test]
    async fn enabled_query_params_are_appended() {
        let server = serve_once("200 OK", "text/plain", "ok");
        let client = client();

        let mut s = spec("GET", server.url("/search?existing=1"));
        s.params = vec![
            KeyValueItem::new("q", "中文 空格"),
            KeyValueItem::new("page", "2"),
        ];

        execute(&client, &s).await.expect("应当成功");

        let raw = server.request();
        assert!(raw.starts_with("GET /search?existing=1&"), "实际：{raw}");
        // key/value 里的特殊字符必须被百分号编码，否则服务端会解错
        assert!(
            raw.contains("q=%E4%B8%AD%E6%96%87+%E7%A9%BA%E6%A0%BC"),
            "实际：{raw}"
        );
        assert!(raw.contains("page=2"), "实际：{raw}");
    }

    #[tokio::test]
    async fn disabled_query_params_are_not_sent() {
        let server = serve_once("200 OK", "text/plain", "ok");
        let client = client();

        let mut s = spec("GET", server.url("/search"));
        s.params = vec![
            KeyValueItem::new("keep", "1"),
            KeyValueItem {
                key: "skip".into(),
                value: "x".into(),
                enabled: false,
                desc: None,
            },
            KeyValueItem::new("", "empty-key-should-be-ignored"),
        ];

        execute(&client, &s).await.expect("应当成功");

        let raw = server.request();
        assert!(raw.contains("keep=1"));
        assert!(!raw.contains("skip"), "未勾选的行不该出现在 URL 里：{raw}");
        assert!(!raw.contains("empty-key"), "空 key 该被跳过：{raw}");
    }

    #[tokio::test]
    async fn repeated_headers_are_all_sent() {
        let server = serve_once("200 OK", "text/plain", "ok");
        let client = client();

        let mut s = spec("GET", server.url("/"));
        s.headers = vec![
            KeyValueItem::new("X-Trace", "a"),
            KeyValueItem::new("X-Trace", "b"),
        ];

        execute(&client, &s).await.expect("应当成功");

        let raw = server.request().to_lowercase();
        assert!(
            raw.contains("x-trace: a") && raw.contains("x-trace: b"),
            "实际：{raw}"
        );
    }

    #[tokio::test]
    async fn url_without_scheme_gets_http_prepended() {
        // 直接粘 127.0.0.1:8080/health 是最常见的操作
        let server = serve_once("200 OK", "text/plain", "healthy");
        let client = client();

        let host_port = server.url("").replace("http://", "");
        let out = execute(&client, &spec("GET", format!("{host_port}/healthz")))
            .await
            .expect("应当成功");

        assert_eq!(out.body, "healthy");
        assert!(server.request().starts_with("GET /healthz"), "路径要保留");
    }

    #[tokio::test]
    async fn error_status_is_a_result_not_a_failure() {
        let server = serve_once("404 Not Found", "application/json", r#"{"error":"nope"}"#);
        let client = client();

        // 404 是**业务结果**，不是 Reqly 的错误 —— 必须原样交给用户看
        let out = execute(&client, &spec("GET", server.url("/missing")))
            .await
            .expect("4xx 不该返回 Err");

        assert_eq!(out.status, 404);
        assert_eq!(out.status_text, "Not Found");
        assert_eq!(out.body, r#"{"error":"nope"}"#);
    }

    #[tokio::test]
    async fn redirect_is_followed_and_reported() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = serve_on(
            listener,
            vec![
                format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/final\r\n\
                     Content-Length: 0\r\nConnection: close\r\n\r\n"
                ),
                raw_response("200 OK", "text/plain", "landed"),
            ],
        );

        let client = client();
        let out = execute(&client, &spec("GET", server.url("/start")))
            .await
            .expect("跟随重定向后应当成功");

        assert_eq!(out.status, 200);
        assert_eq!(out.body, "landed");
        assert!(out.redirected, "必须告诉前端发生过重定向");
        assert!(out.url.ends_with("/final"), "最终 URL 实际：{}", out.url);
    }

    #[tokio::test]
    async fn too_many_redirects_is_classified() {
        // 自己指向自己的 302 → 必然触发重定向上限
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = serve_on(
            listener,
            vec![format!(
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}/\r\n\
                 Content-Length: 0\r\nConnection: close\r\n\r\n"
            )],
        );

        let mut s = spec("GET", server.url("/"));
        s.settings.max_redirects = 2;
        let client = client_for_spec(&s);

        let err = execute(&client, &s).await.unwrap_err();
        assert_eq!(err.code(), codes::REDIRECT, "实际：{err}");
    }

    #[tokio::test]
    async fn no_follow_redirects_surfaces_the_3xx() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let server = serve_on(
            listener,
            vec![format!(
                "HTTP/1.1 301 Moved Permanently\r\nLocation: http://127.0.0.1:{port}/final\r\n\
                 Content-Length: 0\r\nConnection: close\r\n\r\n"
            )],
        );

        let mut s = spec("GET", server.url("/old"));
        s.settings.follow_redirects = false;
        // ⚠️ client 必须**由这份 settings 建出来**。之前这里写的是 `Client::new()`，
        // 那是默认「跟随 10 跳」的策略 —— 断言「应当拿到 301」永远不可能成立。
        let client = client_for_spec(&s);

        let out = execute(&client, &s).await.expect("应当把 301 交给用户");
        assert_eq!(out.status, 301);
        assert!(!out.redirected, "没有跟随就不该标记为已重定向");
        assert!(out
            .headers
            .iter()
            .any(|h| h.name.eq_ignore_ascii_case("location") && h.value.ends_with("/final")));
    }

    #[tokio::test]
    async fn oversized_body_is_truncated_but_size_is_accurate() {
        let big = "x".repeat(500);
        let server = serve_once("200 OK", "text/plain", &big);
        let client = client();

        let out = execute_with_limit(&client, &spec("GET", server.url("/big")), 100)
            .await
            .expect("应当成功");

        assert!(out.truncated);
        assert_eq!(out.body.len(), 100);
        assert_eq!(out.body_size, 500, "字节数必须是**截断前**的真实值");
    }

    #[tokio::test]
    async fn truncation_in_the_middle_of_a_multibyte_char_is_still_text() {
        // 每个汉字 3 字节，切在 4 字节处正好切断第二个字
        let server = serve_once("200 OK", "text/plain; charset=utf-8", "中文内容测试");
        let client = client();

        let out = execute_with_limit(&client, &spec("GET", server.url("/cn")), 4)
            .await
            .expect("应当成功");

        assert_eq!(
            out.body_encoding,
            BodyEncoding::Text,
            "截断不该把它变成二进制"
        );
        assert_eq!(out.body, "中");
        assert_eq!(out.body_size, 18);
    }

    #[tokio::test]
    async fn binary_body_is_marked_instead_of_mojibake() {
        // PNG 魔数：0x89 不是合法的 UTF-8 起始字节
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.read(&mut [0u8; 1024]);
                let mut response = b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\n\
                                     Content-Length: 4\r\nConnection: close\r\n\r\n"
                    .to_vec();
                response.extend_from_slice(&[0x89, 0x50, 0x4e, 0x47]);
                let _ = stream.write_all(&response);
            }
        });

        let client = client();
        let out = execute(
            &client,
            &spec("GET", format!("http://127.0.0.1:{port}/logo.png")),
        )
        .await
        .expect("应当成功");

        assert_eq!(out.body_encoding, BodyEncoding::Binary);
        assert!(out.body.is_empty(), "二进制内容不该被硬转成字符串");
        assert_eq!(out.body_size, 4);
    }

    // ============================================================
    // 失败路径（真连、真超时，不 mock）
    // ============================================================

    #[tokio::test]
    async fn connection_refused_is_classified() {
        // 1 号端口不会有服务监听
        let client = client();
        let err = execute(&client, &spec("GET", "http://127.0.0.1:1/".into()))
            .await
            .expect_err("应当连不上");

        assert_eq!(err.code(), codes::CONN_REFUSED, "实际：{err}");
        assert!(err.retryable(), "连接被拒是可以重试的");
    }

    #[tokio::test]
    async fn dns_failure_is_classified() {
        // `.invalid` 是 RFC 2606 保留的顶级域，保证解析不出来
        let client = client();
        let err = execute(
            &client,
            &spec("GET", "http://reqly-does-not-exist.invalid/".into()),
        )
        .await
        .expect_err("域名不该解析成功");

        assert_eq!(err.code(), codes::DNS, "实际：{err}");
        // 前端要能拿到域名来拼「域名解析失败：xxx」
        let params = err.params().expect("DNS 错误要带 host");
        assert_eq!(params["host"], "reqly-does-not-exist.invalid");
    }

    #[tokio::test]
    async fn timeout_is_classified() {
        // 接受连接但永不响应 —— 这才能测到「读超时」而不是「连不上」
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming() {
                match stream {
                    Ok(s) => held.push(s), // 拿着连接不放、也不写任何东西
                    Err(_) => break,
                }
            }
        });

        let mut s = spec("GET", format!("http://127.0.0.1:{port}/slow"));
        s.settings.timeout_ms = 300;
        let client = client_for_spec(&s);

        let started = Instant::now();
        let err = execute(&client, &s).await.expect_err("应当超时");

        assert_eq!(err.code(), codes::TIMEOUT, "实际：{err}");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "超时该在 300ms 左右返回，实际 {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn invalid_url_is_rejected_before_sending() {
        let client = client();

        for bad in ["", "   ", "http://a b/", "ftp://example.com/x"] {
            let err = execute(&client, &spec("GET", bad.to_string()))
                .await
                .unwrap_err();
            assert_eq!(
                err.code(),
                codes::URL,
                "`{bad}` 应当被判为地址非法，实际：{err}"
            );
        }
    }

    #[tokio::test]
    async fn invalid_method_is_rejected() {
        let client = client();
        let err = execute(&client, &spec("GE T", "http://127.0.0.1:1/".into()))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            AppError::InvalidInput {
                field: "method",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn invalid_header_name_is_rejected() {
        let client = client();
        let mut s = spec("GET", "http://127.0.0.1:1/".into());
        s.headers = vec![KeyValueItem::new("Bad Header", "x")];

        let err = execute(&client, &s).await.unwrap_err();
        assert!(
            matches!(
                &err,
                AppError::InvalidInput {
                    field: "headers",
                    ..
                }
            ),
            "实际：{err}"
        );
    }

    #[tokio::test]
    async fn malformed_json_body_is_rejected_locally() {
        let client = client();
        let mut s = spec("POST", "http://127.0.0.1:1/".into());
        s.body_type = BodyType::Json;
        s.body = "{ not json".into();

        // 本地就能判出来的错，不该白跑一趟网络
        let err = execute(&client, &s).await.unwrap_err();
        assert_eq!(err.code(), codes::BODY_NOT_JSON, "实际：{err}");
    }

    #[tokio::test]
    async fn multipart_and_binary_are_reported_as_not_implemented() {
        let client = client();

        for body_type in [BodyType::Form, BodyType::Binary] {
            let mut s = spec("POST", "http://127.0.0.1:1/".into());
            s.body_type = body_type;
            let err = execute(&client, &s).await.unwrap_err();
            assert_eq!(err.code(), codes::NOT_IMPLEMENTED, "实际：{err}");
        }
    }

    #[tokio::test]
    async fn empty_json_body_sends_nothing() {
        let server = serve_once("200 OK", "text/plain", "ok");
        let client = client();

        let mut s = spec("POST", server.url("/post"));
        s.body_type = BodyType::Json;
        s.body = "   ".into();

        execute(&client, &s).await.expect("空 body 不该报错");
        let raw = server.request();
        assert_eq!(body_of(&raw), "", "空 body 不该发出任何内容");
    }

    // ============================================================
    // 取消
    // ============================================================

    #[tokio::test]
    async fn cancelled_request_stops_immediately() {
        // 接受连接但永不响应：只有取消能把它从这个 await 里拽出来
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming() {
                match stream {
                    Ok(s) => held.push(s),
                    Err(_) => break,
                }
            }
        });

        let mut s = spec("GET", format!("http://127.0.0.1:{port}/hang"));
        // 超时给足 30s，保证这次返回**一定**是取消造成的，不是超时兜住的
        s.settings.timeout_ms = 30_000;
        let client = client_for_spec(&s);

        let token = CancellationToken::new();
        let canceller = token.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            canceller.cancel();
        });

        let started = Instant::now();
        let err = execute_with_cancel(&client, &s, Some(&token))
            .await
            .expect_err("应当被取消");

        assert_eq!(err.code(), codes::CANCELLED, "实际：{err}");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "取消该立刻生效（不该等 30s 超时），实际 {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn already_cancelled_token_sends_nothing() {
        let server = serve_once("200 OK", "text/plain", "should-not-arrive");

        let token = CancellationToken::new();
        token.cancel();

        let err = execute_with_cancel(&client(), &spec("GET", server.url("/x")), Some(&token))
            .await
            .expect_err("已经取消的信号应当立刻返回");

        assert_eq!(err.code(), codes::CANCELLED, "实际：{err}");
        assert!(
            server.requests.try_recv().is_err(),
            "取消之后一个字节都不该发出去"
        );
    }
}
