//! 测试用的临时库与播种助手。
//!
//! 只给需要**连接池**的测试用（单条连接的 SQL 单测直接 `Connection::open_in_memory`
//! 就够了，见各仓储文件）。这里存在的理由是：Token 刷新与请求流水线都要走
//! `with_conn`，而那条路径要的是 `SqlitePool`。
//!
//! 用临时**文件**而不是内存库：`create_memory_pool()` 只有一条连接，
//! 会把并发路径压成串行 —— 而单飞锁那条测试要测的正是并发。

use std::path::PathBuf;

use crate::storage::migrate;
use crate::storage::pool::{create_pool, SqlitePool};

/// 「析构即删除」的临时目录句柄。
///
/// ## ⚠️ 它必须声明在 `pool` **之后**
///
/// 结构体字段的 drop 顺序就是**声明顺序**，而 `Drop::drop` 跑在字段析构**之前**。
/// 所以如果把它放在 `pool` 前面（或者把删除写在 `impl Drop for TestDb` 里），
/// 删除发生时连接池里的连接还开着 —— Windows 删不掉被打开的文件，
/// 结果是**每个测试都在 TEMP 里留一个删不掉的目录**，跑几次就攒一堆。
/// 这个坑真踩过：`impl Drop` 里的 `remove_dir_all` 一直静默失败（`let _ =` 吞了）。
///
/// 顺带一提，失败时也不再吞掉：临时目录没删掉本身不是测试失败，
/// 但它是「这里有点问题」的信号，值得在 `--nocapture` 下看得见。
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.0) {
            println!("[reqly-test] 临时目录没能删掉 {}: {e}", self.0.display());
        }
    }
}

pub struct TestDb {
    // ⚠️ 顺序有意义：`pool` 必须在 `_dir` 之前，见 `TempDir` 的说明。
    // 字段名的下划线是给 `dead_code` 看的：它只被**析构**读，没有任何代码读过它。
    pub pool: SqlitePool,
    _dir: TempDir,
}

impl TestDb {
    /// `tag` 只用于目录名，方便失败时在 TEMP 里认出是哪个测试留下的。
    pub fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "reqly-test-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        // 建之前先扫掉同名残留（上一次跑崩了会留下）
        let _ = std::fs::remove_dir_all(&dir);
        let pool = create_pool(&dir.join("reqly.db")).expect("建池");

        {
            let mut conn = pool.get().expect("连接");
            migrate::run(&mut conn).expect("迁移");
        }

        Self {
            pool,
            _dir: TempDir(dir),
        }
    }

    /// 跑一段 SQL。多数表的 `created_at` / `updated_at` 是 NOT NULL 且无默认值，
    /// 所以播种语句里必须自己带上时间戳。
    pub fn exec(&self, sql: &str) {
        let conn = self.pool.get().expect("连接");
        conn.execute_batch(sql).expect("播种 SQL");
    }

    pub fn count(&self, table: &str) -> i64 {
        let conn = self.pool.get().expect("连接");
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .expect("计数")
    }
}

/// 一个工作区 + 一个文件夹 + 三个请求 + 一份 Token 源。
///
/// 固定 id：request 1 = 取 token 的那个（`fetch_url` 由调用方给），
/// request 2 / 3 = 两个业务接口；`token_source` id=1 归属 **request 2**。
/// 「一个接口一份」是 008 之后的硬约束，所以第二个业务接口（request 3）
/// 留给"另一条来源"的测试自己去建。
///
/// 全部固定 id，让测试里的断言可以直接写死 —— 播种代码本身不该成为被测对象。
pub fn seed_token_workspace(db: &TestDb, fetch_url: &str) {
    db.exec(&format!(
        "INSERT INTO workspace (id, name, created_at, updated_at)
           VALUES (1, '', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
         INSERT INTO folder (id, workspace_id, name, is_token_source, created_at, updated_at)
           VALUES (1, 1, 'auth', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
         INSERT INTO request (id, folder_id, workspace_id, name, method, url, created_at, updated_at)
           VALUES (1, 1, 1, 'login', 'POST', '{fetch_url}',
                   '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                  (2, 1, 1, 'biz-1', 'GET', 'http://127.0.0.1:1/biz-1',
                   '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                  (3, 1, 1, 'biz-2', 'GET', 'http://127.0.0.1:1/biz-2',
                   '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
         INSERT INTO token_source
           (id, workspace_id, request_id, name, fetch_request_id, access_path, refresh_path,
            expires_in_path, expires_in_unit, auto_refresh, skew_seconds, created_at, updated_at)
           VALUES (1, 1, 2, 'oauth', 1, '$.access_token', '$.refresh_token',
                   '$.expires_in', 'seconds', 1, 30,
                   '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');"
    ));
}
