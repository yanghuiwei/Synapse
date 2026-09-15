//! 应用设置仓储（键值）。
//!
//! 存 theme / locale / mcp 开关这类全局键值。值统一是 TEXT，
//! 需要布尔或数字时由读取方解析 —— 避免 schema 里堆一堆类型字段。

use rusqlite::{Connection, Row};

use crate::error::AppResult;
use crate::storage::models::Setting;
use crate::storage::now_iso;

const COLUMNS: &str = "key, value, updated_at";

fn map_row(row: &Row<'_>) -> rusqlite::Result<Setting> {
    Ok(Setting {
        key: row.get("key")?,
        value: row.get("value")?,
        updated_at: row.get("updated_at")?,
    })
}

/// 读一个键。不存在返回 `None`（不是错误 —— 首次启动读 theme 就是这种情况）。
pub fn get(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    let sql = format!("SELECT {COLUMNS} FROM setting WHERE key = ?1");
    match conn.query_row(&sql, [key], map_row) {
        Ok(s) => Ok(Some(s.value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// 写一个键（upsert）。返回写入后的时间戳。
pub fn set(conn: &Connection, key: &str, value: &str) -> AppResult<String> {
    let now = now_iso();
    conn.execute(
        "INSERT INTO setting (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        rusqlite::params![key, value, now],
    )?;
    Ok(now)
}

/// 全部设置。前端启动时一次拉完，省得为每个键跑一趟 IPC。
pub fn get_all(conn: &Connection) -> AppResult<Vec<Setting>> {
    let sql = format!("SELECT {COLUMNS} FROM setting ORDER BY key ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], map_row)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 删除一个键。返回是否真的删掉了一行。
pub fn delete(conn: &Connection, key: &str) -> AppResult<bool> {
    let affected = conn.execute("DELETE FROM setting WHERE key = ?1", [key])?;
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrate;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().expect("db");
        migrate::run(&mut conn).expect("migrate");
        conn
    }

    #[test]
    fn missing_key_is_none_not_error() {
        let conn = db();
        assert_eq!(get(&conn, "theme").unwrap(), None);
    }

    #[test]
    fn set_then_get_roundtrip() {
        let conn = db();
        set(&conn, "theme", "dark").unwrap();
        assert_eq!(get(&conn, "theme").unwrap().as_deref(), Some("dark"));
    }

    #[test]
    fn set_is_upsert_not_duplicate_insert() {
        let conn = db();
        set(&conn, "theme", "dark").unwrap();
        set(&conn, "theme", "light").unwrap();
        set(&conn, "theme", "system").unwrap();

        assert_eq!(get(&conn, "theme").unwrap().as_deref(), Some("system"));
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM setting WHERE key = 'theme'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1, "upsert must not create duplicate keys");
    }

    /// 验收项：set_setting('theme','dark') 后重启仍要保持
    #[test]
    fn value_survives_reopen() {
        let dir = std::env::temp_dir().join(format!("reqly-setting-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("reqly.db");
        let _ = std::fs::remove_file(&path);

        {
            let mut conn = Connection::open(&path).unwrap();
            migrate::run(&mut conn).unwrap();
            set(&conn, "theme", "dark").unwrap();
        }
        {
            let mut conn = Connection::open(&path).unwrap();
            migrate::run(&mut conn).unwrap();
            assert_eq!(get(&conn, "theme").unwrap().as_deref(), Some("dark"));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_all_is_sorted_and_delete_reports_effect() {
        let conn = db();
        set(&conn, "zeta", "1").unwrap();
        set(&conn, "alpha", "2").unwrap();

        let all = get_all(&conn).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].key, "alpha");
        assert_eq!(all[1].key, "zeta");

        assert!(delete(&conn, "alpha").unwrap());
        assert!(!delete(&conn, "alpha").unwrap(), "second delete is a no-op");
    }
}
