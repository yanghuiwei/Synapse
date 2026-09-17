//! 文件对话框与「把响应存成文件」。
//!
//! ## 为什么对话框在 Rust 侧弹，而不是前端用 JS 插件
//!
//! `tauri-plugin-dialog` 两边都提供了入口。走 Rust 侧（`DialogExt`）少三处接线：
//! 不用装 JS 包、不用在 `capabilities/` 里开权限、不用过一遍 IPC 序列化。
//! 而 capability 本来只管**前端**发起的调用 —— 后端自己调插件不受它约束，
//! 所以这条路上没有「权限没配、运行时静默失败」那种坑。
//!
//! ## 为什么 `save_response_body` 要校验来源路径
//!
//! 它的入参是一个后端自己刚吐给前端的临时文件路径。**只让它读那个临时区**，
//! 是为了万一这个入口被别的东西调到时，它不会变成一个「读任意文件 + 写到任意位置」
//! 的组合 —— 那等于把整个磁盘交出去。校验成本是两行 `canonicalize`。

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use crate::error::{AppError, AppResult};
use crate::AppState;

/// 另存的结果。字段名与前端 `types.ts` 的 `SavedFile` 一一对应。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedFile {
    /// 最终落盘的绝对路径
    pub path: String,
    pub bytes: u64,
}

/// 弹「选择文件」。用户取消返回 `None`（**不是错误** —— 取消是正常操作）。
///
/// 用 `blocking_pick_file` + `spawn_blocking`：对话框是**阻塞**的模态调用，
/// 直接放在 async 命令里会占住 tokio 的工作线程；而放在主线程上会把界面冻住。
#[tauri::command]
pub async fn pick_file(app: tauri::AppHandle) -> AppResult<Option<String>> {
    let picked =
        tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_file())
            .await
            .map_err(|e| AppError::Internal(format!("file dialog task failed: {e}")))?;

    Ok(picked
        .and_then(|p| path_of(&p))
        .map(|p| p.display().to_string()))
}

/// 把一份「文件型」响应另存到用户选的位置。
///
/// `source_path` 是 [`crate::http::BodyFile`] 里的临时路径，`file_name` 是它的建议名
/// （对话框的预填值）。返回 `None` = 用户在对话框里取消了。
#[tauri::command]
pub async fn save_response_body(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    source_path: String,
    file_name: String,
) -> AppResult<Option<SavedFile>> {
    let source = PathBuf::from(&source_path);
    let root = &state.blob_dir;

    // 见模块头：这个入口只认临时区里的文件
    let real_source = source.canonicalize().map_err(|e| AppError::InvalidInput {
        field: "sourcePath",
        reason: format!("这份响应内容已经不在临时区里了：{source_path}（{e}）"),
    })?;
    let real_root = root.canonicalize().unwrap_or_else(|_| root.clone());
    if !real_source.starts_with(&real_root) {
        return Err(AppError::InvalidInput {
            field: "sourcePath",
            reason: "只允许保存本次响应生成的临时文件".into(),
        });
    }

    let suggested = file_name.clone();
    let target = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_file_name(&suggested)
            .blocking_save_file()
    })
    .await
    .map_err(|e| AppError::Internal(format!("file dialog task failed: {e}")))?;

    let Some(target) = target.and_then(|p| path_of(&p)) else {
        return Ok(None);
    };

    std::fs::copy(&real_source, &target)?;
    let bytes = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);

    Ok(Some(SavedFile {
        path: target.display().to_string(),
        bytes,
    }))
}

/// 把插件的 `FilePath` 收敛成 `PathBuf`。
///
/// 它有一个 `Url` 变体（拖拽 / 远程来源），而这里只处理本地文件 ——
/// 认不出的返回 `None`，而不是硬拼一个看起来像路径的字符串。
fn path_of(file: &tauri_plugin_dialog::FilePath) -> Option<PathBuf> {
    file.clone().into_path().ok()
}
