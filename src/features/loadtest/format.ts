/**
 * 压测面板共用的两个格式化口径。
 *
 * 搬出来而不是各组件各写一份：侧栏的历史行和应用内的报告要显示**同一组数字**
 * （耗时、成功率），两边各写一份，迟早会在同一块屏幕上出现
 * 「2.27 s」和「2.27s」两种写法。
 */

/** 毫秒 → 「820 ms」/「1.24 s」。压测动辄几十秒，这里比 `formatMs` 更常看秒 */
export function formatDuration(ms: number): string {
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(2)} s`;
}

/**
 * `0.0 ~ 1.0` → `「97.3%」`。
 *
 * `LoadtestSummary.successRate` 刻意存小数而不是百分数（见 `types.ts`），
 * 「乘 100」是**显示层**的事 —— 收在这里，避免每个调用点自己乘一次、
 * 有的乘了有的忘了。小数位跟报告里那格一致（1 位）。
 */
export function formatPercent(rate: number): string {
  return `${(rate * 100).toFixed(1)}%`;
}
