//! 工作区命令。
//!
//! 当前版本没有「多工作区切换」的交互，`get_active_workspace` 直接返回
//! [`repo::workspace::ensure_default`] 的结果（没有就建一个），
//! 这样前端启动流程永远是「调一个命令拿到能用的工作区」，
//! 不需要自己处理「首次启动没有工作区」的分支。
//! 将来加切换时，把它改成读 `setting.activeWorkspaceId` 即可。

use tauri::State;

use crate::error::AppResult;
use crate::storage::models::Workspace;
use crate::storage::{repo, with_conn};
use crate::AppState;

#[tauri::command]
pub async fn list_workspaces(state: State<'_, AppState>) -> AppResult<Vec<Workspace>> {
    with_conn(&state.pool, |conn| repo::workspace::list(conn)).await
}

#[tauri::command]
pub async fn get_active_workspace(state: State<'_, AppState>) -> AppResult<Workspace> {
    with_conn(&state.pool, |conn| repo::workspace::ensure_default(conn)).await
}
