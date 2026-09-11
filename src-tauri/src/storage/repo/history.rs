//! 历史记录仓储。
//!
//! 历史是**写入最频繁**的表（每次发请求一行），所以：
//! * 列表默认只取存活行，并按 `executed_at DESC` 走 `idx_history_time` 索引
//! * 分页用 LIMIT/OFFSET（历史是「新的在最前」，用户很少翻到很深的地方）
//! * `workspace_id` 没有外键约束 —— 工作区被删后历史仍应可读（design.md §3.2）

use rusqlite::{Connection, Row};

use crate::error::{AppError, AppResult};
use crate::storage::models::{HistoryEntry, NewHistoryEntry};
use crate::storage::now_iso;
use crate::storage::repo::{opt_text, parse_json_column, to_json_column};

const COLUMNS: &str = "id, workspace_id, request_id, request_name, snapshot, status, \
     duration_ms, resp_size, error, environment, executed_at, deleted_at";

fn map_row(row: &Row<'_>) -> AppResult<HistoryEntry> {
    let snapshot_raw: String = row.get("snapshot")?;
    Ok(HistoryEntry {
        id: row.get("id")?,
        workspace_id: row.get("workspace_id")?,
        request_id: row.get("request_id")?,
        request_name: row.get("request_name")?,
        snapshot: parse_json_column(&snapshot_raw, "history.snapshot")?,
        status: row.get("status")?,
        duration_ms: row.get("duration_ms")?,
        resp_size: row.get("resp_size")?,
        error: row.get("error")?,
        environment: opt_text(row, "environment")?,
        executed_at: row.get("executed_at")?,
        deleted_at: opt_text(row, "deleted_at")?,
    })
}

fn collect(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> AppResult<Vec<HistoryEntry>> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query(params)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(map_row(row)?);
    }
    Ok(out)
}

/// 写一条历史。返回新建的行。
pub fn insert(conn: &Connection, entry: &NewHistoryEntry) -> AppResult<HistoryEntry> {
    let snapshot = to_json_column(&entry.snapshot, "history.snapshot")?;
    conn.execute(
        "INSERT INTO history
           (workspace_id, request_id, request_name, snapshot, status,
            duration_ms, resp_size, error, environment, executed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            entry.workspace_id,
            entry.request_id,
            entry.request_name,
            snapshot,
            entry.status,
            entry.duration_ms,
            entry.resp_size,
            entry.error,
            entry.environment,
            now_iso(),
        ],
    )?;
    get(conn, conn.last_insert_rowid())
}

/// 存活历史，时间倒序。
pub fn list(
    conn: &Connection,
    workspace_id: i64,
    limit: i64,
    offset: i64,
) -> AppResult<Vec<HistoryEntry>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM history
         WHERE workspace_id = ?1 AND deleted_at IS NULL
         ORDER BY executed_at DESC, id DESC
         LIMIT ?2 OFFSET ?3"
    );
    collect(conn, &sql, &[&workspace_id, &limit, &offset])
}

/// 按请求名 / 地址模糊搜索（P3 的 HistoryList 用）
pub fn search(
    conn: &Connection,
    workspace_id: i64,
    keyword: &str,
    limit: i64,
    offset: i64,
) -> AppResult<Vec<HistoryEntry>> {
    let pattern = format!("%{keyword}%");
    let sql = format!(
        "SELECT {COLUMNS} FROM history
         WHERE workspace_id = ?1 AND deleted_at IS NULL AND request_name LIKE ?2
         ORDER BY executed_at DESC, id DESC
         LIMIT ?3 OFFSET ?4"
    );
    collect(conn, &sql, &[&workspace_id, &pattern, &limit, &offset])
}

pub fn count(conn: &Connection, workspace_id: i64) -> AppResult<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM history WHERE workspace_id = ?1 AND deleted_at IS NULL",
        [workspace_id],
        |r| r.get(0),
    )?)
}

pub fn list_deleted(conn: &Connection, workspace_id: i64) -> AppResult<Vec<HistoryEntry>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM history
         WHERE workspace_id = ?1 AND deleted_at IS NOT NULL
         ORDER BY deleted_at DESC, id DESC"
    );
    collect(conn, &sql, &[&workspace_id])
}

pub fn get(conn: &Connection, id: i64) -> AppResult<HistoryEntry> {
    let sql = format!("SELECT {COLUMNS} FROM history WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([id])?;
    match rows.next()? {
        Some(row) => map_row(row),
        None => Err(AppError::NotFound {
            entity: "history",
            id,
        }),
    }
}

pub fn soft_delete(conn: &Connection, id: i64) -> AppResult<()> {
    let affected = conn.execute(
        "UPDATE history SET deleted_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
        rusqlite::params![id, now_iso()],
    )?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "history",
            id,
        });
    }
    Ok(())
}

pub fn restore(conn: &Connection, id: i64) -> AppResult<HistoryEntry> {
    let affected = conn.execute("UPDATE history SET deleted_at = NULL WHERE id = ?1", [id])?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "history",
            id,
        });
    }
    get(conn, id)
}

pub fn purge(conn: &Connection, id: i64) -> AppResult<()> {
    let affected = conn.execute("DELETE FROM history WHERE id = ?1", [id])?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "history",
            id,
        });
    }
    Ok(())
}

/// 把全部存活历史移进回收站（UI 上的「清空历史」）。
///
/// 刻意不做硬删除：design.md §3.3 的语义是「删除 = 进回收站」。
pub fn trash_all(conn: &Connection, workspace_id: i64) -> AppResult<usize> {
    let affected = conn.execute(
        "UPDATE history SET deleted_at = ?2
         WHERE workspace_id = ?1 AND deleted_at IS NULL",
        rusqlite::params![workspace_id, now_iso()],
    )?;
    Ok(affected)
}

/// 清空回收站（硬删除）。`workspace_id = None` 表示全库清理。
pub fn purge_trash(conn: &Connection, workspace_id: Option<i64>) -> AppResult<usize> {
    let affected = match workspace_id {
        Some(id) => conn.execute(
            "DELETE FROM history WHERE workspace_id = ?1 AND deleted_at IS NOT NULL",
            [id],
        )?,
        None => conn.execute("DELETE FROM history WHERE deleted_at IS NOT NULL", [])?,
    };
    Ok(affected)
}

/// 清理回收站里超过 `retention_days` 天的记录（design.md §3.3：启动时调用）。
pub fn purge_expired(conn: &Connection, retention_days: i64) -> AppResult<usize> {
    if retention_days <= 0 {
        // 0 或负数视为「不自动清理」，避免误配把回收站清空
        return Ok(0);
    }
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(retention_days))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let affected = conn.execute(
        "DELETE FROM history WHERE deleted_at IS NOT NULL AND deleted_at < ?1",
        [cutoff],
    )?;
    Ok(affected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrate;
    use serde_json::json;

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

    fn entry(name: &str) -> NewHistoryEntry {
        NewHistoryEntry {
            workspace_id: 1,
            request_id: None,
            request_name: name.into(),
            snapshot: json!({ "method": "GET", "url": "https://example.com" }),
            status: Some(200),
            duration_ms: Some(42),
            resp_size: Some(128),
            error: None,
            environment: Some("dev".into()),
        }
    }

    /// 让 executed_at 可控，才能测排序与保留期
    fn insert_at(conn: &Connection, name: &str, executed_at: &str) -> i64 {
        let snapshot = r#"{"method":"GET","url":"https://example.com"}"#;
        conn.execute(
            "INSERT INTO history (workspace_id, request_name, snapshot, executed_at)
             VALUES (1, ?1, ?2, ?3)",
            rusqlite::params![name, snapshot, executed_at],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn insert_roundtrips_snapshot() {
        let conn = db();
        let h = insert(&conn, &entry("health")).unwrap();
        assert_eq!(h.request_name, "health");
        assert_eq!(h.status, Some(200));
        assert_eq!(h.snapshot["method"], "GET");
        assert_eq!(h.duration_ms, Some(42));
        assert_eq!(h.environment.as_deref(), Some("dev"));
        assert!(h.deleted_at.is_none());
        assert!(h.executed_at.ends_with('Z'));
    }

    #[test]
    fn list_is_newest_first_and_paginates() {
        let conn = db();
        insert_at(&conn, "oldest", "2026-01-01T00:00:00.000Z");
        insert_at(&conn, "middle", "2026-02-01T00:00:00.000Z");
        insert_at(&conn, "newest", "2026-03-01T00:00:00.000Z");

        let page1 = list(&conn, 1, 2, 0).unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].request_name, "newest");
        assert_eq!(page1[1].request_name, "middle");

        let page2 = list(&conn, 1, 2, 2).unwrap();
        assert_eq!(page2.len(), 1);
        assert_eq!(page2[0].request_name, "oldest");

        assert_eq!(count(&conn, 1).unwrap(), 3);
    }

    #[test]
    fn search_matches_request_name() {
        let conn = db();
        insert_at(&conn, "list users", "2026-01-01T00:00:00.000Z");
        insert_at(&conn, "create order", "2026-01-02T00:00:00.000Z");

        let hits = search(&conn, 1, "order", 10, 0).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].request_name, "create order");
        assert!(search(&conn, 1, "nothing", 10, 0).unwrap().is_empty());
    }

    #[test]
    fn trash_then_restore_then_purge() {
        let conn = db();
        let h = insert(&conn, &entry("x")).unwrap();

        assert_eq!(trash_all(&conn, 1).unwrap(), 1);
        assert!(list(&conn, 1, 10, 0).unwrap().is_empty());
        assert_eq!(list_deleted(&conn, 1).unwrap().len(), 1);

        restore(&conn, h.id).unwrap();
        assert_eq!(list(&conn, 1, 10, 0).unwrap().len(), 1);

        soft_delete(&conn, h.id).unwrap();
        assert_eq!(purge_trash(&conn, Some(1)).unwrap(), 1);
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn soft_delete_is_not_idempotent() {
        let conn = db();
        let h = insert(&conn, &entry("x")).unwrap();
        soft_delete(&conn, h.id).unwrap();
        assert!(matches!(
            soft_delete(&conn, h.id).unwrap_err(),
            AppError::NotFound {
                entity: "history",
                ..
            }
        ));
    }

    /// 保留期清理只动「已在回收站且超期」的行
    #[test]
    fn purge_expired_only_removes_old_trashed_rows() {
        let conn = db();
        let fresh = insert_at(&conn, "fresh-alive", "2026-01-01T00:00:00.000Z");
        let old_trashed = insert_at(&conn, "old-trashed", "2020-01-01T00:00:00.000Z");
        let recent_trashed = insert_at(&conn, "recent-trashed", "2026-01-01T00:00:00.000Z");

        conn.execute(
            "UPDATE history SET deleted_at = '2020-01-01T00:00:00.000Z' WHERE id = ?1",
            [old_trashed],
        )
        .unwrap();
        conn.execute(
            "UPDATE history SET deleted_at = ?2 WHERE id = ?1",
            rusqlite::params![recent_trashed, now_iso()],
        )
        .unwrap();

        let removed = purge_expired(&conn, 30).unwrap();
        assert_eq!(removed, 1, "only the 2020 trashed row is expired");

        assert!(get(&conn, fresh).is_ok(), "alive rows are untouched");
        assert!(get(&conn, old_trashed).is_err());
        assert!(get(&conn, recent_trashed).is_ok());
    }

    #[test]
    fn purge_expired_disabled_for_non_positive_days() {
        let conn = db();
        let h = insert_at(&conn, "x", "2020-01-01T00:00:00.000Z");
        conn.execute(
            "UPDATE history SET deleted_at = '2020-01-01T00:00:00.000Z' WHERE id = ?1",
            [h],
        )
        .unwrap();

        assert_eq!(purge_expired(&conn, 0).unwrap(), 0);
        assert_eq!(purge_expired(&conn, -5).unwrap(), 0);
        assert!(get(&conn, h).is_ok());
    }

    /// 历史刻意不加工作区外键：工作区被删后仍要读得到
    #[test]
    fn history_has_no_workspace_foreign_key() {
        let conn = db();
        let snapshot = "{}";
        conn.execute(
            "INSERT INTO history (workspace_id, request_name, snapshot, executed_at)
             VALUES (987654, 'ghost', ?1, '2026-01-01T00:00:00.000Z')",
            [snapshot],
        )
        .expect("no FK should block this");
    }

    #[test]
    fn missing_row_reports_not_found() {
        let conn = db();
        assert!(matches!(
            get(&conn, 404).unwrap_err(),
            AppError::NotFound {
                entity: "history",
                ..
            }
        ));
        assert!(matches!(
            purge(&conn, 404).unwrap_err(),
            AppError::NotFound { .. }
        ));
    }
}
