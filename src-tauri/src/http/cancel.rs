//! 进行中请求的登记表 —— 「取消」功能的落点。
//!
//! ## 为什么需要它
//!
//! 一个卡住的请求（对端不响应、大文件慢慢下）会让界面一直转圈，
//! 「取消」是这类工具最常用的救命按钮。而 Tauri 命令一旦跑起来，
//! 前端没有任何手段把它停下来 —— 必须由后端维护一张「谁正在跑」的表，
//! 让 `cancel_request` 拿到对应请求的信号。
//!
//! ## 为什么用 `CancellationToken` 而不是 `AtomicBool`
//!
//! `AtomicBool` 只能被**轮询**，而真正会卡住的地方恰恰是 `await`：
//! 「等 TCP 连接」「等响应头」「等 body」。轮询在这些点上到不了。
//! `CancellationToken` 是一个能被 `tokio::select!` 竞争的 future ——
//! 信号一发出，请求会在下一个 await 点被 drop，连接随之关闭。
//! 也就是说取消是**真的把请求打断**，不是「结果丢掉但连接还挂着」。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

/// 一次进行中请求的登记项。
///
/// 用 `Arc` 包一层是为了让 guard 能判断「表里那一条还是不是我」——
/// 见 [`InflightGuard::drop`]。
struct Entry {
    token: CancellationToken,
}

pub struct InflightRegistry {
    entries: Mutex<HashMap<String, Arc<Entry>>>,
}

impl Default for InflightRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl InflightRegistry {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// 登记一次执行，返回 RAII 句柄。句柄 Drop 时自动注销，
    /// 所以清理**不依赖调用方记得做什么**（`?` 提前返回、future 被 drop 都能兜住）。
    pub fn register(&self, id: impl Into<String>) -> InflightGuard<'_> {
        let id = id.into();
        let entry = Arc::new(Entry {
            token: CancellationToken::new(),
        });
        lock(&self.entries).insert(id.clone(), Arc::clone(&entry));
        InflightGuard {
            registry: self,
            id,
            entry,
        }
    }

    /// 取消指定执行。返回 `false` 表示它已经结束了（或 id 从来不存在）。
    ///
    /// 这不是错误：用户点取消时请求恰好刚回完，是很常见的事，
    /// 前端拿到 `false` 就当作「本来就没在跑」处理，不要弹错误提示。
    pub fn cancel(&self, id: &str) -> bool {
        match lock(&self.entries).get(id) {
            Some(entry) => {
                entry.token.cancel();
                true
            }
            None => false,
        }
    }

    /// 当前有几个在跑。仅供测试与诊断。
    #[cfg(test)]
    pub fn count(&self) -> usize {
        lock(&self.entries).len()
    }
}

/// 进行中请求的 RAII 句柄。**必须持有到请求结束为止**（绑到局部变量上即可），
/// 中途丢掉会让这张表以为请求已经结束了。
pub struct InflightGuard<'a> {
    registry: &'a InflightRegistry,
    id: String,
    entry: Arc<Entry>,
}

impl InflightGuard<'_> {
    /// 交给执行器的取消信号
    pub fn token(&self) -> &CancellationToken {
        &self.entry.token
    }
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        let mut entries = lock(&self.registry.entries);
        // 只在「表里那一条确实是我登记的」时才删。
        // 同一个 id 被登记两次（前端 bug，或以后做「重发」复用了 id）时，
        // 先结束的那次不该把后一次的登记顺手删掉 —— 那会让后一次彻底无法取消。
        let is_mine = entries
            .get(&self.id)
            .is_some_and(|current| Arc::ptr_eq(current, &self.entry));
        if is_mine {
            entries.remove(&self.id);
        }
    }
}

/// `Mutex` 中毒说明之前持锁的线程 panic 了。这张表里只是一堆取消信号，
/// 没有「写坏一半」的概念，接着用就行。
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_reaches_the_registered_token() {
        let registry = InflightRegistry::new();
        let guard = registry.register("run-1");

        assert!(!guard.token().is_cancelled());
        assert!(registry.cancel("run-1"), "刚登记的 id 应当能取消");
        assert!(guard.token().is_cancelled(), "信号要真的传到 token 上");
    }

    #[test]
    fn cancelling_an_unknown_id_reports_false_instead_of_erroring() {
        let registry = InflightRegistry::new();
        assert!(
            !registry.cancel("nobody"),
            "没这个 id 不是错误，只是没取消到"
        );
    }

    #[test]
    fn guard_unregisters_on_drop() {
        let registry = InflightRegistry::new();
        {
            let _guard = registry.register("run-1");
            assert_eq!(registry.count(), 1);
        }
        assert_eq!(registry.count(), 0, "guard 走了就该自动注销");
        assert!(!registry.cancel("run-1"), "已经结束的请求不该还能被取消");
    }

    #[test]
    fn a_finished_run_does_not_unregister_the_newer_one() {
        let registry = InflightRegistry::new();
        let old = registry.register("same-id");
        let new = registry.register("same-id");

        // 老的那次结束（比如超时返回了）
        drop(old);
        assert_eq!(registry.count(), 1, "后一次的登记必须还在");

        assert!(registry.cancel("same-id"), "后一次应当仍然可取消");
        assert!(new.token().is_cancelled());
    }
}
