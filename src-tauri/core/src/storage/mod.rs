//! 存储层：连接池、迁移、数据模型与仓储。
//!
//! 分层约定：
//! * `pool` / `migrate` —— 基础设施
//! * `models` —— 纯数据结构（serde），与前端 `types.ts` 一一对应
//! * `repo` —— 同步的 SQL 封装，函数签名一律 `(&Connection, ...) -> AppResult<T>`
//! * **异步边界只在这一层的 [`with_conn`]** —— 命令层调 `with_conn(pool, |conn| repo::xxx(conn, ...))`，
//!   repo 内部是纯同步代码，既好写单测，也不会把 `spawn_blocking` 撒得到处都是。
//!
//! 为什么不给 repo 函数加 `async`：SQLite 是阻塞 API，包成 async 只是假象。
//! 真正要做的是把它挪到阻塞线程池上，这件事只需在一处发生。

pub mod migrate;
pub mod models;
pub mod pool;
pub mod repo;
pub mod seed;

/// 需要连接池的测试才用它。`cfg(test)` 下才编译，不进产物。
#[cfg(test)]
pub mod testkit;

use chrono::{SecondsFormat, Utc};

use crate::error::{AppError, AppResult};
use pool::SqlitePool;

/// 统一时间戳：ISO8601 带毫秒 + `Z` 后缀，例如 `2026-09-11T06:00:00.000Z`。
///
/// 为什么不用 SQLite 的 `datetime('now')`：它产出 `2026-09-11 06:00:00`（无 T、无时区），
/// 前端 `new Date(...)` 会按**本地时间**解析，跨时区直接错 8 小时。
/// 所有时间戳一律由 Rust 侧生成。
pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// 异步 → 阻塞的唯一边界。
///
/// 在 tokio 的阻塞线程池上执行 `f`，避免 SQLite 的同步 IO 卡住异步运行时
/// （卡住的后果是所有命令一起变慢，而不只是这一条）。
pub async fn with_conn<T, F>(pool: &SqlitePool, f: F) -> AppResult<T>
where
    F: FnOnce(&mut rusqlite::Connection) -> AppResult<T> + Send + 'static,
    T: Send + 'static,
{
    let pool = pool.clone();
    tokio::task::spawn_blocking(move || {
        let mut conn = pool.get()?;
        f(&mut conn)
    })
    .await
    .map_err(|e| AppError::Internal(format!("blocking task failed: {e}")))?
}
