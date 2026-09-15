//! 「当前正在跑的那一轮压测」的登记表。
//!
//! ## 为什么不复用 `http::InflightRegistry`
//!
//! 两者的形态看着一样（一张表 + 一个取消令牌 + RAII 守卫），但生命周期不同：
//! `InflightGuard<'_>` 带着**借用**，必须活在发起它的命令函数里；
//! 而压测的守卫要交给 `tokio::spawn` 出来的引擎任务 —— 那是个 `'static` 任务，
//! 借来的守卫搬不进去。所以这里的守卫自己持有一份 `Arc` 化的内部状态。
//!
//! ## 为什么同时只允许一轮
//!
//! 两轮压测同时打同一个接口，双方测到的都是**对方造成的劣化**：
//! 延迟分位会一起变差，而报告上看起来像是接口自己出了问题。
//! 更糟的是它们会各按自己的并发数去要连接池，把池子拉成两个方向。
//! 所以 `begin` 在有人占坑时直接拒绝，而不是排队 —— 排队的话用户会以为
//! 自己点的「开始」生效了，实际要等上一轮跑完。

use std::sync::{Arc, Mutex, MutexGuard};

use tokio_util::sync::CancellationToken;

/// 一轮进行中的压测。
struct Running {
    run_id: i64,
    cancel: CancellationToken,
}

/// 可克隆的「当前压测」槽。
///
/// 克隆共享同一份内部状态（`Arc`），所以能随守卫一起搬进引擎任务。
#[derive(Clone, Default)]
pub struct LoadtestRegistry {
    slot: Arc<Mutex<Option<Arc<Running>>>>,
}

/// 占坑句柄。Drop 时自动让出槽位，所以清理**不依赖调用方记得收尾**
/// （`?` 提前返回、任务被取消都能兜住）。
pub struct ActiveRun {
    registry: LoadtestRegistry,
    running: Arc<Running>,
}

impl LoadtestRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 占坑。**已经有一轮在跑时返回 `None`**（见模块头）。
    ///
    /// 返回 `None` 不是错误：调用方把它翻译成 `E_INVALID_INPUT`，
    /// 由前端提示「已经有一轮在跑」。
    pub fn begin(&self, run_id: i64) -> Option<ActiveRun> {
        let mut slot = lock(&self.slot);
        if slot.is_some() {
            return None;
        }
        let running = Arc::new(Running {
            run_id,
            cancel: CancellationToken::new(),
        });
        *slot = Some(Arc::clone(&running));
        Some(ActiveRun {
            registry: self.clone(),
            running,
        })
    }

    /// 取消当前这一轮。返回 `false` 表示本来就没在跑。
    ///
    /// 不是错误：用户点取消时这一轮刚好跑完，是很常见的事，
    /// 前端按「本来就没在跑」处理即可（与 `cancel_request` 同约定）。
    pub fn cancel_current(&self) -> bool {
        // 锁内只做「把 Arc 取出来」，取消动作放到锁外：`cancel()` 会唤醒
        // 所有在等这个令牌的任务，抱着锁做等于把它们的调度挂在锁上。
        // 这也是为什么先把 `Arc` clone 出来再 match。
        let running = lock(&self.slot).clone();
        match running {
            Some(running) => {
                running.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// 当前这一轮的 run id。
    pub fn current_run_id(&self) -> Option<i64> {
        lock(&self.slot).as_ref().map(|r| r.run_id)
    }
}

impl ActiveRun {
    pub fn run_id(&self) -> i64 {
        self.running.run_id
    }

    /// 这一轮的取消令牌。传给引擎；引擎只**读**它，不会自己取消。
    pub fn token(&self) -> &CancellationToken {
        &self.running.cancel
    }
}

impl Drop for ActiveRun {
    fn drop(&mut self) {
        let mut slot = lock(&self.registry.slot);
        // 只在自己还是槽里那一个时才清。
        //
        // 现在 `begin` 的占坑语义保证了这个条件恒成立；留着它是防御 ——
        // 一旦哪天为了「取消后能立刻开新的一轮」把 `begin` 改成可抢占，
        // 旧守卫的 drop 就会把新那一轮从槽里抹掉，然后 `cancel_current`
        // 再也找不到人。那种 bug 只在偶发时序里出现，靠复现抓不到，
        // 所以宁可在唯一一处收尾点写上判断。用指针比较而不是 run_id：
        // 自增 id 理论上会被复用，地址不会。
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &self.running))
        {
            *slot = None;
        }
    }
}

/// 拿锁。中毒说明之前持锁的线程 panic 了，但槽里只有一个 `Option<Arc<_>>`，
/// 数据本身没有「被写坏」的概念，直接接着用。
///
/// 与 `http::cancel` 里那份是同一个约定，只是这里的锁属于本模块，
/// 借用不到那边的私有 helper。
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_takes_the_slot_and_drop_releases_it() {
        let registry = LoadtestRegistry::new();
        assert_eq!(registry.current_run_id(), None);

        {
            let active = registry.begin(7).expect("空槽应当占得上");
            assert_eq!(active.run_id(), 7);
            assert_eq!(registry.current_run_id(), Some(7));
        }

        assert_eq!(registry.current_run_id(), None, "守卫 drop 后槽要空出来");
    }

    #[test]
    fn a_second_run_is_rejected_while_one_is_active() {
        let registry = LoadtestRegistry::new();
        let _first = registry.begin(1).expect("第一轮");

        assert!(
            registry.begin(2).is_none(),
            "两轮同时打同一个接口，双方测到的都是对方造成的劣化"
        );
        assert_eq!(registry.current_run_id(), Some(1), "占坑的还得是第一轮");
    }

    #[test]
    fn slot_frees_up_after_the_active_run_ends() {
        let registry = LoadtestRegistry::new();
        {
            let _first = registry.begin(1).expect("第一轮");
        }
        assert_eq!(registry.begin(2).map(|a| a.run_id()), Some(2));
    }

    #[test]
    fn cancel_signals_the_active_token() {
        let registry = LoadtestRegistry::new();
        let active = registry.begin(3).expect("占坑");

        assert!(!active.token().is_cancelled());
        assert!(registry.cancel_current());
        assert!(active.token().is_cancelled(), "取消要打到引擎那个令牌上");
    }

    #[test]
    fn cancel_returns_false_when_idle() {
        let registry = LoadtestRegistry::new();
        assert!(!registry.cancel_current(), "没在跑时取消不是错误");
    }

    #[test]
    fn cancelling_does_not_release_the_slot() {
        // 取消只是发信号，引擎还在把在飞的请求收完（超时 30 秒时可能还要等 30 秒）。
        // 这期间槽必须仍然被占着，否则用户可以再开一轮，两轮就重叠了。
        let registry = LoadtestRegistry::new();
        let active = registry.begin(5).expect("占坑");
        registry.cancel_current();

        assert!(
            registry.begin(6).is_none(),
            "在飞的请求还没结束，不能开新一轮"
        );
        drop(active);
        assert!(registry.begin(6).is_some());
    }

    /// 守的是 `ActiveRun::drop` 里那个 `Arc::ptr_eq` 判断。
    ///
    /// 公开 API 走不出「槽里已经换成另一轮、而旧守卫还活着」的局面
    /// （`begin` 会拒绝），所以直接改槽把它造出来。
    #[test]
    fn a_stale_guard_drop_does_not_clear_a_newer_run() {
        let registry = LoadtestRegistry::new();
        let stale = registry.begin(1).expect("第一轮");

        let newer = Arc::new(Running {
            run_id: 2,
            cancel: CancellationToken::new(),
        });
        *lock(&registry.slot) = Some(Arc::clone(&newer));

        drop(stale);

        assert_eq!(
            registry.current_run_id(),
            Some(2),
            "旧守卫的 drop 把新那一轮从槽里抹掉了 —— cancel 会再也找不到人"
        );
    }
}
