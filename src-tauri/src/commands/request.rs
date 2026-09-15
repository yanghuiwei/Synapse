//! 接口请求命令。

use tauri::State;

use crate::error::AppResult;
use crate::http::{HttpResponsePayload, RequestSpec};
use crate::storage::models::ApiRequest;
use crate::storage::{repo, with_conn};
use crate::AppState;

// 编排本体已搬去 `reqly-core`（桌面 / MCP / CLI 共用一份）。这里转发路径，
// 让本文件的调用点与测试一行都不用改。
pub use reqly_core::service::request::{record_last_response, send_request_with};

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
    send_request_with(&state, &run_id, &spec).await
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
