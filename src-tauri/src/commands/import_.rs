//! 导入命令（Tauri 包装层）。
//!
//! 解析、计划、落库的全部实现在 `reqly-core` —— 这里只是把 `State` 拆开、
//! 转交给那边。**这一点是刻意的**：MCP 的 `import_curl` 与 CLI 的
//! `reqly import curl` 走的是同一份 `build_plan` / `apply_plan`，
//! 所以「AI 粘一段 curl」与「用户在界面里点导入」的冲突判定、名字去重、
//! 回收站行为不可能分叉。

use tauri::State;

use crate::error::AppResult;
use crate::storage::with_conn;
use crate::AppState;

pub use reqly_core::service::import::{
    apply_plan, build_plan, import_into_folder, to_preview, Conflict, ImportLabels, ImportOutcome,
    ImportPreview, ImportSource, Plan,
};

/// 只看不写：解析 + 比对，回一棵差异树。
///
/// **没有 `include_environments` 参数**：预览里永远列出文件里带的环境，
/// 用户才知道「原来还有一批变量可以一起导」。真写不写由提交那一步的开关决定，
/// 而不是让预览瞒着不说。
#[tauri::command]
pub async fn preview_import(
    state: State<'_, AppState>,
    source: ImportSource,
    conflict: Conflict,
) -> AppResult<ImportPreview> {
    with_conn(&state.pool, move |conn| {
        let plan = build_plan(conn, &source, conflict, true)?;
        Ok(to_preview(&plan))
    })
    .await
}

/// 落库。**会重新解析一遍**（预览到落库之间用户可能换了文件），
/// 但走的是同一套 [`build_plan`]，所以名字与冲突判定和预览完全一致。
#[tauri::command]
pub async fn commit_import(
    state: State<'_, AppState>,
    source: ImportSource,
    conflict: Conflict,
    include_environments: bool,
) -> AppResult<ImportOutcome> {
    with_conn(&state.pool, move |conn| {
        let plan = build_plan(conn, &source, conflict, include_environments)?;
        apply_plan(conn, &plan)
    })
    .await
}
