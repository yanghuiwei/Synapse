//! 压测统计。
//!
//! 这一层**不碰网络、不碰数据库、不碰 tokio** —— 只吃样本吐汇总。
//! 理由见模块头的说明：算错的 P95 和算对的 P95 长得一模一样。
//!
//! ## 三个需要钉死的口径
//!
//! **1. 什么叫「成功」。** 口径是「拿到了响应，且状态码 < 400」。
//! 拿不到响应（DNS / 连接被拒 / TLS / 超时）一律算失败 —— 这类失败在
//! `loadtest_sample.status` 里是 NULL，天然与「服务端返回了 4xx」区分得开。
//! 3xx 算成功：跟随重定向关闭时它就是这次请求的**最终答案**，
//! 把它算成失败会凭空造出一批"错误"，而这个数字是用户会拿去做判断的。
//!
//! **2. TPS 与平均 RPS 不是同一个数。**
//! `tps` 是**实际吞吐**（含失败的请求总数 / 耗时）—— 它回答"客户端一共打出去多少"；
//! `throughput.avg_rps` 是**有效吞吐**（只数成功 / 耗时）—— 它回答"这个接口能扛多少"。
//! 两者在失败率高的压测里会明显分开，合并成一个数会让「接口扛不住」
//! 和「客户端打太多打崩了」看起来一样。
//!
//! **3. 峰值 RPS 只统计「完整的 1 秒窗口」。**
//! 按 `offset_ms / 1000` 分桶后，首尾两个桶可能只覆盖半秒 ——
//! 把它们算进去只会得到一个偏小的峰值。所以只有「这次压测的时长
//! 真的盖满了这一秒」的桶才参与取最大值。整轮不足 1 秒时退化成
//! 「按实际时长折算的 RPS」，并在 [`ThroughputSummary::peak_window_is_partial`] 里说明。

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::loadtest::latency::LatencyHistogram;
use crate::storage::models::LoadtestSample;

/// 每种错误类型最多留几条样本（挑**最慢**的那几条）。
///
/// 挑最慢而不是最早的：一批超时里，最早的那条往往只是"碰巧先撞上"，
/// 而最慢的那条最接近超时阈值，对定位最有信息量。
const TOP_ERROR_SAMPLES: usize = 5;

/// 实时失败日志保留多少条（design.md §7.4）。
///
/// 只记失败，所以这个环在正常的压测里几乎不增长 ——
/// 而失败成片出现时，用户最想看的正是「最新冒出来的是哪个错」。
const RECENT_FAILURES: usize = 50;

/// 判定「成功」的状态码上限。见模块头的口径 1。
const SUCCESS_STATUS_MAX: i64 = 400;

// ============================================================
// 输出结构
// ============================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencySummary {
    pub min_ms: u64,
    pub mean_ms: f64,
    pub p50_ms: u64,
    pub p90_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
    pub max_ms: u64,
}

impl LatencySummary {
    /// 一条样本都没有时的零值。
    ///
    /// 不用 `Default::default()`：那样 `min_ms` / `max_ms` 会是 0，
    /// 而界面上「最小 0ms / 最大 0ms」看起来像真的量到了，比"没有数据"更糟。
    /// 前端靠 `LoadtestSummary::total == 0` 判断要不要显示空态，这里只保证不 panic。
    fn empty() -> Self {
        Self {
            min_ms: 0,
            mean_ms: 0.0,
            p50_ms: 0,
            p90_ms: 0,
            p95_ms: 0,
            p99_ms: 0,
            max_ms: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThroughputSummary {
    /// **有效**吞吐：只数成功的请求。与 `LoadtestSummary::tps` 的区别见模块头口径 2
    pub avg_rps: f64,
    pub peak_rps: u64,
    /// 峰值那个窗口是不是被首尾截断了（整轮不足 1 秒，或最后一个窗口没满）。
    /// 为 `true` 时界面要说明"这个峰值是按不足 1 秒的实际时长折算的"
    pub peak_window_is_partial: bool,
    pub total_bytes: u64,
    pub avg_bytes: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusBucket {
    /// `None` = **没拿到响应**（连接失败 / 超时 / TLS）。
    ///
    /// 刻意用 null 而不是 0 当哨兵：0 不是合法 HTTP 状态码，
    /// 但拿它当特殊值意味着前端必须记住「0 是特殊的那一个」——
    /// 而 `null` 在类型上就逼着调用方处理。
    pub status: Option<i64>,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBucket {
    /// 后端错误码（`E_TIMEOUT` / `E_DNS` / …），**不是**文案。
    /// 契约同全局：后端只给码，句子在前端 i18n 里。
    pub code: String,
    pub count: u64,
    /// 最慢的若干条样本的 `seq`，供界面直接跳到那一条
    pub slowest_seqs: Vec<i64>,
    /// 其中最慢的那条的耗时，方便一眼看出"离超时还有多远"
    pub slowest_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadtestSummary {
    pub total: u64,
    pub succeeded: u64,
    pub failed: u64,
    /// 0.0 ~ 1.0。**不用百分数**：谁乘 100 谁自己决定，避免出现"99"和"0.99"混用
    pub success_rate: f64,
    pub elapsed_ms: u64,
    /// 实际吞吐（含失败），见模块头口径 2
    pub tps: f64,
    pub latency: LatencySummary,
    pub throughput: ThroughputSummary,
    /// 按状态码分组，`null` 组排在最前（`None < Some(_)`）
    pub statuses: Vec<StatusBucket>,
    /// 按错误码分组，条数多的排前面
    pub errors: Vec<ErrorBucket>,
    /// 是不是被用户取消的。**必须告诉用户** ——
    /// 一份只跑了三分之一就停下的报告，成功率和 P95 都只代表那一小段，
    /// 不说清楚就会被当成完整结果引用
    pub cancelled: bool,
    /// 整轮压测里被**冻结成同一个值**的动态函数占位符原文
    /// （如 `{{$uuid}}` / `{{$randomInt}}`）。
    ///
    /// 来源：请求只在开工前解析一次（见 [`crate::loadtest::engine`] 的说明），
    /// 所以这些本该"每个请求都不一样"的占位符会在整轮里保持同一个值。
    /// 这**不一定错** —— 但用户必须知道，否则他会以为压测在打 500 个不同的
    /// 幂等键，实际打的是 500 遍同一个。空数组 = 没有这类占位符。
    #[serde(default)]
    pub frozen_functions: Vec<String>,
}

/// 失败日志里的一行。
///
/// 同时带 `status` 与 `code`，因为「失败」在手感上有两种：
/// * 服务端答了 4xx/5xx —— 有 `status`，`code` 是 `None`
/// * 压根没拿到响应（DNS / 超时 / TLS）—— `status` 是 `None`，有 `code`
///
/// 两种都算失败（见模块头口径 1），但**原因完全不同**，合成一行会让
/// 「接口返回 500」和「连不上」看起来一模一样。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureLine {
    pub seq: i64,
    pub status: Option<i64>,
    /// 后端错误码，不是文案。服务端正常应答时这里是 `None`
    pub code: Option<String>,
    pub duration_ms: u64,
    pub offset_ms: i64,
}

/// 压测进行中的一次进度快照（每 200ms 推给前端）。
///
/// 故意只带**廉价**字段：进度事件跑在最热的路径上，
/// 每次都算一遍完整分位会把压测本身拖慢。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressSnapshot {
    pub sent: u64,
    pub succeeded: u64,
    pub failed: u64,
    /// 这一刻的 P95。直方图支持随时取分位，所以直接给
    pub p95_ms: u64,
    /// 最近一个采样窗口的完成速率（不是从头算的平均值）——
    /// 进度条上要看的正是"现在快不快"
    pub current_rps: f64,
    pub elapsed_ms: u64,
    /// 最近 [`RECENT_FAILURES`] 条失败，最新的在后（design.md §7.4 的实时日志）。
    ///
    /// 放在进度事件里而不是让前端再发一次请求去查：失败日志的意义就是"跟着跑",
    /// 而每 200ms 多一次 IPC 去取它，等于自己给压测加负载。
    /// 代价是每帧多带几十条，而正常压测里这个数组是空的。
    pub recent_failures: Vec<FailureLine>,
}

// ============================================================
// 累积器
// ============================================================

/// 按错误码累积：条数 + 最慢的几条样本。
#[derive(Debug, Default)]
struct ErrorGroup {
    count: u64,
    /// 按耗时降序维护，只留 [`TOP_ERROR_SAMPLES`] 条。
    /// 长度最多 5，所以每次插入都重排比维护堆更划算
    slowest: Vec<(u64, i64)>,
}

impl ErrorGroup {
    fn push(&mut self, duration_ms: u64, seq: i64) {
        self.count += 1;
        self.slowest.push((duration_ms, seq));
        if self.slowest.len() > TOP_ERROR_SAMPLES {
            self.slowest
                .sort_unstable_by_key(|(ms, _)| std::cmp::Reverse(*ms));
            self.slowest.truncate(TOP_ERROR_SAMPLES);
        }
    }
}

/// 样本累积器。
///
/// ## 为什么是单线程的
///
/// 引擎里所有 worker 把样本丢进一个 channel，**由唯一一个收集任务**
/// 喂给这个累积器。这样直方图不需要原子操作也不需要加锁 ——
/// 而在压测这种每秒上万次写入的场景里，任何加锁都是拿被测系统的
/// CPU 去换统计，直接把要测的数字本身改掉。
pub struct StatsAccumulator {
    histogram: LatencyHistogram,
    sent: u64,
    succeeded: u64,
    failed: u64,
    total_bytes: u64,
    /// 键是 `offset_ms / 1000`，值是这一秒内**完成**的请求数
    per_second: BTreeMap<u64, u64>,
    statuses: BTreeMap<Option<i64>, u64>,
    errors: BTreeMap<String, ErrorGroup>,
    /// 最近若干条失败，最新的在后。见 [`RECENT_FAILURES`]
    recent_failures: VecDeque<FailureLine>,
}

impl Default for StatsAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl StatsAccumulator {
    pub fn new() -> Self {
        Self {
            histogram: LatencyHistogram::new(),
            sent: 0,
            succeeded: 0,
            failed: 0,
            total_bytes: 0,
            per_second: BTreeMap::new(),
            statuses: BTreeMap::new(),
            errors: BTreeMap::new(),
            recent_failures: VecDeque::new(),
        }
    }

    /// 吃一条样本。
    pub fn record(&mut self, sample: &LoadtestSample) {
        self.sent += 1;
        self.total_bytes += sample.bytes.unwrap_or(0).max(0) as u64;

        *self.statuses.entry(sample.status).or_insert(0) += 1;
        *self
            .per_second
            .entry((sample.started_offset_ms.max(0) / 1000) as u64)
            .or_insert(0) += 1;

        let ok = matches!(sample.status, Some(s) if s < SUCCESS_STATUS_MAX);
        if ok {
            self.succeeded += 1;
        } else {
            self.failed += 1;
            // 失败日志环。**两种失败都记**：服务端答 4xx/5xx 与压根没拿到响应，
            // 在手感上都是"这次没成"，只是原因不同（见 `FailureLine` 的说明）
            if self.recent_failures.len() == RECENT_FAILURES {
                self.recent_failures.pop_front();
            }
            self.recent_failures.push_back(FailureLine {
                seq: sample.seq,
                status: sample.status,
                code: sample.error.clone().filter(|c| !c.is_empty()),
                duration_ms: sample.duration_ms.max(0) as u64,
                // 与 `LoadtestSample::started_offset_ms` 同一个类型（i64）：
                // 两边一个是 i64 一个是 u64 的话，跨层传的时候必然要来回转
                offset_ms: sample.started_offset_ms.max(0),
            });
        }

        // 有错误码就归组。注意「服务端返回 500」是**没有**错误码的
        // （后端顺利拿到了响应），所以它只进 statuses 不进 errors —— 这是对的：
        // errors 回答的是"为什么没拿到响应"，statuses 回答的是"服务端说了什么"
        if let Some(code) = sample.error.as_deref().filter(|c| !c.is_empty()) {
            self.errors
                .entry(code.to_string())
                .or_default()
                .push(sample.duration_ms.max(0) as u64, sample.seq);
        }

        // 亚毫秒的请求记成 1ms —— 这里的误差是测量分辨率本身的极限
        // （我们记的就是整毫秒），而不是把它抹成 0 后让 min 变成一个没意义的数字。
        self.histogram.record(sample.duration_ms.max(1) as u64);
    }

    pub fn sent(&self) -> u64 {
        self.sent
    }

    pub fn succeeded(&self) -> u64 {
        self.succeeded
    }

    pub fn failed(&self) -> u64 {
        self.failed
    }

    /// 这一刻的 P95。给进度事件用（不构造完整汇总）。
    pub fn p95_ms(&self) -> u64 {
        self.histogram.value_at_quantile(0.95)
    }

    pub fn snapshot(&self, elapsed_ms: u64, current_rps: f64) -> ProgressSnapshot {
        ProgressSnapshot {
            sent: self.sent,
            succeeded: self.succeeded,
            failed: self.failed,
            p95_ms: self.p95_ms(),
            current_rps,
            elapsed_ms,
            recent_failures: self.recent_failures.iter().cloned().collect(),
        }
    }

    /// 出汇总。
    ///
    /// `elapsed_ms` 由调用方给而不是内部读时钟：这样测试能构造
    /// 「正好 2000ms 跑了 100 个请求」这种确定的输入，断言 `tps == 50.0`。
    pub fn summarize(&self, elapsed_ms: u64, cancelled: bool) -> LoadtestSummary {
        let elapsed_secs = elapsed_ms as f64 / 1000.0;
        // 耗时可能是 0（全部请求同步失败）。除 0 会得到 inf，
        // 序列化成 JSON 是 null，前端一乘就变成 NaN —— 直接给 0 更诚实
        let per_sec = |n: u64| {
            if elapsed_secs > 0.0 {
                n as f64 / elapsed_secs
            } else {
                0.0
            }
        };

        let (peak_rps, partial) = self.peak_rps(elapsed_ms);

        let latency = if self.histogram.is_empty() {
            LatencySummary::empty()
        } else {
            let q = |p: f64| self.histogram.value_at_quantile(p);
            LatencySummary {
                min_ms: self.histogram.min(),
                mean_ms: self.histogram.mean(),
                p50_ms: q(0.50),
                p90_ms: q(0.90),
                p95_ms: q(0.95),
                p99_ms: q(0.99),
                max_ms: self.histogram.max(),
            }
        };

        let avg_bytes = if self.sent > 0 {
            self.total_bytes as f64 / self.sent as f64
        } else {
            0.0
        };

        let mut errors: Vec<ErrorBucket> = self
            .errors
            .iter()
            .map(|(code, group)| {
                let mut slowest = group.slowest.clone();
                slowest.sort_unstable_by_key(|(ms, _)| std::cmp::Reverse(*ms));
                ErrorBucket {
                    code: code.clone(),
                    count: group.count,
                    slowest_seqs: slowest.iter().map(|(_, seq)| *seq).collect(),
                    slowest_ms: slowest.first().map(|(ms, _)| *ms).unwrap_or(0),
                }
            })
            .collect();
        // 条数多的排前面；同条数按错误码字典序，保证两次跑结果顺序一致
        // （顺序会变的话，导出 JSON 做 CI 比对就没法用 diff 了）
        errors.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.code.cmp(&b.code)));

        LoadtestSummary {
            total: self.sent,
            succeeded: self.succeeded,
            failed: self.failed,
            success_rate: if self.sent > 0 {
                self.succeeded as f64 / self.sent as f64
            } else {
                0.0
            },
            elapsed_ms,
            tps: per_sec(self.sent),
            latency,
            throughput: ThroughputSummary {
                avg_rps: per_sec(self.succeeded),
                peak_rps,
                peak_window_is_partial: partial,
                total_bytes: self.total_bytes,
                avg_bytes,
            },
            statuses: self
                .statuses
                .iter()
                .map(|(status, count)| StatusBucket {
                    status: *status,
                    count: *count,
                })
                .collect(),
            errors,
            cancelled,
            // 由引擎在拿到 spec 之后填（这一层不认识 spec）
            frozen_functions: Vec::new(),
        }
    }

    /// 峰值 RPS + 「这个峰值是否被首尾截断」。
    ///
    /// 见模块头口径 3。整轮不足 1 秒时，任何窗口都不完整，
    /// 此时按实际时长折算给出一个等价的 RPS，并置 `partial = true` 让界面去说明。
    fn peak_rps(&self, elapsed_ms: u64) -> (u64, bool) {
        if self.per_second.is_empty() {
            return (0, false);
        }

        let elapsed = elapsed_ms.max(1);
        // 一个桶算"盖满"的条件：这次压测至少持续到这个桶的右边界
        let complete = |second: u64| elapsed >= (second + 1) * 1000;

        let full_peak = self
            .per_second
            .iter()
            .filter(|(second, _)| complete(**second))
            .map(|(_, count)| *count)
            .max();

        match full_peak {
            Some(peak) => (peak, false),
            None => {
                // 整轮不足 1 秒：按实际时长折算，并明说是折算值
                let peak = self
                    .per_second
                    .values()
                    .copied()
                    .max()
                    .unwrap_or(0)
                    .saturating_mul(1000)
                    / elapsed;
                (peak, true)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(seq: i64, offset_ms: i64, duration_ms: i64, status: Option<i64>) -> LoadtestSample {
        LoadtestSample {
            id: 0,
            run_id: 1,
            seq,
            status,
            duration_ms,
            bytes: Some(120),
            error: None,
            started_offset_ms: offset_ms,
        }
    }

    fn failed(seq: i64, offset_ms: i64, duration_ms: i64, code: &str) -> LoadtestSample {
        LoadtestSample {
            status: None,
            error: Some(code.to_string()),
            ..sample(seq, offset_ms, duration_ms, None)
        }
    }

    fn feed(acc: &mut StatsAccumulator, samples: &[LoadtestSample]) {
        for s in samples {
            acc.record(s);
        }
    }

    // ---------------- 计数与成功率 ----------------

    #[test]
    fn counts_split_success_and_failure_by_status() {
        let mut acc = StatsAccumulator::new();
        feed(
            &mut acc,
            &[
                sample(0, 0, 10, Some(200)),
                sample(1, 1, 20, Some(201)),
                sample(2, 2, 30, Some(302)), // 3xx 算成功，见模块头口径 1
                sample(3, 3, 40, Some(404)),
                sample(4, 4, 50, Some(500)),
                failed(5, 5, 60, "E_TIMEOUT"), // 没拿到响应
            ],
        );

        let s = acc.summarize(1000, false);
        assert_eq!(s.total, 6);
        assert_eq!(s.succeeded, 3, "2xx 与 3xx 都算成功");
        assert_eq!(s.failed, 3, "4xx / 5xx / 无响应都算失败");
        assert!((s.success_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn http_4xx_goes_into_statuses_but_not_errors() {
        let mut acc = StatsAccumulator::new();
        feed(&mut acc, &[sample(0, 0, 10, Some(500))]);

        let s = acc.summarize(100, false);
        assert_eq!(
            s.errors.len(),
            0,
            "后端的 5xx 是拿到了响应的，不该进「为什么没拿到响应」那张表"
        );
        assert_eq!(s.statuses.len(), 1);
        assert_eq!(s.statuses[0].status, Some(500));
    }

    #[test]
    fn statuses_are_grouped_and_put_null_first() {
        let mut acc = StatsAccumulator::new();
        feed(
            &mut acc,
            &[
                sample(0, 0, 10, Some(200)),
                sample(1, 1, 10, Some(200)),
                sample(2, 2, 10, Some(404)),
                failed(3, 3, 10, "E_DNS"),
            ],
        );

        let s = acc.summarize(100, false);
        assert_eq!(s.statuses[0].status, None, "无响应那一组排最前");
        assert_eq!(s.statuses[0].count, 1);
        assert_eq!(s.statuses[1].status, Some(200));
        assert_eq!(s.statuses[1].count, 2);
        assert_eq!(s.statuses[2].status, Some(404));
    }

    // ---------------- 延迟分位 ----------------

    #[test]
    fn empty_accumulator_summarizes_to_zeros_without_panicking() {
        let s = StatsAccumulator::new().summarize(0, false);

        assert_eq!(s.total, 0);
        assert_eq!(s.success_rate, 0.0);
        // 除 0 必须给 0 而不是 inf —— inf 序列化成 JSON 是 null，前端一乘就 NaN
        assert_eq!(s.tps, 0.0);
        assert_eq!(s.throughput.avg_rps, 0.0);
        assert_eq!(s.throughput.avg_bytes, 0.0);
        assert_eq!(s.latency, LatencySummary::empty());
        assert_eq!(s.throughput.peak_rps, 0);
    }

    #[test]
    fn single_sample_gives_a_degenerate_but_consistent_distribution() {
        let mut acc = StatsAccumulator::new();
        feed(&mut acc, &[sample(0, 0, 42, Some(200))]);

        let l = acc.summarize(1000, false).latency;
        // 只有一个值时所有分位都等于它 —— 这一条看似废话，
        // 但它挡住的是「分位函数在 n=1 时返回 0」这类实现 bug
        for (name, v) in [
            ("min", l.min_ms),
            ("p50", l.p50_ms),
            ("p90", l.p90_ms),
            ("p95", l.p95_ms),
            ("p99", l.p99_ms),
            ("max", l.max_ms),
        ] {
            assert_eq!(v, 42, "{name} 应当等于唯一的那个样本");
        }
        assert!((l.mean_ms - 42.0).abs() < 1e-9);
    }

    #[test]
    fn percentiles_are_non_decreasing() {
        let mut acc = StatsAccumulator::new();
        // 1..=500 铺开，分位之间必须是单调的
        for i in 1..=500 {
            acc.record(&sample(i, i * 2, i, Some(200)));
        }

        let l = acc.summarize(1000, false).latency;
        assert!(l.min_ms <= l.p50_ms, "{l:?}");
        assert!(l.p50_ms <= l.p90_ms, "{l:?}");
        assert!(l.p90_ms <= l.p95_ms, "{l:?}");
        assert!(l.p95_ms <= l.p99_ms, "{l:?}");
        assert!(l.p99_ms <= l.max_ms, "{l:?}");

        // P50 应该落在中间那一段（hdrhistogram 有桶宽，给个宽松的带）
        assert!((240..=270).contains(&l.p50_ms), "P50 = {}", l.p50_ms);
        assert!(l.max_ms >= 490, "max = {}", l.max_ms);
    }

    /// 亚毫秒请求会被记成 1ms（量程从 1 起），而不是被抹成 0
    #[test]
    fn sub_millisecond_samples_are_recorded_as_one_not_zero() {
        let mut acc = StatsAccumulator::new();
        acc.record(&sample(0, 0, 0, Some(200)));

        let l = acc.summarize(100, false).latency;
        assert_eq!(l.min_ms, 1);
        assert_eq!(l.max_ms, 1);
    }

    // ---------------- 吞吐 ----------------

    #[test]
    fn tps_counts_everything_while_avg_rps_counts_only_successes() {
        let mut acc = StatsAccumulator::new();
        // 2 秒里 8 条：4 成功 4 失败
        for i in 0..4 {
            acc.record(&sample(i, i * 100, 10, Some(200)));
            acc.record(&failed(i + 4, i * 100, 10, "E_TIMEOUT"));
        }

        let s = acc.summarize(2000, false);
        assert!(
            (s.tps - 4.0).abs() < 1e-9,
            "实际吞吐 = 8 / 2s，实际 {}",
            s.tps
        );
        assert!(
            (s.throughput.avg_rps - 2.0).abs() < 1e-9,
            "有效吞吐 = 4 / 2s，实际 {}",
            s.throughput.avg_rps
        );
    }

    #[test]
    fn peak_rps_ignores_partial_second_buckets() {
        let mut acc = StatsAccumulator::new();
        // 第 0 秒塞 10 条，第 1 秒塞 3 条；压测只跑了 1500ms
        // → 第 1 秒那个桶只盖了半秒，不能算进峰值
        for i in 0..10 {
            acc.record(&sample(i, i * 50, 5, Some(200)));
        }
        for i in 0..3 {
            acc.record(&sample(10 + i, 1000 + i * 100, 5, Some(200)));
        }

        let s = acc.summarize(1500, false);
        assert_eq!(s.throughput.peak_rps, 10, "峰值取盖满的那一秒");
        assert!(!s.throughput.peak_window_is_partial);
    }

    #[test]
    fn peak_rps_counts_the_last_bucket_when_it_is_exactly_full() {
        let mut acc = StatsAccumulator::new();
        for i in 0..4 {
            acc.record(&sample(i, i * 10, 5, Some(200))); // 第 0 秒
        }
        for i in 0..9 {
            acc.record(&sample(4 + i, 1000 + i * 10, 5, Some(200))); // 第 1 秒
        }

        // 正好 2000ms → 第 1 秒那个桶也盖满了
        let s = acc.summarize(2000, false);
        assert_eq!(s.throughput.peak_rps, 9);
        assert!(!s.throughput.peak_window_is_partial);
    }

    #[test]
    fn a_run_shorter_than_one_second_reports_a_derived_peak_and_flags_it() {
        let mut acc = StatsAccumulator::new();
        for i in 0..20 {
            acc.record(&sample(i, i * 4, 3, Some(200)));
        }

        let s = acc.summarize(500, false);
        assert!(
            s.throughput.peak_window_is_partial,
            "不足 1 秒时必须标记出来，否则界面会把这个数当真实峰值展示"
        );
        assert_eq!(s.throughput.peak_rps, 40, "20 条 / 0.5s 折算成 40/s");
    }

    #[test]
    fn bytes_are_summed_and_averaged_over_all_samples() {
        let mut acc = StatsAccumulator::new();
        let mut a = sample(0, 0, 5, Some(200));
        a.bytes = Some(1000);
        let mut b = failed(1, 10, 5, "E_TIMEOUT");
        b.bytes = None; // 没拿到响应就没有字节数
        acc.record(&a);
        acc.record(&b);

        let t = acc.summarize(100, false).throughput;
        assert_eq!(t.total_bytes, 1000);
        assert!((t.avg_bytes - 500.0).abs() < 1e-9, "平均要除以全部样本数");
    }

    // ---------------- 错误分组 ----------------

    #[test]
    fn errors_are_grouped_by_code_and_ordered_by_count() {
        let mut acc = StatsAccumulator::new();
        for i in 0..3 {
            acc.record(&failed(i, i * 10, 100, "E_TIMEOUT"));
        }
        acc.record(&failed(10, 100, 20, "E_DNS"));

        let s = acc.summarize(1000, false);
        assert_eq!(s.errors.len(), 2);
        assert_eq!(s.errors[0].code, "E_TIMEOUT");
        assert_eq!(s.errors[0].count, 3);
        assert_eq!(s.errors[1].code, "E_DNS");
        assert_eq!(s.errors[1].count, 1);
    }

    #[test]
    fn only_the_slowest_few_samples_are_kept_per_error_code() {
        let mut acc = StatsAccumulator::new();
        for i in 0..20 {
            acc.record(&failed(i, i * 10, 10 + i * 10, "E_TIMEOUT"));
        }

        let bucket = &acc.summarize(1000, false).errors[0];
        assert_eq!(bucket.count, 20, "计数要是全量");
        assert_eq!(bucket.slowest_seqs.len(), TOP_ERROR_SAMPLES);
        assert_eq!(bucket.slowest_ms, 200, "最慢的那条是 i=19");
        // 挑的是最慢的 5 条：seq 15..19
        let mut seqs = bucket.slowest_seqs.clone();
        seqs.sort_unstable();
        assert_eq!(seqs, vec![15, 16, 17, 18, 19]);
    }

    #[test]
    fn error_order_is_stable_for_json_diffing() {
        let mut acc = StatsAccumulator::new();
        // 三个都是 2 条 —— 排序必须退化到按码字典序，否则两次跑的导出没法 diff
        for code in ["E_TLS", "E_DNS", "E_TIMEOUT"] {
            acc.record(&failed(0, 0, 10, code));
            acc.record(&failed(1, 10, 10, code));
        }

        let codes: Vec<String> = acc
            .summarize(1000, false)
            .errors
            .into_iter()
            .map(|e| e.code)
            .collect();
        // 注意 E_TIMEOUT 排在 E_TLS **前面**：这是字节序（'I' < 'L'），
        // 不是"按单词义理的顺序"。期望值必须是程序会算出来的那个顺序，
        // 否则这条测试就只是在验证我手排对不对 —— 而它刚刚就排错过一次。
        assert_eq!(codes, vec!["E_DNS", "E_TIMEOUT", "E_TLS"]);
    }

    #[test]
    fn an_empty_error_string_is_ignored_instead_of_becoming_a_group() {
        let mut acc = StatsAccumulator::new();
        let mut s = failed(0, 0, 10, "");
        s.error = Some(String::new());
        acc.record(&s);

        assert_eq!(acc.summarize(100, false).errors.len(), 0);
    }

    // ---------------- 进度快照 ----------------

    #[test]
    fn snapshot_reports_live_numbers_without_building_a_summary() {
        let mut acc = StatsAccumulator::new();
        feed(
            &mut acc,
            &[
                sample(0, 0, 10, Some(200)),
                sample(1, 5, 30, Some(200)),
                failed(2, 8, 60, "E_TIMEOUT"),
            ],
        );

        let snap = acc.snapshot(300, 12.5);
        assert_eq!(snap.sent, 3);
        assert_eq!(snap.succeeded, 2);
        assert_eq!(snap.failed, 1);
        assert_eq!(snap.elapsed_ms, 300);
        assert!((snap.current_rps - 12.5).abs() < 1e-9);
        assert!(snap.p95_ms >= 30, "P95 实际 {}", snap.p95_ms);
    }

    // ---------------- 实时失败日志 ----------------

    #[test]
    fn failure_log_carries_both_kinds_of_failure() {
        let mut acc = StatsAccumulator::new();
        feed(
            &mut acc,
            &[
                sample(0, 0, 10, Some(200)),
                // 服务端答了 500：有 status，没有错误码
                sample(1, 5, 40, Some(500)),
                // 压根没拿到响应：没有 status，有错误码
                failed(2, 8, 60, "E_TIMEOUT"),
            ],
        );

        let log = acc.snapshot(300, 0.0).recent_failures;
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].seq, 1);
        assert_eq!(log[0].status, Some(500));
        assert_eq!(log[0].code, None, "服务端正常应答的失败没有错误码");
        assert_eq!(log[1].seq, 2);
        assert_eq!(log[1].status, None);
        assert_eq!(log[1].code.as_deref(), Some("E_TIMEOUT"));
    }

    #[test]
    fn failure_log_keeps_the_newest_and_drops_the_oldest() {
        let mut acc = StatsAccumulator::new();
        for seq in 0..(RECENT_FAILURES as i64 + 10) {
            acc.record(&failed(seq, seq, 5, "E_TIMEOUT"));
        }

        let log = acc.snapshot(1000, 0.0).recent_failures;
        assert_eq!(
            log.len(),
            RECENT_FAILURES,
            "环必须有界，否则长压测会吃满内存"
        );
        // 最新的在后：留下的是最后 50 条
        assert_eq!(log.last().unwrap().seq, RECENT_FAILURES as i64 + 9);
        assert_eq!(log[0].seq, 10);
    }

    #[test]
    fn failure_log_stays_empty_when_everything_succeeds() {
        let mut acc = StatsAccumulator::new();
        feed(
            &mut acc,
            &[sample(0, 0, 10, Some(200)), sample(1, 1, 20, Some(302))],
        );
        assert!(acc.snapshot(100, 0.0).recent_failures.is_empty());
    }

    #[test]
    fn snapshot_on_an_empty_accumulator_does_not_panic() {
        let snap = StatsAccumulator::new().snapshot(0, 0.0);
        assert_eq!(snap.sent, 0);
        assert_eq!(snap.p95_ms, 0);
    }

    // ---------------- 取消 ----------------

    #[test]
    fn cancel_flag_is_carried_into_the_summary() {
        let mut acc = StatsAccumulator::new();
        acc.record(&sample(0, 0, 10, Some(200)));

        assert!(acc.summarize(500, true).cancelled);
        assert!(!acc.summarize(500, false).cancelled);
    }
}
