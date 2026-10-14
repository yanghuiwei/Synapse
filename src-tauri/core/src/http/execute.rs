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
use crate::template::resolver::UnresolvedPlaceholder;

/// 内联返回给前端的响应体上限。
///
/// 8MB 的依据：前端在 2MB 以上就退化到纯文本模式，8MB 是「还能塞进 IPC 且不卡死」的边界。
/// 再大就该走流式落盘（design.md §12 把它列为低风险项，留给 P8）。
const MAX_INLINE_BODY: usize = 8 * 1024 * 1024;

/// 前端传过来的请求定义。
///
/// 刻意**不**从数据库读：这样「还没保存的新请求」也能直接发（Postman 的核心体验），
/// 压测（P6）也能对一份改了一半的草稿发起 —— 保存和发送是两件事。
///
/// `Serialize` 是为了历史快照：AI 路径（MCP / CLI）没有前端那一步「发完再写历史」，
/// 由 `Ops` 自己把 spec 序列化进 `history.snapshot`。形状必须与前端写进去的那份
/// 一模一样（都来自这个结构 + `camelCase`），否则历史回填会一边能解一边解不了。
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// 这条请求**在库里的 id**。
    ///
    /// 用途只剩一个：Token 源是**逐请求一份**的（008 迁移），所以要知道
    /// 「这次发的是哪一条请求」才能查到它自己那份取 token 配置。
    /// 所以它不是可有可无的元数据，而是这次发送语义的一部分 ——
    /// 放进 spec 里，历史快照回填之后再发送也能找回同样的上下文。
    ///
    /// ⚠️ 可空是**语义**上的，不只是兼容旧快照：未保存的草稿没有 id，
    /// 这时 `{{token.access}}` 会被原样保留并报「没有可用 Token 源」。
    /// 让 `None` 退化去借用别的请求的 token，正是 008 要消灭的行为。
    ///
    /// 008 之前这里是 `folder_id`（沿分类链向上找最近的 Token 源）。
    #[serde(default)]
    pub request_id: Option<i64>,
    /// `multipart/form-data` 的字段行。**只有 `body_type = form` 时才被读取**。
    ///
    /// ## 为什么这里要多开一条通道
    ///
    /// 其余类型的 body 都是一段文本，`body` 一个字段装得下。multipart 装不下：
    /// 它每一行要么是文本、要么是一个**磁盘上的文件**（文件内容不进内存，
    /// 更不该跟着 IPC 走一趟），而 boundary 又必须由发送方生成 ——
    /// 也就是说这份内容只能由后端来拼，前端给不出成品串。
    ///
    /// ## 与 `body` 的关系
    ///
    /// `form_fields` 非空时它是**权威**；为空而 `body` 非空时退回解析 `body`
    /// （`k=v&k=@路径`，curl 的 `-F` 那套），这样 MCP / CLI / 导入器
    /// 只写 `body` 列也能发出 multipart 请求。
    #[serde(default)]
    pub form_fields: Vec<KeyValueItem>,
}

impl RequestSpec {
    /// 给日志/历史用的简短描述（不含 query，避免把 token 写进日志）
    pub fn label(&self) -> String {
        format!("{} {}", self.method.trim().to_uppercase(), self.url.trim())
    }
}

/// 库里的一行 → 要发出去的请求定义。
///
/// **只有一处转换**：发送主路径（`http::pipeline`）与「取 token 的那个请求」
/// （`http::token`）都走它。两处各写一遍的话，将来加字段必然漏一处 ——
/// 而漏在取 token 那条路径上的表现是「token 接口的参数没有展开」，极难查。
impl From<&crate::storage::models::ApiRequest> for RequestSpec {
    fn from(request: &crate::storage::models::ApiRequest) -> Self {
        Self {
            method: request.method.clone(),
            url: request.url.clone(),
            params: request.params.clone(),
            headers: request.headers.clone(),
            body_type: request.body_type,
            body: request.body.clone(),
            settings: request.settings.clone(),
            request_id: Some(request.id),
            // 无条件带上：`build_body` 只在 `body_type = form` 时读它，
            // 按类型过滤反而多一处可能在「切类型后忘同步」的地方。
            //
            // 这条路径**必须**带上它：Token 源那个取 token 的请求如果是
            // multipart 表单（OAuth 的 `client_credentials` 就是），
            // 字段漏掉的表现是「token 接口收到一个空表单」，服务端回 400，
            // 而界面上看不出任何异常。
            //
            // ⚠️ 取的是 `form_fields`，不是 `body_rows` —— 后者是 urlencoded 那一槽，
            // 拿错了会把 urlencoded 的键值当成 multipart 字段发出去。
            form_fields: request.form_fields.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

/// 一份「这是个文件」的响应落在磁盘上的位置（导出 Excel / PDF / 图片…）。
///
/// ## 为什么先落在临时区，而不是直接问用户存哪儿
///
/// 落盘发生**在执行时**（那会儿字节还在手里），而问用户存哪儿是个交互动作。
/// 合成一步的代价是：每一次「看起来像文件」的响应都弹一次对话框 ——
/// 包括用户只是想看一眼状态码的那些请求。
/// 所以分两步：执行时写进会话临时区，用户点了「保存到文件」才复制到他选的地方。
///
/// ## 为什么不是「前端拿字节自己存」
///
/// 两个原因：① 超过内联上限的响应在前端只剩被截断的前 8MB，存下来是个坏文件；
/// ② 二进制内容本来就没进 `body`（见 [`BodyEncoding::Binary`]）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyFile {
    /// 临时区里的绝对路径。**不是**用户的最终落点。
    pub path: String,
    /// 建议的文件名：Content-Disposition → URL 末段 → `download-<时间戳>.<扩展名>`
    pub name: String,
    /// 完整字节数（不受内联截断影响）
    pub size: u64,
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
    /// 这份响应看起来是个文件、且执行时给了临时区 → 完整字节已经落在那儿。
    ///
    /// `None` 的两种含义（前端只需按「有没有保存按钮」处理，不必区分）：
    /// ① 这不是文件响应（绝大多数情况）；② 调用方没给临时区
    /// （MCP / CLI / 压测路径都不给 —— 它们不需要这个入口）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_file: Option<BodyFile>,
    /// 本次发送里**没能解析**的占位符（它们的原文被原样留在了请求里）。
    ///
    /// 挂在响应上而不是单开一条事件：它就是「这一次发送」的属性，
    /// 前端拿到响应时顺手就能把提示渲染在结果旁边。
    /// 由流水线填（见 `http::pipeline`）—— 执行层本身不碰模板。
    #[serde(default)]
    pub unresolved_placeholders: Vec<UnresolvedPlaceholder>,
    /// 本次发送里 Token 的**实际使用情况**：刷没刷、走的哪个取 Token 接口、还剩多久。
    ///
    /// 与 `body_file` 同一个约定：没用到 Token 时字段**不出现**（而不是给一个
    /// `null`）——「没有」只有一种表示，前端一个 `if` 就够。
    ///
    /// 由流水线填（见 `http::pipeline::token_context`）：执行层既不知道模板、
    /// 也不知道 Token 源，它是**纯粹的执行器**。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<crate::http::token::TokenUsage>,
    /// 本次**真实发出**的请求对应的 curl，凭据已打码。
    ///
    /// 由编排层填（见 `service::request`）而不是执行层：执行到这里时请求已经
    /// 变成 builder 了，「用户写了什么、被解析成了什么」这层信息只剩编排层知道。
    ///
    /// 明文单独占一个字段（[`Self::curl_plain`]）而不是让调用方自己决定要不要打码 ——
    /// 打码是**谁在看**的问题：桌面端是用户本人在看自己机器上的 token，
    /// 而 MCP / CLI 的输出会被 AI 读走。那条路径上两个字段都不填（见 `service::ops`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curl: Option<String>,
    /// 同上，但凭据是明文。**只有桌面端会拿到**。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub curl_plain: Option<String>,
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
    execute_inner(client, spec, MAX_INLINE_BODY, cancel, None).await
}

/// 可取消 + 给一个临时目录：**看起来是文件**的响应会把完整字节落到那里，
/// 并在结果里回一个 [`BodyFile`]（前端据此给「保存到文件」）。
///
/// 不给目录（MCP / CLI / 压测）就是 [`execute_with_cancel`] 的行为 ——
/// 那些入口没有「点一下存到磁盘」这个动作，白白占磁盘没有意义。
pub async fn execute_with_blobs(
    client: &Client,
    spec: &RequestSpec,
    cancel: Option<&CancellationToken>,
    blobs: &std::path::Path,
) -> AppResult<HttpResponsePayload> {
    execute_inner(client, spec, MAX_INLINE_BODY, cancel, Some(blobs)).await
}

/// 带上限的版本。单独拆出来是为了让截断逻辑可测 ——
/// 否则测试得真造一个 8MB 响应。
pub async fn execute_with_limit(
    client: &Client,
    spec: &RequestSpec,
    max_inline: usize,
) -> AppResult<HttpResponsePayload> {
    execute_inner(client, spec, max_inline, None, None).await
}

async fn execute_inner(
    client: &Client,
    spec: &RequestSpec,
    max_inline: usize,
    cancel: Option<&CancellationToken>,
    blobs: Option<&std::path::Path>,
) -> AppResult<HttpResponsePayload> {
    let method = parse_method(&spec.method)?;
    let mut url = parse_url(&spec.url)?;
    apply_query_params(&mut url, &spec.params);
    // 凭据不再有单独的一支：`X-Api-Key` 之类就是 `params` / `headers` 里的一行，
    // 上面那次 `apply_query_params` 已经把它算进去了。

    let built = build_body(&spec.body_type, &spec.body, &spec.form_fields)?;
    let mut headers = build_headers(&spec.headers)?;
    // 用户显式设了 content-type 就以他为准（Postman 的行为，也是唯一不让人意外的行为）
    match &built {
        BuiltBody::Text(_, Some(ct)) => {
            if !headers.contains_key(CONTENT_TYPE) {
                headers.insert(CONTENT_TYPE, HeaderValue::from_static(ct));
            }
        }
        // multipart 的 content-type 里必须带 boundary，而 boundary 是 reqwest
        // 按这份 form 现生成的 —— 用户手写的那条注定对不上（他没有 boundary 可写）。
        // 所以这里清掉它，让 reqwest 自己设，而不是发两条冲突的头出去。
        BuiltBody::Multipart(_) => {
            headers.remove(CONTENT_TYPE);
        }
        // 文本体但没有缺省类型（目前只有「空 JSON」这一种）→ 什么都不加，
        // 与 `Text(_, None)` 的语义一致：这段内容不自带类型
        BuiltBody::Text(_, None) | BuiltBody::None => {}
    }

    let mut builder = client.request(method, url.clone()).headers(headers);
    match built {
        BuiltBody::None => {}
        BuiltBody::Text(text, _) => builder = builder.body(text),
        BuiltBody::Multipart(form) => builder = builder.multipart(form),
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

    // 落盘用**完整**字节（`bytes`），不是被截断的 `slice`。
    // 导出 Excel 这类响应的体积往往就在内联上限附近，拿截断的那段存出来是个坏文件。
    let body_file = match blobs {
        Some(dir) => {
            match spill_response_file(dir, &bytes, &headers_out, &final_url, body_encoding) {
                Ok(found) => found,
                Err(e) => {
                    // 落盘失败**不**让这次请求失败：响应本身是好的，用户少一个下载按钮而已。
                    // 把它升成错误会让「服务端答 200、界面报红」这种最费解的现象出现。
                    eprintln!("[reqly] 响应落盘失败（不影响本次响应）：{e}");
                    None
                }
            }
        }
        None => None,
    };

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
        body_file,
        // 占位符报告与 Token 使用报告都归流水线填 —— 执行层不知道它们的存在
        unresolved_placeholders: Vec::new(),
        token_usage: None,
        // 同上：curl 要的是「解析后的 spec」，那是编排层手里才有的东西
        curl: None,
        curl_plain: None,
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
/// `pub(crate)` 而不是私有：curl 导出（[`crate::http::curl`]）必须用**同一个**拼法，
/// 否则那条命令与真实请求在编码上会有出入，而它存在的全部意义就是「一模一样」。
pub(crate) fn parse_url(raw: &str) -> AppResult<Url> {
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

/// `pub(crate)`：curl 导出共用同一份拼接（见 [`parse_url`] 同一条理由）。
pub(crate) fn apply_query_params(url: &mut Url, params: &[KeyValueItem]) {
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

/// 组装好的请求体。
///
/// 用枚举而不是 `Option<String>`：multipart 的成品**不是字符串**
/// （boundary 由 reqwest 现生成），而且它与文本体对 `Content-Type` 的处理正好相反
/// —— 文本体允许被用户手写的头覆盖，multipart 必须由我们设。
enum BuiltBody {
    /// 没有请求体
    None,
    /// 一段文本 + 缺省 content-type（用户显式设了就以他为准）
    Text(String, Option<&'static str>),
    /// `multipart/form-data`，`Content-Type` 由 reqwest 连 boundary 一起生成
    Multipart(reqwest::multipart::Form),
}

/// 返回组装好的请求体。
fn build_body(
    body_type: &BodyType,
    body: &str,
    form_fields: &[KeyValueItem],
) -> AppResult<BuiltBody> {
    Ok(match body_type {
        BodyType::None => BuiltBody::None,
        BodyType::Json => {
            if body.trim().is_empty() {
                return Ok(BuiltBody::None);
            }
            // 顺手校验：JSON 写错了当场告诉用户，比等服务端回一个语焉不详的 400 强得多
            serde_json::from_str::<serde_json::Value>(body).map_err(|e| {
                AppError::http(
                    codes::BODY_NOT_JSON,
                    format!("request body is not valid json: {e}"),
                )
            })?;
            BuiltBody::Text(body.to_string(), Some("application/json"))
        }
        // GraphQL 的 body 本身就是 JSON 字符串 `{"query":"…","variables":{…}}`
        BodyType::Graphql => BuiltBody::Text(body.to_string(), Some("application/json")),
        BodyType::Urlencoded => {
            BuiltBody::Text(body.to_string(), Some("application/x-www-form-urlencoded"))
        }
        BodyType::Raw => BuiltBody::Text(body.to_string(), Some("text/plain; charset=utf-8")),
        BodyType::Form => {
            // 字段以 `form_fields` 为准；它空而 `body` 非空时说明这份内容来自
            // 只认 `body` 的写入方（MCP / CLI / 导入器），退回按 `k=v&k=@路径` 解析。
            let owned;
            let fields: &[KeyValueItem] = if form_fields.is_empty() {
                owned = form_fields_from_body(body);
                &owned
            } else {
                form_fields
            };

            // 空表单：与空 JSON body 一样，什么都不发 —— 发一个只有结束边界的
            // 空 multipart 只会让服务端回一个莫名其妙的 400。
            // 计数在这里做是因为 `Form` 没有公开的读取接口。
            let usable = fields.iter().any(|f| f.enabled && !f.key.trim().is_empty());
            if usable {
                BuiltBody::Multipart(build_multipart(fields)?)
            } else {
                BuiltBody::None
            }
        }
        // 要读磁盘上的一个文件当整个请求体，涉及文件选择器（design.md 排在 P8）。
        // 明确报「未实现」，而不是把一段 JSON 当文件内容发出去。
        BodyType::Binary => return Err(AppError::NotImplemented { feature: "binary" }),
    })
}

/// 拼 multipart。文件字段在这一步才真正读盘 —— 编辑期只存路径。
fn build_multipart(fields: &[KeyValueItem]) -> AppResult<reqwest::multipart::Form> {
    let mut form = reqwest::multipart::Form::new();

    for field in fields
        .iter()
        .filter(|f| f.enabled && !f.key.trim().is_empty())
    {
        let name = field.key.trim().to_string();

        let Some(path) = field
            .file
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
        else {
            form = form.text(name, field.value.clone());
            continue;
        };

        let bytes = std::fs::read(path).map_err(|e| AppError::InvalidInput {
            field: "formFields",
            reason: format!("文件字段「{name}」读不出来：{path}（{e}）"),
        })?;
        let file_name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());

        let mut part = reqwest::multipart::Part::bytes(bytes).file_name(file_name.clone());
        // 显式带上 Content-Type：.NET / Java 那几套 multipart 解析器是**按 part 的
        // Content-Type 判断「这是不是文件」的**，缺了它有些框架会当成普通表单值，
        // 表现是服务端说「文件字段为空」而请求明明带着几百 KB。
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(mime_for(&file_name)));
        part = part.headers(headers);

        form = form.part(name, part);
    }

    Ok(form)
}

/// `k=v&k=@C:\path\a.xlsx` → 字段行。
///
/// `@` 前缀是 curl `-F name=@file` 的写法（导入器与 MCP 都从那条路径进来）。
/// 只把这一个前缀当文件标记，其余原样 —— 值里出现 `@`（邮箱、`@handler`）不受影响。
///
/// **只服务 multipart**：urlencoded 那边值里的 `@` 很常见（`email=@qq.com`），
/// 按文件解释会让那行在发送时被当成路径去读盘。
///
/// ## 换行也算分隔符
///
/// 先按行切开，每行再走 `form_urlencoded::parse`。只认 `&` 的话，「一行一个
/// `k=v`」的 body（curl 的多行 `--data`、`.http` 文件、从接口文档整段复制下来的
/// 表单内容）会变成**一个**字段，键是第一个 `=` 左边那截，值是剩下全部 ——
/// 表现是服务端回「没有 grant_type」，而送出去的表单看着并不空。
///
/// 与前端 `lib/urlencoded.ts` 的 `PAIR_SEPARATOR_RE` 是**同一个口径**：
/// 这条路径服务 MCP / CLI / 只看 `body` 的历史快照，只改前端等于只修一半。
///
/// `pub(crate)`：curl 导出的 multipart 也要按同一份口径拆行，
/// 否则「导出 → 导入」会把多行表单并成一个字段。
pub(crate) fn form_fields_from_body(body: &str) -> Vec<KeyValueItem> {
    let mut fields = Vec::new();

    for line in body.split(['\n', '\r']) {
        if line.trim().is_empty() {
            continue;
        }
        for (key, value) in url::form_urlencoded::parse(line.as_bytes()) {
            let (key, value) = (key.into_owned(), value.into_owned());
            fields.push(match value.strip_prefix('@') {
                Some(path) if !path.trim().is_empty() => KeyValueItem {
                    key,
                    value: String::new(),
                    enabled: true,
                    desc: None,
                    file: Some(path.to_string()),
                },
                _ => KeyValueItem::new(key, value),
            });
        }
    }

    fields
}

/// 按扩展名给个 Content-Type。认不出的一律 `application/octet-stream` ——
/// 猜错比不猜更坏（服务端会按错误类型去解析内容）。
fn mime_for(file_name: &str) -> &'static str {
    let ext = file_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/vnd.microsoft.icon",
        "pdf" => "application/pdf",
        "txt" | "log" | "md" => "text/plain; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "json" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html; charset=utf-8",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "tar" => "application/x-tar",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------
// 「这是一个文件」的响应：落盘 + 命名
// ---------------------------------------------------------------

/// 临时区里最多留几份。超了按修改时间删最旧的。
///
/// 为什么要上限：每发一次导出类请求就落一份文件，不设限就是慢性占盘 ——
/// 而用户真正会去点「保存」的只是最后那几次。
const KEEP_BLOBS: usize = 24;

/// 把一份「看起来是文件」的响应落到临时区。
///
/// 返回 `Ok(None)` = 判断下来这不是文件响应（绝大多数请求）；
/// 返回 `Err` = 磁盘写不进去，调用方按「没有下载入口」处理即可。
fn spill_response_file(
    dir: &std::path::Path,
    bytes: &[u8],
    headers: &[HttpHeader],
    final_url: &str,
    encoding: BodyEncoding,
) -> std::io::Result<Option<BodyFile>> {
    let content_type = content_type_of(headers);
    if !looks_like_file(headers, &content_type, encoding) {
        return Ok(None);
    }

    std::fs::create_dir_all(dir)?;

    let name = suggested_file_name(headers, final_url, &content_type);
    // 前缀一个时间戳：同一个接口导出两次不该互相覆盖
    // （覆盖掉一份刚生成、用户可能正在看的文件是没法挽回的）
    let path = dir.join(format!("{}-{name}", stamp()));
    std::fs::write(&path, bytes)?;
    prune_blobs(dir, KEEP_BLOBS);

    Ok(Some(BodyFile {
        path: path.display().to_string(),
        name,
        size: bytes.len() as u64,
    }))
}

/// 这份响应该不该给「保存到文件」。
///
/// 三个判据取并集，且**都是「是」的充分条件**：
/// * 解不出 UTF-8（`Binary`）—— 内容本来就不是文本；
/// * `Content-Disposition: attachment` —— 服务端明说了这是下载；
/// * Content-Type 落在二进制清单里 —— 导出接口的常规形态。
///
/// 为什么不单看第一条：CSV / 纯文本导出是合法 UTF-8，会被当成文本渲染，
/// 而用户要的是文件。反过来只看 Content-Type 又会漏掉那些把 Excel 标成
/// `text/plain` 的老接口。
fn looks_like_file(headers: &[HttpHeader], content_type: &str, encoding: BodyEncoding) -> bool {
    if encoding == BodyEncoding::Binary {
        return true;
    }

    if headers.iter().any(|h| {
        h.name.eq_ignore_ascii_case("content-disposition")
            && h.value.to_ascii_lowercase().contains("attachment")
    }) {
        return true;
    }

    const BINARY_TYPES: [&str; 15] = [
        "application/octet-stream",
        "application/zip",
        "application/pdf",
        "application/gzip",
        "application/x-7z-compressed",
        "application/vnd.rar",
        "application/x-tar",
        "application/msword",
        "application/vnd.ms-excel",
        "application/vnd.ms-powerpoint",
        "application/vnd.openxmlformats-",
        "image/",
        "audio/",
        "video/",
        "font/",
    ];
    BINARY_TYPES.iter().any(|t| content_type.starts_with(t))
}

/// 建议的文件名：`Content-Disposition` → URL 末段 → `download-<时间戳>.<扩展名>`。
fn suggested_file_name(headers: &[HttpHeader], final_url: &str, content_type: &str) -> String {
    if let Some(name) = headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-disposition"))
        .and_then(|h| filename_from_disposition(&h.value))
    {
        return name;
    }

    if let Some(name) = filename_from_url(final_url) {
        return name;
    }

    format!("download-{}.{}", stamp(), extension_for(content_type))
}

/// 从 `Content-Disposition` 里取文件名。
///
/// 两种写法都要认：`filename*=UTF-8''%E6%8A%A5%E8%A1%A8.xlsx`（RFC 5987，
/// 中文名走这条）与 `filename="report.xlsx"`。**先看带星号的** ——
/// 带星号的那条是扩展，服务端同时给两条时它是更准的那个。
fn filename_from_disposition(value: &str) -> Option<String> {
    for param in value.split(';').skip(1) {
        let param = param.trim();
        let Some(rest) = param.strip_prefix("filename*=") else {
            continue;
        };
        // `UTF-8''<百分号编码>`
        let encoded = rest.splitn(3, '\'').nth(2).unwrap_or("");
        let decoded = decode_percent(encoded);
        if let Some(name) = sanitize_file_name(&decoded) {
            return Some(name);
        }
    }

    for param in value.split(';').skip(1) {
        let param = param.trim();
        let Some(rest) = param.strip_prefix("filename=") else {
            continue;
        };
        if let Some(name) = sanitize_file_name(rest.trim().trim_matches('"')) {
            return Some(name);
        }
    }

    None
}

/// URL 的最后一段。要求带扩展名 —— `…/export` 这种取出来的「名字」
/// 存到磁盘上是个没有类型的文件，不如退回带时间戳的兜底名。
fn filename_from_url(url: &str) -> Option<String> {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let last = without_query.rsplit('/').find(|s| !s.is_empty())?;
    let decoded = decode_percent(last);
    if !decoded.contains('.') {
        return None;
    }
    sanitize_file_name(&decoded)
}

/// 洗掉路径分隔符与 Windows 不允许的字符。
///
/// 这不是洁癖：`filename` 来自服务端，`../` 能把它变成往任意目录写文件。
fn sanitize_file_name(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    // Windows 上结尾的点和空格会被静默吃掉，留下一个打不开的文件
    let cleaned = cleaned.trim().trim_matches(['.', ' ']).to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn decode_percent(text: &str) -> String {
    url::form_urlencoded::parse(text.replace('+', " ").as_bytes())
        .map(|(k, _)| k.into_owned())
        .next()
        .unwrap_or_else(|| text.to_string())
}

fn extension_for(content_type: &str) -> &'static str {
    let ct = content_type.split(';').next().unwrap_or("").trim();
    match ct {
        "application/pdf" => "pdf",
        "application/zip" | "application/x-zip-compressed" => "zip",
        "application/gzip" => "gz",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/msword" => "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.ms-powerpoint" => "ppt",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "text/csv" => "csv",
        "text/plain" => "txt",
        "text/html" => "html",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        _ => "bin",
    }
}

/// 文件名前缀用的时间戳。用本地时间 —— 它是给用户认的，不是给机器比大小的。
fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S%3f").to_string()
}

/// 只保留最近的 `keep` 份，其余按修改时间从旧到新删掉。删不掉就算了 ——
/// 临时区清理失败不该影响任何一次发送。
fn prune_blobs(dir: &std::path::Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((meta.modified().ok()?, entry.path()))
        })
        .collect();

    if files.len() <= keep {
        return;
    }

    files.sort_by_key(|(time, _)| *time);
    for (_, path) in files.iter().take(files.len() - keep) {
        let _ = std::fs::remove_file(path);
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
            request_id: None,
            form_fields: Vec::new(),
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
                file: None,
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

    /// 只剩 `binary` 还没实现 —— multipart 从这一版起是真的发出去了。
    #[tokio::test]
    async fn binary_body_is_reported_as_not_implemented() {
        let client = client();
        let mut s = spec("POST", "http://127.0.0.1:1/".into());
        s.body_type = BodyType::Binary;

        // 明确报「未实现」，而不是把一段 JSON 当文件内容发出去
        let err = execute(&client, &s).await.unwrap_err();
        assert_eq!(err.code(), codes::NOT_IMPLEMENTED, "实际：{err}");
    }

    // ============================================================
    // multipart（`body_type = form`）
    // ============================================================

    fn field(key: &str, value: &str) -> KeyValueItem {
        KeyValueItem::new(key.to_string(), value.to_string())
    }

    /// 文件行：`value` 留空、`file` 是磁盘路径（与前端 `FormFieldTable` 存的一致）
    fn file_field(key: &str, path: &str) -> KeyValueItem {
        KeyValueItem {
            file: Some(path.to_string()),
            ..field(key, "")
        }
    }

    /// 一次性临时目录，`Drop` 时整个删掉。
    /// 名字带 pid + 纳秒：测试并行跑，同名目录会互相踩。
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("时钟应当在 unix epoch 之后")
                .as_nanos();
            let dir = std::env::temp_dir()
                .join(format!("reqly-blob-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("建临时目录");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 从请求头里取出 multipart 的 boundary（头名大小写不保证）
    fn boundary_of(raw: &str) -> String {
        let head = raw
            .split_once("\r\n\r\n")
            .expect("请求应当有头")
            .0
            .to_lowercase();
        head.lines()
            .find_map(|l| l.split("boundary=").nth(1))
            .expect("multipart 应当带 boundary")
            .trim()
            .to_string()
    }

    #[tokio::test]
    async fn form_body_is_sent_as_multipart_with_a_generated_boundary() {
        let server = serve_once("200 OK", "text/plain", "ok");
        let client = client();

        let mut s = spec("POST", server.url("/upload"));
        s.body_type = BodyType::Form;
        s.form_fields = vec![field("name", "reqly"), field("note", "a b")];

        execute(&client, &s).await.expect("multipart 应当发得出去");

        let raw = server.request();
        let head = raw
            .split_once("\r\n\r\n")
            .expect("请求应当有头")
            .0
            .to_lowercase();

        // boundary 由 reqwest 现生成。用户手写的那条 Content-Type 必须被清掉 ——
        // 他自己生成不出对的 boundary，两条头同时发出去服务端只会挑一条
        assert_eq!(
            head.matches("content-type:").count(),
            1,
            "Content-Type 只该有一条：{head}"
        );

        let boundary = boundary_of(&raw);
        let body = body_of(&raw);
        assert!(body.contains("name=\"name\""), "实际：{body}");
        assert!(body.contains("reqly"), "实际：{body}");
        // 值原样进去，不做 urlencoded 编码 —— multipart 不是 `k=v&…`
        assert!(body.contains("a b"), "实际：{body}");
        // 收尾边界在：结构是完整的，服务端不会读到一半卡住
        assert!(
            body.trim_end().ends_with(&format!("--{boundary}--")),
            "实际：{body}"
        );
    }

    #[tokio::test]
    async fn multipart_file_field_is_read_from_disk_with_name_and_part_type() {
        let scratch = Scratch::new("upload");
        let path = scratch.0.join("note.txt");
        std::fs::write(&path, "file-bytes-123").expect("写测试文件");

        let server = serve_once("200 OK", "text/plain", "ok");
        let client = client();

        let mut s = spec("POST", server.url("/upload"));
        s.body_type = BodyType::Form;
        s.form_fields = vec![file_field("doc", &path.to_string_lossy())];

        execute(&client, &s)
            .await
            .expect("带文件的 multipart 应当发得出去");

        let raw = server.request();
        let lower = raw.to_lowercase();

        assert!(lower.contains("filename=\"note.txt\""), "实际：{raw}");
        // part 的 Content-Type 必须显式带上：.NET / Java 那几套解析器靠它判断
        // 「这是不是文件」，缺了会当成普通表单值，报「文件字段为空」
        assert!(
            lower.contains("content-type: text/plain; charset=utf-8"),
            "实际：{raw}"
        );
        assert!(raw.contains("file-bytes-123"), "实际：{raw}");
    }

    #[tokio::test]
    async fn an_unreadable_upload_file_is_a_typed_input_error() {
        let client = client();
        let mut s = spec("POST", "http://127.0.0.1:1/".into());
        s.body_type = BodyType::Form;
        s.form_fields = vec![file_field("doc", r"C:\reqly\definitely\not\here\nope.txt")];

        // 路径填错是**填错了**，不是网络问题。分类错了用户会去查网络和超时
        let err = execute(&client, &s).await.unwrap_err();
        assert_eq!(err.code(), codes::INVALID_INPUT, "实际：{err}");
    }

    #[tokio::test]
    async fn form_body_string_is_parsed_when_there_are_no_field_rows() {
        let server = serve_once("200 OK", "text/plain", "ok");
        let client = client();

        // 只写 `body` 的写入方（MCP / CLI / 导入器）走这条回退路径：
        // 没有 `form_fields` 时按 `k=v&k=@路径` 解析，而不是发一个空表单
        let mut s = spec("POST", server.url("/upload"));
        s.body_type = BodyType::Form;
        s.body = "name=reqly&note=hello%20there".into();

        execute(&client, &s).await.expect("应当发出 multipart");

        let body = body_of(&server.request()).to_string();
        assert!(body.contains("name=\"name\""), "实际：{body}");
        assert!(body.contains("reqly"), "实际：{body}");
        // 百分号编码在解析这一步解回来 —— 服务端要看到的是原文
        assert!(body.contains("hello there"), "实际：{body}");
    }

    /// 「一行一个 `k=v`」的 body 也得拆成多个字段。
    ///
    /// 只认 `&` 的话整段会变成一个字段（键是第一行 `=` 左边那截，值是剩下全部），
    /// 症状是服务端回「没有 grant_type」，而请求看着并不空 —— 这条形态来自
    /// curl 的多行 `--data`、`.http` 文件，以及从接口文档整段复制下来的表单内容。
    #[tokio::test]
    async fn a_newline_separated_form_body_becomes_one_field_per_line() {
        let server = serve_once("200 OK", "text/plain", "ok");
        let client = client();

        let mut s = spec("POST", server.url("/token"));
        s.body_type = BodyType::Form;
        // 末尾留一个换行：不能因此多出一个空字段
        s.body = "client_id=abc\ngrant_type=client_credentials\nscope=BillingApi UserApi\n".into();

        execute(&client, &s).await.expect("应当发出 multipart");

        let body = body_of(&server.request()).to_string();
        assert_eq!(
            body.matches("Content-Disposition").count(),
            3,
            "换行分隔的三行应当是三个字段，实际：{body}"
        );
        assert!(body.contains("name=\"grant_type\""), "实际：{body}");
        assert!(body.contains("client_credentials"), "实际：{body}");
        // 值里不该再留着别的行 —— 这一串粘连文本是「没拆开」的特征
        assert!(
            !body.contains("client_id=abc\ngrant_type"),
            "值里还裹着下一行，说明没按换行拆：{body}"
        );
    }

    #[tokio::test]
    async fn an_all_blank_form_sends_no_body_at_all() {
        let server = serve_once("200 OK", "text/plain", "ok");
        let client = client();

        let mut s = spec("POST", server.url("/post"));
        s.body_type = BodyType::Form;
        // 键名为空 = 这一行没写。发一个只有结束边界的空 multipart
        // 只会换来一个莫名其妙的 400
        s.form_fields = vec![field("", "leftover")];

        execute(&client, &s).await.expect("空表单不该报错");
        assert_eq!(body_of(&server.request()), "", "空表单不该发任何内容");
    }

    // ============================================================
    // 「文件型」响应落盘
    // ============================================================

    /// 带 `Content-Disposition` 的响应（`raw_response` 不接受附加头）
    fn attachment_response(content_type: &str, disposition: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: {content_type}\r\n\
             Content-Disposition: {disposition}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[tokio::test]
    async fn an_attachment_response_is_spilled_with_its_full_bytes() {
        let scratch = Scratch::new("spill");
        let client = client();
        let server = serve(vec![attachment_response(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "attachment; filename=\"report.xlsx\"",
            "XLSX-BYTES",
        )]);

        let out = execute_with_blobs(
            &client,
            &spec("GET", server.url("/export")),
            None,
            &scratch.0,
        )
        .await
        .expect("应当成功");

        let file = out.body_file.expect("attachment 响应应当落盘");
        assert_eq!(file.name, "report.xlsx");
        assert_eq!(file.size, 10);
        assert_eq!(
            std::fs::read(&file.path).expect("读回落盘的文件"),
            b"XLSX-BYTES"
        );
    }

    /// CSV 是合法 UTF-8，但服务端明说了这是下载 —— 两条信息各管各的
    #[tokio::test]
    async fn a_utf8_csv_export_still_gets_a_download_entry() {
        let scratch = Scratch::new("spill-csv");
        let client = client();
        let server = serve(vec![attachment_response(
            "text/csv; charset=utf-8",
            "attachment; filename=export.csv",
            "a,b\n1,2\n",
        )]);

        let out = execute_with_blobs(
            &client,
            &spec("GET", server.url("/export")),
            None,
            &scratch.0,
        )
        .await
        .expect("应当成功");

        // 内容是文本 ⇒ 界面上照样能展开预览
        assert_eq!(out.body_encoding, BodyEncoding::Text);
        // 但**同时**有下载入口。只看「解不解得出 UTF-8」会把 CSV 导出漏掉
        let file = out.body_file.expect("attachment 响应应当落盘");
        assert_eq!(file.name, "export.csv");
    }

    #[tokio::test]
    async fn a_plain_json_response_is_never_spilled() {
        let scratch = Scratch::new("spill-none");
        let client = client();
        let server = serve_once("200 OK", "application/json", r#"{"ok":true}"#);

        let out = execute_with_blobs(&client, &spec("GET", server.url("/api")), None, &scratch.0)
            .await
            .expect("应当成功");

        assert!(out.body_file.is_none(), "JSON 不该产生下载入口");
    }

    #[tokio::test]
    async fn a_truncated_response_still_spills_its_complete_bytes() {
        let scratch = Scratch::new("spill-truncated");
        let client = client();

        let payload = "A".repeat(4096);
        let server = serve(vec![format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/octet-stream\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n{payload}",
            payload.len()
        )]);

        // 内联上限压到 64 字节。导出类的体积往往正好卡在 8MB 那条线上，
        // 拿被截断的那一段存出来就是个坏文件
        let out = super::execute_inner(
            &client,
            &spec("GET", server.url("/big")),
            64,
            None,
            Some(&scratch.0),
        )
        .await
        .expect("应当成功");

        assert!(out.truncated, "前提：这个响应应当被截断");
        assert_eq!(out.body.len(), 64);

        let file = out.body_file.expect("octet-stream 应当落盘");
        assert_eq!(file.size, 4096);
        assert_eq!(
            std::fs::read(&file.path).expect("读回落盘的文件").len(),
            4096,
            "落盘的必须是完整字节，不是被截断的那一段"
        );
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
