//! Token 源与缓存仓储（design.md §4.3）。
//!
//! 注意 `token_source` / `token_cache` 两张表都**没有 `deleted_at`**：
//! Token 配置属于「机器用的配置」而不是「用户的内容」，
//! 进回收站只会让 P4 的刷新逻辑要处理一堆边界情况。删除即硬删。

use rusqlite::{Connection, Row};

use crate::error::{AppError, AppResult};
use crate::storage::models::{ExpiryUnit, TokenCache, TokenSource};
use crate::storage::now_iso;
use crate::storage::repo::{bool_to_int, opt_text, ORDER_BY_NAME};

const SOURCE_COLUMNS: &str = "id, workspace_id, folder_id, name, fetch_request_id, \
     access_path, refresh_path, expires_in_path, expires_in_unit, refresh_request_id, \
     auto_refresh, skew_seconds, created_at, updated_at";

const CACHE_COLUMNS: &str =
    "id, token_source_id, access_token, refresh_token, expires_at, raw_response, updated_at";

fn map_source(row: &Row<'_>) -> rusqlite::Result<TokenSource> {
    let unit_raw: String = row.get("expires_in_unit")?;
    Ok(TokenSource {
        id: row.get("id")?,
        workspace_id: row.get("workspace_id")?,
        folder_id: row.get("folder_id")?,
        name: row.get("name")?,
        fetch_request_id: row.get("fetch_request_id")?,
        access_path: row.get("access_path")?,
        refresh_path: row.get("refresh_path")?,
        expires_in_path: row.get("expires_in_path")?,
        expires_in_unit: ExpiryUnit::parse(&unit_raw),
        refresh_request_id: row.get("refresh_request_id")?,
        auto_refresh: row.get("auto_refresh")?,
        skew_seconds: row.get("skew_seconds")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn map_cache(row: &Row<'_>) -> rusqlite::Result<TokenCache> {
    Ok(TokenCache {
        id: row.get("id")?,
        token_source_id: row.get("token_source_id")?,
        access_token: row.get("access_token")?,
        refresh_token: row.get("refresh_token")?,
        expires_at: opt_text(row, "expires_at")?,
        raw_response: opt_text(row, "raw_response")?,
        updated_at: row.get("updated_at")?,
    })
}

fn collect_sources(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> AppResult<Vec<TokenSource>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, map_source)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn list_by_workspace(conn: &Connection, workspace_id: i64) -> AppResult<Vec<TokenSource>> {
    let sql =
        format!("SELECT {SOURCE_COLUMNS} FROM token_source WHERE workspace_id = ?1{ORDER_BY_NAME}");
    collect_sources(conn, &sql, &[&workspace_id])
}

pub fn list_by_folder(conn: &Connection, folder_id: i64) -> AppResult<Vec<TokenSource>> {
    let sql =
        format!("SELECT {SOURCE_COLUMNS} FROM token_source WHERE folder_id = ?1{ORDER_BY_NAME}");
    collect_sources(conn, &sql, &[&folder_id])
}

/// 沿「最近的在前」的文件夹链找第一个 Token 源。
///
/// 判定依据是 `token_source` 表里**真的有没有行**，而不是文件夹上的
/// `is_token_source` 标记 —— 那个标记只是 UI 的入口引导（建 Token 源时顺手打上），
/// 拿它当硬条件会让「先建源、后打标记」这条顺序把源变成隐形的。
pub fn first_in_chain(conn: &Connection, chain: &[i64]) -> AppResult<Option<TokenSource>> {
    for folder_id in chain {
        if let Some(source) = list_by_folder(conn, *folder_id)?.into_iter().next() {
            return Ok(Some(source));
        }
    }
    Ok(None)
}

pub fn get(conn: &Connection, id: i64) -> AppResult<TokenSource> {
    let sql = format!("SELECT {SOURCE_COLUMNS} FROM token_source WHERE id = ?1");
    conn.query_row(&sql, [id], map_source).map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::NotFound {
            entity: "token_source",
            id,
        },
        other => AppError::Db(other),
    })
}

pub fn create(
    conn: &Connection,
    workspace_id: i64,
    folder_id: i64,
    name: &str,
    fetch_request_id: i64,
) -> AppResult<TokenSource> {
    let now = now_iso();
    conn.execute(
        "INSERT INTO token_source (workspace_id, folder_id, name, fetch_request_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        rusqlite::params![workspace_id, folder_id, name, fetch_request_id, now],
    )?;
    get(conn, conn.last_insert_rowid())
}

/// 全量保存。
///
/// `folder_id` 要指向被标记为 `is_token_source` 的文件夹 ——
/// 这里**只校验文件夹存在**，不校验那个标记：标记是 UI 层的引导，
/// 不该做成数据库层的硬约束（否则先建 Token 源再打标记的流程会失败）。
pub fn save(conn: &Connection, source: &TokenSource) -> AppResult<TokenSource> {
    let affected = conn.execute(
        "UPDATE token_source SET
           folder_id          = ?2,
           name               = ?3,
           fetch_request_id   = ?4,
           access_path        = ?5,
           refresh_path       = ?6,
           expires_in_path    = ?7,
           expires_in_unit    = ?8,
           refresh_request_id = ?9,
           auto_refresh       = ?10,
           skew_seconds       = ?11,
           updated_at         = ?12
         WHERE id = ?1",
        rusqlite::params![
            source.id,
            source.folder_id,
            source.name,
            source.fetch_request_id,
            source.access_path,
            source.refresh_path,
            source.expires_in_path,
            source.expires_in_unit.as_db_str(),
            source.refresh_request_id,
            bool_to_int(source.auto_refresh),
            source.skew_seconds,
            now_iso(),
        ],
    )?;

    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "token_source",
            id: source.id,
        });
    }
    get(conn, source.id)
}

/// 硬删除。缓存行由 `token_cache` 的外键级联带走。
pub fn delete(conn: &Connection, id: i64) -> AppResult<()> {
    let affected = conn.execute("DELETE FROM token_source WHERE id = ?1", [id])?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "token_source",
            id,
        });
    }
    Ok(())
}

// ============================================================
// 缓存
// ============================================================

pub fn get_cache(conn: &Connection, token_source_id: i64) -> AppResult<Option<TokenCache>> {
    let sql = format!("SELECT {CACHE_COLUMNS} FROM token_cache WHERE token_source_id = ?1");
    match conn.query_row(&sql, [token_source_id], map_cache) {
        Ok(c) => Ok(Some(c)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// 写入 / 覆盖缓存。每个 Token 源只保留一行（唯一索引兜底）。
pub fn upsert_cache(
    conn: &Connection,
    token_source_id: i64,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: Option<&str>,
    raw_response: Option<&str>,
) -> AppResult<TokenCache> {
    conn.execute(
        "INSERT INTO token_cache
           (token_source_id, access_token, refresh_token, expires_at, raw_response, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(token_source_id) DO UPDATE SET
           access_token  = excluded.access_token,
           refresh_token = excluded.refresh_token,
           expires_at    = excluded.expires_at,
           raw_response  = excluded.raw_response,
           updated_at    = excluded.updated_at",
        rusqlite::params![
            token_source_id,
            access_token,
            refresh_token,
            expires_at,
            raw_response,
            now_iso(),
        ],
    )?;

    get_cache(conn, token_source_id)?.ok_or_else(|| {
        AppError::Internal(format!(
            "token_cache row vanished after upsert (source #{token_source_id})"
        ))
    })
}

pub fn clear_cache(conn: &Connection, token_source_id: i64) -> AppResult<bool> {
    let affected = conn.execute(
        "DELETE FROM token_cache WHERE token_source_id = ?1",
        [token_source_id],
    )?;
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrate;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().expect("db");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate::run(&mut conn).expect("migrate");
        conn.execute_batch(
            "INSERT INTO workspace (id, name, created_at, updated_at)
               VALUES (1, 'w', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO folder (id, workspace_id, name, is_token_source, created_at, updated_at)
               VALUES (1, 1, 'auth', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO request (id, folder_id, workspace_id, name, created_at, updated_at)
               VALUES (1, 1, 1, 'login', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn create_uses_design_defaults() {
        let conn = db();
        let s = create(&conn, 1, 1, "iPSA-OAuth", 1).unwrap();
        assert_eq!(s.access_path, "$.access_token");
        assert_eq!(s.refresh_path, "$.refresh_token");
        assert_eq!(s.expires_in_path, "$.expires_in");
        assert_eq!(s.expires_in_unit, ExpiryUnit::Seconds);
        assert_eq!(s.skew_seconds, 30);
        assert!(s.auto_refresh);
        assert_eq!(s.refresh_request_id, None);
    }

    #[test]
    fn folder_and_name_must_be_unique() {
        let conn = db();
        create(&conn, 1, 1, "dup", 1).unwrap();
        assert!(
            create(&conn, 1, 1, "dup", 1).is_err(),
            "unique index on (folder_id, name) must reject duplicates"
        );
    }

    #[test]
    fn save_updates_every_configurable_field() {
        let conn = db();
        let mut s = create(&conn, 1, 1, "oauth", 1).unwrap();
        s.access_path = "$.data.token".into();
        s.expires_in_unit = ExpiryUnit::Millis;
        s.skew_seconds = 120;
        s.auto_refresh = false;

        let saved = save(&conn, &s).unwrap();
        assert_eq!(saved.access_path, "$.data.token");
        assert_eq!(saved.expires_in_unit, ExpiryUnit::Millis);
        assert_eq!(saved.skew_seconds, 120);
        assert!(!saved.auto_refresh);
    }

    #[test]
    fn cache_upsert_keeps_a_single_row() {
        let conn = db();
        let s = create(&conn, 1, 1, "oauth", 1).unwrap();

        upsert_cache(
            &conn,
            s.id,
            "token-a",
            Some("r1"),
            Some("2026-01-01T00:00:00.000Z"),
            None,
        )
        .unwrap();
        let updated =
            upsert_cache(&conn, s.id, "token-b", None, None, Some("{\"raw\":true}")).unwrap();

        assert_eq!(updated.access_token, "token-b");
        assert_eq!(
            updated.refresh_token, None,
            "refresh token is overwritten, not merged"
        );
        assert_eq!(updated.expires_at, None);
        assert_eq!(updated.raw_response.as_deref(), Some("{\"raw\":true}"));

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        let _ = "token-a";
    }

    #[test]
    fn cache_is_scoped_per_source() {
        let conn = db();
        let a = create(&conn, 1, 1, "a", 1).unwrap();
        let b = create(&conn, 1, 1, "b", 1).unwrap();

        upsert_cache(&conn, a.id, "ta", None, None, None).unwrap();
        upsert_cache(&conn, b.id, "tb", None, None, None).unwrap();

        assert_eq!(get_cache(&conn, a.id).unwrap().unwrap().access_token, "ta");
        assert_eq!(get_cache(&conn, b.id).unwrap().unwrap().access_token, "tb");
        assert!(clear_cache(&conn, a.id).unwrap());
        assert!(get_cache(&conn, a.id).unwrap().is_none());
        assert_eq!(get_cache(&conn, b.id).unwrap().unwrap().access_token, "tb");
    }

    #[test]
    fn delete_source_cascades_to_cache() {
        let conn = db();
        let s = create(&conn, 1, 1, "oauth", 1).unwrap();
        upsert_cache(&conn, s.id, "t", None, None, None).unwrap();

        delete(&conn, s.id).unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn missing_source_reports_not_found() {
        let conn = db();
        assert!(matches!(
            get(&conn, 42).unwrap_err(),
            AppError::NotFound {
                entity: "token_source",
                ..
            }
        ));
        assert!(matches!(
            delete(&conn, 42).unwrap_err(),
            AppError::NotFound { .. }
        ));
    }
}
