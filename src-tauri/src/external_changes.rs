//! 外部进程改了库 → 通知前端刷新（design.md §8.1）。
//!
//! ## 补的是哪一条欠账
//!
//! §8.1 写着「MCP 进程直接读写同一个 SQLite 文件……桌面 App 打开时通过文件监听
//! 刷新 UI」，但**文件监听一直没做**。后果很具体：AI 通过 MCP 或 CLI 新建了一个
//! 请求，前端不做任何事 —— 用户得自己切一下板块、或者重启 App，才看得到。
//! 从用户视角看，那和「AI 说的东西其实没生效」没区别。
//!
//! ## 为什么盯 `PRAGMA data_version` 而不是文件 mtime
//!
//! * **mtime 更粗**。Windows 上文件时间的精度可能到 15ms 量级，连着两次写只看到
//!   一次变化；而且 mtime 只说明「文件动过」，改的是哪一类数据完全不知道。
//! * **`data_version` 是 SQLite 自己维护的计数器**，直接回答「这个库有没有被
//!   **别的连接**提交过」。读它不需要碰磁盘，一秒一次的开销可以忽略。
//!
//! ## ⚠️ 两条只有实测才知道的规矩
//!
//! 这个 pragma 的**文档保证**只有一条：**同一条连接**的两次读之间，别的连接提交过
//! 就不同。围着它我们实测了两件事（同一进程、同一个库文件，迁移后从另一个池插一行）：
//!
//! **① 全新连接第一次读到的不作数。** 它拿到的既不是提交前的值、也不是提交后的值，
//! 而是 pager 播种 + WAL 头部状态凑出来的一个数。同一份代码我们跑出过
//! 「固定连接 2→3、新连接 2」和「固定连接 3、新连接 2」两种组合 —— 也就是说
//! 新连接读到什么**没有任何保证**，取决于时序。
//!
//! 推论有两条，都写进了实现：
//!   * **[`watch`] 必须固定握一条连接**，不能每轮 `pool.get()` 换一条。换连接等于拿
//!     「播种值」和「已建立状态的值」混着比，表现是**随机漏报**（这次提交被整个吞掉，
//!     `previous` 也不前进）。代价是池里长期占一条连接（`MAX_POOL_SIZE` 是 8），
//!     可以忽略。
//!   * **基线要预热一次再取。** `watch` 先空读一次丢掉结果，第二条读才是基线；
//!     否则第一次「变更」有可能撞上播种值而被吃掉。
//!
//! **② 这条规矩不建议写成断言。** 我们试着把「新连接读到旧值」钉成测试，结果它自己
//! 在 8 次连跑里红了一次 —— 断言 SQLite 的播种细节等于断言一个未定义行为。
//! 所以上面两条以文字记录，只把**文档保证的那条**写成测试
//! （`a_pinned_connection_sees_an_external_commit`）。
//!
//! ## 一个诚实的取舍
//!
//! `data_version` 认的是「别的连接」，而**桌面端自己写库走的是池里另一条连接**，
//! 所以自己人的写也会让它变。这里不为此再加一层「本地写计数器」—— 那要把计数塞进
//! 存储层的每条写路径，代价远大于收益。多出来的后果只是偶尔一次冗余刷新
//! （前端那边是幂等的），而漏掉外部改动才是真问题。
//!
//! ## 为什么失败只是把这个功能关掉
//!
//! 刷新是体验，不是功能。拿不到连接（池满 10 秒都没空）、读不到 `data_version`
//! （库文件刚被删、盘突然没了）只说明盯不了，不该把启动流程拖垮。

use std::time::Duration;

use tauri::{AppHandle, Emitter};

use reqly_core::error::AppResult;
use reqly_core::storage::pool::SqlitePool;

/// 事件名。**必须与前端 `src/lib/ipc.ts` 的 `DB_CHANGED_EVENT` 逐字一致** ——
/// 它是两边的唯一约定，写错了不会有任何报错，只是界面永远不刷新。
pub const DB_CHANGED_EVENT: &str = "reqly://db-changed";

/// 轮询间隔。
///
/// 1 秒是「人眼感觉不到延迟」与「一秒钟最多一次无谓查询」之间的折中。
/// 再密一点就开始为「本地工具」这个定位做无意义的开销了。
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// 起一个后台任务盯着库。
///
/// 只吃 [`SqlitePool`] 而不是整个 `AppState`：要的只有池，而池是 r2d2 的 `Pool`
/// （内部 `Arc`），clone 一份几乎不要钱 —— 这样也不必为了让任务拿到 `'static`
/// 的 `AppState` 去改所有命令的 `State<'_>` 签名。
///
/// 用 `spawn_blocking` 而不是 `spawn`：循环体是同步的（`std::thread::sleep` +
/// 同步 SQLite 读），塞进异步任务只会占住一条工作线程。
pub fn spawn(app: AppHandle, pool: SqlitePool) {
    // 直接当语句调用、不接 `JoinHandle`：`spawn_blocking` 是**立刻**起任务的，
    // 丢掉句柄只是「不等它、也不管它」，循环会照跑到进程结束。
    // （别写成 `let _ = …`，clippy 的 `let_underscore_future` 会当成「忘了 poll」。）
    tauri::async_runtime::spawn_blocking(move || watch(app, pool));
}

/// 同步循环，整段跑在阻塞线程池上。
fn watch(app: AppHandle, pool: SqlitePool) {
    // ⚠️ 连接只取这一次，之后一直握着 —— 理由见模块头「①」。
    // 不要把它改成循环里每轮 `pool.get()` / `with_conn`：那会随机漏报。
    let conn = match pool.get() {
        Ok(conn) => conn,
        Err(error) => {
            eprintln!("[reqly] 外部改动监听未启用（取不到连接）：{error}");
            return;
        }
    };

    // 预热：全新连接的**第一次**读拿到的是播种值，拿它当基线有可能把开始的第一次
    // 变更吃掉（模块头「①」）。多花一次 PRAGMA，换基线是「已建立状态」的值。
    let _ = read_data_version(&conn);

    let mut previous = match read_data_version(&conn) {
        Ok(version) => version,
        Err(error) => {
            eprintln!("[reqly] 外部改动监听未启用（读不到 data_version）：{error}");
            return;
        }
    };

    loop {
        std::thread::sleep(POLL_INTERVAL);

        // 读失败就跳过这一轮：库被短暂占住是正常现象，下一轮自然会读到新值，
        // 不必因此把整个监听停掉。
        let Ok(version) = read_data_version(&conn) else {
            continue;
        };
        if version == previous {
            continue;
        }
        previous = version;

        // 发不出去（窗口正在关）不算错误，忽略返回值即可
        let _ = app.emit(DB_CHANGED_EVENT, ());
    }
}

/// 读一次 `PRAGMA data_version`。
///
/// 收 `&Connection` 而不是池：让「同一条连接」这件事在类型上就没法绕过 ——
/// 传池进来就会有人再写一次 `pool.get()`。
fn read_data_version(conn: &rusqlite::Connection) -> AppResult<i64> {
    Ok(conn.query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use reqly_core::storage::migrate;
    use reqly_core::storage::pool::create_pool;

    /// 「析构即删」的临时目录。与 `core::storage::testkit` 同一个理由：
    /// 池必须先于目录析构 —— Windows 删不掉还开着的文件，
    /// 顺序反了的表现是每个测试都在 TEMP 里留一个删不掉的目录。
    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 建一个跑完迁移的真实库文件。用文件而不是内存库：
    /// 这个功能的前提就是「多个连接 / 多个进程看同一个文件」。
    fn temp_db(tag: &str) -> (SqlitePool, TempDir) {
        let dir = std::env::temp_dir().join(format!(
            "reqly-extchange-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");

        let pool = create_pool(&dir.join("reqly.db")).expect("建池");
        {
            let mut conn = pool.get().expect("连接");
            migrate::run(&mut conn).expect("迁移");
        }
        (pool, TempDir(dir))
    }

    /// 写一行，返回**保持活着的池** —— 调用方必须把返回值绑到一个变量上。
    ///
    /// 为什么不能丢：池被丢掉时最后一条连接会关闭，SQLite 会顺手做一次 checkpoint
    /// 并删掉 WAL。那一下会把主库头部的 change counter 刷新，把「固定连接看得见变更」
    /// 与「新连接看不见」这两件事一起搅浑。
    ///
    /// 也**不能**从被测的池里再取一条连接来写：r2d2 很可能把刚还回去的那条原样发回，
    /// 于是读和写是同一条连接 —— 而自己提交自己是不会变的，测试就会随机假红。
    /// 另开一个池才等价于「另一个进程」。
    fn write_one_row_from_another_process(path: &std::path::Path) -> SqlitePool {
        let external = create_pool(path).expect("外部池");
        external
            .get()
            .expect("外部连接")
            .execute_batch(
                "INSERT INTO workspace (id, name, created_at, updated_at)
                   VALUES (9, 'ext', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');",
            )
            .expect("写一行");
        external
    }

    /// 这是整个功能成立的前提：**握着同一条连接，别的连接提交之后 `data_version` 会变。**
    ///
    /// 如果它不变，监听会「安静地不工作」—— 代码全对、日志全无、界面永远不刷新。
    /// 所以这条测试比它看起来重要。
    #[test]
    fn a_pinned_connection_sees_an_external_commit() {
        let (pool, dir) = temp_db("commit");
        let path = dir.0.join("reqly.db");

        // 固定握一条池连接来读 —— 这正是 `watch` 的做法
        let reader = pool.get().expect("读连接");
        // 预热，理由同 `watch`：基线必须是「已建立状态」的读，不能是播种值
        let _ = read_data_version(&reader);
        let before = read_data_version(&reader).expect("读 data_version");

        let _external = write_one_row_from_another_process(&path);

        let after = read_data_version(&reader).expect("再读一次");
        assert_ne!(
            after, before,
            "固定连接必须看到别的连接的提交，否则这个监听永远不会触发"
        );
    }

    /// 反向控制：没人写就不该报变化。
    ///
    /// 少了它，一个「每次都硬发事件」的实现也能让上面那条测试通过 ——
    /// 而那种实现会把界面按轮询频率反复重拉。
    #[test]
    fn data_version_is_stable_when_nothing_writes() {
        let (pool, _dir) = temp_db("stable");
        let conn = pool.get().expect("连接");
        let _ = read_data_version(&conn);
        let first = read_data_version(&conn).expect("读一次");
        let second = read_data_version(&conn).expect("读两次");
        assert_eq!(first, second, "没有写入却报了变化，界面会被无谓地反复刷新");
    }

    /// 事件名是 Rust 与 TS 之间**唯一**的约定，写错了不会有任何报错
    /// （只会「界面永远不刷新」）。所以这里直接去读前端源码把字面量对一遍。
    ///
    /// 用 `CARGO_MANIFEST_DIR` 定位而不是相对路径：测试的工作目录不保证是 crate 根。
    #[test]
    fn the_event_name_matches_the_frontend_constant() {
        let ipc_ts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src/lib/ipc.ts");
        let source = std::fs::read_to_string(&ipc_ts)
            .unwrap_or_else(|e| panic!("读不到 {}: {e}", ipc_ts.display()));

        assert!(
            source.contains(&format!("\"{DB_CHANGED_EVENT}\"")),
            "前端 src/lib/ipc.ts 里找不到事件名 `{DB_CHANGED_EVENT}` —— \
             两边的字面量已经不一致，界面不会刷新但不会有任何报错"
        );
    }
}
