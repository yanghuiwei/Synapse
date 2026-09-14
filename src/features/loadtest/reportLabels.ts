import type { TranslateFn } from "@/lib/i18n";
import { DEFAULT_LOCALE } from "@/lib/i18n";
import type { ReportLabels } from "@/lib/types";

/**
 * 报表文案。
 *
 * 后端 `loadtest::report` 是**唯一**「后端产文本」的地方 —— 一份要发给别人的
 * 成品文档不能只有数字没有标签。但文案来源仍然只有一个：`zh-CN.ts`。
 * 前端把这一包文字随命令送过去，后端只负责排版。
 *
 * 为什么不让后端自己写中文：那样中文字面量就散布到两个语言栈里了，
 * 以后加语言得改两处 —— 而且后端那份多半会漏。
 */
export function buildReportLabels(t: TranslateFn): ReportLabels {
  return {
    lang: DEFAULT_LOCALE,
    title: t("loadtest.result"),
    runId: t("loadtest.runId"),
    target: t("loadtest.target"),
    startedAt: t("loadtest.startedAt"),
    finishedAt: t("loadtest.finishedAt"),
    elapsed: t("loadtest.elapsedShort"),

    sectionConfig: t("loadtest.config"),
    sectionSummary: t("loadtest.summary"),
    sectionCharts: t("loadtest.charts"),
    sectionLatency: t("loadtest.latency"),
    sectionThroughput: t("loadtest.throughput"),
    sectionStatus: t("loadtest.statusDist"),
    sectionErrors: t("loadtest.errorDist"),
    sectionSlowest: t("loadtest.slowestRequests"),
    sectionNotes: t("loadtest.notes"),

    chartLatency: t("loadtest.chartLatency"),
    chartRps: t("loadtest.chartRps"),
    chartHistogram: t("loadtest.chartHistogram"),

    requestsSent: t("loadtest.totalRequests"),
    successRate: t("loadtest.successRate"),
    tps: t("loadtest.tps"),

    min: t("loadtest.min"),
    mean: t("loadtest.mean"),
    p50: t("loadtest.p50"),
    p90: t("loadtest.p90"),
    p95: t("loadtest.p95"),
    p99: t("loadtest.p99"),
    max: t("loadtest.max"),

    avgRps: t("loadtest.rps"),
    peakRps: t("loadtest.peakRps"),
    totalBytes: t("loadtest.totalBytes"),
    avgBytes: t("loadtest.avgBytes"),

    status: t("loadtest.statusCode"),
    count: t("loadtest.count"),
    code: t("loadtest.code"),
    seq: t("loadtest.seq"),
    durationMs: t("loadtest.durationMs"),

    concurrency: t("loadtest.concurrency"),
    total: t("loadtest.requests"),
    timeout: t("loadtest.timeout"),
    followRedirects: t("loadtest.followRedirects"),
    keepAlive: t("loadtest.keepAlive"),
    thinkTime: t("loadtest.thinkTime"),
    httpVersion: t("loadtest.httpVersion"),
    warmup: t("loadtest.warmup"),

    yes: t("loadtest.yes"),
    no: t("loadtest.no"),
    noResponse: t("loadtest.noResponse"),

    cancelledNotice: t("loadtest.cancelledNotice"),
    samplesPrunedNotice: t("loadtest.samplesPruned"),
    frozenNotice: t("loadtest.frozenNotice"),
    peakPartialNotice: t("loadtest.peakPartial"),

    // 能力边界。导出的报告往往会被转给别人看，**必须**一起带上
    notes: [t("loadtest.scopeNote")],
  };
}
