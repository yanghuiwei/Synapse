//! 进程级共享状态。
//!
//! 它从桌面 crate 搬到这里，是因为三个入口**必须共用同一份语义**：
//! 「客户端连接池只有一份」「Token 航班锁只有一份」「最近一次响应只有一份」——
//! 这些约束不会因为触发方是 GUI 还是 AI 而改变。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::error::AppResult;
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
            eprintln!(
                "[reqly] db ready: {} (workspace #{})",
                db_path.display(),
                ws.id
            );
        }

        Ok(Self {
            pool,
            db_path,
            clients: ClientCache::new(),
            inflight: InflightRegistry::new(),
            tokens: TokenRefresher::new(),
            last_response: Mutex::new(None),
            loadtests: LoadtestRegistry::new(),
        })
    }
}
