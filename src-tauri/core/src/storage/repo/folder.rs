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

const COLUMNS: &str = "id, workspace_id, parent_id, name, sort_order, is_token_source, is_system, \
     created_at, updated_at, deleted_at";

/// 系统分类（「token刷新」）的名字。
///
/// **刻意是字面量，不走前端 i18n**：这一行由程序在启动时建好、用户既不能删
/// 也不能改名，所以名字必须是确定的；而且它会出现在 MCP / CLI 的输出里，
/// 那两个入口根本没有 i18n 表可查。与默认工作区那个「名字留空、前端补文案」
/// 的处理不同 —— 那里的名字是**展示用**的，这里的名字是**约定**。
pub const SYSTEM_FOLDER_NAME: &str = "token刷新";

/// 系统分类的 `sort_order`：取负值钉在整棵树最上面。
///
/// 不用 0 —— 那是用户第一个根分类的位置（`create` 算的是 `MAX(sort_order) + 1`），
/// 撞上了就得靠 id 分胜负，而系统分类的 id 未必更小（存量库补建时它反而最大）。
const SYSTEM_SORT_ORDER: i64 = -1;

fn map_row(row: &Row<'_>) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: row.get("id")?,
        workspace_id: row.get("workspace_id")?,
        parent_id: row.get("parent_id")?,
        name: row.get("name")?,
        sort_order: row.get("sort_order")?,
        is_token_source: row.get("is_token_source")?,
        is_system: row.get("is_system")?,
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
    collect(conn, &sql, &[&workspace_id])
}

/// 回收站里的文件夹
pub fn list_deleted(conn: &Connection, workspace_id: i64) -> AppResult<Vec<Folder>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM folder
         WHERE workspace_id = ?1 AND deleted_at IS NOT NULL
         ORDER BY deleted_at DESC, id DESC"
    );
    collect(conn, &sql, &[&workspace_id])
}

/// 逐行收集。
///
/// `map_row` 只做列类型转换（不做 JSON 解析了，见下），所以直接喂给 `query_map` 就行 ——
/// 不必像 `repo::request` 那样手写 `rows.next()` 循环。
fn collect(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> AppResult<Vec<Folder>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, map_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 按 id 取。软删除的也能取到（回收站要用）。
pub fn get(conn: &Connection, id: i64) -> AppResult<Folder> {
    let sql = format!("SELECT {COLUMNS} FROM folder WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([id])?;
    match rows.next()? {
        Some(row) => Ok(map_row(row)?),
        None => Err(AppError::NotFound {
            entity: "folder",
            id,
        }),
    }
}

/// 从某个文件夹向上的完整链，**最近的在前**，含自身。
///
/// **只有 Token 源查找还在用它**。曾经的授权继承（design.md §4.4）已经被 007
/// 迁移拆掉 —— 凭据现在落在 `request.headers` 里，分类不再持有任何授权配置，
/// 所以「往上找授权」这件事在运行时根本没有第二处调用点。
///
/// N+1 次查询，但文件夹树的深度是个位数，用一条递归 SQL 换这点开销不划算 ——
/// 而且递归 SQL 想把「按深度排序」和「防环」同时写对，比这里复杂得多。
///
/// 防环：每走一级都要求 parent 没走过。SQL 侧的递归查询靠 `UNION` 去重，
/// 这是同一件事的 Rust 侧一半。
pub fn ancestry(conn: &Connection, folder_id: i64) -> AppResult<Vec<Folder>> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut current = Some(folder_id);

    while let Some(id) = current {
        if !seen.insert(id) {
            break;
        }
        let folder = match get(conn, id) {
            Ok(f) => f,
            // 父级被彻底删过（外键 ON DELETE CASCADE 只会带走子节点，
            // 但这个 id 可能来自一条脏数据）—— 链就断在这里，不报错
            Err(AppError::NotFound { .. }) => break,
            Err(e) => return Err(e),
        };
        current = folder.parent_id;
        out.push(folder);
    }

    Ok(out)
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

// ---------------------------------------------------------------
// 系统分类（「token刷新」）
// ---------------------------------------------------------------

/// 挡在删除 / 改名 / 移动前面。`action` 只进日志与 debug 信息，不进 UI 文案。
fn ensure_not_system(folder: &Folder, action: &'static str) -> AppResult<()> {
    if folder.is_system {
        return Err(AppError::SystemFolder {
            id: folder.id,
            action,
        });
    }
    Ok(())
}

/// 某工作区的系统分类。
///
/// **刻意不过滤 `deleted_at`**：`ensure_system_folder` 靠它判「已经建过了吗」，
/// 而既然有 `(workspace_id) WHERE is_system = 1` 那条唯一索引，漏掉一个软删除的
/// 系统分类就会让补建直接撞唯一约束。系统分类本来就删不掉（见各处的
/// `ensure_not_system`），这个分支在正常库里走不到。
pub fn system_folder(conn: &Connection, workspace_id: i64) -> AppResult<Option<Folder>> {
    let sql = format!("SELECT {COLUMNS} FROM folder WHERE workspace_id = ?1 AND is_system = 1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([workspace_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(map_row(row)?)),
        None => Ok(None),
    }
}

/// 保证工作区有且只有一个系统分类「token刷新」。返回它。
///
/// 幂等：已经存在就原样返回。存量库（004 之前建的）靠这条补上，
/// 所以它出现在启动路径里而不是迁移脚本里 —— 迁移脚本拿不到「给每个
/// 工作区都来一条」的循环，而这里本来就有工作区 id。
///
/// 顺手把 `is_token_source` 也置 1：Token 来源固定配在这个分类下，
/// 集合树上那个「Token」角标就得亮着，否则用户看不出它是干嘛的。
pub fn ensure_system_folder(conn: &Connection, workspace_id: i64) -> AppResult<Folder> {
    if let Some(existing) = system_folder(conn, workspace_id)? {
        return Ok(existing);
    }

    let now = now_iso();
    conn.execute(
        "INSERT INTO folder
           (workspace_id, parent_id, name, sort_order, is_token_source, is_system, created_at, updated_at)
         VALUES (?1, NULL, ?2, ?3, 1, 1, ?4, ?4)",
        rusqlite::params![workspace_id, SYSTEM_FOLDER_NAME, SYSTEM_SORT_ORDER, now],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn rename(conn: &Connection, id: i64, name: &str) -> AppResult<Folder> {
    // 先取一次而不是看 `execute` 的影响行数：一箭三雕 —— 不存在还是报
    // `NotFound`（与原来一致），多了个守卫的落点，而且不用再判 affected。
    let folder = get(conn, id)?;
    ensure_not_system(&folder, "renamed")?;

    conn.execute(
        "UPDATE folder SET name = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![id, name, now_iso()],
    )?;
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
    // 系统分类必须待在根上：它承载的是「整个工作区的 Token」，挪进任何一棵子树
    // 都会让那个子树之外的东西看不见它
    ensure_not_system(&folder, "moved")?;

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
    ensure_not_system(&get(conn, id)?, "deleted")?;

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
/// 返回 `(文件夹数, 请求数)`。
///
/// 为什么按**类别**分开返回、而不是一个总数：界面上要显示
/// 「已彻底删除：3 个分类 / 12 个接口」，而调用方拿不到这两个数字 ——
/// 请求数是靠显式 DELETE 得来的，文件夹数只能从子树大小推。
/// 给一个混合的总数等于把「拆不开」这件事推给每一处调用点。
pub fn purge_tree(conn: &Connection, id: i64) -> AppResult<(usize, usize)> {
    // 系统分类进不了回收站（`soft_delete_tree` 就挡住了），这道守卫是**兜底**：
    // 「清空回收站」会遍历回收站里每个根节点调它，万一哪天库里真出现一个
    // 被标删的系统分类，宁可让这次清空报错，也不要静默把它连同 Token 配置一起抹掉
    ensure_not_system(&get(conn, id)?, "purged")?;

    // request.folder_id 是 ON DELETE SET NULL，所以必须先显式删掉子树的请求，
    // 否则它们会变成「没有文件夹的孤儿」而不是被一并删除。
    let ids = subtree_ids(conn, id)?;
    if ids.is_empty() {
        return Err(AppError::NotFound {
            entity: "folder",
            id,
        });
    }

    let mut requests = 0usize;
    for folder_id in &ids {
        requests += conn.execute("DELETE FROM request WHERE folder_id = ?1", [folder_id])?;
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

    Ok((ids.len(), requests))
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

        let (folders, requests) = purge_tree(&conn, root.id).unwrap();
        assert_eq!(folders, 3, "三层树必须报 3 个文件夹（少算的坑）");
        assert_eq!(requests, 2);

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
        let f = create(&conn, 1, None, "草稿").unwrap();
        assert!(!f.is_token_source);
        assert!(set_token_source(&conn, f.id, true).unwrap().is_token_source);
        assert!(
            !set_token_source(&conn, f.id, false)
                .unwrap()
                .is_token_source
        );
    }

    #[test]
    fn ensure_system_folder_is_idempotent_and_lands_first() {
        let conn = db();
        let normal = create(&conn, 1, None, "我的接口").unwrap();

        let sys = ensure_system_folder(&conn, 1).unwrap();
        assert_eq!(sys.name, SYSTEM_FOLDER_NAME);
        assert_eq!(sys.parent_id, None, "系统分类必须在根上");
        assert!(sys.is_system && sys.is_token_source, "同时是 Token 分组");
        assert_eq!(sys.sort_order, -1, "排在最上面");

        let again = ensure_system_folder(&conn, 1).unwrap();
        assert_eq!(again.id, sys.id, "第二次调用必须复用同一行");
        assert_eq!(
            list_by_workspace(&conn, 1).unwrap().len(),
            2,
            "不该建出第二个"
        );

        let listed = list_by_workspace(&conn, 1).unwrap();
        assert_eq!(listed[0].id, sys.id, "系统分类排第一");
        assert_eq!(listed[1].id, normal.id);

        // 另一个工作区各有一份
        conn.execute(
            "INSERT INTO workspace (id, name, created_at, updated_at)
             VALUES (2, 'w2', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        )
        .unwrap();
        let other = ensure_system_folder(&conn, 2).unwrap();
        assert_ne!(other.id, sys.id);
    }

    /// 系统分类是全项目唯一一处「用户动不了」的分类 ——
    /// 删 / 改名 / 移动三条路都必须堵死，而且报的是**专用错误码**
    /// （前端靠它把菜单项置灰，而不是等点了才弹一个「输入不合法」）
    #[test]
    fn system_folder_cannot_be_deleted_renamed_or_moved() {
        let conn = db();
        let sys = ensure_system_folder(&conn, 1).unwrap();
        let target = create(&conn, 1, None, "别处").unwrap();

        let is_system_err = |e: AppError| matches!(e, AppError::SystemFolder { .. });

        assert!(is_system_err(soft_delete_tree(&conn, sys.id).unwrap_err()));
        assert!(is_system_err(purge_tree(&conn, sys.id).unwrap_err()));
        assert!(is_system_err(
            rename(&conn, sys.id, "改名试试").unwrap_err()
        ));
        assert!(is_system_err(
            move_to(&conn, sys.id, Some(target.id)).unwrap_err()
        ));

        // 一根汗毛都没动
        let after = get(&conn, sys.id).unwrap();
        assert_eq!(after.name, SYSTEM_FOLDER_NAME);
        assert_eq!(after.parent_id, None);
        assert!(after.deleted_at.is_none());

        // 普通分类不受影响
        soft_delete_tree(&conn, target.id).unwrap();
        assert_eq!(list_deleted(&conn, 1).unwrap().len(), 1);
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
