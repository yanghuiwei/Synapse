//! 发一次请求的编排：模板解析 → 执行 → 记下「最近一次响应」。
//!
//! ## 为什么模板解析在编排层而不在 `http::execute` 里
//!
//! 解析要查库（环境变量、Token 源、文件夹链上的授权继承），而 `http::execute`
//! 是**纯粹的 HTTP 执行器**：给它一个 spec 就发出去，不碰数据库。把两者混在一起，
//! 执行层就再也没法单测（每个用例都得先造一个库）。
//! 所以编排放在这里：`Pipeline::prepare` 把草稿变成可发送的请求，再交给执行器。

use std::sync::Arc;

use crate::error::{AppError, AppResult};
use crate::http::{execute_with_blobs, BodyEncoding, HttpResponsePayload, Pipeline, RequestSpec};
use crate::AppState;

/// 发一次请求，把结果原样交给调用方。
///
/// `run_id` 由调用方生成（桌面端是前端给的 UUID），它只服务于一件事：
/// 让「取消」能找回这次执行。**不**用它做去重 —— 同一份请求连点两次发送
/// 就是两次真实请求（调试时经常故意这么干）。
///
/// 顺序上刻意「先登记、后建 client」：`ClientCache::get` 里可能要读系统证书库，
/// 不是瞬时的。先登记，用户在这个窗口里点取消才能取消到。
///
/// 收 `&AppState` 而不是 `State<'_, AppState>` —— 后者没有公开构造函数，
/// 非 Tauri 进程（`reqly-mcp` / `reqly-cli`）造不出来。桌面端、MCP、CLI
/// 三条路径共用这一份编排，而不是各写一遍：一旦分叉，症状是
/// 「桌面端点得对、AI 那边悄悄不对」，那种 bug 不会有人去查。
///
/// `emit_curl` 决定要不要在结果里附上「本次真实发出的请求」对应的 curl。
/// **只有桌面端传 `true`**：那份东西里带着展开后的 token，而 MCP / CLI 的输出
/// 会进 AI 的上下文 —— 凭据不该从那条路出去（判定名单见 `crate::redact`）。
pub async fn send_request_with(
    state: &AppState,
    run_id: &str,
    spec: &RequestSpec,
    emit_curl: bool,
) -> AppResult<HttpResponsePayload> {
    if run_id.trim().is_empty() {
        return Err(AppError::InvalidInput {
            field: "runId",
            reason: "不能为空：取消功能要靠它找回这次执行".into(),
        });
    }

    let guard = state.inflight.register(run_id);
    let client = state.clients.get(&spec.settings)?;

    // `{{resp.$.path}}` 的来源。取出 `Arc` 后**立刻**把锁放掉 ——
    // `std::sync::MutexGuard` 不是 `Send`，抱着它穿过下面的 await 点根本编不过。
    let previous = state
        .last_response
        .lock()
        .ok()
        .and_then(|slot| slot.clone());

    let pipeline = Pipeline::new(&state.pool, &state.clients, &state.tokens);
    let prepared = pipeline
        .prepare(spec, previous.as_deref(), Some(guard.token()))
        .await?;

    // ⚠️ `guard` 是 RAII 句柄，必须活到请求结束 —— 提前丢掉会让登记表
    // 以为这次已经跑完了，「取消」就再也找不到它。
    //
    // 走 `execute_with_blobs` 而不是 `execute_with_cancel`：导出类接口
    // （Excel / PDF / 图片流）的字节只在这一次执行里存在，先落到会话临时区，
    // 用户之后点「保存到文件」才拿得到。其余入口（MCP / CLI / 压测）
    // 仍然走不带目录的那条，行为一字不变。
    let mut payload = execute_with_blobs(
        &client,
        &prepared.spec,
        Some(guard.token()),
        &state.blob_dir,
    )
    .await?;

    // 未解析的占位符随响应回给调用方：它们没被替换掉，原文已经原样发出去了，
    // 用户需要知道「你发出去的不是你以为的那串 URL」。
    payload.unresolved_placeholders = prepared.unresolved;

    // Token 使用报告同理：它是**这一次执行**的事实（刷了没、走的哪个接口），
    // 只有编排层知道（执行期才有答案，事后从库里读不出来）。
    payload.token_usage = prepared.token;

    // curl 取的是 `prepared.spec` 而不是调用方传来的 `spec`：前者占位符已展开、
    // token 已注入、授权继承已解开。拿原始草稿拼出来的会是 `{{token.access}}`
    // 这种原文 —— 而用户打开这个面板要核对的恰恰是「那一刻真实发出去的是什么」。
    //
    // 两份都给：打码那份用于截图/分享，明文那份是核对 401 时真正要看的。
    // 桌面端是本机用户在看自己的凭据，与 `token_cache` 一直明文存着同一个口径。
    if emit_curl {
        payload.curl = Some(crate::http::to_curl(&prepared.spec, true));
        payload.curl_plain = Some(crate::http::to_curl(&prepared.spec, false));
    }

    record_last_response(state, &payload);
    Ok(payload)
}

/// 把这次响应记成「最近一次成功的响应」。
///
/// 只有**能解析成 JSON** 的文本响应才记录；其余情况一律**清空**，
/// 包括二进制、空体、和不是 JSON 的文本。
///
/// 「清空」而不是「保留上一次」是刻意的：`{{resp.$.xxx}}` 引用的是「你刚看到的那次响应」。
/// 若在响应不可解析时留着旧值，就会出现「显示的是这一次的响应，但模板取的是上一次的字段」——
/// 这种错没法从界面上看出来，是最贵的一类 bug。
///
/// 桌面端与 MCP/CLI 侧必须是同一套判定，否则 AI 发的请求和桌面端发的请求
/// 会让 `{{resp.$.xxx}}` 指向不同的东西。
pub fn record_last_response(state: &AppState, payload: &HttpResponsePayload) {
    let parsed = match payload.body_encoding {
        BodyEncoding::Text => serde_json::from_str::<serde_json::Value>(&payload.body).ok(),
        // Lossy 里可能混着替换字符，Binary 的 body 干脆是空的，都不该拿来当 JSON 用
        BodyEncoding::Lossy | BodyEncoding::Binary => None,
    };

    if let Ok(mut slot) = state.last_response.lock() {
        *slot = parsed.map(Arc::new);
    }
}
