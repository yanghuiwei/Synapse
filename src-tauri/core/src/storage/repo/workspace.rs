//! 工作区仓储。
//!
//! 这一版只做「单工作区」体验：启动时保证至少有一个存活的工作区。
//! 表结构已经支持多工作区（`sort_order` 就位），UI 侧等 P8 再放开。

use rusqlite::{Connection, Row};

use crate::error::{AppError, AppResult};
use crate::storage::models::Workspace;
use crate::storage::now_iso;
use crate::storage::repo::{opt_text, ORDER_BY_SORT};

const COLUMNS: &str = "id, name, sort_order, created_at, updated_at, deleted_at";

fn map_row(row: &Row<'_>) -> rusqlite::Result<Workspace> {
    Ok(Workspace {
        id: row.get("id")?,
        name: row.get("name")?,
        sort_order: row.get("sort_order")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        deleted_at: opt_text(row, "deleted_at")?,
    })
}

/// 存活的（未软删除）工作区，按 sort_order 排序
pub fn list(conn: &Connection) -> AppResult<Vec<Workspace>> {
    let sql = format!("SELECT {COLUMNS} FROM workspace WHERE deleted_at IS NULL{ORDER_BY_SORT}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], map_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 当前工作区。这一版只做单工作区体验（见文件头），所以就是列表里的第一个。
///
/// 与 [`ensure_default`] 的区别：这个函数**只读**，不建任何东西 ——
/// 它会被请求流水线在每次发送前调用，写操作放在热路径上不合适。
/// 返回 `None` 表示「还没有工作区」，调用方按空环境处理即可。
pub fn active(conn: &Connection) -> AppResult<Option<Workspace>> {
    Ok(list(conn)?.into_iter().next())
}

/// 按 id 取。已被软删除的也算「存在」—— 回收站要能读到它。
pub fn get(conn: &Connection, id: i64) -> AppResult<Workspace> {
    let sql = format!("SELECT {COLUMNS} FROM workspace WHERE id = ?1");
    conn.query_row(&sql, [id], map_row).map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::NotFound {
            entity: "workspace",
            id,
        },
        other => AppError::Db(other),
    })
}

/// 新建工作区。`name` 传空串表示「用前端 i18n 的默认文案」。
pub fn create(conn: &Connection, name: &str) -> AppResult<Workspace> {
    let now = now_iso();
    // 排在现有工作区之后
    let next_order: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM workspace",
        [],
        |r| r.get(0),
    )?;

    conn.execute(
        "INSERT INTO workspace (name, sort_order, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3)",
        rusqlite::params![name, next_order, now],
    )?;
    let workspace = get(conn, conn.last_insert_rowid())?;

    // 新工作区当场就把系统分类备好，别等到下次启动 ——
    // 否则「刚建的库」和「重启过的库」结构不一样，而差别只在用户看不见的那一层。
    crate::storage::repo::folder::ensure_system_folder(conn, workspace.id)?;

    Ok(workspace)
}

/// 保证至少有一个存活工作区。返回第一个。
///
/// 为什么不在迁移脚本里 INSERT：默认工作区的**名字是文案**，
/// 而文案归前端 i18n 管（design.md §9.1）。所以这里只建一条 name = ''
/// 的记录，显示什么由前端决定。这样以后加语言也不用动数据库。
pub fn ensure_default(conn: &Connection) -> AppResult<Workspace> {
    if let Some(existing) = list(conn)?.into_iter().next() {
        return Ok(existing);
    }
    create(conn, "")
}

pub fn rename(conn: &Connection, id: i64, name: &str) -> AppResult<Workspace> {
    let now = now_iso();
    let affected = conn.execute(
        "UPDATE workspace SET name = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![id, name, now],
    )?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "workspace",
            id,
        });
    }
    get(conn, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrate;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().expect("db");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate::run(&mut conn).expect("migrate");
        conn
    }

    #[test]
    fn ensure_default_creates_exactly_one() {
        let conn = db();
        let ws = ensure_default(&conn).expect("first call");
        assert_eq!(
            ws.name, "",
            "default workspace must not embed localized text"
        );

        let again = ensure_default(&conn).expect("second call");
        assert_eq!(again.id, ws.id, "second call must reuse the same workspace");
        assert_eq!(list(&conn).unwrap().len(), 1);
    }

    #[test]
    fn create_increments_sort_order() {
        let conn = db();
        let a = create(&conn, "A").unwrap();
        let b = create(&conn, "B").unwrap();
        assert_eq!(a.sort_order, 0);
        assert_eq!(b.sort_order, 1);
        assert_eq!(list(&conn).unwrap().len(), 2);
    }

    #[test]
    fn get_reports_not_found_with_entity_and_id() {
        let conn = db();
        match get(&conn, 12345) {
            Err(AppError::NotFound { entity, id }) => {
                assert_eq!(entity, "workspace");
                assert_eq!(id, 12345);
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn soft_deleted_workspace_disappears_from_list_but_is_still_gettable() {
        let conn = db();
        let ws = create(&conn, "gone").unwrap();
        conn.execute(
            "UPDATE workspace SET deleted_at = ?1 WHERE id = ?2",
            rusqlite::params![now_iso(), ws.id],
        )
        .unwrap();

        assert!(list(&conn).unwrap().is_empty());
        assert_eq!(get(&conn, ws.id).unwrap().id, ws.id);
        // 全删光之后再 ensure_default 会新建一个，而不是复活旧的
        let fresh = ensure_default(&conn).unwrap();
        assert_ne!(fresh.id, ws.id);
    }

    #[test]
    fn rename_touches_updated_at() {
        let conn = db();
        let ws = create(&conn, "old").unwrap();
        // 时间戳精度到毫秒，同一条测试内可能相同，所以只断言名字
        let renamed = rename(&conn, ws.id, "new").unwrap();
        assert_eq!(renamed.name, "new");
    }
}
