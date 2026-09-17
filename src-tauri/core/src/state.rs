//! 进程级共享状态。
//!
//! 它从桌面 crate 搬到这里，是因为三个入口**必须共用同一份语义**：
//! 「客户端连接池只有一份」「Token 航班锁只有一份」「最近一次响应只有一份」——
//! 这些约束不会因为触发方是 GUI 还是 AI 而改变。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::error::{AppError, AppResult};
use crate::http::{ClientCache, InflightRegistry, TokenRefresher};
use crate::loadtest::LoadtestRegistry;
use crate::storage::pool::{self, SqlitePool};
use crate::storage::{migrate, repo};

/// 全局状态。
///
/// 桌面端由 `#[tauri::command]` 通过 `State<'_, AppState>` 取用；
/// MCP 与 CLI 侧由自己持有的 `Arc<AppState>` 取用 —— 两边是同一个结构，
/// 只是所有权来源不同。
pub struct AppState {
    pub pool: SqlitePool,
    /// 保留绝对路径：设置页要显示、出问题时用户要能自己去打开
    pub db_path: PathBuf,
    /// 「文件型」响应（导出 Excel / PDF / 图片）的临时落点。
    ///
    /// 为什么必须有这么一个目录：那些响应的字节只存在于执行的那一刻，
    /// 而「保存到文件」是用户之后才点的动作。不先落一份，用户点保存时
    /// 手里什么都没有 —— 除了被内联上限截断的那 8MB。
    pub blob_dir: PathBuf,
    /// 按 `RequestSettings` 复用的 HTTP 客户端。**必须全局只有一份** ——
    /// 它的价值在于内部的连接池（keep-alive），每次命令都新建就白搭了。
    pub clients: ClientCache,
    /// 进行中的请求登记表，`cancel_request` 靠它找回要打断的那次执行。
    pub inflight: InflightRegistry,
    /// Token 源的「航班锁」（design.md §4.3）。也**必须全局只有一份** ——
    /// 每个命令各建一个的话，「并发 5 个请求只刷一次」立刻退化成刷 5 次。
    pub tokens: TokenRefresher,
    /// 最近一次成功响应的 JSON，`{{resp.$.path}}` 的来源。
    ///
    /// 进程级、最近一次 —— design.md §4.2 写的是「最近一次响应（会话内）」，
    /// 没有按请求/按文件夹细分。细分要回答「同一个请求自己的上一次算不算」，
    /// 那是 P5 的事；先把语义钉成最简单的一种。
    ///
    /// 存 `Arc<Value>` 而不是 `Value`：读取时克隆一个 `Arc` 就能把锁放下，
    /// 不必抱着 `MutexGuard` 穿过 await（`std` 的 guard 不是 `Send`）。
    pub last_response: Mutex<Option<Arc<serde_json::Value>>>,
    /// 「当前正在跑的那一轮压测」的占坑与取消（P6）。
    ///
    /// 与 `inflight` 分开而不是合表：那一张是按 `runId` 索引、面向「一次请求」的，
    /// 而压测同时只允许一轮、且守卫要能搬进 `tokio::spawn` 出来的引擎任务
    /// （见 `loadtest::registry` 的模块说明）。
    pub loadtests: LoadtestRegistry,
}

impl AppState {
    /// 从数据目录建起整套运行时：建池 → 跑迁移 → 保证默认工作区。
    ///
    /// 桌面端、MCP、CLI 共用这一条启动路径。区别只在路径从哪来 —— 桌面端由
    /// `AppHandle` 反推（`app_data_dir`），另外两个入口直接吃 `--db` 参数，
    /// 所以这里收的是**已经解析好的**路径。
    ///
    /// 日志一律走 **stderr**：MCP 的 stdio 通道把 stdout 当作 JSON-RPC 流，
    /// 往 stdout 写一行日志就会污染协议、让客户端握手直接失败。
    /// CLI 同理 —— stdout 是数据，stderr 才是给人看的。
    pub fn new(db_path: PathBuf) -> AppResult<Self> {
        let pool = pool::create_pool(&db_path)?;

        {
            let mut conn = pool.get()?;

            let applied = migrate::run(&mut conn)?;
            if applied.is_empty() {
                eprintln!(
                    "[reqly] schema up to date (v{})",
                    migrate::current_version(&conn)?
                );
            } else {
                eprintln!("[reqly] applied migrations: {applied:?}");
            }

            let ws = repo::workspace::ensure_default(&conn)?;
            // 系统分类「token刷新」：Token 来源固定配在它下面。**必须在这里补一句** ——
            // 004 迁移只加了列，存量库里那个分类是启动时才建出来的，
            // 而三个入口（桌面 / MCP / CLI）共用这条路径，漏了就会有人打开一个
            // 「没有分类可配 Token」的库。
            let sys = repo::folder::ensure_system_folder(&conn, ws.id)?;
            eprintln!(
                "[reqly] db ready: {} (workspace #{}, system folder #{})",
                db_path.display(),
                ws.id,
                sys.id
            );
        }

        Ok(Self {
            pool,
            // 「文件型」响应的临时区：与库同一个目录，跟着库走。
            // 不做「启动即清空」—— 桌面端与 MCP 可能同时开着，先启动的那个
            // 会把另一个刚落的文件删掉，表现为「保存按钮点了报文件不存在」。
            // 增长由写入侧的 `prune_blobs` 按数量兜住（见 `http::execute`）。
            blob_dir: db_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("blobs"),
            db_path,
            clients: ClientCache::new(),
            inflight: InflightRegistry::new(),
            tokens: TokenRefresher::new(),
            last_response: Mutex::new(None),
            loadtests: LoadtestRegistry::new(),
        })
    }
}

/// SQLite 的 WAL 兄弟文件后缀。
///
/// SQLite 的约定是**在库文件名后面直接接后缀**，不是换扩展名 ——
/// `reqly.db` 的 WAL 叫 `reqly.db-wal`，不叫 `reqly.wal`。
/// 抄错不会报错，只是「WAL 里那部分已提交、还没 checkpoint 的数据没跟着搬」，
/// 表现是搬迁后少掉最近几条记录。
const SQLITE_SIDECARS: [&str; 2] = ["-wal", "-shm"];

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// 把旧 identifier 目录下的库一次性搬到新目录。返回 `Ok(true)` 表示确实搬了。
///
/// **为什么需要它**：`identifier` 同时是 Tauri 的 app data 目录名
/// （Windows 上是 `%APPDATA%\<identifier>\`），所以改 identifier 等于换库目录 ——
/// 新目录是空的，老用户那条 `reqly.db` 还躺在旧目录里，界面上「什么都没有」，
/// 而且用户看不出来发生了什么，只会以为数据丢了。
///
/// 四条刻意的选择：
/// 1. **只在新目录还没有库时才搬**。已经有库说明用户已经在新目录里干过活了，
///    这时把旧库盖上去就是拿旧数据覆盖新数据 —— 宁可什么都不做。
/// 2. **复制而不是移动，旧目录保留**。移动中途失败会两边都不完整；
///    复制最坏只是留一份旧的，用户还能自己捞回来。
/// 3. **`-wal` / `-shm` 一起搬**。WAL 里可能压着已提交但还没 checkpoint 的数据。
/// 4. **先落临时名、最后才 rename 成真名**。「新目录有没有库」是靠
///    `new_db.exists()` 判断的，所以要让这个判断尽量晚地变真 —— 中途失败（磁盘满、
///    权限不够）时新目录仍然是空的，下次启动会重试，而不是卡在一个半成品上。
///    代价是失败时可能留下 `.tmp` 残留，下一次拷贝会直接覆盖它。
pub fn migrate_legacy_db(legacy_db: &Path, new_db: &Path) -> AppResult<bool> {
    if new_db.exists() || !legacy_db.exists() {
        return Ok(false);
    }

    let parent = new_db.parent().ok_or_else(|| {
        AppError::Internal(format!("db path has no parent: {}", new_db.display()))
    })?;
    std::fs::create_dir_all(parent)
        .map_err(|e| AppError::Internal(format!("cannot create {}: {e}", parent.display())))?;

    let staging = with_suffix(new_db, ".tmp");
    std::fs::copy(legacy_db, &staging).map_err(|e| {
        AppError::Internal(format!(
            "cannot copy legacy db {} -> {}: {e}",
            legacy_db.display(),
            staging.display()
        ))
    })?;

    // 兄弟文件先跟着临时名走，等主库 rename 成功后再逐个改名。
    let mut sidecars = Vec::new();
    for suffix in SQLITE_SIDECARS {
        let from = with_suffix(legacy_db, suffix);
        if !from.exists() {
            continue;
        }
        let to = with_suffix(&staging, suffix);
        std::fs::copy(&from, &to)
            .map_err(|e| AppError::Internal(format!("cannot copy {}: {e}", from.display())))?;
        sidecars.push((to, with_suffix(new_db, suffix)));
    }

    std::fs::rename(&staging, new_db).map_err(|e| {
        AppError::Internal(format!(
            "cannot move {} -> {}: {e}",
            staging.display(),
            new_db.display()
        ))
    })?;
    for (from, to) in sidecars {
        std::fs::rename(&from, &to)
            .map_err(|e| AppError::Internal(format!("cannot move {}: {e}", from.display())))?;
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一次性临时目录，`Drop` 时整个删掉。
    /// 名字里带 pid + 纳秒：测试并行跑，同名目录会互相踩。
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be past the unix epoch")
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "reqly-migrate-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).expect("create scratch dir");
            Self(dir)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// 造一个「旧目录」并写进内容，返回 (旧库路径, 新库路径)。
        fn arrange(tag: &str, legacy_body: &[u8]) -> (Self, PathBuf, PathBuf) {
            let scratch = Self::new(tag);
            let legacy = scratch.path("old/reqly.db");
            let new = scratch.path("new/reqly.db");
            std::fs::create_dir_all(legacy.parent().expect("has parent")).expect("mkdir old");
            std::fs::write(&legacy, legacy_body).expect("write legacy db");
            (scratch, legacy, new)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn legacy_db_and_its_wal_sidecar_are_copied() {
        let (_scratch, legacy, new) = Scratch::arrange("copy", b"main-db");
        // WAL 名字是 `reqly.db-wal`，不是 `reqly.wal` —— 这里钉住这个拼法
        std::fs::write(with_suffix(&legacy, "-wal"), b"wal-bytes").expect("write wal");

        assert!(migrate_legacy_db(&legacy, &new).expect("migration should succeed"));

        assert_eq!(std::fs::read(&new).expect("new db exists"), b"main-db");
        assert_eq!(
            std::fs::read(with_suffix(&new, "-wal")).expect("new wal exists"),
            b"wal-bytes"
        );
        // 旧目录保留：复制而不是移动，用户还有一份可以自己捞
        assert!(legacy.exists(), "旧库不该被删掉");
    }

    #[test]
    fn migration_is_a_no_op_once_the_new_db_exists() {
        let (_scratch, legacy, new) = Scratch::arrange("idempotent", b"old");
        std::fs::create_dir_all(new.parent().expect("has parent")).expect("mkdir new");
        std::fs::write(&new, b"already-mine").expect("write new db");

        // 已经有库 → 既不上报「搬了」，也不覆盖用户在新目录里的数据
        assert!(!migrate_legacy_db(&legacy, &new).expect("migration should succeed"));
        assert_eq!(std::fs::read(&new).expect("new db exists"), b"already-mine");
    }

    #[test]
    fn nothing_happens_when_there_is_no_legacy_db() {
        let scratch = Scratch::new("absent");
        let legacy = scratch.path("old/reqly.db");
        let new = scratch.path("new/reqly.db");

        assert!(!migrate_legacy_db(&legacy, &new).expect("migration should succeed"));
        assert!(!new.exists(), "没有旧库时不该凭空造一个出来");
    }

    #[test]
    fn a_failed_copy_leaves_the_target_absent_so_it_can_be_retried() {
        // 让「新目录」的父路径是个文件 → `create_dir_all` 失败。
        // 关键是失败之后 `new_db.exists()` 仍然是 false，下次启动会重试。
        let (scratch, legacy, _) = Scratch::arrange("retry", b"old");
        let blocked = scratch.path("blocker");
        std::fs::write(&blocked, b"i am a file, not a dir").expect("write blocker");
        let new = blocked.join("reqly.db");

        assert!(migrate_legacy_db(&legacy, &new).is_err());
        assert!(!new.exists());
    }
}
