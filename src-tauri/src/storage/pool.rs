//! SQLite 连接池。
//!
//! 用 r2d2 而不是给每个命令开一条连接：SQLite 单文件在 WAL 模式下支持
//! 多读单写，池化可以避免频繁 open/close 的开销（每次都涉及文件句柄 + 页缓存重建）。

use std::path::Path;

use r2d2_sqlite::SqliteConnectionManager;

use crate::error::AppResult;

pub type SqlitePool = r2d2::Pool<SqliteConnectionManager>;
pub type PooledConn = r2d2::PooledConnection<SqliteConnectionManager>;

/// 连接数上限。
///
/// SQLite 的写是串行的（同一时刻只有一个写事务），开太多连接只会让
/// 它们在 `busy_timeout` 上排队，不会提高吞吐。8 条足够覆盖
/// 「UI 查询 + 后台写历史 + 压测采样落盘」的并发面。
const MAX_POOL_SIZE: u32 = 8;

/// 拿连接时最多等多久（秒）。超过就返回错误而不是无限挂住 UI。
const CONNECTION_TIMEOUT_SECS: u64 = 10;

/// 每条连接建立时都会执行的 PRAGMA。
///
/// 为什么要**逐连接**设置：`journal_mode` 是持久化到库文件里的，
/// 但 `foreign_keys` / `busy_timeout` / `synchronous` 都是**连接级**的，
/// 池里每条新连接都要重新设置，否则会出现「有时外键生效有时不生效」的灵异问题。
fn init_connection(conn: &mut rusqlite::Connection) -> rusqlite::Result<()> {
    // WAL 会返回一行（返回设置后的模式），所以必须用 query_row 而不是 execute_batch
    // —— execute_batch 遇到会返回行的语句会报 ExecuteReturnedResults。
    let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;

    conn.execute_batch(
        "PRAGMA foreign_keys = ON;      -- 外键约束默认是关的，必须显式开
         PRAGMA busy_timeout = 5000;    -- 被写锁挡住时等 5s 再报 SQLITE_BUSY
         PRAGMA synchronous = NORMAL;   -- WAL 下的推荐值：崩溃不丢库，性能远好于 FULL
         PRAGMA temp_store = MEMORY;    -- 临时表放内存
         PRAGMA cache_size = -8000;     -- 约 8MB 页缓存（负值 = KB）
        ",
    )?;
    Ok(())
}

/// 建连接池。会自动创建 `db_path` 的父目录。
///
/// ⚠️ WAL 模式**不支持网络驱动器**（SMB/NFS）。数据目录必须落在本地磁盘，
/// 否则 SQLite 会直接报错。
pub fn create_pool(db_path: &Path) -> AppResult<SqlitePool> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let manager = SqliteConnectionManager::file(db_path).with_init(init_connection);

    let pool = r2d2::Pool::builder()
        .max_size(MAX_POOL_SIZE)
        .connection_timeout(std::time::Duration::from_secs(CONNECTION_TIMEOUT_SECS))
        .build(manager)?;

    Ok(pool)
}

/// 建一个只用于测试的内存连接池。
///
/// ⚠️ `:memory:` 的每条连接都是**独立**的库。要让池里多条连接共享同一份内存库，
/// 必须用 `file::memory:?cache=shared` 的 URI 形式并开 `SQLITE_OPEN_URI`。
/// 单测里一般直接用 [`crate::storage::migrate::run`] 配合单条连接，
/// 这个函数只给「需要池」的测试用。
#[cfg(test)]
pub fn create_memory_pool() -> AppResult<SqlitePool> {
    let manager = SqliteConnectionManager::memory().with_init(init_connection);
    let pool = r2d2::Pool::builder().max_size(1).build(manager)?;
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_parent_directory_and_applies_pragmas() {
        let dir = std::env::temp_dir().join(format!("reqly-pool-test-{}", std::process::id()));
        let db_path = dir.join("nested").join("reqly.db");
        let _ = std::fs::remove_dir_all(&dir);

        let pool = create_pool(&db_path).expect("pool");
        assert!(db_path.exists(), "db file should have been created");

        let conn = pool.get().expect("conn");
        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .expect("pragma");
        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .expect("pragma");
        let busy: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .expect("pragma");

        assert_eq!(foreign_keys, 1, "foreign_keys must be ON");
        assert_eq!(journal.to_lowercase(), "wal", "journal_mode must be WAL");
        assert_eq!(busy, 5000, "busy_timeout must be 5000ms");

        drop(conn);
        drop(pool);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
