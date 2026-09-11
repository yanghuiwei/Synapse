//! P1 验收：把「应用启动」这件事在进程内走一遍。
//!
//! 对照 `docs/plan.md` §3 的验收标准：
//!   1. 空库首次启动自动建表
//!   2. 重复启动不报错
//!   3. `set_setting('theme','dark')` 后重启保持
//!   4. 迁移是幂等的
//!
//! 为什么用**真实文件库**而不是 `:memory:`：验收要覆盖的是
//! 「建目录 → 建池 → WAL 落盘 → 关掉再打开」这条真实路径，
//! 内存库把文件系统那一半全绕过去了（而 WAL 恰恰是文件系统的行为）。

use std::path::PathBuf;

use reqly_lib::storage::{migrate, pool, repo};

/// 每个测试用独立目录，避免并行跑测试时互相踩。
fn temp_db_path(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("reqly-startup-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("reqly.db")
}

/// `sqlite_master` 里的表名都建出来了
fn assert_tables_exist(conn: &rusqlite::Connection) {
    const TABLES: [&str; 12] = [
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
        "setting",
        // 迁移机制自己的元数据表，由 ensure_version_table() 在脚本执行前建
        "schema_migration",
    ];

    for table in TABLES {
        let found: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |r| r.get(0),
            )
            .expect("query sqlite_master");
        assert_eq!(found, 1, "表 `{table}` 应该已经建出来");
    }
}

/// 删掉测试用的目录。
///
/// ⚠️ Windows 上只要池里还有活着的连接，`remove_dir_all` 就会失败（os error 32，文件被占用）。
/// 所以调用方必须先让池出作用域；这里再对残余句柄做几次短暂重试。
fn cleanup(db_path: &std::path::Path) {
    let Some(dir) = db_path.parent() else { return };
    for _ in 0..10 {
        if std::fs::remove_dir_all(dir).is_ok() || !dir.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}

/// 标准启动流程：建池 → 跑迁移 → 保证有默认工作区
fn boot(db_path: &std::path::Path) -> Result<pool::SqlitePool, String> {
    let pool = pool::create_pool(db_path).map_err(|e| e.to_string())?;
    let mut conn = pool.get().map_err(|e| e.to_string())?;
    migrate::run(&mut conn).map_err(|e| e.to_string())?;
    repo::workspace::ensure_default(&conn).map_err(|e| e.to_string())?;
    Ok(pool)
}

#[test]
fn first_launch_creates_schema_at_latest_version() {
    let db_path = temp_db_path("fresh");

    let pool = pool::create_pool(&db_path).expect("建池");
    assert!(db_path.exists(), "数据库文件应该被创建");

    let mut conn = pool.get().expect("取连接");
    let applied = migrate::run(&mut conn).expect("首次迁移");

    assert!(!applied.is_empty(), "空库首次启动必须执行迁移");
    assert_eq!(
        migrate::current_version(&conn).expect("当前版本"),
        migrate::latest_version(),
        "迁移完应该正好是最新版本"
    );
    assert_tables_exist(&conn);

    // 空库不该有任何业务数据
    let requests: i64 = conn
        .query_row("SELECT COUNT(*) FROM request", [], |r| r.get(0))
        .expect("count");
    assert_eq!(requests, 0);

    drop(conn);
    drop(pool);
    cleanup(&db_path);
}

/// 建表 → 建默认工作区 → 关掉 → 再启动一次，全程不许报错
#[test]
fn restarting_is_clean_and_idempotent() {
    let db_path = temp_db_path("restart");

    // ---- 第一次启动 ----
    let workspace_id = {
        let pool = boot(&db_path).expect("首次启动");
        let conn = pool.get().expect("取连接");
        let ws = repo::workspace::ensure_default(&conn).expect("默认工作区");

        // 只剩一个工作区（ensure_default 不能每次启动都塞一个新的）
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM workspace", [], |r| r.get(0))
            .expect("count");
        assert_eq!(count, 1, "默认工作区只能有一个");
        ws.id
    };

    // ---- 第二次启动（模拟重启） ----
    // 池必须在这段作用域内出栈：Windows 上句柄没释放就删不掉目录
    {
        let pool = boot(&db_path).expect("第二次启动不应报错");
        let mut conn = pool.get().expect("取连接");

        let applied = migrate::run(&mut conn).expect("重跑迁移不应报错");
        assert!(applied.is_empty(), "已是最新版本时不应再执行迁移");

        let ws = repo::workspace::ensure_default(&conn).expect("默认工作区");
        assert_eq!(ws.id, workspace_id, "重启后应复用同一个工作区");
    }

    cleanup(&db_path);
}

/// 验收第 3 条：设置写进去，关库再打开还在
#[test]
fn setting_survives_restart() {
    let db_path = temp_db_path("settings");

    {
        let pool = boot(&db_path).expect("首次启动");
        let conn = pool.get().expect("取连接");

        assert_eq!(
            repo::setting::get(&conn, "theme").expect("读设置"),
            None,
            "新库里没有 theme"
        );
        repo::setting::set(&conn, "theme", "dark").expect("写设置");
    }

    {
        let pool = boot(&db_path).expect("重启");
        let conn = pool.get().expect("取连接");
        assert_eq!(
            repo::setting::get(&conn, "theme")
                .expect("读设置")
                .as_deref(),
            Some("dark"),
            "重启后 theme 必须还是 dark"
        );
    }

    cleanup(&db_path);
}

/// 迁移幂等：连续跑三次，只有第一次有动作，版本始终等于最新
#[test]
fn migrations_are_idempotent() {
    let db_path = temp_db_path("idempotent");

    let pool = pool::create_pool(&db_path).expect("建池");
    let mut conn = pool.get().expect("取连接");

    let first = migrate::run(&mut conn).expect("第一次迁移");
    assert!(!first.is_empty());

    for round in 2..=3 {
        let again = migrate::run(&mut conn).expect("再次迁移");
        assert!(again.is_empty(), "第 {round} 次迁移不该有动作");
    }

    assert_eq!(
        migrate::current_version(&conn).expect("当前版本"),
        migrate::latest_version()
    );

    drop(conn);
    drop(pool);
    cleanup(&db_path);
}

/// 校验和：跑过迁移的库里，`schema_migration` 的条数 == 已应用版本数，
/// 且没有重复版本（重复说明迁移被跑了两遍）
#[test]
fn schema_migration_table_has_no_duplicate_versions() {
    let db_path = temp_db_path("versions");

    // 池与连接都在这个作用域里出栈，之后才删目录
    {
        let pool = boot(&db_path).expect("启动");
        let mut conn = pool.get().expect("取连接");
        migrate::run(&mut conn).expect("再跑一次");

        let applied = migrate::applied_versions(&conn).expect("已应用版本");
        let mut unique = applied.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(applied.len(), unique.len(), "版本号出现重复");

        let max: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migration",
                [],
                |r| r.get(0),
            )
            .expect("max version");
        assert_eq!(max, migrate::latest_version());
    }

    cleanup(&db_path);
}
