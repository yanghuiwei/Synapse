//! 接口请求命令。

use std::sync::Arc;

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::http::{execute_with_cancel, BodyEncoding, HttpResponsePayload, Pipeline, RequestSpec};
use crate::storage::models::ApiRequest;
use crate::storage::{repo, with_conn};
use crate::AppState;

/// 某个工作区下**存活**的请求，平铺返回（含未归档到文件夹的）。
#[tauri::command]
pub async fn list_requests(
    state: State<'_, AppState>,
    workspace_id: i64,
) -> AppResult<Vec<ApiRequest>> {
    with_conn(&state.pool, move |conn| {
        repo::request::list_by_workspace(conn, workspace_id)
    })
    .await
}

/// 发一次请求，把结果原样交给前端。
///
/// `runId` 由前端生成（UUID），它只服务于一件事：让 [`cancel_request`] 找回这次执行。
/// **不**用它做去重 —— 同一份请求连点两次发送就是两次真实请求（调试时经常故意这么干）。
///
/// 顺序上刻意「先登记、后建 client」：`ClientCache::get` 里可能要读系统证书库，
/// 不是瞬时的。先登记，用户在这个窗口里点取消才能取消到。
///
/// ## 为什么模板解析在命令层而不在 `execute` 里
///
/// 解析要查库（环境变量、Token 源、文件夹链上的授权继承），而 `http::execute`
/// 是**纯粹的 HTTP 执行器**：给它一个 spec 就发出去，不碰数据库。把两者混在一起，
/// 执行层就再也没法单测（每个用例都得先造一个库）。
/// 所以编排放在这里：`Pipeline::prepare` 把草稿变成可发送的请求，再交给执行器。
#[tauri::command]
pub async fn send_request(
    state: State<'_, AppState>,
    run_id: String,
    spec: RequestSpec,
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
        .prepare(&spec, previous.as_deref(), Some(guard.token()))
        .await?;

    // ⚠️ `guard` 是 RAII 句柄，必须活到请求结束 —— 提前丢掉会让登记表
    // 以为这次已经跑完了，`cancel_request` 就再也找不到它。
    let mut payload = execute_with_cancel(&client, &prepared.spec, Some(guard.token())).await?;

    // 未解析的占位符随响应回给前端：它们没被替换掉，原文已经原样发出去了，
    // 用户需要知道「你发出去的不是你以为的那串 URL」。
    payload.unresolved_placeholders = prepared.unresolved;

    record_last_response(&state, &payload);
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
fn record_last_response(state: &AppState, payload: &HttpResponsePayload) {
    let parsed = match payload.body_encoding {
        BodyEncoding::Text => serde_json::from_str::<serde_json::Value>(&payload.body).ok(),
        // Lossy 里可能混着替换字符，Binary 的 body 干脆是空的，都不该拿来当 JSON 用
        BodyEncoding::Lossy | BodyEncoding::Binary => None,
    };

    if let Ok(mut slot) = state.last_response.lock() {
        *slot = parsed.map(Arc::new);
    }
}

/// 打断一次进行中的请求。返回值是「**有没有找到**这次执行」。
///
/// `false` 表示它已经结束了（或 id 从来不存在），**不是错误** ——
/// 用户点取消时请求刚好回完，是很常见的事，前端按「本来就没在跑」处理即可。
///
/// 为什么外面还套一层 `AppResult`：Tauri 的**异步命令必须返回 `Result`**
/// （`Future<Output = T>` 要求能被序列化成 IPC 响应，非 Result 类型编不过）。
///
/// 写成 `async` 而不是同步命令：Tauri 的同步命令跑在主线程上，
/// 而这里要抢一把 `Mutex`（虽然只有纳秒级）—— 没必要让 UI 冒这个险。
#[tauri::command]
pub async fn cancel_request(state: State<'_, AppState>, run_id: String) -> AppResult<bool> {
    Ok(state.inflight.cancel(&run_id))
}
