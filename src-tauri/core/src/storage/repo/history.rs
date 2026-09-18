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
use crate::storage::repo::{bool_to_int, opt_flag, opt_text, parse_json_column, to_json_column};

const COLUMNS: &str = "id, workspace_id, request_id, request_name, snapshot, status, \
     duration_ms, resp_size, error, environment, executed_at, deleted_at, \
     curl, curl_masked, resp_headers, resp_body, resp_body_truncated";

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
        curl: opt_text(row, "curl")?,
        curl_masked: opt_text(row, "curl_masked")?,
        // 头部那列是 JSON 数组。坏了就报错而不是退化成空数组 —— 空数组会被
        // 界面显示成「这次响应没有任何头」，与「数据坏了」是两回事。
        resp_headers: opt_text(row, "resp_headers")?
            .map(|raw| parse_json_column(&raw, "history.resp_headers"))
            .transpose()?,
        resp_body: opt_text(row, "resp_body")?,
        resp_body_truncated: opt_flag(row, "resp_body_truncated")?,
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
    // 头部的 `None` 与 `Some([])` 是两回事（前者=没拿到响应，后者=响应真没有头），
    // 所以这里在 Rust 侧序列化而**不是**在 SQL 里 `json(...)` 兜一把。
    let resp_headers = entry
        .resp_headers
        .as_ref()
        .map(|value| to_json_column(value, "history.resp_headers"))
        .transpose()?;

    conn.execute(
        "INSERT INTO history
           (workspace_id, request_id, request_name, snapshot, status,
            duration_ms, resp_size, error, environment, executed_at,
            curl, curl_masked, resp_headers, resp_body, resp_body_truncated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
            entry.curl,
            entry.curl_masked,
            resp_headers,
            entry.resp_body,
            entry.resp_body_truncated.map(bool_to_int),
        ],
    )?;
    get(conn, conn.last_insert_rowid())
}

/// 某条请求最近一次**带 curl 的**执行。
///
/// `curl IS NOT NULL` 这个条件不能省：升级上来的库里，最新那条可能是 006 之前的
/// 记录（没有 curl）。若只按时间取最新一条，用户会看到「上次没有记录」——
/// 而其实再往前一条就有，表现是「这功能时灵时不灵」。
pub fn latest_with_curl(conn: &Connection, request_id: i64) -> AppResult<Option<HistoryEntry>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM history
         WHERE request_id = ?1 AND deleted_at IS NULL AND curl IS NOT NULL
         ORDER BY executed_at DESC, id DESC
         LIMIT 1"
    );
    Ok(collect(conn, &sql, &[&request_id])?.into_iter().next())
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

/// 与 [`search`] 配套的**匹配**总数。
///
/// 不能拿 [`count`] 顶替：那是全部存活历史的条数，搜索时会把分页
/// 判断带偏 —— 搜出 3 条却告诉前端「共 128 条」，滚到底会一直请求空页。
pub fn count_matching(conn: &Connection, workspace_id: i64, keyword: &str) -> AppResult<i64> {
    let pattern = format!("%{keyword}%");
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM history
         WHERE workspace_id = ?1 AND deleted_at IS NULL AND request_name LIKE ?2",
        rusqlite::params![workspace_id, pattern],
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
            curl: None,
            curl_masked: None,
            resp_headers: None,
            resp_body: None,
            resp_body_truncated: None,
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

    /// 带 request_id 与 curl 的历史行。curl 的 `None` 是**有意义**的取值
    /// （那条历史发生在本版之前或写入方没给），所以这里显式区分两种输入。
    fn entry_with_curl(id: i64, name: &str, curl: Option<&str>) -> NewHistoryEntry {
        let mut e = entry(name);
        e.request_id = Some(id);
        e.curl = curl.map(str::to_string);
        e.curl_masked = curl.map(|c| format!("[masked]{c}"));
        e
    }

    #[test]
    fn curl_columns_round_trip_and_stay_null_when_absent() {
        let conn = db();

        let saved = insert(
            &conn,
            &entry_with_curl(7, "带 curl", Some("curl -X GET 'https://x/y'")),
        )
        .unwrap();
        assert_eq!(saved.curl.as_deref(), Some("curl -X GET 'https://x/y'"));
        assert_eq!(
            saved.curl_masked.as_deref(),
            Some("[masked]curl -X GET 'https://x/y'")
        );

        let without = insert(&conn, &entry("没有 curl")).unwrap();
        assert_eq!(without.curl, None, "没给过 curl 的行必须读回 None");
        assert_eq!(without.curl_masked, None);
    }

    /// 响应三列的回归网。**三态**是这里的重点：`None`（这条历史早于 010）
    /// 与「明确没截断 / 明确没有头」必须读得出来是两回事。
    #[test]
    fn response_columns_round_trip_and_keep_their_three_states() {
        let conn = db();

        let mut with_response = entry("带响应");
        with_response.resp_size = Some(4096);
        with_response.resp_headers = Some(json!([
            { "name": "content-type", "value": "application/json" }
        ]));
        with_response.resp_body = Some(r#"{"ok":true}"#.into());
        with_response.resp_body_truncated = Some(true);

        let saved = insert(&conn, &with_response).unwrap();
        assert_eq!(
            saved.resp_headers.as_ref().unwrap()[0]["name"],
            "content-type"
        );
        assert_eq!(saved.resp_body.as_deref(), Some(r#"{"ok":true}"#));
        assert_eq!(saved.resp_body_truncated, Some(true));
        assert_eq!(
            saved.resp_size,
            Some(4096),
            "真实大小那列不受截断影响 —— 界面靠它说明「被截掉的是多少」"
        );

        // 不给就是 None。这里若退化成 `Some(false)` / `Some([])`，
        // 升级上来的老库会满屏显示「未截断」—— 一个我们并不知道的结论。
        let without = insert(&conn, &entry("没有响应")).unwrap();
        assert!(without.resp_headers.is_none());
        assert!(without.resp_body.is_none());
        assert!(without.resp_body_truncated.is_none());
    }

    /// 「上次跑的是什么」要跳过没有 curl 的行：升级上来的库里，
    /// 最新那条常常正是 006 之前的记录，直接按时间取最新一条会让用户看到空面板。
    #[test]
    fn latest_with_curl_skips_rows_that_do_not_have_one() {
        let conn = db();

        let older = insert(
            &conn,
            &entry_with_curl(7, "旧的有 curl", Some("curl -X GET 'https://x/old'")),
        )
        .unwrap();
        insert(&conn, &entry_with_curl(7, "新的没有", None)).unwrap();

        let found = latest_with_curl(&conn, 7)
            .unwrap()
            .expect("应当找回旧的那条");
        assert_eq!(found.id, older.id);
        assert_eq!(found.curl.as_deref(), Some("curl -X GET 'https://x/old'"));
    }

    #[test]
    fn latest_with_curl_ignores_other_requests_and_trash() {
        let conn = db();
        insert(
            &conn,
            &entry_with_curl(8, "别人的", Some("curl 'https://other'")),
        )
        .unwrap();

        assert!(
            latest_with_curl(&conn, 7).unwrap().is_none(),
            "别的请求的 curl 不该被捞过来"
        );

        let mine = insert(
            &conn,
            &entry_with_curl(7, "我的", Some("curl 'https://mine'")),
        )
        .unwrap();
        soft_delete(&conn, mine.id).unwrap();
        assert!(
            latest_with_curl(&conn, 7).unwrap().is_none(),
            "进了回收站的记录不该再被当成「上次跑的」"
        );
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

        // 计数必须跟着关键字走，否则「还有下一页」会一直为真
        assert_eq!(count_matching(&conn, 1, "order").unwrap(), 1);
        assert_eq!(count_matching(&conn, 1, "nothing").unwrap(), 0);
        assert_eq!(count(&conn, 1).unwrap(), 2, "不带关键字时才是全量");
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
