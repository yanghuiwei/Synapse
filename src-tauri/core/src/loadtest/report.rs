//! 压测报告渲染（design.md §7.4）：HTML / CSV / JSON。
//!
//! 纯字符串拼接 —— 不碰网络、不碰数据库、不读时钟，所以能完整单测。
//!
//! ## 为什么这一层是唯一「后端产文本」的地方
//!
//! 全局契约是「后端只返回错误码，不返回面向用户的文案」。但报表是个例外：
//! 它是一份要发给同事、贴进工单、归档的**成品文档**，不能只有数字没有标签。
//!
//! 折中办法是**标签由调用方传进来**（命令层 → 最终来自前端 `zh-CN.ts`），
//! 本层只负责排版。这样依然只有一个文案来源，将来加英文界面也不用改这里。
//! [`ReportLabels`] 的 `Default` 给一组**语言中立**的技术标签
//! （`P50` / `RPS` / `Bytes` 这类本来就是国际通用写法），
//! 既当测试夹具，也当前端漏传某一项时的兜底 —— 报告宁可少一句说明，
//! 也不能因为缺一个标签就导不出来。
//!
//! ## 柱状图/折线图先聚合，不是一条样本画一个元素
//!
//! 一轮压测可能有 50 万条样本。直接循环 `push_str` 出 50 万个 `<circle>`
//! 会产出一份两三百 MB 的 HTML，浏览器打开就死。所以图表的数据源是
//! **按秒聚合后的几十个点**，体积与样本数无关。
//! （原始样本的去处是 CSV 导出，那里的读者是 Excel 而不是浏览器。）
//!
//! ## 关于硬编码色值
//!
//! 项目铁律是「业务组件禁止硬编码色值」，但那条规则针对的是 `src/` 下的
//! 应用组件 —— 它们能拿到 `theme.css` 的令牌。这份 HTML 是**独立文件**，
//! 打开时没有 Reqly 的样式表，拿不到任何令牌。
//! 所以这里把色值定义成报告内部的 CSS 变量（`--ink` / `--brand` …），
//! 取值刻意抄自 `theme.css` 的 **light** 主题，保证报告和界面看起来是一家的。
//! 只在这一处定义，不散落在各个标签上。
//!
//! ## JSON 导出里刻意没有时间戳与运行 id
//!
//! `design.md` §7.4 说 JSON 是「便于 CI 比对」。那就意味着**同一份代码
//! 跑两次，输出必须逐字节相同**，否则 diff 里全是噪声，比对功能等于没有。
//! 所以结构化导出只有 `summary` 本身；`started_at` 这类信息留给 HTML
//! —— 那是给人看的，人需要知道这是哪一次。

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::loadtest::engine::LoadtestConfig;
use crate::loadtest::latency::LatencyHistogram;
use crate::loadtest::stats::LoadtestSummary;
use crate::storage::models::{LoadtestRun, LoadtestSample};

/// HTML 里「最慢的请求」表格最多列几行。
///
/// 20 是拿来看的：再多就没人读了，而去看完整列表的正确姿势是导 CSV。
const MAX_SLOWEST_ROWS: usize = 20;

/// 报告里附上多少条失败样本。
///
/// 比实时的 50 条略少，但比只给错误码分组有用得多 ——
/// 「E_TIMEOUT × 83，最慢 30000ms」和「这 20 条具体长什么样」是两个层次的信息。
const MAX_FAILURE_ROWS: usize = 20;

/// 图表里的横轴最多标几个刻度。按秒数自适应，避免刻度重叠成一坨黑
const MAX_AXIS_TICKS: usize = 8;

// ============================================================
// 标签
// ============================================================

/// 报告里所有面向用户的文字。
///
/// 容器级的 `#[serde(default)]` 让前端可以**只传它想覆盖的那几项**：
/// 缺的落到 [`Default`]（语言中立的技术标签）。这样前端 i18n 还没补齐时，
/// 报告仍然是一份可用文档，而不是一片空白。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReportLabels {
    /// 文档语言，写进 `<html lang>`。
    /// 不是为了 SEO —— 浏览器靠它决定中日韩文本的断行规则与字体回退，
    /// 标错了中文报告会按西文规则断行。
    pub lang: String,

    pub title: String,
    /// 「运行 #7」里的「运行」那个词。单放一个 `#7` 太隐晦
    pub run_id: String,
    pub target: String,
    pub started_at: String,
    pub finished_at: String,
    pub elapsed: String,

    pub section_config: String,
    pub section_summary: String,
    pub section_charts: String,
    pub section_latency: String,
    pub section_throughput: String,
    pub section_status: String,
    pub section_errors: String,
    pub section_slowest: String,
    /// 失败样本列表。与「错误分布」不是一回事：那里是按码分组计数，
    /// 这里是逐条明细（含服务端返回的 4xx/5xx，它们没有错误码）
    pub section_failures: String,
    pub section_notes: String,

    pub chart_latency: String,
    pub chart_rps: String,
    pub chart_histogram: String,

    // 四张指标卡 / 概要表
    pub requests_sent: String,
    pub success_rate: String,
    pub tps: String,
    pub p95: String,

    // 延迟分位
    pub min: String,
    pub mean: String,
    pub p50: String,
    pub p90: String,
    pub p99: String,
    pub max: String,

    // 吞吐
    pub avg_rps: String,
    pub peak_rps: String,
    pub total_bytes: String,
    pub avg_bytes: String,

    // 分布表列头
    pub status: String,
    pub count: String,
    pub code: String,

    // 最慢请求表列头
    pub seq: String,
    pub duration_ms: String,

    // 配置表
    pub concurrency: String,
    pub total: String,
    pub timeout: String,
    pub follow_redirects: String,
    pub keep_alive: String,
    pub think_time: String,
    pub http_version: String,
    pub warmup: String,
    pub yes: String,
    pub no: String,

    // 条件性提示
    pub no_response: String,
    pub cancelled_notice: String,
    /// 概要还在、但采样明细已被保留策略裁掉时的说明。
    ///
    /// 不说这句的话，这种报告看起来像「这个接口一条数据都没采到」——
    /// 而真相是「采到了，只是被清理掉了」。两者的结论完全相反。
    pub samples_pruned_notice: String,
    pub frozen_notice: String,
    pub peak_partial_notice: String,

    /// §7.5 的能力边界说明，逐条作为列表项渲染。空数组 = 不渲染这一节。
    ///
    /// 做成**数组而不是定长字段**：这份清单是产品话术，未来会增删，
    /// 不该因为加一条说明就要动 Rust 结构体。
    pub notes: Vec<String>,
}

impl Default for ReportLabels {
    fn default() -> Self {
        // 全部用语言中立的技术写法。这些词在中文界面里也基本照用
        // （「P95」「RPS」「Bytes」），所以当兜底不会显得违和。
        Self {
            lang: "en".into(),
            title: "Load Test Report".into(),
            run_id: "Run".into(),
            target: "Target".into(),
            started_at: "Started".into(),
            finished_at: "Finished".into(),
            elapsed: "Elapsed".into(),
            section_config: "Configuration".into(),
            section_summary: "Summary".into(),
            section_charts: "Charts".into(),
            section_latency: "Latency".into(),
            section_throughput: "Throughput".into(),
            section_status: "Status codes".into(),
            section_errors: "Errors".into(),
            section_slowest: "Slowest requests".into(),
            section_failures: "Failures".into(),
            section_notes: "Scope and limits".into(),
            chart_latency: "Latency over time (P95 per second)".into(),
            chart_rps: "Requests per second".into(),
            chart_histogram: "Latency distribution".into(),
            requests_sent: "Requests".into(),
            success_rate: "Success rate".into(),
            tps: "TPS".into(),
            p95: "P95".into(),
            min: "Min".into(),
            mean: "Mean".into(),
            p50: "P50".into(),
            p90: "P90".into(),
            p99: "P99".into(),
            max: "Max".into(),
            avg_rps: "Avg RPS".into(),
            peak_rps: "Peak RPS".into(),
            total_bytes: "Transferred".into(),
            avg_bytes: "Avg response".into(),
            status: "Status".into(),
            count: "Count".into(),
            code: "Code".into(),
            seq: "Seq".into(),
            duration_ms: "Duration (ms)".into(),
            concurrency: "Concurrency".into(),
            total: "Total requests".into(),
            timeout: "Timeout".into(),
            follow_redirects: "Follow redirects".into(),
            keep_alive: "Keep-alive".into(),
            think_time: "Think time".into(),
            http_version: "HTTP version".into(),
            warmup: "Warm-up".into(),
            yes: "yes".into(),
            no: "no".into(),
            no_response: "no response".into(),
            cancelled_notice:
                "This run was cancelled. The numbers below only cover the part that ran.".into(),
            samples_pruned_notice:
                "This run's per-request samples were pruned to keep the database small; only the summary remains.".into(),
            frozen_notice:
                "These dynamic placeholders were resolved once and stayed frozen for the whole run:"
                    .into(),
            peak_partial_notice: "Peak RPS is extrapolated from a window shorter than one second."
                .into(),
            notes: Vec::new(),
        }
    }
}

// ============================================================
// 入口
// ============================================================

/// 渲染自包含 HTML 报告（内联 CSS + 内联 SVG，无外部依赖，可直接发人）。
pub fn render_html(
    run: &LoadtestRun,
    summary: &LoadtestSummary,
    samples: &[LoadtestSample],
    labels: &ReportLabels,
) -> String {
    // 先聚合一次，图表与两张样本表都从这一份里取 —— 见 [`LoadtestChart`]
    let chart = chart_data(samples);

    let mut html = String::with_capacity(32 * 1024);
    html.push_str(&html_head(run, labels));
    html.push_str(&html_header(run, summary, labels));
    // 概要还在、明细没了 —— 只可能是被保留策略裁掉的。
    // 报告必须如实说明，否则读者会以为"这个接口一条数据都没采到"。
    if summary.total > 0 && samples.is_empty() {
        html.push_str(&format!(
            r#"<p class="notice sub">{}</p>"#,
            escape_html(&labels.samples_pruned_notice)
        ));
    }
    html.push_str(&html_config(run, labels));
    html.push_str(&html_summary_cards(summary, labels));
    html.push_str(&html_latency_table(summary, labels));
    html.push_str(&html_throughput_table(summary, labels));
    html.push_str(&html_charts(&chart, labels));
    html.push_str(&html_status_table(summary, labels));
    html.push_str(&html_error_table(summary, labels));
    html.push_str(&sample_table(
        &labels.section_slowest,
        &chart.slowest,
        labels,
    ));
    html.push_str(&sample_table(
        &labels.section_failures,
        &chart.failures,
        labels,
    ));
    html.push_str(&html_notes(summary, labels));
    html.push_str("</main>\n</body>\n</html>\n");
    html
}

/// 原始样本导出（design.md §7.4 的列顺序）。
///
/// `error` 空着表示「拿到了响应」，与 `status` 为空的「没拿到响应」是两码事 ——
/// 前者 500，后者是超时。CSV 里就看这两列的组合。
pub fn render_csv(samples: &[LoadtestSample]) -> String {
    let mut csv = String::with_capacity(samples.len() * 48 + 64);
    csv.push_str("seq,status,duration_ms,bytes,error,offset_ms\n");
    for s in samples {
        csv.push_str(&s.seq.to_string());
        csv.push(',');
        // status 为 None 时留**空字段**而不是 0：0 不是合法状态码，
        // 写成 0 会让 Excel 里筛「状态码 < 400」把失败样本当成成功
        if let Some(status) = s.status {
            csv.push_str(&status.to_string());
        }
        csv.push(',');
        csv.push_str(&s.duration_ms.to_string());
        csv.push(',');
        if let Some(bytes) = s.bytes {
            csv.push_str(&bytes.to_string());
        }
        csv.push(',');
        csv.push_str(&csv_field(s.error.as_deref().unwrap_or("")));
        csv.push(',');
        csv.push_str(&s.started_offset_ms.to_string());
        csv.push('\n');
    }
    csv
}

/// 结构化导出，给 CI 比对用。见模块头「刻意没有时间戳」。
pub fn render_json(summary: &LoadtestSummary) -> AppResult<String> {
    serde_json::to_string_pretty(summary)
        .map_err(|e| AppError::Internal(format!("serialize loadtest summary: {e}")))
}

// ============================================================
// 图表数据
// ============================================================

/// 图表与列表要用的**聚合**结果。
///
/// ## 为什么在后端算好再给前端
///
/// 一轮压测可能有 50 万条明细。把它们整体走 IPC 变成 JSON 是几十 MB，
/// 而图表真正需要的只是**几十个聚合点**（每秒一个）。采样明细的去处是
/// CSV 导出（读者是 Excel）和「最慢的 N 条」表格，不是折线图。
///
/// ## 为什么内联 SVG 和前端面板用同一份数据
///
/// 「每秒 P95」这件事只该有一个实现。HTML 报告里的折线与前端面板里的折线
/// 若各算一遍最近秩法，迟早会在某个边界（整除、单样本）上差 1，
/// 然后没人说得清哪个对。
///
/// 没有 `PartialEq`：里面的样本用的是 `LoadtestSample`，那个类型不带这个 derive
/// —— 为了报表去动共享模型不值当，而这里的相等性判断本来也没有意义。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadtestChart {
    pub timeline: Vec<TimelinePoint>,
    pub histogram: Vec<HistogramBucket>,
    /// 最慢的若干条明细
    pub slowest: Vec<LoadtestSample>,
    /// 失败的若干条明细。见 [`MAX_FAILURE_ROWS`]
    pub failures: Vec<LoadtestSample>,
}

/// 时间线上的一个点（一秒）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelinePoint {
    pub second: u64,
    /// 这一秒内**完成**的请求数（含失败）
    pub count: u64,
    pub p95_ms: u64,
}

/// 延迟分布的一个桶，按 2 的幂划分（见 [`svg_histogram`] 的说明）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistogramBucket {
    pub lower_ms: u64,
    /// 上界（含）。最后一个桶是 `u64::MAX` 时表示"及以上"
    pub upper_ms: u64,
    pub count: u64,
}

/// 算出一轮的全部图表数据。
pub fn chart_data(samples: &[LoadtestSample]) -> LoadtestChart {
    LoadtestChart {
        timeline: second_buckets(samples)
            .iter()
            .map(|b| TimelinePoint {
                second: b.second,
                count: b.count,
                p95_ms: b.p95(),
            })
            .collect(),
        histogram: histogram_buckets(samples),
        slowest: slowest_samples(samples, MAX_SLOWEST_ROWS),
        // 失败日志取**最早**的若干条：报告是事后看的，用户想知道的是
        // "一开始出的什么错"—— 等错已经稳定复现了，看第 5000 条没有新信息。
        failures: samples
            .iter()
            .filter(|s| !is_success(s))
            .take(MAX_FAILURE_ROWS)
            .cloned()
            .collect(),
    }
}

/// 与 `stats.rs` 的 `SUCCESS_STATUS_MAX` 同一个口径。
///
/// 这里重写一遍而不是把它导出来共用：那是个内部常量，导出去会让
/// 「改成功口径」变成一次跨模块改动，而它本该只在一处决定。
/// 两边不一致的后果是报告里的「失败列表」和概要里的「失败数」对不上 ——
/// 有一条测试专门盯着这件事（`chart_failures_match_the_summary_count`）。
fn is_success(sample: &LoadtestSample) -> bool {
    matches!(sample.status, Some(status) if status < 400)
}

/// 按 2 的幂分桶的延迟分布。
///
/// 为什么不是等宽分桶：延迟是长尾分布，等宽分桶下前几个桶吃掉所有样本，
/// 剩下 99% 的宽度在画空气。2 的幂分桶（1ms / 2-3 / 4-7 / 8-15 …）
/// 让每个桶的量级一致，长尾也能占据看得见的位置。
fn histogram_buckets(samples: &[LoadtestSample]) -> Vec<HistogramBucket> {
    let mut counts: Vec<u64> = Vec::new();
    for s in samples {
        let ms = s.duration_ms.max(1) as u64;
        // floor(log2(ms))：`leading_zeros` 在 ms >= 1 时最多 63，不会越界
        let idx = (63 - ms.leading_zeros()) as usize;
        if counts.len() <= idx {
            counts.resize(idx + 1, 0);
        }
        counts[idx] += 1;
    }

    counts
        .iter()
        .enumerate()
        .map(|(index, count)| HistogramBucket {
            lower_ms: 1u64 << index.min(62),
            upper_ms: if index >= 62 {
                u64::MAX
            } else {
                (1u64 << (index + 1)) - 1
            },
            count: *count,
        })
        .collect()
}

/// 最慢的 `limit` 条，从慢到快。
fn slowest_samples(samples: &[LoadtestSample], limit: usize) -> Vec<LoadtestSample> {
    // 用引用排序而不是克隆样本：50 万条克隆一份是几十 MB 的无谓开销，
    // 排序 50 万个 8 字节引用则是几毫秒
    let mut refs: Vec<&LoadtestSample> = samples.iter().collect();
    refs.sort_unstable_by_key(|s| std::cmp::Reverse(s.duration_ms));
    refs.truncate(limit);
    refs.into_iter().cloned().collect()
}

// ============================================================
// HTML —— 骨架与样式
// ============================================================

fn html_head(run: &LoadtestRun, labels: &ReportLabels) -> String {
    // 色值抄自 theme.css 的 light 主题，见模块头「关于硬编码色值」
    format!(
        r#"<!doctype html>
<html lang="{lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — {name}</title>
<style>
:root{{
  color-scheme: light;
  --canvas:#ffffff; --subtle:#f8fafc; --line:#e2e8f0; --line-strong:#cbd5e1;
  --ink:#0f172a; --ink-muted:#475569; --ink-faint:#94a3b8;
  --brand:#2563eb; --ok:#16a34a; --warn:#d97706; --err:#dc2626;
  --sans:"Segoe UI Variable Text","Segoe UI",system-ui,-apple-system,"PingFang SC","Microsoft YaHei UI","Microsoft YaHei",sans-serif;
  --mono:"Cascadia Code",ui-monospace,Consolas,"Courier New",monospace;
}}
*{{box-sizing:border-box}}
body{{margin:0;background:var(--canvas);color:var(--ink);font-family:var(--sans);font-size:14px;line-height:1.6}}
main{{max-width:960px;margin:0 auto;padding:40px 24px 64px}}
h1{{font-size:24px;line-height:1.3;margin:0 0 8px}}
h2{{font-size:15px;margin:40px 0 12px;padding-bottom:8px;border-bottom:1px solid var(--line);color:var(--ink-muted);font-weight:600}}
p{{margin:0 0 6px}}
code{{font-family:var(--mono);font-size:12px;background:var(--subtle);border:1px solid var(--line);border-radius:6px;padding:1px 6px}}
.eyebrow{{font-size:12px;letter-spacing:.08em;text-transform:uppercase;color:var(--ink-faint);margin:0 0 6px}}
.meta{{color:var(--ink-muted);font-size:13px}}
.meta b{{color:var(--ink);font-weight:600}}
.cards{{display:grid;grid-template-columns:repeat(4,1fr);gap:12px;margin-top:16px}}
.card{{background:var(--subtle);border:1px solid var(--line);border-radius:12px;padding:14px 16px}}
.card .k{{font-size:12px;color:var(--ink-muted);margin:0 0 4px}}
.card .v{{font-family:var(--mono);font-size:22px;font-weight:600;margin:0}}
table{{width:100%;border-collapse:collapse;font-size:13px}}
th,td{{text-align:left;padding:7px 10px;border-bottom:1px solid var(--line)}}
th{{color:var(--ink-muted);font-weight:600;font-size:12px;background:var(--subtle)}}
td.num,th.num{{text-align:right;font-family:var(--mono)}}
.kv td:first-child{{color:var(--ink-muted);width:200px}}
.two{{display:grid;grid-template-columns:1fr 1fr;gap:32px}}
.chart{{margin:0 0 4px;border:1px solid var(--line);border-radius:12px;padding:14px 16px 8px;background:var(--canvas)}}
.chart h3{{margin:0 0 6px;font-size:13px;font-weight:600;color:var(--ink-muted)}}
.chart svg{{display:block;width:100%;height:auto}}
.notice{{border-left:3px solid var(--warn);background:#fffbeb;border-radius:0 8px 8px 0;padding:10px 14px;margin:16px 0 0;color:#78350f;font-size:13px}}
.notice.bad{{border-left-color:var(--err);background:#fef2f2;color:#7f1d1d}}
.notice.sub{{border-left-color:var(--line-strong);background:var(--subtle);color:var(--ink-muted)}}
.chips{{display:flex;flex-wrap:wrap;gap:6px;margin:8px 0 0;padding:0;list-style:none}}
ul.notes{{margin:0;padding-left:20px;color:var(--ink-muted);font-size:13px}}
ul.notes li{{margin:2px 0}}
footer{{margin-top:48px;padding-top:16px;border-top:1px solid var(--line);color:var(--ink-faint);font-size:12px}}
.pos{{color:var(--ok)}}
.neg{{color:var(--err)}}
</style>
</head>
<body>
<main>
"#,
        lang = escape_html(&labels.lang),
        title = escape_html(&labels.title),
        name = escape_html(&run.request_name),
    )
}

fn html_header(run: &LoadtestRun, summary: &LoadtestSummary, labels: &ReportLabels) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        r#"<header>
<p class="eyebrow">{title}</p>
<h1>{name}</h1>
<p class="meta">{run_id} <b>#{id}</b> · {started} <b>{started_at}</b>"#,
        title = escape_html(&labels.title),
        name = escape_html(&run.request_name),
        run_id = escape_html(&labels.run_id),
        id = run.id,
        started = escape_html(&labels.started_at),
        started_at = escape_html(&run.started_at),
    ));

    if let Some(finished) = run.finished_at.as_deref() {
        out.push_str(&format!(
            " · {label} <b>{value}</b>",
            label = escape_html(&labels.finished_at),
            value = escape_html(finished),
        ));
    }
    out.push_str(&format!(
        " · {label} <b>{value}</b></p>\n",
        label = escape_html(&labels.elapsed),
        value = format_duration(summary.elapsed_ms),
    ));

    if summary.cancelled {
        out.push_str(&format!(
            r#"<p class="notice bad">{}</p>"#,
            escape_html(&labels.cancelled_notice)
        ));
    }
    if summary.throughput.peak_window_is_partial {
        out.push_str(&format!(
            r#"<p class="notice sub">{}</p>"#,
            escape_html(&labels.peak_partial_notice)
        ));
    }
    if !summary.frozen_functions.is_empty() {
        out.push_str(&format!(
            r#"<p class="notice">{notice}</p><ul class="chips">{chips}</ul>"#,
            notice = escape_html(&labels.frozen_notice),
            chips = summary
                .frozen_functions
                .iter()
                .map(|f| format!("<li><code>{}</code></li>", escape_html(f)))
                .collect::<String>(),
        ));
    }
    out.push_str("</header>\n");

    // 目标地址来自 config（`loadtest_run` 表没有 url 列 —— 它属于请求，
    // 而请求随时可能被改；归档进 config 的这一份才是当时打的那个地址）
    if let Some(target) = target_of(run) {
        out.push_str(&format!(
            r#"<p class="meta" style="margin-top:10px">{label} <code>{value}</code></p>"#,
            label = escape_html(&labels.target),
            value = escape_html(&target),
        ));
    }
    out
}

fn html_config(run: &LoadtestRun, labels: &ReportLabels) -> String {
    // 解不出来就整节不渲染：报告不能因为一次结构变更就导不出来
    let Some(config) = config_of(run) else {
        return String::new();
    };

    let flag = |b: bool| escape_html(if b { &labels.yes } else { &labels.no });
    let rows: [(String, String); 8] = [
        (labels.concurrency.clone(), config.concurrency.to_string()),
        (labels.total.clone(), config.total.to_string()),
        (labels.timeout.clone(), format_duration(config.timeout_ms)),
        (
            labels.follow_redirects.clone(),
            flag(config.follow_redirects),
        ),
        (labels.keep_alive.clone(), flag(config.keep_alive)),
        (
            labels.think_time.clone(),
            format_duration(config.think_time_ms),
        ),
        (
            labels.http_version.clone(),
            http_version_text(config.http_version),
        ),
        (labels.warmup.clone(), config.warmup_requests.to_string()),
    ];

    let body: String = rows
        .iter()
        .map(|(k, v)| {
            format!(
                "<tr><td>{}</td><td class=\"num\">{}</td></tr>",
                escape_html(k),
                escape_html(v)
            )
        })
        .collect();

    format!(
        "<h2>{}</h2>\n<table class=\"kv\"><tbody>{}</tbody></table>\n",
        escape_html(&labels.section_config),
        body
    )
}

fn html_summary_cards(summary: &LoadtestSummary, labels: &ReportLabels) -> String {
    // 成功率上色：口径是「状态码 < 400」，所以 100% 是正常的，
    // 用品牌色而不是绿色 —— 一片绿会让人以为"绿色=有告警"
    let rate_class = if summary.success_rate >= 0.999 {
        ""
    } else if summary.failed > 0 {
        " neg"
    } else {
        ""
    };

    format!(
        r#"<h2>{section}</h2>
<div class="cards">
  <div class="card"><p class="k">{sent}</p><p class="v">{sent_v}</p></div>
  <div class="card"><p class="k">{rate}</p><p class="v{rate_class}">{rate_v}</p></div>
  <div class="card"><p class="k">{tps}</p><p class="v">{tps_v}</p></div>
  <div class="card"><p class="k">{p95}</p><p class="v">{p95_v}</p></div>
</div>
<p class="meta" style="margin-top:10px">{ok} {succeeded} · {bad} {failed}</p>
"#,
        section = escape_html(&labels.section_summary),
        sent = escape_html(&labels.requests_sent),
        sent_v = summary.total,
        rate = escape_html(&labels.success_rate),
        rate_class = rate_class,
        rate_v = format_pct(summary.success_rate),
        tps = escape_html(&labels.tps),
        tps_v = format_rate(summary.tps),
        p95 = escape_html(&labels.p95),
        p95_v = format_ms(summary.latency.p95_ms),
        ok = escape_html(&labels.yes),
        succeeded = summary.succeeded,
        bad = escape_html(&labels.no),
        failed = summary.failed,
    )
}

fn html_latency_table(summary: &LoadtestSummary, labels: &ReportLabels) -> String {
    let l = &summary.latency;
    let rows: [(String, String); 7] = [
        (labels.min.clone(), format_ms(l.min_ms)),
        (labels.mean.clone(), format_ms_f(summary.latency.mean_ms)),
        (labels.p50.clone(), format_ms(l.p50_ms)),
        (labels.p90.clone(), format_ms(l.p90_ms)),
        (labels.p95.clone(), format_ms(l.p95_ms)),
        (labels.p99.clone(), format_ms(l.p99_ms)),
        (labels.max.clone(), format_ms(l.max_ms)),
    ];
    metric_table(&labels.section_latency, &rows)
}

fn html_throughput_table(summary: &LoadtestSummary, labels: &ReportLabels) -> String {
    let t = &summary.throughput;
    let rows: [(String, String); 4] = [
        (labels.avg_rps.clone(), format_rate(t.avg_rps)),
        (labels.peak_rps.clone(), t.peak_rps.to_string()),
        (labels.total_bytes.clone(), format_bytes(t.total_bytes)),
        (labels.avg_bytes.clone(), format_bytes_f(t.avg_bytes)),
    ];
    metric_table(&labels.section_throughput, &rows)
}

fn html_status_table(summary: &LoadtestSummary, labels: &ReportLabels) -> String {
    if summary.statuses.is_empty() {
        return String::new();
    }
    let body: String = summary
        .statuses
        .iter()
        .map(|b| match b.status {
            Some(status) => {
                // 只有 4xx/5xx 才红；3xx 是成功（见 stats.rs 口径 1）
                let class = if status < 400 { "pos" } else { "neg" };
                format!(
                    "<tr><td class=\"{class}\">{status}</td><td class=\"num\">{}</td></tr>",
                    b.count
                )
            }
            // 没有状态码 = 压根没拿到响应。单独标出来，因为它是唯一
            // "服务端没说话"的一组，最需要被看见
            None => format!(
                "<tr><td class=\"neg\">{}</td><td class=\"num\">{}</td></tr>",
                escape_html(&labels.no_response),
                b.count
            ),
        })
        .collect();

    format!(
        r#"<h2>{section}</h2>
<table>
<thead><tr><th>{status}</th><th class="num">{count}</th></tr></thead>
<tbody>{body}</tbody>
</table>
"#,
        section = escape_html(&labels.section_status),
        status = escape_html(&labels.status),
        count = escape_html(&labels.count),
        body = body,
    )
}

fn html_error_table(summary: &LoadtestSummary, labels: &ReportLabels) -> String {
    if summary.errors.is_empty() {
        return String::new();
    }
    let body: String = summary
        .errors
        .iter()
        .map(|e| {
            format!(
                "<tr><td><code>{code}</code></td><td class=\"num\">{count}</td><td class=\"num\">{slowest}</td></tr>",
                code = escape_html(&e.code),
                count = e.count,
                slowest = format_ms(e.slowest_ms),
            )
        })
        .collect();

    format!(
        r#"<h2>{section}</h2>
<table>
<thead><tr><th>{code}</th><th class="num">{count}</th><th class="num">{slowest}</th></tr></thead>
<tbody>{body}</tbody>
</table>
"#,
        section = escape_html(&labels.section_errors),
        code = escape_html(&labels.code),
        count = escape_html(&labels.count),
        slowest = escape_html(&labels.max),
        body = body,
    )
}

/// 「序号 / 状态 / 耗时 / 错误码」四列表格。最慢请求与失败列表共用。
///
/// 入参是**已经挑好的**若干条（见 [`chart_data`]）—— 这里不再排序，
/// 因为挑哪些条是「报告想说什么」的决策，不该藏在渲染函数里。
fn sample_table(section: &str, rows: &[LoadtestSample], labels: &ReportLabels) -> String {
    if rows.is_empty() {
        return String::new();
    }

    let body: String = rows
        .iter()
        .map(|s| {
            let status = match s.status {
                Some(code) => code.to_string(),
                None => escape_html(&labels.no_response),
            };
            let detail = s
                .error
                .as_deref()
                .filter(|e| !e.is_empty())
                .map(|e| format!("<code>{}</code>", escape_html(e)))
                .unwrap_or_default();
            format!(
                "<tr><td class=\"num\">{seq}</td><td class=\"num\">{status}</td><td class=\"num\">{ms}</td><td>{detail}</td></tr>",
                seq = s.seq,
                status = status,
                ms = s.duration_ms,
                detail = detail,
            )
        })
        .collect();

    format!(
        r#"<h2>{section}</h2>
<table>
<thead><tr><th class="num">{seq}</th><th class="num">{status}</th><th class="num">{ms}</th><th>{code}</th></tr></thead>
<tbody>{body}</tbody>
</table>
"#,
        section = escape_html(section),
        seq = escape_html(&labels.seq),
        status = escape_html(&labels.status),
        ms = escape_html(&labels.duration_ms),
        code = escape_html(&labels.code),
        body = body,
    )
}

fn html_notes(summary: &LoadtestSummary, labels: &ReportLabels) -> String {
    let mut out = String::new();
    if !labels.notes.is_empty() {
        let items: String = labels
            .notes
            .iter()
            .map(|n| format!("<li>{}</li>", escape_html(n)))
            .collect();
        out.push_str(&format!(
            "<h2>{}</h2>\n<ul class=\"notes\">{}</ul>\n",
            escape_html(&labels.section_notes),
            items
        ));
    }
    out.push_str(&format!(
        "<footer>{} · {} {} · {} {}</footer>\n",
        escape_html(&labels.title),
        escape_html(&labels.requests_sent),
        summary.total,
        escape_html(&labels.elapsed),
        format_duration(summary.elapsed_ms),
    ));
    out
}

/// 「标签 / 值」两列表，延迟与吞吐共用。
fn metric_table(section: &str, rows: &[(String, String)]) -> String {
    let body: String = rows
        .iter()
        .map(|(k, v)| {
            format!(
                "<tr><td>{}</td><td class=\"num\">{}</td></tr>",
                escape_html(k),
                escape_html(v)
            )
        })
        .collect();
    format!(
        "<h2>{}</h2>\n<table class=\"kv\"><tbody>{}</tbody></table>\n",
        escape_html(section),
        body
    )
}

// ============================================================
// 图表（内联 SVG）
// ============================================================

/// 按秒聚合出来的一个点。
struct SecondBucket {
    second: u64,
    count: u64,
    /// 这一秒内延迟的直方图。
    ///
    /// 复用 [`LatencyHistogram`] 而不是自己排序取分位，是为了让图上的 P95
    /// 与概要里的 P95 是**同一个定义**。两处各写一遍最近秩法，
    /// 迟早会在某个边界（整除、单样本）上差 1，然后没人说得清哪个对。
    histogram: LatencyHistogram,
}

impl SecondBucket {
    fn p95(&self) -> u64 {
        self.histogram.value_at_quantile(0.95)
    }
}

/// 把样本按 `started_offset_ms / 1000` 分桶。
///
/// 分桶键与 `stats.rs` 的 `per_second` 保持一致 —— 都是**开始**时刻。
/// 若这里改用完成时刻，图上的峰值会与概要里的峰值对不上。
fn second_buckets(samples: &[LoadtestSample]) -> Vec<SecondBucket> {
    let mut map: std::collections::BTreeMap<u64, SecondBucket> = std::collections::BTreeMap::new();
    for s in samples {
        let second = (s.started_offset_ms.max(0) / 1000) as u64;
        let bucket = map.entry(second).or_insert_with(|| SecondBucket {
            second,
            count: 0,
            histogram: LatencyHistogram::new(),
        });
        bucket.count += 1;
        bucket.histogram.record(s.duration_ms.max(1) as u64);
    }
    map.into_values().collect()
}

/// 绘图区的几何与坐标映射。
///
/// 抽出来是为了让两张图共用同一套边距 —— 各算一份的下场是
/// 折线图和柱状图的基线差 3px，看起来像没对齐。
struct Plot {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
}

const CHART_W: f64 = 900.0;
const CHART_H: f64 = 200.0;
const PAD_LEFT: f64 = 58.0;
const PAD_RIGHT: f64 = 14.0;
const PAD_TOP: f64 = 14.0;
const PAD_BOTTOM: f64 = 28.0;

impl Plot {
    fn new() -> Self {
        Self {
            left: PAD_LEFT,
            top: PAD_TOP,
            width: CHART_W - PAD_LEFT - PAD_RIGHT,
            height: CHART_H - PAD_TOP - PAD_BOTTOM,
        }
    }

    fn bottom(&self) -> f64 {
        self.top + self.height
    }

    /// 折线图上第 `i` 个点（共 `n` 个）的横坐标。只有一个点时落在起点。
    fn point_x(&self, i: usize, n: usize) -> f64 {
        if n <= 1 {
            return self.left;
        }
        self.left + self.width * (i as f64 / (n - 1) as f64)
    }

    /// 纵坐标。`max` 为 0 时压到基线，避免除零得到 NaN
    /// —— NaN 进 SVG 属性会让整张图消失（浏览器直接不画）。
    fn y(&self, value: u64, max: u64) -> f64 {
        if max == 0 {
            return self.bottom();
        }
        self.bottom() - self.height * (value as f64 / max as f64)
    }

    /// 第 `i` 根柱子的矩形（柱状图专用，横轴按数量等分）。
    fn bar(&self, i: usize, n: usize, value: u64, max: u64) -> (f64, f64, f64, f64) {
        let slot = self.width / n.max(1) as f64;
        // 留 15% 间隙，但至少 1px，否则柱多的时候会粘成一块色块
        let gap = (slot * 0.15).max(1.0);
        let w = (slot - gap).max(0.5);
        let x = self.left + slot * i as f64 + gap / 2.0;
        let y = self.y(value, max);
        (x, y, w, self.bottom() - y)
    }
}

fn html_charts(chart: &LoadtestChart, labels: &ReportLabels) -> String {
    if chart.timeline.is_empty() {
        return String::new();
    }

    let mut out = format!("<h2>{}</h2>\n", escape_html(&labels.section_charts));
    out.push_str(&chart_card(
        &labels.chart_latency,
        &svg_line(
            &chart.timeline,
            &labels.chart_latency,
            |p| p.p95_ms,
            format_ms,
        ),
    ));
    out.push_str(&chart_card(
        &labels.chart_rps,
        &svg_bars(
            &chart.timeline,
            &labels.chart_rps,
            |p| p.count,
            |v| v.to_string(),
        ),
    ));
    out.push_str(&chart_card(
        &labels.chart_histogram,
        &svg_histogram(&chart.histogram, &labels.chart_histogram),
    ));
    out
}

fn chart_card(title: &str, svg: &str) -> String {
    format!(
        "<div class=\"chart\"><h3>{}</h3>{}</div>\n",
        escape_html(title),
        svg
    )
}

/// 折线图。每个时间点取一次 `pick()`。
fn svg_line(
    points: &[TimelinePoint],
    aria: &str,
    pick: impl Fn(&TimelinePoint) -> u64,
    fmt: impl Fn(u64) -> String,
) -> String {
    let plot = Plot::new();
    let values: Vec<u64> = points.iter().map(&pick).collect();
    let max = values.iter().copied().max().unwrap_or(0).max(1);
    let n = values.len();

    let path: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!("{:.1},{:.1}", plot.point_x(i, n), plot.y(*v, max)))
        .collect::<Vec<_>>()
        .join(" ");

    let mut svg = svg_open(aria);
    svg.push_str(&grid(&plot, max, &fmt));
    svg.push_str(&format!(
        r#"<polyline fill="none" stroke="var(--brand)" stroke-width="2" stroke-linejoin="round" points="{path}"/>"#
    ));
    // 只有一个点时 polyline 什么都不画（需要至少两个坐标），所以补一个圆点。
    // 整轮不足 1 秒时就是这种情况，而那是完全正常的短压测
    if n == 1 {
        let x = plot.point_x(0, 1);
        let y = plot.y(values[0], max);
        svg.push_str(&format!(
            r#"<circle cx="{x:.1}" cy="{y:.1}" r="3" fill="var(--brand)"/>"#
        ));
    }
    svg.push_str(&x_axis(
        &plot,
        &points.iter().map(|p| p.second).collect::<Vec<_>>(),
        n,
    ));
    svg.push_str("</svg>");
    svg
}

/// 柱状图。每个时间点取一次 `pick()`。
fn svg_bars(
    points: &[TimelinePoint],
    aria: &str,
    pick: impl Fn(&TimelinePoint) -> u64,
    fmt: impl Fn(u64) -> String,
) -> String {
    let plot = Plot::new();
    let values: Vec<u64> = points.iter().map(&pick).collect();
    let max = values.iter().copied().max().unwrap_or(0).max(1);
    let n = values.len();

    let mut svg = svg_open(aria);
    svg.push_str(&grid(&plot, max, &fmt));
    for (i, v) in values.iter().enumerate() {
        let (x, y, w, h) = plot.bar(i, n, *v, max);
        // 高度为 0 的柱不画：`<rect height="0">` 是合法但无意义的元素
        if h <= 0.0 {
            continue;
        }
        svg.push_str(&format!(
            r#"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" fill="var(--brand)" opacity="0.75"/>"#
        ));
    }
    svg.push_str(&x_axis(
        &plot,
        &points.iter().map(|p| p.second).collect::<Vec<_>>(),
        n,
    ));
    svg.push_str("</svg>");
    svg
}

/// 延迟分布柱状图。桶由 [`histogram_buckets`] 按 2 的幂划好。
fn svg_histogram(buckets: &[HistogramBucket], aria: &str) -> String {
    if buckets.is_empty() {
        return String::new();
    }

    let values: Vec<u64> = buckets.iter().map(|b| b.count).collect();
    let max = values.iter().copied().max().unwrap_or(0).max(1);
    let plot = Plot::new();
    let n = values.len();

    let mut svg = svg_open(aria);
    svg.push_str(&grid(&plot, max, &|v: u64| v.to_string()));
    for (i, bucket) in buckets.iter().enumerate() {
        let (x, y, w, h) = plot.bar(i, n, bucket.count, max);
        if h <= 0.0 {
            continue;
        }
        // `<title>` 是 SVG 原生的悬浮提示 —— 不写一行 JS 就能看到桶区间
        svg.push_str(&format!(
            r#"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" fill="var(--brand)" opacity="0.75"><title>{}</title></rect>"#,
            escape_html(&bucket_range_text(bucket))
        ));
    }

    // 横轴标每个桶的下界，仍然按 `MAX_AXIS_TICKS` 抽稀
    let labels: Vec<u64> = buckets.iter().map(|b| b.lower_ms).collect();
    svg.push_str(&tick_labels(&plot, &labels, n, format_ms));
    svg.push_str("</svg>");
    svg
}

fn svg_open(aria: &str) -> String {
    // 刻意**不**加 `preserveAspectRatio="none"`：那样横向拉伸时连刻度文字
    // 一起拉扁。默认的等比缩放配合 CSS 的 `height:auto` 会让整张图随宽度缩放，
    // 文字始终是正的字形。
    format!(
        r#"<svg viewBox="0 0 {w} {h}" role="img" aria-label="{aria}">"#,
        w = CHART_W,
        h = CHART_H,
        aria = escape_html(aria),
    )
}

/// 水平网格线 + 纵轴刻度。
fn grid(plot: &Plot, max: u64, fmt: &impl Fn(u64) -> String) -> String {
    let mut out = String::new();
    for i in 0..=4 {
        let value = max * i / 4;
        let y = plot.y(value, max);
        out.push_str(&format!(
            r#"<line x1="{x1:.1}" y1="{y:.1}" x2="{x2:.1}" y2="{y:.1}" stroke="var(--line)" stroke-width="1"/>"#,
            x1 = plot.left,
            x2 = plot.left + plot.width,
            y = y,
        ));
        out.push_str(&format!(
            r#"<text x="{tx:.1}" y="{ty:.1}" font-size="11" fill="var(--ink-faint)" text-anchor="end">{label}</text>"#,
            tx = plot.left - 6.0,
            ty = y + 4.0,
            label = escape_html(&fmt(value)),
        ));
    }
    out
}

/// 横轴刻度标签（从秒号抽稀到最多 [`MAX_AXIS_TICKS`] 个）。
fn x_axis(plot: &Plot, seconds: &[u64], n: usize) -> String {
    tick_labels(plot, seconds, n, |s| format!("{s}s"))
}

fn tick_labels(plot: &Plot, labels: &[u64], n: usize, fmt: impl Fn(u64) -> String) -> String {
    if n == 0 {
        return String::new();
    }
    // 抽稀：900px 宽塞不下几十个标签，密了就糊成一坨黑
    let step = (n as f64 / MAX_AXIS_TICKS as f64).ceil().max(1.0) as usize;

    let mut out = String::new();
    for i in (0..n).step_by(step) {
        let x = plot.point_x(i, n);
        let label = labels.get(i).copied().unwrap_or(0);
        out.push_str(&format!(
            r#"<text x="{x:.1}" y="{y:.1}" font-size="11" fill="var(--ink-faint)" text-anchor="middle">{label}</text>"#,
            y = plot.bottom() + 16.0,
            label = escape_html(&fmt(label)),
        ));
    }
    out
}

fn bucket_range_text(bucket: &HistogramBucket) -> String {
    if bucket.upper_ms == u64::MAX {
        format!("{}+ ms", bucket.lower_ms)
    } else if bucket.lower_ms == bucket.upper_ms {
        format!("{} ms", bucket.lower_ms)
    } else {
        format!("{}–{} ms", bucket.lower_ms, bucket.upper_ms)
    }
}

// ============================================================
// 小工具
// ============================================================

/// 从归档的 `loadtest_run.config` 里取出压测配置。
///
/// 形状是 `{ "target": { "method", "url" }, "config": { ...LoadtestConfig } }`
/// —— 见 `commands::loadtest::StoredRunConfig`。
/// 拿不到就返回 `None`，调用方整节不渲染：报告不能因为一次归档结构变更就导不出来。
fn config_of(run: &LoadtestRun) -> Option<LoadtestConfig> {
    serde_json::from_value(run.config.get("config")?.clone()).ok()
}

/// 目标地址（`METHOD url`）。
///
/// 存在 run 的 config 里而不是从请求表现查：请求随时可能被改地址或被删，
/// 而报告要说明的是「当时打的是哪个地址」。
fn target_of(run: &LoadtestRun) -> Option<String> {
    let target = run.config.get("target")?;
    let url = target
        .get("url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?;
    let method = target.get("method").and_then(|v| v.as_str()).unwrap_or("");

    Some(if method.is_empty() {
        url.to_string()
    } else {
        format!("{method} {url}")
    })
}

fn http_version_text(v: crate::storage::models::HttpVersion) -> String {
    use crate::storage::models::HttpVersion;
    match v {
        HttpVersion::Auto => "auto".into(),
        HttpVersion::Http1 => "HTTP/1.1".into(),
        HttpVersion::Http2 => "HTTP/2".into(),
    }
}

/// HTML 转义。**所有**插进 HTML 的值都要过这一道 ——
/// 请求名是用户输入，错误码来自网络，一个没转义就是一个 HTML 注入。
fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 8);
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// CSV 字段转义（RFC 4180）：含逗号 / 引号 / 换行就整体加引号，内部的引号翻倍。
///
/// 这里**没有**做公式注入防护（把开头的 `=` `+` `-` `@` 加前缀）。
/// 理由是这张表的每一列都是封闭集合或数字：`status` 是状态码、
/// `duration_ms`/`bytes`/`offset_ms` 是数字、`error` 是 [`crate::error::codes`] 里的常量。
/// 唯一来自用户的字段（请求名）不在导出列里。
/// 哪天加了自由文本列，必须回来补这一层。
fn csv_field(raw: &str) -> String {
    if raw.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", raw.replace('"', "\"\""))
    } else {
        raw.to_string()
    }
}

/// 字节数，1024 进制（网络流量语境下的惯例）。
fn format_bytes(n: u64) -> String {
    format_bytes_f(n as f64)
}

fn format_bytes_f(n: f64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = n;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// 耗时。压测报告里毫秒级的东西要保留整毫秒 —— 一个接口的 P95 是 12ms 还是
/// 12.4ms 不重要，但「12ms」和「0.01s」的区别很重要。
fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.2} s", ms as f64 / 1000.0)
    } else {
        let m = ms / 60_000;
        let s = (ms % 60_000) as f64 / 1000.0;
        format!("{m} m {s:.1} s")
    }
}

fn format_ms(ms: u64) -> String {
    format!("{ms} ms")
}

fn format_ms_f(ms: f64) -> String {
    format!("{ms:.1} ms")
}

/// 速率。`0` 直接给 "0" 而不是 "0.0" —— 全 0 的报告里，
/// 一排 "0.0" 看起来像是算失败了。
fn format_rate(v: f64) -> String {
    if v <= 0.0 {
        "0".into()
    } else if v >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

fn format_pct(ratio: f64) -> String {
    format!("{:.2}%", ratio * 100.0)
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loadtest::stats::StatsAccumulator;
    use serde_json::json;

    fn sample(seq: i64, offset_ms: i64, duration_ms: i64, status: Option<i64>) -> LoadtestSample {
        LoadtestSample {
            id: seq,
            run_id: 1,
            seq,
            status,
            duration_ms,
            bytes: Some(200),
            error: None,
            started_offset_ms: offset_ms,
        }
    }

    fn run(config: serde_json::Value) -> LoadtestRun {
        LoadtestRun {
            id: 7,
            request_id: Some(3),
            request_name: "GET /api/orders".into(),
            config,
            summary: json!({}),
            started_at: "2026-09-14 09:00:00".into(),
            finished_at: Some("2026-09-14 09:00:12".into()),
        }
    }

    fn summary_of(samples: &[LoadtestSample], elapsed_ms: u64) -> LoadtestSummary {
        let mut acc = StatsAccumulator::new();
        for s in samples {
            acc.record(s);
        }
        acc.summarize(elapsed_ms, false)
    }

    // ---------------- CSV ----------------

    #[test]
    fn csv_header_is_stable() {
        assert_eq!(
            render_csv(&[]),
            "seq,status,duration_ms,bytes,error,offset_ms\n"
        );
    }

    #[test]
    fn csv_leaves_status_empty_when_there_was_no_response() {
        let mut s = sample(0, 12, 30_000, None);
        s.error = Some("E_TIMEOUT".into());
        let csv = render_csv(&[s]);
        let row = csv.lines().nth(1).unwrap();
        // 第二列必须是空的：写成 0 会被 Excel 当成"状态码 0"，筛 <400 就把超时算成成功
        assert_eq!(row, "0,,30000,200,E_TIMEOUT,12");
    }

    #[test]
    fn csv_quotes_fields_that_would_break_the_columns() {
        let mut s = sample(1, 0, 5, None);
        s.error = Some("weird,\"quoted\"\ncode".into());
        let csv = render_csv(&[s]);
        assert!(csv.contains("\"weird,\"\"quoted\"\"\ncode\""), "got: {csv}");
    }

    #[test]
    fn csv_keeps_rows_in_the_order_given() {
        let csv = render_csv(&[
            sample(0, 0, 1, Some(200)),
            sample(1, 1, 2, Some(500)),
            sample(2, 2, 3, Some(404)),
        ]);
        let seqs: Vec<&str> = csv
            .lines()
            .skip(1)
            .map(|line| line.split(',').next().unwrap())
            .collect();
        assert_eq!(seqs, ["0", "1", "2"]);
    }

    // ---------------- JSON ----------------

    #[test]
    fn json_is_identical_for_identical_summaries() {
        // 这条守的是 design.md §7.4「便于 CI 比对」：两次运行的输出要能直接 diff。
        // 只要有人在里面塞了时间戳或运行时 id，这条就会红。
        let samples = [sample(0, 0, 10, Some(200)), sample(1, 5, 20, Some(200))];
        let a = render_json(&summary_of(&samples, 1000)).unwrap();
        let b = render_json(&summary_of(&samples, 1000)).unwrap();
        assert_eq!(a, b);
        assert!(!a.contains("started"), "结构化导出不该带运行时刻: {a}");
    }

    #[test]
    fn json_roundtrips_back_into_a_summary() {
        let samples = [sample(0, 0, 10, Some(200)), sample(1, 5, 20, Some(500))];
        let summary = summary_of(&samples, 1000);
        let text = render_json(&summary).unwrap();
        let back: LoadtestSummary = serde_json::from_str(&text).unwrap();
        assert_eq!(back, summary);
    }

    // ---------------- 转义 ----------------

    #[test]
    fn html_escapes_user_controlled_text() {
        let mut r = run(json!({"concurrency": 1}));
        r.request_name = r#"<script>alert("x")</script>"#.into();
        let html = render_html(&r, &summary_of(&[], 0), &[], &ReportLabels::default());
        assert!(!html.contains("<script>"), "请求名没被转义");
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn html_escapes_error_codes_and_notes() {
        let mut s = sample(0, 0, 5, None);
        s.error = Some("<b>E_X</b>".into());
        let labels = ReportLabels {
            notes: vec!["</ul><script>x</script>".into()],
            ..ReportLabels::default()
        };
        let html = render_html(&run(json!({})), &summary_of(&[s], 1000), &[], &labels);
        assert!(!html.contains("<b>E_X</b>"));
        assert!(html.contains("&lt;b&gt;E_X&lt;/b&gt;"));
        assert!(!html.contains("<script>x"));
    }

    // ---------------- HTML 结构 ----------------

    #[test]
    fn html_reports_zero_samples_without_nan_or_infinity() {
        // 取消得极早、或者全部请求在派发前就失败时，会走到这里。
        // 一旦有 NaN 混进 SVG 坐标，浏览器会整张图不画；混进表格就是一片 NaN。
        let html = render_html(
            &run(json!({})),
            &summary_of(&[], 0),
            &[],
            &ReportLabels::default(),
        );
        assert!(!html.contains("NaN"), "报告里出现 NaN");
        // inf 只可能出现在数字单元格或带单位的毫秒里，所以按这两种形态判
        assert!(
            !html.contains(">inf<") && !html.contains("inf ms"),
            "报告里出现 inf"
        );
        assert!(html.contains("GET /api/orders"));
    }

    #[test]
    fn html_shows_the_cancelled_notice_only_when_cancelled() {
        let samples = [sample(0, 0, 10, Some(200))];
        let labels = ReportLabels {
            cancelled_notice: "CANCELLED_MARKER".into(),
            ..ReportLabels::default()
        };

        let mut acc = StatsAccumulator::new();
        acc.record(&samples[0]);
        let running = acc.summarize(1000, false);
        let stopped = acc.summarize(1000, true);

        let clean = render_html(&run(json!({})), &running, &samples, &labels);
        assert!(!clean.contains("CANCELLED_MARKER"));

        let cancelled = render_html(&run(json!({})), &stopped, &samples, &labels);
        assert!(cancelled.contains("CANCELLED_MARKER"));
    }

    #[test]
    fn html_lists_frozen_functions_as_chips() {
        let samples = [sample(0, 0, 10, Some(200))];
        let mut summary = summary_of(&samples, 1000);
        summary.frozen_functions = vec!["{{$uuid}}".into(), "{{$randomInt}}".into()];

        let html = render_html(
            &run(json!({})),
            &summary,
            &samples,
            &ReportLabels::default(),
        );
        assert!(html.contains("<code>{{$uuid}}</code>"));
        assert!(html.contains("<code>{{$randomInt}}</code>"));
    }

    #[test]
    fn html_hides_the_notes_section_when_there_are_no_notes() {
        let samples = [sample(0, 0, 10, Some(200))];
        let labels = ReportLabels {
            section_notes: "NOTES_HEADING".into(),
            ..ReportLabels::default()
        };
        let html = render_html(
            &run(json!({})),
            &summary_of(&samples, 1000),
            &samples,
            &labels,
        );
        assert!(!html.contains("NOTES_HEADING"), "空说明清单不该渲染标题");
    }

    #[test]
    fn html_renders_the_target_from_config() {
        let html = render_html(
            &run(json!({"target": {"method": "POST", "url": "https://api.example.com/orders"}})),
            &summary_of(&[], 0),
            &[],
            &ReportLabels::default(),
        );
        assert!(html.contains("POST https://api.example.com/orders"));
    }

    #[test]
    fn html_renders_the_config_table_from_the_stored_config() {
        let config = json!({
            "target": {"method": "GET", "url": "https://api.example.com/orders"},
            "config": {
                "concurrency": 50,
                "total": 500,
                "timeoutMs": 15000,
                "followRedirects": false,
                "keepAlive": true,
                "thinkTimeMs": 0,
                "httpVersion": "http2",
                "warmupRequests": 20,
            }
        });
        let html = render_html(
            &run(config),
            &summary_of(&[], 0),
            &[],
            &ReportLabels::default(),
        );
        assert!(html.contains("<td class=\"num\">50</td>"));
        assert!(html.contains("HTTP/2"));
        assert!(html.contains("15.00 s"));
    }

    #[test]
    fn html_survives_a_config_that_no_longer_deserializes() {
        // 老版本存下来的 config 少了字段 / 换了形状时，报告必须还能出，
        // 只是没有配置表。绝不能因为"归档数据格式变了"就导不出报告。
        let html = render_html(
            &run(json!({"unexpected": true})),
            &summary_of(&[], 0),
            &[],
            &ReportLabels::default(),
        );
        assert!(html.contains("GET /api/orders"));
        assert!(!html.contains("Configuration"));
    }

    #[test]
    fn html_marks_the_peak_partial_notice_when_the_window_was_cut() {
        let samples = [sample(0, 200, 10, Some(200))];
        let labels = ReportLabels {
            peak_partial_notice: "PARTIAL_MARKER".into(),
            ..ReportLabels::default()
        };
        // 整轮 300ms，任何 1 秒桶都没盖满 → peak_window_is_partial = true
        let html = render_html(
            &run(json!({})),
            &summary_of(&samples, 300),
            &samples,
            &labels,
        );
        assert!(html.contains("PARTIAL_MARKER"));
    }

    #[test]
    fn html_caps_the_slowest_table_and_puts_the_slowest_first() {
        let samples: Vec<LoadtestSample> =
            (0..40).map(|i| sample(i, i, 10 + i, Some(200))).collect();
        let html = render_html(
            &run(json!({})),
            &summary_of(&samples, 1000),
            &samples,
            &ReportLabels::default(),
        );
        // 39 号最慢（10 + 39 = 49ms），必须在
        assert!(html.contains("<td class=\"num\">39</td>"));
        // 0 号最快，被挤出前 20 名
        assert!(!html.contains("<td class=\"num\">0</td>"));
    }

    #[test]
    fn html_never_emits_one_element_per_sample() {
        // 这条防的是"图表按样本逐点渲染"的退化 —— 50 万样本那样写会产出
        // 一份几百 MB 的 HTML。这里 5000 个样本只该产生几十个图形元素。
        let samples: Vec<LoadtestSample> = (0..5000)
            .map(|i| sample(i, i * 3, 5 + (i % 7), Some(200)))
            .collect();
        let html = render_html(
            &run(json!({})),
            &summary_of(&samples, 15000),
            &samples,
            &ReportLabels::default(),
        );
        let rects = html.matches("<rect").count();
        assert!(rects < 200, "图形元素数与样本数脱钩失败，rect = {rects}");
    }

    // ---------------- 分桶 ----------------

    #[test]
    fn second_buckets_group_by_start_offset() {
        let samples = [
            sample(0, 0, 10, Some(200)),
            sample(1, 999, 10, Some(200)),
            sample(2, 1000, 10, Some(200)),
            sample(3, 2500, 10, Some(200)),
        ];
        let buckets = second_buckets(&samples);
        let counts: Vec<(u64, u64)> = buckets.iter().map(|b| (b.second, b.count)).collect();
        assert_eq!(counts, [(0, 2), (1, 1), (2, 1)]);
    }

    #[test]
    fn second_buckets_carry_a_p95_per_second() {
        let mut samples = Vec::new();
        // 第 0 秒：10 个 10ms；第 1 秒：10 个 10ms + 10 个 500ms
        for i in 0..10 {
            samples.push(sample(i, i, 10, Some(200)));
        }
        for i in 0..10 {
            samples.push(sample(10 + i, 1000 + i, 10, Some(200)));
            samples.push(sample(20 + i, 1000 + i, 500, Some(200)));
        }
        let buckets = second_buckets(&samples);
        assert_eq!(buckets[0].p95(), 10);
        // 20 个样本里 10 个是 500ms：最近秩法下 ⌈0.95×20⌉ = 19 → 落进 500ms 段
        assert_eq!(buckets[1].p95(), 500);
    }

    #[test]
    fn histogram_buckets_double_each_step() {
        let samples = [
            sample(0, 0, 1, Some(200)),   // 桶 0：1
            sample(1, 0, 3, Some(200)),   // 桶 1：2-3
            sample(2, 0, 7, Some(200)),   // 桶 2：4-7
            sample(3, 0, 100, Some(200)), // 桶 6：64-127
        ];
        let html = svg_histogram(&histogram_buckets(&samples), "h");
        assert!(html.contains("1 ms"));
        assert!(html.contains("2–3 ms"));
        assert!(html.contains("4–7 ms"));
        assert!(html.contains("64–127 ms"));
    }

    #[test]
    fn histogram_treats_sub_millisecond_as_one_ms() {
        // 0ms 会让 log2 直接炸（leading_zeros 得 64），必须被夹到 1
        let samples = [sample(0, 0, 0, Some(200))];
        let html = svg_histogram(&histogram_buckets(&samples), "h");
        assert!(html.contains("1 ms"));
        assert!(!html.contains("NaN"));
    }

    #[test]
    fn histogram_keeps_the_sample_count_in_its_buckets() {
        // 分桶不能丢样本：把桶里的数加起来必须等于总数。
        // 这条盯的是 `resize` 那段的边界（桶号越界时要先把中间补齐，
        // 否则更慢的样本会落到错的桶里、总数对不上）
        let samples = [
            sample(0, 0, 1, Some(200)),
            sample(1, 0, 50, Some(200)),
            sample(2, 0, 5000, Some(200)),
            sample(3, 0, 1, Some(200)),
        ];
        let total: u64 = histogram_buckets(&samples).iter().map(|b| b.count).sum();
        assert_eq!(total, 4);
    }

    #[test]
    fn svg_charts_have_no_nan_coordinates() {
        // 全失败、零字节、单样本 —— 三种最容易除出 NaN 的输入
        for samples in [
            vec![sample(0, 0, 0, None)],
            vec![sample(0, 0, 30_000, None)],
            (0..3).map(|i| sample(i, i * 1200, 5, Some(500))).collect(),
        ] {
            let chart = chart_data(&samples);
            for svg in [
                svg_line(&chart.timeline, "a", |p| p.p95_ms, format_ms),
                svg_bars(&chart.timeline, "a", |p| p.count, |v| v.to_string()),
                svg_histogram(&chart.histogram, "a"),
            ] {
                assert!(!svg.contains("NaN"), "SVG 里有 NaN: {svg}");
            }
        }
    }

    #[test]
    fn single_point_line_chart_still_draws_something() {
        let chart = chart_data(&[sample(0, 0, 10, Some(200))]);
        assert_eq!(chart.timeline.len(), 1);
        let svg = svg_line(&chart.timeline, "a", |p| p.p95_ms, format_ms);
        // polyline 只有一个点时什么都不画，所以必须有 circle 兜底
        assert!(svg.contains("<circle"));
    }

    // ---------------- 图表数据 ----------------

    #[test]
    fn chart_failures_match_the_summary_failure_count() {
        // 报告里的「失败列表」和概要里的「失败数」必须同一口径。
        // 两边各写一遍 `status < 400` 的话，迟早会在某个边界上分叉，
        // 而症状是"概要说失败 12 个，列表里只有 9 条"——没人说得清哪个对。
        let mut samples: Vec<LoadtestSample> =
            (0..10).map(|i| sample(i, i, 10, Some(200))).collect();
        samples.push(sample(10, 10, 20, Some(302))); // 3xx 算成功
        samples.push(sample(11, 11, 30, Some(404)));
        samples.push(sample(12, 12, 40, Some(500)));
        let mut timeout = sample(13, 13, 50, None);
        timeout.error = Some("E_TIMEOUT".into());
        samples.push(timeout);

        let summary = summary_of(&samples, 1000);
        let chart = chart_data(&samples);

        assert_eq!(summary.failed, 3);
        assert_eq!(chart.failures.len(), summary.failed as usize);
        // 404 / 500 没有错误码，超时有 —— 见 FailureLine/sample_table 的说明
        assert!(chart.failures.iter().any(|s| s.error.is_none()));
        assert!(chart.failures.iter().any(|s| s.error.is_some()));
    }

    #[test]
    fn chart_data_keeps_timeline_and_histogram_in_step_with_the_summary() {
        let samples: Vec<LoadtestSample> = (0..600)
            .map(|i| sample(i, i * 20, 5 + (i % 11), Some(200)))
            .collect();
        let chart = chart_data(&samples);

        let timeline_total: u64 = chart.timeline.iter().map(|p| p.count).sum();
        let histogram_total: u64 = chart.histogram.iter().map(|b| b.count).sum();
        assert_eq!(timeline_total, 600);
        assert_eq!(histogram_total, 600, "分桶不能丢样本");
        // 每秒一个点，且秒号递增
        assert!(chart.timeline.windows(2).all(|w| w[0].second < w[1].second));
    }

    #[test]
    fn chart_data_caps_the_sample_lists() {
        let samples: Vec<LoadtestSample> =
            (0..500).map(|i| sample(i, i, i + 1, Some(500))).collect();
        let chart = chart_data(&samples);
        assert_eq!(chart.slowest.len(), MAX_SLOWEST_ROWS);
        assert_eq!(chart.failures.len(), MAX_FAILURE_ROWS);
        // 最慢的排最前
        assert_eq!(chart.slowest[0].seq, 499);
    }

    #[test]
    fn chart_data_on_an_empty_round_is_empty_not_wrong() {
        let chart = chart_data(&[]);
        assert!(chart.timeline.is_empty());
        assert!(chart.histogram.is_empty());
        assert!(chart.slowest.is_empty());
        assert!(chart.failures.is_empty());
    }

    // ---------------- 标签 ----------------

    #[test]
    fn html_says_so_when_the_samples_were_pruned() {
        let samples = [sample(0, 0, 10, Some(200))];
        let summary = summary_of(&samples, 1000);
        let labels = ReportLabels {
            samples_pruned_notice: "PRUNED_MARKER".into(),
            ..ReportLabels::default()
        };

        // 明细还在时不该出现
        let with_samples = render_html(&run(json!({})), &summary, &samples, &labels);
        assert!(!with_samples.contains("PRUNED_MARKER"));

        // 概要还在、明细空了 = 被裁过
        let pruned = render_html(&run(json!({})), &summary, &[], &labels);
        assert!(pruned.contains("PRUNED_MARKER"));
    }

    #[test]
    fn labels_default_to_a_language_neutral_set() {
        let labels = ReportLabels::default();
        assert_eq!(labels.p95, "P95");
        assert_eq!(labels.lang, "en");
        assert!(labels.notes.is_empty());
        assert!(!labels.cancelled_notice.is_empty());
    }

    #[test]
    fn labels_let_the_caller_override_just_one_field() {
        // 这条保证前端可以渐进式补 i18n：先传标题，其余继续用兜底
        let labels: ReportLabels = serde_json::from_str(r#"{"title":"压测报告"}"#).unwrap();
        assert_eq!(labels.title, "压测报告");
        assert_eq!(labels.p95, "P95");
    }

    #[test]
    fn labels_roundtrip_through_json() {
        let labels = ReportLabels {
            lang: "zh-CN".into(),
            notes: vec!["能做 A".into(), "不能做 B".into()],
            ..ReportLabels::default()
        };
        let text = serde_json::to_string(&labels).unwrap();
        let back: ReportLabels = serde_json::from_str(&text).unwrap();
        assert_eq!(back, labels);
    }

    // ---------------- 格式化 ----------------

    #[test]
    fn bytes_use_binary_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
    }

    #[test]
    fn durations_switch_units_at_a_second_and_a_minute() {
        assert_eq!(format_duration(0), "0 ms");
        assert_eq!(format_duration(999), "999 ms");
        assert_eq!(format_duration(1000), "1.00 s");
        assert_eq!(format_duration(59_999), "60.00 s");
        assert_eq!(format_duration(60_000), "1 m 0.0 s");
        assert_eq!(format_duration(125_500), "2 m 5.5 s");
    }

    #[test]
    fn rates_drop_the_decimal_when_they_are_large_or_zero() {
        assert_eq!(format_rate(0.0), "0");
        assert_eq!(format_rate(0.04), "0.0");
        assert_eq!(format_rate(12.34), "12.3");
        assert_eq!(format_rate(1234.5), "1234");
    }

    #[test]
    fn percentages_always_show_two_decimals() {
        assert_eq!(format_pct(0.0), "0.00%");
        assert_eq!(format_pct(0.998), "99.80%");
        assert_eq!(format_pct(1.0), "100.00%");
    }
}
