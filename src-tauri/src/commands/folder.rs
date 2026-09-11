//! 文件夹（分类）命令。

use tauri::State;

use crate::error::AppResult;
use crate::storage::models::Folder;
use crate::storage::{repo, with_conn};
use crate::AppState;

/// 某个工作区下**存活**的文件夹，整棵树平铺返回。
///
/// 刻意不在后端组树：树形结构在 UI 里还要按展开状态、筛选关键字、
/// 拖拽顺序重新算，后端提前组好反而要再拍平。前端用 `parentId` 自己组。
#[tauri::command]
pub async fn list_folders(state: State<'_, AppState>, workspace_id: i64) -> AppResult<Vec<Folder>> {
    with_conn(&state.pool, move |conn| {
        repo::folder::list_by_workspace(conn, workspace_id)
    })
    .await
}
