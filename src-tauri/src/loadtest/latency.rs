//! 延迟直方图。
//!
//! ## 为什么这里没有用 hdrhistogram
//!
//! `design.md` §7.3 定的是 `hdrhistogram`。实际落地时换成这个自研版本，
//! 理由是我们的需求比通用库窄得多：
//!
//! * **只需要整数毫秒上的分位。** 记录单位本来就是毫秒
//!   （`loadtest_sample.duration_ms`），不存在"纳秒精度被抹掉"的损失。
//! * **毫秒分辨率下按值索引是精确的。** `counts[i]` 就是"耗时恰好 i 毫秒的样本数"，
//!   分位数是数出来的，不是插出来的。hdrhistogram 用对数分桶，
//!   3 位有效数字仍有约 0.1% 的相对误差 —— 对 P95 来说那是"桶宽"级别的偏差，
//!   而且它的分位曲线在样本少时会出现台阶。
//! * **内存有界且可算。** 条目数 = 出现过的最大毫秒值。压测单请求超时通常 ≤120s，
//!   也就是 12 万个 `u32` ≈ 470KB；就算有人配了 10 分钟超时也才 2.4MB。
//!   而"收一个 `Vec<u64>` 再排序"在这点上才是真问题：1000 万个样本就是 80MB。
//! * **零依赖。** 这个仓库的构建环境拉不到新的 crates（见 `refs/rust.md`），
//!   而一个按值索引的计数器不值得为它引入取包与编译的整条链路。
//!
//! 换回 hdrhistogram 的代价被刻意压到最低：**只有这一个文件碰直方图后端**，
//! 外面（`stats.rs`）只用 `record` / `min` / `max` / `mean` / `value_at_quantile`
//! 这五个方法。要换就整体替换本文件的 `LatencyHistogram` 实现。

/// 按毫秒值索引的计数器。
///
/// `counts[i]` = 耗时恰好 `i` 毫秒的样本数。数组按需增长，
/// 所以「从没出现过 30 秒的请求」就不会为那一段付内存。
pub struct LatencyHistogram {
    counts: Vec<u32>,
    total: u64,
    /// 耗时之和。用来算 mean —— 单独存一份而不是每次遍历 `counts`：
    /// `counts` 可能有十几万格，而 mean 在进度事件里会被频繁取用。
    sum: u64,
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl LatencyHistogram {
    pub fn new() -> Self {
        // 预置到 1024ms：绝大多数接口的延迟都在这个范围内，
        // 预置能省掉压测前几百个样本上的反复扩容
        Self {
            counts: vec![0; 1024],
            total: 0,
            sum: 0,
        }
    }

    pub fn record(&mut self, ms: u64) {
        let index = ms as usize;
        if index >= self.counts.len() {
            // 一次扩到需求大小的两倍：否则「延迟缓慢爬升」的场景会每次记录都扩容
            self.counts.resize(index.saturating_mul(2).max(1024), 0);
        }
        // 饱和加而不是普通 `+=`：`u32` 要溢出的前提是**同一个精确毫秒值**
        // 出现 40 亿次（即总请求数超过 40 亿），单机压测到不了。
        // 真到了那一步，该先看的是客户端 CPU 而不是计数器。但一旦真溢出，
        // release 下会回绕成 0 —— 那会让 P95 凭空变小，属于「静默算错」，
        // 是这个模块最不能出的一类 bug。所以宁可慢一个分支也要夹住。
        self.counts[index] = self.counts[index].saturating_add(1);
        self.total += 1;
        self.sum = self.sum.saturating_add(ms);
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    pub fn len(&self) -> u64 {
        self.total
    }

    pub fn min(&self) -> u64 {
        self.counts
            .iter()
            .position(|c| *c > 0)
            .map(|i| i as u64)
            .unwrap_or(0)
    }

    pub fn max(&self) -> u64 {
        self.counts
            .iter()
            .rposition(|c| *c > 0)
            .map(|i| i as u64)
            .unwrap_or(0)
    }

    pub fn mean(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        self.sum as f64 / self.total as f64
    }

    /// 分位数，**精确值**（最近秩法）。
    ///
    /// `q` 会被夹到 `[0, 1]`。定义：返回满足「累计计数 ≥ ⌈q × n⌉」的最小耗时值。
    /// 用最近秩而不是线性插值：
    ///
    /// * `n = 1` 时插值公式会算出 0.5 倍，那是个不存在的数；最近秩直接返回那唯一的值
    /// * 插值会造出"P95 = 132.7ms"这种听起来精确、实际是编出来的数字
    ///
    /// 返回 `u64` 而不是 `f64`：耗时本来就是整毫秒，分位数凭什么比样本更精细。
    pub fn value_at_quantile(&self, q: f64) -> u64 {
        if self.total == 0 {
            return 0;
        }
        let q = q.clamp(0.0, 1.0);
        // 最近秩：⌈q × n⌉，且至少为 1（q=0 时也要落在第一个元素上）
        let rank = ((q * self.total as f64).ceil() as u64).max(1);
        let mut seen: u64 = 0;
        for (index, count) in self.counts.iter().enumerate() {
            seen += u64::from(*count);
            if seen >= rank {
                return index as u64;
            }
        }
        // 走不到这里：`seen` 累到 total 时必然已越过 rank
        self.max()
    }
}

impl std::fmt::Debug for LatencyHistogram {
    /// 手写而不是 derive：`counts` 有十几万格，derive 出来的 Debug
    /// 会把断言失败信息淹掉 —— 那可是这个结构最需要可读的地方。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LatencyHistogram")
            .field("count", &self.total)
            .field("min_ms", &self.min())
            .field("max_ms", &self.max())
            .field("mean_ms", &self.mean())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hist(values: &[u64]) -> LatencyHistogram {
        let mut h = LatencyHistogram::new();
        for v in values {
            h.record(*v);
        }
        h
    }

    #[test]
    fn empty_histogram_reports_zeros() {
        let h = LatencyHistogram::new();
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
        assert_eq!(h.min(), 0);
        assert_eq!(h.max(), 0);
        assert_eq!(h.mean(), 0.0);
        assert_eq!(h.value_at_quantile(0.95), 0);
    }

    #[test]
    fn min_max_mean_over_a_known_set() {
        let h = hist(&[10, 20, 30, 40]);
        assert_eq!(h.len(), 4);
        assert_eq!(h.min(), 10);
        assert_eq!(h.max(), 40);
        assert!((h.mean() - 25.0).abs() < 1e-9);
    }

    /// 同一个值出现多次时，计数必须真的累加，而不是只记一次
    #[test]
    fn repeated_values_are_counted() {
        let h = hist(&[5, 5, 5, 9]);
        assert_eq!(h.len(), 4);
        assert_eq!(h.min(), 5);
        assert_eq!(h.max(), 9);
        assert!((h.mean() - 6.0).abs() < 1e-9);

        // 最近秩：q=0.5 → ⌈2⌉ = 2 → 第 2 个（从 1 数）是 5
        assert_eq!(h.value_at_quantile(0.5), 5);
        // q=0.75 → ⌈3⌉ = 3 → 3 个 5 已够，仍是 5
        assert_eq!(h.value_at_quantile(0.75), 5);
        // q=0.8 → ⌈3.2⌉ = 4 → 要第 4 个，是 9
        assert_eq!(h.value_at_quantile(0.8), 9);
    }

    #[test]
    fn quantiles_are_exact_for_a_full_range() {
        // 1..=100：最近秩下 P50 = 50、P90 = 90、P95 = 95、P99 = 99
        let values: Vec<u64> = (1..=100).collect();
        let h = hist(&values);
        assert_eq!(h.value_at_quantile(0.50), 50);
        assert_eq!(h.value_at_quantile(0.90), 90);
        assert_eq!(h.value_at_quantile(0.95), 95);
        assert_eq!(h.value_at_quantile(0.99), 99);
        assert_eq!(h.value_at_quantile(1.0), 100);
    }

    #[test]
    fn single_sample_returns_that_value_at_every_quantile() {
        let h = hist(&[42]);
        for q in [0.0, 0.5, 0.95, 0.99, 1.0] {
            assert_eq!(h.value_at_quantile(q), 42, "q={q}");
        }
    }

    #[test]
    fn out_of_range_quantiles_are_clamped() {
        let h = hist(&[3, 7]);
        assert_eq!(h.value_at_quantile(-1.0), 3, "负 q 夹到 0");
        assert_eq!(h.value_at_quantile(0.0), 3);
        assert_eq!(h.value_at_quantile(2.0), 7, "q>1 夹到 1");
    }

    /// 值超过预置容量时必须自动扩容，且不丢数据
    #[test]
    fn growth_beyond_the_initial_capacity_keeps_everything() {
        let h = hist(&[1, 1500, 90_000, 3]);
        assert_eq!(h.len(), 4);
        assert_eq!(h.min(), 1);
        assert_eq!(h.max(), 90_000);
        assert_eq!(h.value_at_quantile(1.0), 90_000);
    }

    /// 0ms 是合法输入（不 panic），分位扫描也要能把 0 报出来
    #[test]
    fn zero_is_a_legal_value() {
        let h = hist(&[0, 0, 4]);
        assert_eq!(h.min(), 0);
        assert_eq!(h.value_at_quantile(0.5), 0);
        assert_eq!(h.value_at_quantile(1.0), 4);
    }

    #[test]
    fn large_sample_count_stays_consistent() {
        let mut h = LatencyHistogram::new();
        for i in 0..10_000u64 {
            h.record(i % 500);
        }
        assert_eq!(h.len(), 10_000);
        assert_eq!(h.min(), 0);
        assert_eq!(h.max(), 499);
        // 每个值出现 20 次：P50 → ⌈5000⌉ → 第 5000 个 → 值 249
        assert_eq!(h.value_at_quantile(0.5), 249);
    }
}
