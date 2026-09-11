//! 文件夹（分类）仓储。
//!
//! 自引用树。两个关键约束：
//! * **不能把节点移动到自己的子孙下** —— 会形成环，之后递归查询会打转。
//!   入口在 [`move_to`] 里挡住，不指望调用方自觉。
//! * 递归查询一律用 `UNION`（不是 `UNION ALL`）—— 万一库里已经有环，
//!   `UNION` 去重能让查询停下来，而 `UNION ALL` 会直接无限循环。

use rusqlite::{Connection, Row};

use crate::error::{AppError, AppResult};
use crate::storage::models::Folder;
use crate::storage::now_iso;
use crate::storage::repo::{bool_to_int, opt_text, ORDER_BY_SORT};

const COLUMNS: &str =
    "id, workspace_id, parent_id, name, sort_order, is_token_source, created_at, updated_at, deleted_at";

fn map_row(row: &Row<'_>) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: row.get("id")?,
        workspace_id: row.get("workspace_id")?,
        parent_id: row.get("parent_id")?,
        name: row.get("name")?,
        sort_order: row.get("sort_order")?,
        is_token_source: row.get("is_token_source")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        deleted_at: opt_text(row, "deleted_at")?,
    })
}

/// 某节点及其全部子孙的 id。包含自身。
///
/// `deleted_at` 不过滤：软删除的子孙也该一起被恢复 / 移动。
fn subtree_cte() -> &'static str {
    "WITH RECURSIVE subtree(id) AS (
       SELECT id FROM folder WHERE id = ?1
       UNION
       SELECT f.id FROM folder f JOIN subtree s ON f.parent_id = s.id
     )"
}

pub fn subtree_ids(conn: &Connection, id: i64) -> AppResult<Vec<i64>> {
    let sql = format!("{} SELECT id FROM subtree", subtree_cte());
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([id], |r| r.get::<_, i64>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 存活（未软删除）的文件夹，整棵树平铺返回 —— 前端按 parentId 自己组树。
pub fn list_by_workspace(conn: &Connection, workspace_id: i64) -> AppResult<Vec<Folder>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM folder WHERE workspace_id = ?1 AND deleted_at IS NULL{ORDER_BY_SORT}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([workspace_id], map_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 回收站里的文件夹
pub fn list_deleted(conn: &Connection, workspace_id: i64) -> AppResult<Vec<Folder>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM folder
         WHERE workspace_id = ?1 AND deleted_at IS NOT NULL
         ORDER BY deleted_at DESC, id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([workspace_id], map_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 按 id 取。软删除的也能取到（回收站要用）。
pub fn get(conn: &Connection, id: i64) -> AppResult<Folder> {
    let sql = format!("SELECT {COLUMNS} FROM folder WHERE id = ?1");
    conn.query_row(&sql, [id], map_row).map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::NotFound {
            entity: "folder",
            id,
        },
        other => AppError::Db(other),
    })
}

pub fn create(
    conn: &Connection,
    workspace_id: i64,
    parent_id: Option<i64>,
    name: &str,
) -> AppResult<Folder> {
    let now = now_iso();
    let next_order: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM folder
         WHERE workspace_id = ?1 AND parent_id IS ?2 AND deleted_at IS NULL",
        rusqlite::params![workspace_id, parent_id],
        |r| r.get(0),
    )?;

    conn.execute(
        "INSERT INTO folder (workspace_id, parent_id, name, sort_order, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        rusqlite::params![workspace_id, parent_id, name, next_order, now],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn rename(conn: &Connection, id: i64, name: &str) -> AppResult<Folder> {
    let affected = conn.execute(
        "UPDATE folder SET name = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![id, name, now_iso()],
    )?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "folder",
            id,
        });
    }
    get(conn, id)
}

/// 标记 / 取消「Token 源分组」（design.md §4.3）
pub fn set_token_source(conn: &Connection, id: i64, is_token_source: bool) -> AppResult<Folder> {
    let affected = conn.execute(
        "UPDATE folder SET is_token_source = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![id, bool_to_int(is_token_source), now_iso()],
    )?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "folder",
            id,
        });
    }
    get(conn, id)
}

/// 移动到新的父节点。`new_parent_id = None` 表示移到根。
///
/// 会拒绝三类非法请求：父节点不存在、父节点是自己、父节点是自己的子孙。
pub fn move_to(conn: &Connection, id: i64, new_parent_id: Option<i64>) -> AppResult<Folder> {
    let folder = get(conn, id)?;

    if let Some(parent_id) = new_parent_id {
        if parent_id == id {
            return Err(AppError::InvalidInput {
                field: "parentId",
                reason: "a folder cannot be its own parent".into(),
            });
        }

        let parent = get(conn, parent_id)?;
        if parent.workspace_id != folder.workspace_id {
            return Err(AppError::InvalidInput {
                field: "parentId",
                reason: "target folder belongs to a different workspace".into(),
            });
        }
        if parent.deleted_at.is_some() {
            return Err(AppError::InvalidInput {
                field: "parentId",
                reason: "target folder is in the trash".into(),
            });
        }

        // 环检查：目标不能落在自己的子树里
        if subtree_ids(conn, id)?.contains(&parent_id) {
            return Err(AppError::InvalidInput {
                field: "parentId",
                reason: "cannot move a folder into its own descendant".into(),
            });
        }
    }

    // 落到目标父节点下的末尾
    let next_order: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM folder
         WHERE workspace_id = ?1 AND parent_id IS ?2 AND deleted_at IS NULL",
        rusqlite::params![folder.workspace_id, new_parent_id],
        |r| r.get(0),
    )?;

    conn.execute(
        "UPDATE folder SET parent_id = ?2, sort_order = ?3, updated_at = ?4 WHERE id = ?1",
        rusqlite::params![id, new_parent_id, next_order, now_iso()],
    )?;
    get(conn, id)
}

/// 级联软删除：整棵子树 + 子树下的所有请求。
///
/// 返回 (受影响文件夹数, 受影响请求数)。
pub fn soft_delete_tree(conn: &Connection, id: i64) -> AppResult<(usize, usize)> {
    let now = now_iso();

    // 先删请求：递归集合要基于「文件夹还没被标删」的状态来算，
    // 顺序反过来的话集合虽然一样，但读代码的人要多想一步。
    let requests = conn.execute(
        &format!(
            "{} UPDATE request SET deleted_at = ?2, updated_at = ?2
             WHERE folder_id IN (SELECT id FROM subtree) AND deleted_at IS NULL",
            subtree_cte()
        ),
        rusqlite::params![id, now],
    )?;

    let folders = conn.execute(
        &format!(
            "{} UPDATE folder SET deleted_at = ?2, updated_at = ?2
             WHERE id IN (SELECT id FROM subtree) AND deleted_at IS NULL",
            subtree_cte()
        ),
        rusqlite::params![id, now],
    )?;

    if folders == 0 {
        return Err(AppError::NotFound {
            entity: "folder",
            id,
        });
    }
    Ok((folders, requests))
}

/// 从回收站还原整棵子树。
///
/// 若父级仍在回收站，本节点会被提到根（design.md §3.3）。
///
/// ⚠️ 已知取舍：子树下**所有**软删除的请求都会被一起还原，
/// 包括在删文件夹之前就被单独删掉的那些。要精确区分需要在 request 上
/// 再加一个「删除来源」标记，当前不值得为它加一列。
pub fn restore_tree(conn: &Connection, id: i64) -> AppResult<Folder> {
    let now = now_iso();
    let folder = get(conn, id)?;

    if let Some(parent_id) = folder.parent_id {
        let parent_alive: i64 = conn.query_row(
            "SELECT COUNT(*) FROM folder WHERE id = ?1 AND deleted_at IS NULL",
            [parent_id],
            |r| r.get(0),
        )?;
        if parent_alive == 0 {
            // 父级还在回收站 → 本节点落到根，避免还原出一个「挂在坟头上的活节点」
            let next_order: i64 = conn.query_row(
                "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM folder
                 WHERE workspace_id = ?1 AND parent_id IS NULL AND deleted_at IS NULL",
                [folder.workspace_id],
                |r| r.get(0),
            )?;
            conn.execute(
                "UPDATE folder SET parent_id = NULL, sort_order = ?2 WHERE id = ?1",
                rusqlite::params![id, next_order],
            )?;
        }
    }

    conn.execute(
        &format!(
            "{} UPDATE request SET deleted_at = NULL, updated_at = ?2
             WHERE folder_id IN (SELECT id FROM subtree) AND deleted_at IS NOT NULL",
            subtree_cte()
        ),
        rusqlite::params![id, now],
    )?;

    conn.execute(
        &format!(
            "{} UPDATE folder SET deleted_at = NULL, updated_at = ?2
             WHERE id IN (SELECT id FROM subtree)",
            subtree_cte()
        ),
        rusqlite::params![id, now],
    )?;

    get(conn, id)
}

/// 彻底删除整棵子树（含请求）。回收站的「彻底删除」用。
///
/// 返回被删掉的总行数（请求 + 文件夹）。
pub fn purge_tree(conn: &Connection, id: i64) -> AppResult<usize> {
    // request.folder_id 是 ON DELETE SET NULL，所以必须先显式删掉子树的请求，
    // 否则它们会变成「没有文件夹的孤儿」而不是被一并删除。
    let ids = subtree_ids(conn, id)?;
    if ids.is_empty() {
        return Err(AppError::NotFound {
            entity: "folder",
            id,
        });
    }

    let mut deleted = 0usize;
    for folder_id in &ids {
        deleted += conn.execute("DELETE FROM request WHERE folder_id = ?1", [folder_id])?;
    }

    // ⚠️ 文件夹数**不能**靠 `execute()` 的返回值累加。
    // SQLite 的 `changes()` **不计入外键级联删掉的行**：这条 DELETE 按 rowid 顺序先删
    // 根节点，子节点是被 `parent_id ON DELETE CASCADE` 带走的，不会被计数，
    // 于是「2 层树只返回 1」这种少算就出现了（对应的回归测试见
    // `purge_removes_subtree_and_its_requests`）。
    // 删完整棵子树是确定性结果，直接取子树大小即可。
    conn.execute(
        &format!(
            "{} DELETE FROM folder WHERE id IN (SELECT id FROM subtree)",
            subtree_cte()
        ),
        [id],
    )?;
    deleted += ids.len();

    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrate;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().expect("db");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate::run(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO workspace (id, name, created_at, updated_at)
             VALUES (1, 'w', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        )
        .unwrap();
        conn
    }

    fn insert_request(conn: &Connection, folder_id: i64, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO request (folder_id, workspace_id, name, created_at, updated_at)
             VALUES (?1, 1, ?2, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            rusqlite::params![folder_id, name],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn creates_a_three_level_tree() {
        let conn = db();
        let root = create(&conn, 1, None, "root").unwrap();
        let mid = create(&conn, 1, Some(root.id), "mid").unwrap();
        let leaf = create(&conn, 1, Some(mid.id), "leaf").unwrap();

        assert_eq!(root.parent_id, None);
        assert_eq!(mid.parent_id, Some(root.id));
        assert_eq!(subtree_ids(&conn, root.id).unwrap().len(), 3);
        assert_eq!(subtree_ids(&conn, leaf.id).unwrap(), vec![leaf.id]);
    }

    #[test]
    fn sort_order_is_per_parent() {
        let conn = db();
        let a = create(&conn, 1, None, "a").unwrap();
        let b = create(&conn, 1, None, "b").unwrap();
        let child = create(&conn, 1, Some(a.id), "child").unwrap();

        assert_eq!(a.sort_order, 0);
        assert_eq!(b.sort_order, 1);
        assert_eq!(child.sort_order, 0, "child must start its own counter");
    }

    #[test]
    fn rejects_moving_into_own_descendant() {
        let conn = db();
        let root = create(&conn, 1, None, "root").unwrap();
        let mid = create(&conn, 1, Some(root.id), "mid").unwrap();
        let leaf = create(&conn, 1, Some(mid.id), "leaf").unwrap();

        let err = move_to(&conn, root.id, Some(leaf.id)).unwrap_err();
        assert!(matches!(
            err,
            AppError::InvalidInput {
                field: "parentId",
                ..
            }
        ));

        // 也不能把自己当父节点
        assert!(matches!(
            move_to(&conn, root.id, Some(root.id)).unwrap_err(),
            AppError::InvalidInput {
                field: "parentId",
                ..
            }
        ));

        // 移到自己原来的父节点是合法的（幂等移动）
        move_to(&conn, mid.id, Some(root.id)).expect("no-op move should succeed");
    }

    #[test]
    fn move_reparents_and_reorders() {
        let conn = db();
        let a = create(&conn, 1, None, "a").unwrap();
        let b = create(&conn, 1, None, "b").unwrap();
        let child = create(&conn, 1, Some(a.id), "child").unwrap();

        let moved = move_to(&conn, child.id, Some(b.id)).unwrap();
        assert_eq!(moved.parent_id, Some(b.id));
        assert_eq!(moved.sort_order, 0);
        assert_eq!(subtree_ids(&conn, a.id).unwrap(), vec![a.id]);
    }

    #[test]
    fn cannot_move_into_trashed_folder() {
        let conn = db();
        let a = create(&conn, 1, None, "a").unwrap();
        let b = create(&conn, 1, None, "b").unwrap();
        soft_delete_tree(&conn, b.id).unwrap();

        assert!(matches!(
            move_to(&conn, a.id, Some(b.id)).unwrap_err(),
            AppError::InvalidInput {
                field: "parentId",
                ..
            }
        ));
    }

    #[test]
    fn soft_delete_cascades_to_descendants_and_requests() {
        let conn = db();
        let root = create(&conn, 1, None, "root").unwrap();
        let mid = create(&conn, 1, Some(root.id), "mid").unwrap();
        let leaf = create(&conn, 1, Some(mid.id), "leaf").unwrap();
        let outside = create(&conn, 1, None, "outside").unwrap();

        insert_request(&conn, mid.id, "r1");
        insert_request(&conn, leaf.id, "r2");
        let keeper = insert_request(&conn, outside.id, "keep");

        let (folders, requests) = soft_delete_tree(&conn, root.id).unwrap();
        assert_eq!(folders, 3);
        assert_eq!(requests, 2);

        assert!(
            list_by_workspace(&conn, 1).unwrap().len() == 1,
            "only `outside` stays"
        );
        assert_eq!(list_deleted(&conn, 1).unwrap().len(), 3);

        let orphan_deleted: Option<String> = conn
            .query_row(
                "SELECT deleted_at FROM request WHERE id = ?1",
                [keeper],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            orphan_deleted.is_none(),
            "unrelated request must stay alive"
        );
    }

    #[test]
    fn restore_brings_back_the_whole_branch() {
        let conn = db();
        let root = create(&conn, 1, None, "root").unwrap();
        let mid = create(&conn, 1, Some(root.id), "mid").unwrap();
        insert_request(&conn, mid.id, "r1");

        soft_delete_tree(&conn, root.id).unwrap();
        restore_tree(&conn, root.id).unwrap();

        assert_eq!(list_by_workspace(&conn, 1).unwrap().len(), 2);
        assert!(list_deleted(&conn, 1).unwrap().is_empty());
        let alive: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM request WHERE deleted_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(alive, 1);
    }

    /// design.md §3.3：父级已删时，还原的节点落到根
    #[test]
    fn restore_falls_back_to_root_when_parent_is_still_trashed() {
        let conn = db();
        let root = create(&conn, 1, None, "root").unwrap();
        let child = create(&conn, 1, Some(root.id), "child").unwrap();

        // 只删 root 自己（不级联），再单独删 child，然后只还原 child
        conn.execute(
            "UPDATE folder SET deleted_at = '2026-01-01T00:00:00.000Z' WHERE id = ?1",
            [root.id],
        )
        .unwrap();

        let restored = restore_tree(&conn, child.id).unwrap();
        assert_eq!(restored.parent_id, None, "orphan must land at root");
    }

    /// 回归：SQLite 的 `changes()` 不计外键级联删掉的行，
    /// 所以层数 ≥ 2 时按 `execute()` 返回值累加会少算。三层树能暴露得最清楚。
    #[test]
    fn purge_removes_subtree_and_its_requests() {
        let conn = db();
        let root = create(&conn, 1, None, "root").unwrap();
        let child = create(&conn, 1, Some(root.id), "child").unwrap();
        let grandchild = create(&conn, 1, Some(child.id), "grandchild").unwrap();
        insert_request(&conn, child.id, "r1");
        insert_request(&conn, grandchild.id, "r2");

        let deleted = purge_tree(&conn, root.id).unwrap();
        assert_eq!(deleted, 5, "2 requests + 3 folders");

        let folders: i64 = conn
            .query_row("SELECT COUNT(*) FROM folder", [], |r| r.get(0))
            .unwrap();
        let requests: i64 = conn
            .query_row("SELECT COUNT(*) FROM request", [], |r| r.get(0))
            .unwrap();
        assert_eq!(folders, 0);
        assert_eq!(requests, 0);
    }

    #[test]
    fn token_source_flag_toggles() {
        let conn = db();
        let f = create(&conn, 1, None, "auth").unwrap();
        assert!(!f.is_token_source);
        assert!(set_token_source(&conn, f.id, true).unwrap().is_token_source);
        assert!(
            !set_token_source(&conn, f.id, false)
                .unwrap()
                .is_token_source
        );
    }

    #[test]
    fn missing_folder_reports_not_found() {
        let conn = db();
        assert!(matches!(
            get(&conn, 999).unwrap_err(),
            AppError::NotFound {
                entity: "folder",
                ..
            }
        ));
        assert!(matches!(
            soft_delete_tree(&conn, 999).unwrap_err(),
            AppError::NotFound {
                entity: "folder",
                ..
            }
        ));
    }
}
