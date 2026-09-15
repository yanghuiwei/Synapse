//! 文件夹相关的校验助手（原 `commands/collection.rs` 的「校验助手」段）。
//!
//! 它必须在这里而不是命令层：`create_request` 这条路径 MCP 与 CLI 也要走，
//! 而「目标文件夹是存活且同工作区的」这条约束三边必须一致 ——
//! 少一次检查就会建出「树里看不见但请求明明还在」的孤儿。

use crate::error::{AppError, AppResult};
use crate::storage::models::Folder;
use crate::storage::repo;

/// 目标文件夹必须是「存活 + 同工作区」的。
///
/// 不做这个检查的话有两个坑：指向回收站里的文件夹会建出一个
/// **看不见的孤儿**（树里查不到，但请求明明还在），指向别的工作区的
/// 文件夹则会让两个工作区的数据串在一起。
pub fn ensure_usable_folder(
    conn: &rusqlite::Connection,
    folder_id: i64,
    workspace_id: i64,
    field: &'static str,
) -> AppResult<Folder> {
    let folder = repo::folder::get(conn, folder_id)?;

    if folder.workspace_id != workspace_id {
        return Err(AppError::InvalidInput {
            field,
            reason: "target folder belongs to a different workspace".into(),
        });
    }
    if folder.deleted_at.is_some() {
        return Err(AppError::InvalidInput {
            field,
            reason: "target folder is in the trash".into(),
        });
    }
    Ok(folder)
}
