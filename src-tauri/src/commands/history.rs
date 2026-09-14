//! 历史记录命令。
//!
//! 历史有两个入口：**发送后写入**（`record_history`）和**用户主动删除**
//! （`delete_history` / `clear_history`）。前者在热路径上，只有一次 INSERT，
//! 所以刻意做得极简 —— 任何多余的校验都会拖慢每次发送。
//!
//! 「清空历史」走软删除（进回收站），只有回收站里的「彻底删除」才硬删，
//! 与 design.md §3.3 的语义一致。

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::storage::models::{HistoryEntry, NewHistoryEntry};
use crate::storage::{repo, with_conn};
use crate::AppState;

/// 单页上限。前端滚到底会自动续页，一次给太多只会让 IPC 序列化变慢。
const MAX_PAGE_SIZE: i64 = 200;
const DEFAULT_PAGE_SIZE: i64 = 50;

/// 一页历史 + 总数。
///
/// 带上 `total` 是为了让前端能显示「3 / 128」这类位置感，
/// 也方便判断「还有没有下一页」——只靠 `items.len() < limit`
/// 在恰好整除时会多请求一次空页。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub items: Vec<HistoryEntry>,
    pub total: i64,
}

/// 列表。`keyword` 为空或全空白时等同于不过滤。
///
/// `limit` / `offset` 一律**在这里夹紧**而不是相信前端：分页参数写错
/// （比如 limit = 0 或负数）在 SQLite 里不会报错，只会静默返回空列表，
/// 那种「界面莫名其妙没数据」的问题最难查。
#[tauri::command]
pub async fn list_history(
    state: State<'_, AppState>,
    workspace_id: i64,
    keyword: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> AppResult<HistoryPage> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);
    let offset = offset.unwrap_or(0).max(0);
    let keyword = keyword
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty());

    with_conn(&state.pool, move |conn| {
        let items = match &keyword {
            Some(k) => repo::history::search(conn, workspace_id, k, limit, offset)?,
            None => repo::history::list(conn, workspace_id, limit, offset)?,
        };
        // 计数必须用与 `items` 同一套过滤条件，否则「还有下一页」会一直为真
        let total = match &keyword {
            Some(k) => repo::history::count_matching(conn, workspace_id, k)?,
            None => repo::history::count(conn, workspace_id)?,
        };
        Ok(HistoryPage { items, total })
    })
    .await
}

#[tauri::command]
pub async fn get_history(state: State<'_, AppState>, id: i64) -> AppResult<HistoryEntry> {
    with_conn(&state.pool, move |conn| repo::history::get(conn, id)).await
}

/// 写一条历史（发送结束后调用）。
///
/// **成功与失败都要写**：`status = None` + `error` 有值代表网络层失败，
/// 这恰恰是用户最想回看的那几条 —— 只记成功的历史等于把排查线索删掉了。
/// 用户主动取消不写（那是「没发生过」）。
#[tauri::command]
pub async fn record_history(
    state: State<'_, AppState>,
    entry: NewHistoryEntry,
) -> AppResult<HistoryEntry> {
    if entry.request_name.trim().is_empty() {
        return Err(AppError::InvalidInput {
            field: "requestName",
            reason: "request name cannot be empty".into(),
        });
    }
    if entry.workspace_id <= 0 {
        return Err(AppError::InvalidInput {
            field: "workspaceId",
            reason: "workspace id must be positive".into(),
        });
    }

    with_conn(&state.pool, move |conn| repo::history::insert(conn, &entry)).await
}

/// 移除一条历史（进回收站）。
#[tauri::command]
pub async fn delete_history(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    with_conn(&state.pool, move |conn| {
        repo::history::soft_delete(conn, id)
    })
    .await
}

#[tauri::command]
pub async fn restore_history(state: State<'_, AppState>, id: i64) -> AppResult<HistoryEntry> {
    with_conn(&state.pool, move |conn| repo::history::restore(conn, id)).await
}

/// 彻底删除一条历史（**不可撤销**）。
#[tauri::command]
pub async fn purge_history(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    with_conn(&state.pool, move |conn| repo::history::purge(conn, id)).await
}

/// 「清空历史」：全部存活历史进回收站。返回条数。
#[tauri::command]
pub async fn clear_history(state: State<'_, AppState>, workspace_id: i64) -> AppResult<usize> {
    with_conn(&state.pool, move |conn| {
        repo::history::trash_all(conn, workspace_id)
    })
    .await
}
