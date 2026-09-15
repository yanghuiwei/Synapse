//! 压测引擎（design.md §7）。
//!
//! 分层：
//! * [`latency`] —— 延迟直方图（按毫秒值索引，给精确分位）
//! * [`stats`] —— 纯统计，不碰网络也不碰数据库，所以能完整单测
//! * [`engine`] —— 并发派发 + 取消 + 采样流，依赖 [`crate::http`]
//! * [`report`] —— 把汇总渲染成 HTML / CSV / JSON，纯字符串拼接
//! * [`registry`] —— 「当前正在跑的那一轮」的占坑与取消，不依赖以上任何一层
//!
//! ## 为什么统计要单独成一层
//!
//! 压测最贵的 bug 是「数字不对」而不是「跑不起来」：一个算错的 P95
//! 看起来和真的一模一样，没人会去质疑它。把统计抽成不依赖网络与数据库的纯函数，
//! 才能针对「只有 1 个样本」「全部失败」「跨秒边界」这类边界写断言。

pub mod engine;
pub mod latency;
pub mod registry;
pub mod report;
pub mod stats;

pub use engine::{run, LoadtestConfig, LoadtestDeps, RunOutcome, MAX_CONCURRENCY, MAX_TOTAL};
pub use registry::{ActiveRun, LoadtestRegistry};
pub use report::{chart_data, LoadtestChart, ReportLabels};
pub use stats::{
    ErrorBucket, FailureLine, LatencySummary, LoadtestSummary, ProgressSnapshot, StatsAccumulator,
    StatusBucket, ThroughputSummary,
};
