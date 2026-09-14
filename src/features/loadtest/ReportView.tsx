import { Snowflake, TriangleAlert } from "lucide-react";

import { useI18n } from "@/lib/i18n";
import type { LoadtestChart, LoadtestRunDetail, LoadtestRun, LoadtestSummary } from "@/lib/types";
import { ExportBar } from "./ExportBar";
import { HistogramChart } from "./HistogramChart";
import { SampleTable } from "./SampleTable";
import { Section, StatCard, BarRow, Notice } from "./primitives";
import { TimelineChart } from "./TimelineChart";

/**
 * 压测报告。
 *
 * 排布顺序是**按重要性**倒推的，不是按数据好拿的程度：
 * 概要 → 需要被看到的话 → 图表 → 分布 → 明细。
 *
 * 一个刻意的取舍：**明细只给「最慢的 20 条」和「失败的 20 条」**，
 * 不做全量列表。一轮 50 万条明细在界面上翻页是没有意义的 ——
 * 那是 CSV 导出的活。全量列表还会让人以为「看完了」，而实际上
 * 分布信息早在概要那几张卡里了。
 */
export function ReportView({
  detail,
  chart,
}: {
  detail: LoadtestRunDetail;
  chart: LoadtestChart | null;
}) {
  const { t } = useI18n();
  const run = detail.run;
  const summary = detail.summary;

  return (
    <div className="flex flex-col gap-5">
      <RunHeader run={run} summary={summary} />

      {summary === null ? (
        // 没有概要 = 这一轮没跑完（进程被杀、或预热阶段就被取消）。
        // 库里那一行还在，所以得说清「这不是一份完整报告」，
        // 不然用户会拿它当结果引用
        <Notice tone="warn">{t("loadtest.unfinished")}</Notice>
      ) : null}

      {summary !== null ? <SummaryBlocks summary={summary} sampleCount={detail.sampleCount} /> : null}

      <Section title={t("loadtest.charts")}>
        {summary !== null && detail.sampleCount === 0 ? (
          <p className="text-[11px] text-ink-faint">{t("loadtest.samplesPrunedShort")}</p>
        ) : null}
        <TimelineChart points={chart?.timeline ?? []} />
        <HistogramChart buckets={chart?.histogram ?? []} className="mt-2" />
      </Section>

      {chart !== null ? (
        <>
          <Section title={t("loadtest.slowestRequests")}>
            <SampleTable kind="slowest" rows={chart.slowest} emptyText={t("loadtest.noSlowest")} />
          </Section>

          <Section title={t("loadtest.failedRequests")}>
            <SampleTable kind="failures" rows={chart.failures} emptyText={t("loadtest.noFailures")} />
          </Section>
        </>
      ) : null}

      <Section title={t("loadtest.export")}>
        <ExportBar disabled={false} hasSummary={summary !== null} />
      </Section>
    </div>
  );
}

// ---------------------------------------------------------------
// 头部：这一轮是什么
// ---------------------------------------------------------------

function RunHeader({ run, summary }: { run: LoadtestRun; summary: LoadtestSummary | null }) {
  const { t } = useI18n();
  const target = targetOf(run);

  return (
    <header className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="font-mono text-[13px] font-semibold text-ink">
          {t("loadtest.reportFor", { id: run.id })}
        </h2>
        {run.finishedAt === null ? (
          <Tag className="border-warn/40 bg-warn/10 text-warn">{t("loadtest.unfinished")}</Tag>
        ) : null}
        {summary?.cancelled === true ? (
          <Tag className="border-err/40 bg-err/10 text-err">{t("loadtest.cancelledTag")}</Tag>
        ) : null}
      </div>

      {/* 地址是这个头部里唯一「必须能选中复制」的东西 */}
      <p className="select-text break-all font-mono text-[11px] text-ink-muted">{target}</p>

      <dl className="flex flex-wrap gap-x-4 gap-y-1 text-[10px] text-ink-faint">
        <Meta label={t("loadtest.startedAt")} value={formatTime(run.startedAt)} />
        <Meta label={t("loadtest.finishedAt")} value={formatTime(run.finishedAt)} />
        {summary !== null ? (
          <Meta label={t("loadtest.elapsedShort")} value={formatDuration(summary.elapsedMs)} />
        ) : null}
      </dl>
    </header>
  );
}

function Meta({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center gap-1">
      <dt>{label}</dt>
      <dd className="tabular font-mono text-ink-muted">{value}</dd>
    </div>
  );
}

function Tag({ children, className }: { children: string; className: string }) {
  return (
    <span className={`rounded-full border px-2 py-[1px] text-[10px] leading-4 ${className}`}>
      {children}
    </span>
  );
}

// ---------------------------------------------------------------
// 概要
// ---------------------------------------------------------------

function SummaryBlocks({ summary, sampleCount }: { summary: LoadtestSummary; sampleCount: number }) {
  const { t } = useI18n();

  const total = summary.total;
  const statusTotal = summary.statuses.reduce((sum, s) => sum + s.count, 0);
  const errorTotal = summary.errors.reduce((sum, e) => sum + e.count, 0);

  return (
    <>
      {/* 该说的话排在数字前面：这几条都会改变数字的解读方式 */}
      {summary.cancelled ? <Notice tone="err">{t("loadtest.cancelledNotice")}</Notice> : null}
      {sampleCount === 0 && total > 0 ? (
        <Notice tone="warn">{t("loadtest.samplesPruned")}</Notice>
      ) : null}
      {summary.frozenFunctions.length > 0 ? (
        <Notice tone="warn">
          <span className="inline-flex items-center gap-1">
            <Snowflake size={11} strokeWidth={2.2} aria-hidden />
            {t("loadtest.frozenNotice")}
          </span>{" "}
          <span className="select-text font-mono">{summary.frozenFunctions.join("、")}</span>
        </Notice>
      ) : null}
      {summary.throughput.peakWindowIsPartial ? (
        <Notice tone="info">{t("loadtest.peakPartial")}</Notice>
      ) : null}

      <Section title={t("loadtest.summary")}>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
          <StatCard label={t("loadtest.totalRequests")} value={String(total)} />
          <StatCard label={t("loadtest.succeeded")} value={String(summary.succeeded)} tone="ok" />
          <StatCard
            label={t("loadtest.failed")}
            value={String(summary.failed)}
            tone={summary.failed > 0 ? "err" : "default"}
          />
          <StatCard
            label={t("loadtest.successRate")}
            // successRate 是 0~1 的小数，乘 100 是**显示层**的事
            value={(summary.successRate * 100).toFixed(1)}
            unit="%"
            tone={summary.failed === 0 ? "ok" : "default"}
          />
          <StatCard
            label={t("loadtest.tps")}
            value={summary.tps.toFixed(1)}
            hint={t("loadtest.tpsHint")}
          />
          <StatCard
            label={t("loadtest.rps")}
            value={summary.throughput.avgRps.toFixed(1)}
            hint={t("loadtest.rpsHint")}
          />
          <StatCard
            label={t("loadtest.peakRps")}
            value={summary.throughput.peakRps.toFixed(1)}
          />
          <StatCard
            label={t("loadtest.totalBytes")}
            value={formatBytesShort(summary.throughput.totalBytes)}
            hint={`${t("loadtest.avgBytes")} ${formatBytesShort(summary.throughput.avgBytes)}`}
          />
        </div>
      </Section>

      <Section title={t("loadtest.latency")}>
        {/* P95 那格给品牌色：它是压测报告里被引用最多的一个数 */}
        <div className="grid grid-cols-3 gap-2 sm:grid-cols-4 lg:grid-cols-7">
          <StatCard label={t("loadtest.min")} value={String(summary.latency.minMs)} unit="ms" />
          <StatCard label={t("loadtest.mean")} value={summary.latency.meanMs.toFixed(1)} unit="ms" />
          <StatCard label={t("loadtest.p50")} value={String(summary.latency.p50Ms)} unit="ms" />
          <StatCard label={t("loadtest.p90")} value={String(summary.latency.p90Ms)} unit="ms" />
          <StatCard label={t("loadtest.p95")} value={String(summary.latency.p95Ms)} unit="ms" tone="brand" />
          <StatCard label={t("loadtest.p99")} value={String(summary.latency.p99Ms)} unit="ms" />
          <StatCard label={t("loadtest.max")} value={String(summary.latency.maxMs)} unit="ms" />
        </div>
      </Section>

      <div className="grid gap-5 lg:grid-cols-2">
        <Section title={t("loadtest.statusDist")}>
          {summary.statuses.length === 0 ? (
            <p className="py-3 text-center text-[11px] text-ink-faint">{t("loadtest.noStatuses")}</p>
          ) : (
            <div className="flex flex-col gap-2">
              {summary.statuses.map((bucket) => {
                const noResponse = bucket.status === null;
                return (
                  <BarRow
                    key={bucket.status ?? "none"}
                    label={noResponse ? t("loadtest.noResponse") : String(bucket.status)}
                    value={`${bucket.count} · ${share(bucket.count, total)}`}
                    ratio={total === 0 ? 0 : bucket.count / total}
                    // 颜色按语义分档，与响应面板里的状态码配色同一套规则
                    barClass={
                      noResponse
                        ? "bg-ink-faint/50"
                        : bucket.status !== null && bucket.status >= 500
                          ? "bg-err"
                          : bucket.status !== null && bucket.status >= 400
                            ? "bg-warn"
                            : "bg-ok"
                    }
                  />
                );
              })}
              {statusTotal !== total ? (
                // 后端按状态码分桶，`总请求数 - 各桶之和` 应当为 0；
                // 不为 0 说明统计口径出了问题，宁可口径暴露出来也别悄悄圆上
                <p className="flex items-center gap-1 text-[10px] text-ink-faint">
                  <TriangleAlert size={10} strokeWidth={2.2} aria-hidden />
                  {t("loadtest.count")}: {statusTotal} / {total}
                </p>
              ) : null}
            </div>
          )}
        </Section>

        <Section title={t("loadtest.errorDist")}>
          {summary.errors.length === 0 ? (
            <p className="py-3 text-center text-[11px] text-ink-faint">
              {/* 有失败但没有错误码 = 全是 4xx/5xx（服务端答了，只是状态码不对） */}
              {summary.failed > 0 ? t("loadtest.noErrors") : t("loadtest.noFailures")}
            </p>
          ) : (
            <div className="flex flex-col gap-2">
              {summary.errors.map((bucket) => (
                <BarRow
                  key={bucket.code}
                  label={bucket.code}
                  value={`${bucket.count} · ${share(bucket.count, total)}`}
                  ratio={total === 0 ? 0 : bucket.count / total}
                  barClass="bg-err"
                />
              ))}
              {/* 错误分布**天生**比失败数少：服务端答了 4xx/5xx 是有响应的，
                  按设计只进状态码分布、不进错误分布（errors 回答的是
                  「为什么没拿到响应」）。所以这里给的是解释，不是对账告警 ——
                  把它当异常报出来会天天误报。 */}
              {errorTotal < summary.failed ? (
                <p className="text-[10px] leading-relaxed text-ink-faint">
                  {t("loadtest.errorGap", { count: summary.failed - errorTotal })}
                </p>
              ) : null}
            </div>
          )}
        </Section>
      </div>
    </>
  );
}

// ---------------------------------------------------------------
// 取值 / 格式化
// ---------------------------------------------------------------

/**
 * 这一轮压的是哪个地址。
 *
 * 快照形状是 `{ target: { method, url }, config: {...} }`。
 * 读不到就退回 `requestName`，再退回 `#id` —— 老记录或库被手改过时，
 * 一个「缺失」比一个空字符串好定位。
 */
function targetOf(run: LoadtestRun): string {
  const raw = run.config["target"];
  if (typeof raw === "object" && raw !== null) {
    const { method, url } = raw as Record<string, unknown>;
    if (typeof method === "string" && typeof url === "string") return `${method} ${url}`;
  }
  return run.requestName !== "" ? run.requestName : `#${run.id}`;
}

function share(count: number, total: number): string {
  return total === 0 ? "—" : `${((count / total) * 100).toFixed(1)}%`;
}

/** 后端给的是 ISO 字符串；解析不了就原样显示，别显示成 `Invalid Date` */
function formatTime(iso: string | null): string {
  if (iso === null) return "—";
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return iso;
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(at.getMonth() + 1)}-${p(at.getDate())} ${p(at.getHours())}:${p(at.getMinutes())}:${p(at.getSeconds())}`;
}

/** 毫秒 → 「1.24 s」/「820 ms」。与 `formatMs` 同口径，但压测里更常看秒 */
function formatDuration(ms: number): string {
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(2)} s`;
}

/** 传输量：压测动辄上百 MB，比 `formatBytes` 多给一位有效数字 */
function formatBytesShort(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
