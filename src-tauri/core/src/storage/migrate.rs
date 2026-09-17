//! 迁移执行器。
//!
//! 设计要点：
//! * SQL 用 `include_str!` **编译期嵌入**二进制 —— 用户机器上只有一个 exe，
//!   不能依赖运行时去磁盘找 .sql 文件。
//! * **幂等**：已应用的版本不会再跑；`schema_migration` 表记录已应用版本。
//! * 每个迁移**单独一个事务**：中途失败时该版本整体回滚，不会留下半张表。
//!   （所以迁移脚本里**不要**自己写 BEGIN/COMMIT。）
//! * 只做向前迁移，不做回滚。开发期改 schema 就直接加 002、003……

use rusqlite::Connection;

use crate::error::{AppError, AppResult};
use crate::storage::now_iso;

pub struct Migration {
    pub version: i64,
    /// 仅用于日志
    pub name: &'static str,
    pub sql: &'static str,
}

/// 全部迁移，**按 version 升序**。
///
/// 加新迁移：新建 `migrations/00N_xxx.sql`，在下面追加一项，version 递增。
/// 已发布的迁移**不要再改**（用户库里已经跑过了，改了也不会重跑）。
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../../migrations/001_init.sql"),
    },
    Migration {
        version: 2,
        name: "folder_auth",
        sql: include_str!("../../migrations/002_folder_auth.sql"),
    },
    Migration {
        version: 3,
        name: "body_by_type",
        sql: include_str!("../../migrations/003_body_by_type.sql"),
    },
    Migration {
        version: 4,
        name: "system_folder",
        sql: include_str!("../../migrations/004_system_folder.sql"),
    },
    Migration {
        version: 5,
        name: "form_fields",
        sql: include_str!("../../migrations/005_form_fields.sql"),
    },
];

/// 代码里定义的最高版本号
pub fn latest_version() -> i64 {
    MIGRATIONS.last().map(|m| m.version).unwrap_or(0)
}

fn version_table_exists(conn: &Connection) -> AppResult<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migration'",
        [],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

fn ensure_version_table(conn: &Connection) -> AppResult<()> {
    // 这张表不属于任何迁移 —— 它是迁移机制自己的元数据，
    // 所以用 IF NOT EXISTS 在跑任何迁移之前先建好。
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migration (
           version    INTEGER PRIMARY KEY,
           applied_at TEXT NOT NULL
         );",
    )?;
    Ok(())
}

/// 已应用的版本号（升序）。表还不存在时返回空。
pub fn applied_versions(conn: &Connection) -> AppResult<Vec<i64>> {
    if !version_table_exists(conn)? {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare("SELECT version FROM schema_migration ORDER BY version ASC")?;
    let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 当前库的 schema 版本（已应用的最大版本号）
pub fn current_version(conn: &Connection) -> AppResult<i64> {
    Ok(applied_versions(conn)?.last().copied().unwrap_or(0))
}

/// 执行所有未应用的迁移，返回**本次实际应用**的版本号列表。
///
/// 重复调用是安全的：第二次会返回空 vec。
pub fn run(conn: &mut Connection) -> AppResult<Vec<i64>> {
    ensure_version_table(conn)?;

    let applied = applied_versions(conn)?;
    let mut newly_applied = Vec::new();

    for migration in MIGRATIONS {
        if applied.contains(&migration.version) {
            continue;
        }

        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)
            .map_err(|e| AppError::Migration {
                version: migration.version,
                reason: format!("{}: {e}", migration.name),
            })?;
        tx.execute(
            "INSERT INTO schema_migration (version, applied_at) VALUES (?1, ?2)",
            rusqlite::params![migration.version, now_iso()],
        )?;
        tx.commit()?;

        newly_applied.push(migration.version);
    }

    Ok(newly_applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_memory() -> Connection {
        Connection::open_in_memory().expect("in-memory db")
    }

    /// 把 WAL 之外的 PRAGMA 也打开，尽量贴近运行时环境
    fn open_memory_with_pragmas() -> Connection {
        let conn = open_memory();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn
    }

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    #[test]
    fn applies_all_migrations_on_empty_db() {
        let mut conn = open_memory();
        let applied = run(&mut conn).expect("migrate");
        assert_eq!(
            applied,
            vec![1, 2, 3, 4, 5],
            "fresh db should apply every migration"
        );
        assert_eq!(current_version(&conn).unwrap(), latest_version());
    }

    #[test]
    fn is_idempotent() {
        let mut conn = open_memory();
        run(&mut conn).expect("first run");
        let second = run(&mut conn).expect("second run");
        assert!(second.is_empty(), "second run must apply nothing");

        let third = run(&mut conn).expect("third run");
        assert!(third.is_empty(), "third run must apply nothing");

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migration", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 5, "version table must have one row per migration");
    }

    /// schema 完整性：design.md §3.2 的表一张都不能少。
    /// 漏表的后果是运行到那个功能才炸，所以用测试兜住。
    #[test]
    fn creates_every_expected_table() {
        let mut conn = open_memory();
        run(&mut conn).unwrap();
        let names = table_names(&conn);

        let expected = [
            "workspace",
            "folder",
            "request",
            "environment",
            "env_var",
            "token_source",
            "token_cache",
            "history",
            "loadtest_run",
            "loadtest_sample",
            "setting", // [补1] 设计稿未定义
            "schema_migration",
        ];
        for t in expected {
            assert!(names.iter().any(|n| n == t), "missing table: {t}");
        }
    }

    #[test]
    fn creates_expected_indexes() {
        let mut conn = open_memory();
        run(&mut conn).unwrap();

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND name LIKE 'idx_%'")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        for idx in [
            "idx_folder_parent",
            "idx_folder_system",
            "idx_request_folder",
            "idx_request_name",
            "idx_history_time",
            "idx_history_req",
            "idx_sample_run",
            "idx_token_cache_source",
        ] {
            assert!(names.iter().any(|n| n == idx), "missing index: {idx}");
        }
    }

    /// 002 是 `ALTER TABLE`，为「继承父级」提供数据源。
    /// 存量库里必须能补上这一列且默认值是 `{}`（= 未配置）。
    #[test]
    fn folder_auth_column_is_added_with_empty_default() {
        let mut conn = open_memory_with_pragmas();
        run(&mut conn).unwrap();

        conn.execute_batch(
            "INSERT INTO workspace (id, name, created_at, updated_at)
               VALUES (1, 'w', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO folder (id, workspace_id, name, created_at, updated_at)
               VALUES (1, 1, 'f', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');",
        )
        .unwrap();

        let auth: String = conn
            .query_row("SELECT auth FROM folder WHERE id = 1", [], |r| r.get(0))
            .expect("folder.auth must exist after migration 2");
        assert_eq!(auth, "{}");
    }

    /// 003 也是 `ALTER TABLE`：存量库里请求体那两列必须补得上，
    /// 且默认值是**空快照** —— 前端读到 `{}` 会用 `body` 兜底填当前类型，
    /// 表现与升级前一致（不是「内容丢了」）。
    #[test]
    fn body_columns_are_added_with_empty_defaults() {
        let mut conn = open_memory_with_pragmas();
        run(&mut conn).unwrap();

        conn.execute_batch(
            "INSERT INTO workspace (id, name, created_at, updated_at)
               VALUES (1, 'w', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO request (id, workspace_id, name, body, created_at, updated_at)
               VALUES (1, 1, 'login', 'a=1&b=2', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');",
        )
        .unwrap();

        let (by_type, rows): (String, String) = conn
            .query_row(
                "SELECT body_by_type, body_rows FROM request WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("request.body_by_type / body_rows must exist after migration 3");
        assert_eq!(by_type, "{}");
        assert_eq!(rows, "[]");
    }

    /// 004：存量库补上 `is_system` 且默认 0（别把老分类变成不可删的），
    /// 同时那个**部分索引**只在 `is_system = 1` 上要求唯一 ——
    /// 普通分类再多也不受影响，系统分类每个工作区只能有一个。
    #[test]
    fn system_folder_column_and_partial_index() {
        let mut conn = open_memory_with_pragmas();
        run(&mut conn).unwrap();

        conn.execute_batch(
            "INSERT INTO workspace (id, name, created_at, updated_at)
               VALUES (1, 'w', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                      (2, 'w2', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO folder (id, workspace_id, name, created_at, updated_at)
               VALUES (1, 1, 'legacy', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');",
        )
        .unwrap();

        let is_system: i64 = conn
            .query_row("SELECT is_system FROM folder WHERE id = 1", [], |r| {
                r.get(0)
            })
            .expect("folder.is_system must exist after migration 4");
        assert_eq!(is_system, 0, "存量分类必须默认成普通分类");

        // 同一个工作区可以有很多普通分类
        conn.execute_batch(
            "INSERT INTO folder (workspace_id, name, created_at, updated_at)
               VALUES (1, 'a', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                      (1, 'b', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');",
        )
        .expect("普通分类不该受部分索引影响");

        // 但系统分类只能有一个
        conn.execute(
            "INSERT INTO folder (workspace_id, name, is_system, created_at, updated_at)
             VALUES (1, 'sys', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        )
        .expect("第一个系统分类应当插得进去");

        let dup = conn.execute(
            "INSERT INTO folder (workspace_id, name, is_system, created_at, updated_at)
             VALUES (1, 'sys2', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        );
        assert!(dup.is_err(), "同一个工作区不该允许第二个系统分类");

        // 换一个工作区则是另一回事
        conn.execute(
            "INSERT INTO folder (workspace_id, name, is_system, created_at, updated_at)
             VALUES (2, 'sys', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        )
        .expect("系统分类的唯一性是按工作区算的");
    }

    /// 外键必须真的生效（PRAGMA foreign_keys 是连接级开关，容易忘）
    #[test]
    fn foreign_keys_are_enforced() {
        let mut conn = open_memory_with_pragmas();
        run(&mut conn).unwrap();

        // folder.workspace_id 指向不存在的 workspace(999) → 必须被拒
        let err = conn.execute(
            "INSERT INTO folder (workspace_id, name, created_at, updated_at)
             VALUES (999, 'orphan', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        );
        assert!(err.is_err(), "FK violation should be rejected");
    }

    /// 删除行为：[补2] 请求被彻底删除时，指向它的 token_source 应级联消失，
    /// 而不是抛外键错误（设计稿漏了 ON DELETE 行为）。
    #[test]
    fn token_source_cascades_when_fetch_request_is_purged() {
        let mut conn = open_memory_with_pragmas();
        run(&mut conn).unwrap();

        conn.execute_batch(
            "INSERT INTO workspace (id, name, created_at, updated_at)
               VALUES (1, 'w', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO folder (id, workspace_id, name, created_at, updated_at)
               VALUES (1, 1, 'f', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO request (id, folder_id, workspace_id, name, created_at, updated_at)
               VALUES (1, 1, 1, 'login', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO token_source (id, workspace_id, folder_id, name, fetch_request_id, created_at, updated_at)
               VALUES (1, 1, 1, 'oauth', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             DELETE FROM request WHERE id = 1;",
        )
        .expect("purge should cascade, not fail on FK");

        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_source", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "token_source should have been cascaded away");
    }

    /// 历史表刻意不加外键：工作区被删后历史仍要能读（design.md §3.2）
    #[test]
    fn history_survives_missing_workspace() {
        let mut conn = open_memory_with_pragmas();
        run(&mut conn).unwrap();

        conn.execute(
            "INSERT INTO history (workspace_id, request_name, snapshot, executed_at)
             VALUES (12345, 'ghost', '{}', '2026-01-01T00:00:00.000Z')",
            [],
        )
        .expect("history must accept a workspace_id with no matching row");
    }
}
