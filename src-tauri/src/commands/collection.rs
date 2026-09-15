//! 集合（文件夹 + 请求）命令，以及回收站的批量操作。
//!
//! 命令层只做三件事：**校验入参 → 调仓储 → 映射错误**。
//!
//! 这里承担了仓储层刻意不做的一类校验：**跨实体的引用是否合法**
//! （新建/移动时目标文件夹必须存在、存活、且同工作区）。
//! 放在命令层而不是仓储层，是因为仓储的 `create` / `move_to_folder`
//! 本身是「无状态的一步写操作」，塞进这些检查会让它们无法被复用成
//! 「导入时需要容忍半成品父节点」之类的场景。

use std::collections::HashSet;

use serde::Serialize;
use tauri::State;

use crate::commands::validate::clean_name;
use crate::error::AppResult;
use crate::storage::models::{ApiRequest, AuthConfig, Folder, HistoryEntry};
use crate::storage::{repo, with_conn};
use crate::AppState;

// ============================================================
// 校验助手
// ============================================================

// 实现已搬去 `reqly-core` —— 建请求那条路径 MCP / CLI 也要走同一次校验。
pub(crate) use reqly_core::service::collection::ensure_usable_folder;

// ============================================================
// 文件夹
// ============================================================

#[tauri::command]
pub async fn create_folder(
    state: State<'_, AppState>,
    workspace_id: i64,
    parent_id: Option<i64>,
    name: String,
) -> AppResult<Folder> {
    let name = clean_name(&name, "name")?;
    with_conn(&state.pool, move |conn| {
        if let Some(parent_id) = parent_id {
            ensure_usable_folder(conn, parent_id, workspace_id, "parentId")?;
        }
        repo::folder::create(conn, workspace_id, parent_id, &name)
    })
    .await
}

#[tauri::command]
pub async fn rename_folder(state: State<'_, AppState>, id: i64, name: String) -> AppResult<Folder> {
    let name = clean_name(&name, "name")?;
    with_conn(&state.pool, move |conn| {
        repo::folder::rename(conn, id, &name)
    })
    .await
}

/// 标记 / 取消「Token 源分组」（design.md §4.3）。
#[tauri::command]
pub async fn set_folder_token_source(
    state: State<'_, AppState>,
    id: i64,
    is_token_source: bool,
) -> AppResult<Folder> {
    with_conn(&state.pool, move |conn| {
        repo::folder::set_token_source(conn, id, is_token_source)
    })
    .await
}

/// 设置分组级授权配置（design.md §4.4 的「继承父级」靠它落数据）。
///
/// 不校验内容：`AuthConfig` 是枚举，形状在类型层就已经定死了；
/// 里面那些字符串字段允许为空（比如 Bearer 的 token 留空等着写 `{{token.access}}`）。
#[tauri::command]
pub async fn set_folder_auth(
    state: State<'_, AppState>,
    id: i64,
    auth: AuthConfig,
) -> AppResult<Folder> {
    with_conn(&state.pool, move |conn| {
        repo::folder::set_auth(conn, id, &auth)
    })
    .await
}

/// 移动到新父节点。`parent_id = None` 表示移到根。
///
/// 环检查（不能移进自己的子孙）在仓储里 —— 那也是唯一能拿到
/// 递归集合的地方。
#[tauri::command]
pub async fn move_folder(
    state: State<'_, AppState>,
    id: i64,
    parent_id: Option<i64>,
) -> AppResult<Folder> {
    with_conn(&state.pool, move |conn| {
        if let Some(parent_id) = parent_id {
            let folder = repo::folder::get(conn, id)?;
            ensure_usable_folder(conn, parent_id, folder.workspace_id, "parentId")?;
        }
        repo::folder::move_to(conn, id, parent_id)
    })
    .await
}

/// 级联软删除的结果。
///
/// 返回两个数字而不是一个 `bool`：界面上要提示「已移到回收站（3 个分类，
/// 12 个接口）」，而这个数量在前端是算不出来的（它只有平铺列表，
/// 不知道后端递归到哪一层）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFolderResult {
    pub folders: usize,
    pub requests: usize,
}

/// 彻底删除一棵子树的计数（`purge_folder` / `purge_trash` 共用）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeTreeResult {
    pub folders: usize,
    pub requests: usize,
}

/// 删除文件夹：整棵子树 + 子树下的请求一起进回收站。
#[tauri::command]
pub async fn delete_folder(state: State<'_, AppState>, id: i64) -> AppResult<DeleteFolderResult> {
    with_conn(&state.pool, move |conn| {
        let (folders, requests) = repo::folder::soft_delete_tree(conn, id)?;
        Ok(DeleteFolderResult { folders, requests })
    })
    .await
}

#[tauri::command]
pub async fn restore_folder(state: State<'_, AppState>, id: i64) -> AppResult<Folder> {
    with_conn(&state.pool, move |conn| {
        repo::folder::restore_tree(conn, id)
    })
    .await
}

/// 彻底删除文件夹（含子树与请求）。回收站里用。
#[tauri::command]
pub async fn purge_folder(state: State<'_, AppState>, id: i64) -> AppResult<PurgeTreeResult> {
    with_conn(&state.pool, move |conn| {
        let (folders, requests) = repo::folder::purge_tree(conn, id)?;
        Ok(PurgeTreeResult { folders, requests })
    })
    .await
}

// ============================================================
// 请求
// ============================================================

#[tauri::command]
pub async fn create_request(
    state: State<'_, AppState>,
    workspace_id: i64,
    folder_id: Option<i64>,
    name: String,
) -> AppResult<ApiRequest> {
    let name = clean_name(&name, "name")?;
    with_conn(&state.pool, move |conn| {
        if let Some(folder_id) = folder_id {
            ensure_usable_folder(conn, folder_id, workspace_id, "folderId")?;
        }
        repo::request::create(conn, workspace_id, folder_id, &name)
    })
    .await
}

/// 全量保存请求定义（编辑器每次改动后落库）。
///
/// 走整对象而不是逐字段 patch：前端本来就持有完整对象，
/// 逐字段接口只会多出一堆「哪些字段能改」的规则要维护。
#[tauri::command]
pub async fn save_request(
    state: State<'_, AppState>,
    request: ApiRequest,
) -> AppResult<ApiRequest> {
    let name = clean_name(&request.name, "name")?;
    with_conn(&state.pool, move |conn| {
        if let Some(folder_id) = request.folder_id {
            ensure_usable_folder(conn, folder_id, request.workspace_id, "folderId")?;
        }
        let mut request = request;
        request.name = name;
        repo::request::save(conn, &request)
    })
    .await
}

#[tauri::command]
pub async fn rename_request(
    state: State<'_, AppState>,
    id: i64,
    name: String,
) -> AppResult<ApiRequest> {
    let name = clean_name(&name, "name")?;
    with_conn(&state.pool, move |conn| {
        repo::request::rename(conn, id, &name)
    })
    .await
}

/// 复制成新请求。`name` 由前端拼（后端不生成「副本」这类文案）。
#[tauri::command]
pub async fn duplicate_request(
    state: State<'_, AppState>,
    id: i64,
    name: String,
) -> AppResult<ApiRequest> {
    let name = clean_name(&name, "name")?;
    with_conn(&state.pool, move |conn| {
        repo::request::duplicate(conn, id, &name)
    })
    .await
}

#[tauri::command]
pub async fn move_request(
    state: State<'_, AppState>,
    id: i64,
    folder_id: Option<i64>,
) -> AppResult<ApiRequest> {
    with_conn(&state.pool, move |conn| {
        if let Some(folder_id) = folder_id {
            let request = repo::request::get(conn, id)?;
            ensure_usable_folder(conn, folder_id, request.workspace_id, "folderId")?;
        }
        repo::request::move_to_folder(conn, id, folder_id)
    })
    .await
}

/// 软删除请求。返回被删的那一行 —— 界面要拿它显示「已移到回收站」并可撤销。
#[tauri::command]
pub async fn delete_request(state: State<'_, AppState>, id: i64) -> AppResult<ApiRequest> {
    with_conn(&state.pool, move |conn| {
        let request = repo::request::get(conn, id)?;
        repo::request::soft_delete(conn, id)?;
        Ok(request)
    })
    .await
}

#[tauri::command]
pub async fn restore_request(state: State<'_, AppState>, id: i64) -> AppResult<ApiRequest> {
    with_conn(&state.pool, move |conn| repo::request::restore(conn, id)).await
}

#[tauri::command]
pub async fn purge_request(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    with_conn(&state.pool, move |conn| repo::request::purge(conn, id)).await
}

// ============================================================
// 回收站
// ============================================================

/// 回收站里的全部内容。
///
/// 合成一个命令而不是让前端发三次：回收站要一次画完三块列表，
/// 分开拿会出现「分类已经出来了，历史还在转」这种半截界面。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashContents {
    pub folders: Vec<Folder>,
    pub requests: Vec<ApiRequest>,
    pub history: Vec<HistoryEntry>,
}

#[tauri::command]
pub async fn list_trash(state: State<'_, AppState>, workspace_id: i64) -> AppResult<TrashContents> {
    with_conn(&state.pool, move |conn| {
        Ok(TrashContents {
            folders: repo::folder::list_deleted(conn, workspace_id)?,
            requests: repo::request::list_deleted(conn, workspace_id)?,
            history: repo::history::list_deleted(conn, workspace_id)?,
        })
    })
    .await
}

/// 「清空回收站」的结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeTrashResult {
    pub folders: usize,
    pub requests: usize,
    pub history: usize,
}

/// 清空回收站的核心逻辑。
///
/// 抽成收 `&Connection` 的自由函数，**只为了能被单测覆盖** ——
/// 这段顺序（先子树、再孤儿请求、最后历史）和计数是 P3 里最容易写错的地方，
/// 而 `#[tauri::command]` 的入参是 `State`，在单测里拿不到。
///
/// 顺序有讲究：
/// 1. 先删文件夹子树 —— `request.folder_id` 是 `ON DELETE SET NULL`，
///    指望级联把请求带走是错的，`purge_tree` 内部会显式删掉子树里的请求
/// 2. 再删**剩下的**软删除请求（它们的文件夹是活的，或者本来就未归档）
/// 3. 最后删软删除的历史
///
/// 第 2 步天然不会重复计数：`list_deleted` 是在第 1 步**之后**查的，
/// 那时子树里的请求已经不在表里了。
fn purge_trash_with_conn(
    conn: &rusqlite::Connection,
    workspace_id: i64,
) -> AppResult<PurgeTrashResult> {
    let deleted_folders = repo::folder::list_deleted(conn, workspace_id)?;
    let deleted_ids: HashSet<i64> = deleted_folders.iter().map(|f| f.id).collect();

    let mut folders = 0usize;
    let mut requests = 0usize;

    for folder in &deleted_folders {
        // 只从「回收站里的根」开始删：子孙会随 `purge_tree` 一起走，
        // 再对它们调一次只会拿到 NotFound。
        if folder.parent_id.is_some_and(|p| deleted_ids.contains(&p)) {
            continue;
        }
        let (f, r) = repo::folder::purge_tree(conn, folder.id)?;
        folders += f;
        requests += r;
    }

    // 剩下的软删除请求：不在回收站子树里的那些
    for request in repo::request::list_deleted(conn, workspace_id)? {
        repo::request::purge(conn, request.id)?;
        requests += 1;
    }

    let history = repo::history::purge_trash(conn, Some(workspace_id))?;

    Ok(PurgeTrashResult {
        folders,
        requests,
        history,
    })
}

/// 清空回收站（**硬删除**，不可撤销）。
///
/// 计数与顺序交给 [`purge_trash_with_conn`]，这里只做「异步边界 + 状态取出」。
#[tauri::command]
pub async fn purge_trash(
    state: State<'_, AppState>,
    workspace_id: i64,
) -> AppResult<PurgeTrashResult> {
    with_conn(&state.pool, move |conn| {
        purge_trash_with_conn(conn, workspace_id)
    })
    .await
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    // AppError 原先是由顶层的 use 带进来的，那行随 ensure_usable_folder 搬去
    // core 之后收窄成了 AppResult，所以这里补一条显式的。
    use crate::commands::validate::MAX_NAME_LEN;
    use crate::error::AppError;
    use crate::storage::migrate;
    use rusqlite::Connection;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().expect("db");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate::run(&mut conn).expect("migrate");
        // 两个工作区：用来验证「跨工作区的数据不许被误伤」
        for (id, name) in [(1_i64, "w1"), (2_i64, "w2")] {
            conn.execute(
                "INSERT INTO workspace (id, name, created_at, updated_at)
                 VALUES (?1, ?2, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                rusqlite::params![id, name],
            )
            .unwrap();
        }
        conn
    }

    fn history(conn: &Connection, workspace_id: i64, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO history (workspace_id, request_name, snapshot, executed_at)
             VALUES (?1, ?2, '{}', '2026-01-01T00:00:00.000Z')",
            rusqlite::params![workspace_id, name],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn clean_name_trims_and_rejects_blank() {
        assert_eq!(clean_name("  login  ", "name").unwrap(), "login");

        for bad in ["", "   ", "\t\n"] {
            assert!(
                matches!(
                    clean_name(bad, "name").unwrap_err(),
                    AppError::InvalidInput { field: "name", .. }
                ),
                "{bad:?} 应该被拒"
            );
        }

        let too_long = "汉".repeat(MAX_NAME_LEN + 1);
        assert!(matches!(
            clean_name(&too_long, "name").unwrap_err(),
            AppError::InvalidInput { .. }
        ));
        // 边界值本身要放行
        assert!(clean_name(&"汉".repeat(MAX_NAME_LEN), "name").is_ok());
    }

    /// 指向回收站里的文件夹会建出看不见的孤儿；指向别的工作区会让两份数据串起来
    #[test]
    fn ensure_usable_folder_blocks_trashed_and_foreign_workspaces() {
        let conn = db();
        let alive = repo::folder::create(&conn, 1, None, "alive").unwrap();
        let trashed = repo::folder::create(&conn, 1, None, "trashed").unwrap();
        repo::folder::soft_delete_tree(&conn, trashed.id).unwrap();
        let foreign = repo::folder::create(&conn, 2, None, "other-ws").unwrap();

        assert!(ensure_usable_folder(&conn, alive.id, 1, "parentId").is_ok());

        assert!(matches!(
            ensure_usable_folder(&conn, trashed.id, 1, "parentId").unwrap_err(),
            AppError::InvalidInput {
                field: "parentId",
                ..
            }
        ));

        assert!(matches!(
            ensure_usable_folder(&conn, foreign.id, 1, "parentId").unwrap_err(),
            AppError::InvalidInput {
                field: "parentId",
                ..
            }
        ));

        assert!(matches!(
            ensure_usable_folder(&conn, 999, 1, "parentId").unwrap_err(),
            AppError::NotFound {
                entity: "folder",
                ..
            }
        ));
    }

    /// 清空回收站：三层子树 + 独立软删的请求 + 历史，都要清掉；
    /// 存活的行与**别的工作区**的行一根汗毛都不能动。
    #[test]
    fn purge_trash_removes_nested_trees_but_spares_alive_and_other_workspaces() {
        let conn = db();

        // 工作区 1：三层树，子层各挂一个请求
        let root = repo::folder::create(&conn, 1, None, "root").unwrap();
        let child = repo::folder::create(&conn, 1, Some(root.id), "child").unwrap();
        let leaf = repo::folder::create(&conn, 1, Some(child.id), "leaf").unwrap();
        repo::request::create(&conn, 1, Some(child.id), "r-in-child").unwrap();
        repo::request::create(&conn, 1, Some(leaf.id), "r-in-leaf").unwrap();

        // 工作区 1：保留区 —— 存活的文件夹与请求
        let keep = repo::folder::create(&conn, 1, None, "keep").unwrap();
        let keep_req = repo::request::create(&conn, 1, Some(keep.id), "r-keep").unwrap();

        // 工作区 1：单独被删的请求（父文件夹是活的）
        let orphan = repo::request::create(&conn, 1, Some(keep.id), "r-orphan").unwrap();
        repo::request::soft_delete(&conn, orphan.id).unwrap();

        // 工作区 1：一条存活历史 + 一条已删历史
        let h_alive = history(&conn, 1, "h-alive");
        let h_trashed = history(&conn, 1, "h-trashed");

        // 工作区 2：同样有回收站内容，必须不受影响
        let other = repo::folder::create(&conn, 2, None, "other-trashed").unwrap();
        repo::folder::soft_delete_tree(&conn, other.id).unwrap();
        let h_other = history(&conn, 2, "h-other");

        // 让工作区 1 的整棵树进回收站
        repo::folder::soft_delete_tree(&conn, root.id).unwrap();
        repo::history::soft_delete(&conn, h_trashed).unwrap();

        let result = purge_trash_with_conn(&conn, 1).unwrap();
        assert_eq!(result.folders, 3, "三层树必须报 3 个文件夹");
        assert_eq!(result.requests, 3, "子树里的 2 个 + 单独删的 1 个");
        assert_eq!(result.history, 1);

        // 工作区 1 的回收站清空了
        assert!(repo::folder::list_deleted(&conn, 1).unwrap().is_empty());
        assert!(repo::request::list_deleted(&conn, 1).unwrap().is_empty());

        // 存活的行还在
        assert!(repo::folder::get(&conn, keep.id).is_ok());
        assert!(repo::request::get(&conn, keep_req.id).is_ok());
        assert!(repo::history::get(&conn, h_alive).is_ok());

        // 被清掉的行确实没了
        assert!(repo::folder::get(&conn, root.id).is_err());
        assert!(repo::request::get(&conn, orphan.id).is_err());
        assert!(repo::history::get(&conn, h_trashed).is_err());

        // 工作区 2 完全没被动
        assert_eq!(repo::folder::list_deleted(&conn, 2).unwrap().len(), 1);
        assert!(repo::history::get(&conn, h_other).is_ok());
    }

    #[test]
    fn purge_trash_on_empty_trash_is_a_noop() {
        let conn = db();
        let result = purge_trash_with_conn(&conn, 1).unwrap();
        assert_eq!((result.folders, result.requests, result.history), (0, 0, 0));
    }
}
