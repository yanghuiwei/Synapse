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
    /// 需要 Rust 侧解析、SQL 写不出来的**一次性**数据变动，与本次迁移同事务执行。
    ///
    /// 两种用法的区别只在 SQL 里：008 只**播种**（SQL 建表，data 铺数据）；
    /// 009 的 SQL 是空的，删除判据同样要解析 URL，整件事都在 data 里。
    ///
    /// 为什么需要它：008 要把「每个接口一份 Token 源」铺出来，而判据要读取 token
    /// 请求的 `scope` —— 那东西藏在请求体里、有三种存法，还要按空白切词与
    /// percent-decode。写进 SQL 要么写不出来（没有 split），要么写出来没人敢改。
    ///
    /// 为什么挂在迁移上而不是应用启动时：迁移天然**只跑一次**，所以
    /// 「用户手工删掉某条 Token 源」不会被下次启动重新长回来。挂在启动路径上就得
    /// 再加一个 `setting` 标记来达到同样的效果。
    pub data: Option<fn(&rusqlite::Connection) -> AppResult<usize>>,
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
        data: None,
    },
    Migration {
        version: 2,
        name: "folder_auth",
        sql: include_str!("../../migrations/002_folder_auth.sql"),
        data: None,
    },
    Migration {
        version: 3,
        name: "body_by_type",
        sql: include_str!("../../migrations/003_body_by_type.sql"),
        data: None,
    },
    Migration {
        version: 4,
        name: "system_folder",
        sql: include_str!("../../migrations/004_system_folder.sql"),
        data: None,
    },
    Migration {
        version: 5,
        name: "form_fields",
        sql: include_str!("../../migrations/005_form_fields.sql"),
        data: None,
    },
    Migration {
        version: 6,
        name: "history_curl",
        sql: include_str!("../../migrations/006_history_curl.sql"),
        data: None,
    },
    Migration {
        version: 7,
        name: "flatten_auth",
        sql: include_str!("../../migrations/007_flatten_auth.sql"),
        data: None,
    },
    Migration {
        version: 8,
        name: "request_scoped_token_source",
        sql: include_str!("../../migrations/008_request_scoped_token_source.sql"),
        // 结构改完立刻铺源：新表是空的，不铺的话 145 个接口会一起显示「还没有配置」
        data: Some(crate::storage::seed::request_token_sources),
    },
    Migration {
        version: 9,
        name: "drop_sources_outside_rule_hosts",
        sql: include_str!("../../migrations/009_drop_sources_outside_rule_hosts.sql"),
        data: Some(crate::storage::seed::drop_sources_outside_rule_hosts),
    },
    Migration {
        version: 10,
        name: "history_response",
        sql: include_str!("../../migrations/010_history_response.sql"),
        data: None,
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

        // 数据变动与它所属的那次迁移**同一个事务**：中途失败则连结构一起回滚，
        // 版本号也不会被登记 —— 下次启动从头再来一遍，不会留下「结构已改、
        // 数据只铺了一半」的中间态。
        if let Some(data) = migration.data {
            data(&tx).map_err(|e| AppError::Migration {
                version: migration.version,
                reason: format!("{}: data step failed: {e}", migration.name),
            })?;
        }

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

    /// 从 `from` 起的所有迁移版本。
    ///
    /// 断言「这个老库补跑了哪些」时用它，**不要写字面量数组**：写死的话
    /// 每加一次迁移都要回来改这里，而漏改的表现是一条一直红的测试 ——
    /// 加 010 时正是这么被抓出来的（两处红色全在这里）。
    fn versions_from(from: i64) -> Vec<i64> {
        MIGRATIONS
            .iter()
            .map(|migration| migration.version)
            .filter(|version| *version >= from)
            .collect()
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

        // 期望值从 `MIGRATIONS` 推导，不写死一串数字：写死的话，每次加迁移都要
        // 记得来这里改一处，而「改的时候顺手把它的意思也改掉」是迟早的事。
        // 版本号本身的正确性由下面那条连续性断言看住。
        let expected: Vec<i64> = MIGRATIONS.iter().map(|m| m.version).collect();
        assert_eq!(applied, expected, "fresh db should apply every migration");
        assert_eq!(
            expected,
            (1..=latest_version()).collect::<Vec<_>>(),
            "version 必须从 1 开始连续递增 —— 跳号会让升级路径上少跑一段"
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
        assert_eq!(
            rows,
            MIGRATIONS.len() as i64,
            "version table must have one row per migration"
        );
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
            // 008 建的三个索引，之前一个都没被钉住
            "idx_token_source_request",
            "idx_token_source_fetch",
            "idx_token_cache_source",
        ] {
            assert!(names.iter().any(|n| n == idx), "missing index: {idx}");
        }
    }

    /// 007 的第三步：两个 auth 列必须都被删干净。
    ///
    /// 这条**取代**了原来那条 `folder_auth_column_is_added_with_empty_default`：
    /// 002 加上这一列、007 又把它删掉，跑完全套迁移之后它注定不存在，
    /// 所以「存量库里必须能补上这一列且默认 `{}`」这个断言的前提已经没了。
    /// 002 真正想保住的东西 —— 老库能平滑升到最新版、`auth` 里的凭据不丢 ——
    /// 由下面那条端到端测试看住。
    #[test]
    fn auth_columns_are_dropped_by_the_flatten_migration() {
        let mut conn = open_memory_with_pragmas();
        run(&mut conn).unwrap();

        for tbl in ["request", "folder"] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = 'auth'",
                    [tbl],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "{tbl}.auth 必须被 007 删掉");
        }
    }

    /// 007 的核心：把「两处真相」合成一处。
    ///
    /// 造一个**已经跑过 001~006** 的库（直接执行那两个 SQL、再把版本号手工登记进
    /// `schema_migration` —— 那正是老用户库里这张表的样子），塞进覆盖整张展开对照表的
    /// 请求，然后让 `run()` 只补跑 007，断言末态的 `headers` / `params`。
    ///
    /// 🔴 001~006 的 SQL 必须**全跑**，不能只跑与 007 相关的那两个：`run()` 会把
    /// 007 / 008 / 009 一起补上，而 008 的播种要读 `body_rows` / `form_fields`
    /// （003 / 005 加出来的列）⇒ 省掉它们，这条升级路径上直接报 `no such column`。
    /// 也就是说「哪几个迁移可以不跑」不是 007 说了算的。
    ///
    /// 版本号仍然只登记到 6：这才是「用户库里那张表」的样子。
    ///
    /// 真库副本上的等价验证见 2026-09-17 工作日志（267 条请求：改动 36 条、
    /// 新增 36 行头、其余 231 条逐字节未变）。这条测试是那个的一次性缩微版。
    #[test]
    fn legacy_auth_is_flattened_into_headers_by_migration_7() {
        let mut conn = open_memory_with_pragmas();
        for sql in [
            include_str!("../../migrations/001_init.sql"),
            include_str!("../../migrations/002_folder_auth.sql"),
            include_str!("../../migrations/003_body_by_type.sql"),
            include_str!("../../migrations/004_system_folder.sql"),
            include_str!("../../migrations/005_form_fields.sql"),
            include_str!("../../migrations/006_history_curl.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch(
            "CREATE TABLE schema_migration (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
             INSERT INTO schema_migration (version, applied_at) VALUES
               (1,'2026-01-01T00:00:00.000Z'), (2,'2026-01-01T00:00:00.000Z'),
               (3,'2026-01-01T00:00:00.000Z'), (4,'2026-01-01T00:00:00.000Z'),
               (5,'2026-01-01T00:00:00.000Z'), (6,'2026-01-01T00:00:00.000Z');",
        )
        .unwrap();

        // 分类 3 配 Bearer；分类 4 显式 none（用来证明它**中止**继承链）
        conn.execute_batch(
            r#"INSERT INTO workspace (id, name, created_at, updated_at)
                 VALUES (1, 'w', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
               INSERT INTO folder (id, workspace_id, name, auth, created_at, updated_at)
                 VALUES (3, 1, 'TS', '{"type":"bearer","token":"{{token.access}}"}',
                         '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                        (4, 1, 'noauth', '{"type":"none"}',
                         '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');"#,
        )
        .unwrap();

        // 一条请求一个身份，注释写明它验的是展开对照表的哪一格
        let cases: [(i64, Option<i64>, &str, &str, &str); 8] = [
            // (id, folder, name, auth, headers)
            (
                10,
                Some(3),
                "inherit-from-folder",
                r#"{"type":"inherit"}"#,
                "[]",
            ),
            (
                11,
                Some(3),
                "keeps-handwritten",
                r#"{"type":"inherit"}"#,
                r#"[{"key":"Authorization","value":"Bearer old","enabled":true}]"#,
            ),
            (
                12,
                Some(4),
                "none-aborts-chain",
                r#"{"type":"inherit"}"#,
                "[]",
            ),
            (13, Some(3), "empty-object-inherits", "{}", "[]"),
            (
                14,
                None,
                "empty-bearer",
                r#"{"type":"bearer","token":""}"#,
                "[]",
            ),
            (
                15,
                None,
                "basic",
                r#"{"type":"basic","username":"u","password":"p"}"#,
                "[]",
            ),
            (
                16,
                None,
                "api-key-in-query",
                r#"{"type":"apiKey","key":"X-Api-Key","value":"K","location":"query"}"#,
                "[]",
            ),
            (
                17,
                None,
                "blank-api-key-is-dropped",
                r#"{"type":"apiKey","key":"","value":"","location":"query"}"#,
                "[]",
            ),
        ];
        for (id, folder, name, auth, headers) in cases {
            conn.execute(
                "INSERT INTO request (id, folder_id, workspace_id, name, auth, headers,
                                      created_at, updated_at)
                 VALUES (?1, ?2, 1, ?3, ?4, ?5, '2026-01-01T00:00:00.000Z',
                         '2026-01-01T00:00:00.000Z')",
                rusqlite::params![id, folder, name, auth, headers],
            )
            .unwrap();
        }

        let applied = run(&mut conn).expect("upgrade");
        assert_eq!(applied, versions_from(7), "老库该补跑 007 起的全部迁移");

        let raw_of = |id: i64| -> String {
            conn.query_row("SELECT headers FROM request WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap()
        };
        let headers_of = |id: i64| -> Vec<(String, String)> {
            serde_json::from_str::<Vec<serde_json::Value>>(&raw_of(id))
                .unwrap()
                .iter()
                .map(|h| {
                    (
                        h["key"].as_str().unwrap_or_default().to_string(),
                        h["value"].as_str().unwrap_or_default().to_string(),
                    )
                })
                .collect()
        };

        // ① 分类继承：物化成一条看得见的头
        assert_eq!(
            headers_of(10),
            vec![(
                "Authorization".to_string(),
                "Bearer {{token.access}}".to_string()
            )]
        );
        // ⚠️ `enabled` 必须是 JSON 布尔。写成 `1` 时 `KeyValueItem.enabled: bool`
        // 会拒绝整条请求 —— 界面表现为「这条请求打不开了」。
        assert!(
            raw_of(10).contains(r#""enabled":true"#),
            "enabled 必须是 JSON 布尔: {}",
            raw_of(10)
        );
        // ② 去重：用户手写的同名头**不**被覆盖（风险 2）
        assert_eq!(
            headers_of(11),
            vec![("Authorization".to_string(), "Bearer old".to_string())]
        );
        // ③ `{"type":"none"}` 中止继承链 ⇒ 一条凭据都不加
        assert!(headers_of(12).is_empty(), "none 必须中止继承");
        // ④ `{}` 与 `{"type":"inherit"}` 等价（旧 from_json 的退化行为）
        assert_eq!(
            headers_of(13),
            vec![(
                "Authorization".to_string(),
                "Bearer {{token.access}}".to_string()
            )]
        );
        // ⑤ 空 token 也要留下**看得见的空值**，而不是什么都不显示
        assert_eq!(
            headers_of(14),
            vec![("Authorization".to_string(), "Bearer ".to_string())]
        );
        // ⑥ Basic 收进 $basic，不需要一种授权类型
        assert_eq!(
            headers_of(15),
            vec![(
                "Authorization".to_string(),
                "{{$basic \"u\" \"p\"}}".to_string()
            )]
        );
        // ⑦ apiKey(location=query) 落进 params，且**不**出现在 headers
        assert!(headers_of(16).is_empty());
        let param16: String = conn
            .query_row("SELECT params FROM request WHERE id = 16", [], |r| r.get(0))
            .unwrap();
        assert!(
            param16.contains(r#""key":"X-Api-Key""#) && param16.contains(r#""value":"K""#),
            "params 里应有 X-Api-Key: K，实际 {param16}"
        );
        // ⑧ 键名为空 ⇒ 两边都不写（照抄旧 query_auth_pair 的 !key.trim().is_empty()）
        assert!(headers_of(17).is_empty());
        let param17: String = conn
            .query_row("SELECT params FROM request WHERE id = 17", [], |r| r.get(0))
            .unwrap();
        assert_eq!(param17, "[]", "空键名不该在 params 里留下东西");

        // ③ 两列都该没了
        for tbl in ["request", "folder"] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = 'auth'",
                    [tbl],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "{tbl}.auth 必须被 007 删掉");
        }
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
    ///
    /// 008 之后 `request_id` 也带 CASCADE：**归属**的那条请求没了，这份配置
    /// 本身就没有意义了（它描述的正是"这个接口怎么拿 token"）。
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
               VALUES (1, 1, 1, 'login', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                      (2, 1, 1, 'me',    '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO token_source (id, workspace_id, request_id, name, fetch_request_id,
                                       created_at, updated_at)
               VALUES (1, 1, 2, 'oauth', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             DELETE FROM request WHERE id = 1;",
        )
        .expect("purge should cascade, not fail on FK");

        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_source", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "token_source should have been cascaded away");

        // 反过来：把**归属**那条请求删掉，配置也该跟着走
        conn.execute_batch(
            "INSERT INTO token_source (id, workspace_id, request_id, name, fetch_request_id,
                                       created_at, updated_at)
               VALUES (2, 1, 2, 'oauth', 2, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             DELETE FROM request WHERE id = 2;",
        )
        .expect("purge should cascade, not fail on FK");

        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_source", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "归属请求被删 ⇒ 配置也该消失");
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

    /// 008 的核心：老的「分类级 / 全工作区一份」Token 源被丢弃，改由 seed 按
    /// host + scope 给每个接口铺一份。
    ///
    /// 这条测试盯三件事，每一件单独看都像"应该没问题"：
    /// 1. **`token_cache` 被显式清空。** 重建 `token_source` 一定会波及缓存
    ///    （`foreign_keys=ON`），所以清理必须是写出来的那一步 —— 见 008 的注释 ①。
    /// 2. **内外网分开**：`localhost` 那条请求没有同 host 的取 token 请求 ⇒ 不建源。
    /// 3. **scope 决定用哪一条**：#101 的 scope 不覆盖 `psaou` ⇒ 落到 #102；
    ///    #200 的 scope 不覆盖 `NewContractAPI` ⇒ 落到 #201。
    #[test]
    fn folder_scoped_source_is_replaced_by_per_request_sources() {
        let mut conn = open_memory_with_pragmas();
        for sql in [
            include_str!("../../migrations/001_init.sql"),
            include_str!("../../migrations/002_folder_auth.sql"),
            include_str!("../../migrations/003_body_by_type.sql"),
            include_str!("../../migrations/004_system_folder.sql"),
            include_str!("../../migrations/005_form_fields.sql"),
            include_str!("../../migrations/006_history_curl.sql"),
            include_str!("../../migrations/007_flatten_auth.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        conn.execute_batch(
            "CREATE TABLE schema_migration (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
             INSERT INTO schema_migration (version, applied_at) VALUES
               (1,'2026-01-01T00:00:00.000Z'), (2,'2026-01-01T00:00:00.000Z'),
               (3,'2026-01-01T00:00:00.000Z'), (4,'2026-01-01T00:00:00.000Z'),
               (5,'2026-01-01T00:00:00.000Z'), (6,'2026-01-01T00:00:00.000Z'),
               (7,'2026-01-01T00:00:00.000Z');",
        )
        .unwrap();

        conn.execute_batch(
            r#"INSERT INTO workspace (id, name, created_at, updated_at)
                 VALUES (1, 'w', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
               INSERT INTO folder (id, workspace_id, name, created_at, updated_at)
                 VALUES (25, 1, 'token刷新', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');

               -- 取 token 请求：名称里的「测试」/「生产」是环境默认候选的标记
               INSERT INTO request (id, workspace_id, name, url, body, created_at, updated_at) VALUES
                 (100, 1, '测试环境拿token',     'http://api.test.example.com/ids/connect/token',
                  'client_id=a&scope=A%20B%20C%20D%20E', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (101, 1, '测试环境拿token (2)', 'http://api.test.example.com/ids/connect/token',
                  'client_id=b' || char(10) || 'scope=A B C', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 -- `psaou` 是给 #2 那类 `/BPM/psaou/…` 用的：没有它，#102 就不是
                 -- 「覆盖集里的候选」，规则 ⑤ 只能退回默认 #101，那条断言也就测不到东西
                 (102, 1, '测试环境拿token (3)', 'http://api.test.example.com/ids/connect/token',
                  'client_id=a&scope=A%20B%20C%20D%20psaou', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (200, 1, '生产拿token',         'http://api.example.com/ids/connect/token',
                  'client_id=c&scope=A%20B%20C%20D%20E', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (201, 1, '拿合同token',         'http://api.example.com/ids/connect/token',
                  'client_id=d&scope=NewContractAPI', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 -- 同一个 host 上确实有取 token 请求，但域名不在白名单里
                 (300, 1, '钉钉获取token',     'https://api.dingtalk.com/v1.0/oauth2/accessToken',
                  'client_id=e&client_secret=f', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');

               -- 业务请求：默认覆盖 / scope 偏离 / 谁都不覆盖 / 另一个 host / 内网 / 回收站
               -- `#6` 是补的：钉钉的 host 上有取 token 请求，但域名不在白名单里 ——
               -- 少了这一条，008 光按「host 相等」配对也能全绿（真库上就是这么漏的）
               INSERT INTO request (id, workspace_id, name, url, deleted_at, created_at, updated_at) VALUES
                 (1, 1, '工时查询', 'http://api.test.example.com/PMPFIN/TimeSheetApi/v1/x',
                  NULL, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (2, 1, '表单查询', 'http://api.test.example.com/BPM/psaou/v1/y',
                  NULL, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (3, 1, '没人覆盖的服务', 'http://api.test.example.com/PSA/NobodyCovers/v1/z',
                  NULL, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (4, 1, '获取报价', 'http://api.example.com/PMPFIN/NewContractAPI/v1/get',
                  NULL, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (5, 1, '本地服务', 'http://localhost:5000/RMS/Api/x',
                  NULL, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (6, 1, '机器人单聊', 'https://api.dingtalk.com/v1.0/robot/oToMessages/batchSend',
                  NULL, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (7, 1, '回收站里的', 'http://api.test.example.com/PMPFIN/TimeSheetApi/v1/x',
                  '2026-01-02T00:00:00.000Z', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');

               -- 老的分类级 Token 源 + 一份**必须被显式删掉**的缓存
               INSERT INTO token_source (id, workspace_id, folder_id, name, fetch_request_id,
                                         created_at, updated_at)
                 VALUES (9, 1, 25, 'Token 刷新', 200, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
               INSERT INTO token_cache (id, token_source_id, access_token, updated_at)
                 VALUES (1, 9, 'KEEP-ME', '2026-01-01T00:00:00.000Z');"#,
        )
        .unwrap();

        let applied = run(&mut conn).expect("upgrade");
        assert_eq!(applied, versions_from(8), "老库该补跑 008 起的全部迁移");

        // ① 老的分类级行没了；旧缓存也随之有意地清干净
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM token_source ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(!ids.contains(&9), "分类级的老行必须被丢弃");

        // 为什么「清空」是对的目标：新表里 source id 从 1 重新长起
        // （seed 铺 145 行就是 1~145）。旧缓存行留着就会指向另一个接口的新源。
        let caches: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            caches, 0,
            "旧缓存必须被显式删掉 —— 留着会张冠李戴，靠级联删则是静默的"
        );

        // ② 每条命中规则的业务请求各得到一份源，且指向 scope 覆盖它的那条取 token 请求
        let fetch_of = |req: i64| -> Option<i64> {
            conn.query_row(
                "SELECT fetch_request_id FROM token_source WHERE request_id = ?1",
                [req],
                |r| r.get(0),
            )
            .ok()
        };
        assert_eq!(fetch_of(1), Some(101), "默认候选覆盖 ⇒ 用默认");
        assert_eq!(fetch_of(2), Some(102), "默认不含 psaou ⇒ 覆盖集里词最少的");
        assert_eq!(fetch_of(3), Some(101), "谁都不覆盖 ⇒ 退回默认");
        assert_eq!(fetch_of(4), Some(201), "生产默认不含 NewContractAPI ⇒ 偏离");
        assert_eq!(fetch_of(5), None, "localhost 没有同 host 的取 token 请求");
        assert_eq!(
            fetch_of(6),
            None,
            "钉钉上**有**同 host 的取 token 请求，但那个域名不在白名单里"
        );
        assert_eq!(fetch_of(7), None, "回收站里的请求不铺");
        for fetch in [100, 101, 102, 200, 201, 300] {
            assert_eq!(
                fetch_of(fetch),
                None,
                "#{fetch} 自己是取 token 请求，不该配源"
            );
        }

        // ③ 播种幂等：重复执行不产生第二行
        let again = crate::storage::seed::request_token_sources(&conn).unwrap();
        assert_eq!(again, 0, "重复播种应当一条都不新建");
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_source", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 4);
    }

    /// 判据闸门：**同一个 host** 上有取 token 请求，但域名不在白名单里 ⇒ 不铺源。
    ///
    /// 这条是补上去的。上面那份 fixture 里没有这种形状 —— 内网 IP / localhost
    /// 是「host 不在白名单上、同时**也没有**同 host 的取 token 请求」，
    /// 于是 008 光按「host 相等」配对也能全绿；真库上却因此多铺了 19 条
    /// （ipsapro 14 + 钉钉 5，见 009 的注释）。
    #[test]
    fn non_rule_hosts_get_no_source_even_with_a_same_host_token_request() {
        let mut conn = open_memory_with_pragmas();
        run(&mut conn).unwrap();

        conn.execute_batch(
            r#"INSERT INTO workspace (id, name, created_at, updated_at)
                 VALUES (1, 'w', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
               INSERT INTO request (id, workspace_id, name, url, created_at, updated_at) VALUES
                 (1, 1, 'ipsapro 业务',  'https://app.example.com/RMS/Api/x',
                  '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (2, 1, '业务回写',      'https://app.example.com/iCan/bk/getToken',
                  '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (3, 1, '钉钉业务',      'https://api.dingtalk.com/v1.0/robot/oToMessages/batchSend',
                  '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (4, 1, '钉钉获取token', 'https://api.dingtalk.com/v1.0/oauth2/accessToken',
                  '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (5, 1, '生产业务',      'http://api.example.com/PMPFIN/TimeSheetApi/v1/x',
                  '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (6, 1, '生产拿token',   'http://api.example.com/ids/connect/token',
                  '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');"#,
        )
        .expect("fixture");

        let created = crate::storage::seed::request_token_sources(&conn).unwrap();
        assert_eq!(created, 1, "只有规则域名上那条业务请求该得到源");

        let owners: Vec<i64> = conn
            .prepare("SELECT request_id FROM token_source ORDER BY request_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(owners, vec![5], "ipsapro 与钉钉那些一条都不该有");
    }

    /// 009 的收尾：把铺到规则域名之外的源收回来。
    ///
    /// 直接调那个函数，不重放整套迁移 —— 008 的判据已经修好了，从最新版出发
    /// **造不出**那种脏数据，能造的只有手工插几条。
    ///
    /// 同时钉住「只删该删的」：规则域名上那条源、以及它名下的缓存都不能被误伤。
    #[test]
    fn stale_sources_outside_the_rule_hosts_are_dropped() {
        let mut conn = open_memory_with_pragmas();
        run(&mut conn).unwrap();

        conn.execute_batch(
            r#"INSERT INTO workspace (id, name, created_at, updated_at)
                 VALUES (1, 'w', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
               INSERT INTO request (id, workspace_id, name, url, created_at, updated_at) VALUES
                 (1, 1, 'ipsapro 业务',  'https://app.example.com/RMS/Api/x',
                  '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (2, 1, '钉钉业务',      'https://api.dingtalk.com/v1.0/robot/oToMessages/batchSend',
                  '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (3, 1, '生产业务',      'http://api.example.com/PMPFIN/TimeSheetApi/v1/x',
                  '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (4, 1, '生产拿token',   'http://api.example.com/ids/connect/token',
                  '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (5, 1, '业务回写',      'https://app.example.com/iCan/bk/getToken',
                  '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (6, 1, '钉钉获取token', 'https://api.dingtalk.com/v1.0/oauth2/accessToken',
                  '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
               -- 三行源：前两行是旧判据才会铺出来的（业务与取 token 请求同 host，
               -- 但域名不在白名单上），第三行是合法的
               INSERT INTO token_source (id, workspace_id, request_id, name, fetch_request_id,
                                         created_at, updated_at) VALUES
                 (1, 1, 1, 'ipsapro 业务', 5, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (2, 1, 2, '钉钉业务',     6, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                 (3, 1, 3, '生产业务',     4, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
               INSERT INTO token_cache (id, token_source_id, access_token, updated_at) VALUES
                 (1, 1, 'STALE', '2026-01-01T00:00:00.000Z'),
                 (2, 2, 'STALE', '2026-01-01T00:00:00.000Z'),
                 (3, 3, 'KEEP',  '2026-01-01T00:00:00.000Z');"#,
        )
        .expect("fixture");

        let removed = crate::storage::seed::drop_sources_outside_rule_hosts(&conn).unwrap();
        assert_eq!(removed, 2, "ipsapro 与钉钉那两条源必须被收回");

        let left: Vec<i64> = conn
            .prepare("SELECT request_id FROM token_source ORDER BY request_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(left, vec![3], "规则域名上的源必须原样活着");

        let cached: Vec<String> = conn
            .prepare("SELECT access_token FROM token_cache ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            cached,
            vec!["KEEP"],
            "被删那两条源名下的缓存随外键级联走，合法那条的缓存不能被误伤"
        );

        // 幂等：再跑一次没有可删的了
        assert_eq!(
            crate::storage::seed::drop_sources_outside_rule_hosts(&conn).unwrap(),
            0
        );
    }
}
