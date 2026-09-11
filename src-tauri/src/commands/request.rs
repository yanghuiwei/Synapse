//! 接口请求命令。

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::http::{execute_with_cancel, HttpResponsePayload, RequestSpec};
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

    // ⚠️ `guard` 是 RAII 句柄，必须活到请求结束 —— 提前丢掉会让登记表
    // 以为这次已经跑完了，`cancel_request` 就再也找不到它。
    execute_with_cancel(&client, &spec, Some(guard.token())).await
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
