import { Snowflake, TriangleAlert } from "lucide-react";

import { useI18n } from "@/lib/i18n";
import type {
  HttpVersion,
  LoadtestChart,
  LoadtestRunDetail,
  LoadtestRun,
  LoadtestSummary,
} from "@/lib/types";
import { Badge } from "@/features/ui/Badge";
import { ExportBar } from "./ExportBar";
import { formatDuration } from "./format";
import { HistogramChart } from "./HistogramChart";
import { runConfigOf, runTargetOf } from "./runSnapshot";
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

      {/* 这一轮**是怎么打的**。排在概要前面：后面每个数字都要靠它解读 ——
          「P95 285 ms」对 8 并发和 50 并发是两件事 */}
      <RunConfig run={run} />

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

/**
 * 报告头部：这一轮的地址与时间。
 *
 * ⚠️ **不再重复「第 #N 轮」那个标题**。报告区原来是请求面板里的一个页签，
 * 那时左侧没有列表，标题是唯一能说明「这是哪一轮」的东西。搬进压测主视图后，
 * 左边 `RunList` 里选中的那一行已经高亮了，再写一遍编号就成了同一句话两次。
 *
 * 保留下来的三样（地址、起止时间、状态标签）左侧列表**都没有** ——
 * 列表给的是方法 + 耗时 + P95 这类用来比较的量，而「这一轮是什么时候打的」
 * 只有在这里能看到。
 */
function RunHeader({ run, summary }: { run: LoadtestRun; summary: LoadtestSummary | null }) {
  const { t } = useI18n();
  const target = runTargetOf(run);

  return (
    <header className="flex flex-col gap-2">
      {/* 标签行只在真有时才出现 —— 否则会留下一个空的 flex 容器占 20px */}
      {run.finishedAt === null || summary?.cancelled === true ? (
        <div className="flex flex-wrap items-center gap-2">
          {run.finishedAt === null ? (
            <Badge tone="warn" size="md">
              {t("loadtest.unfinished")}
            </Badge>
          ) : null}
          {summary?.cancelled === true ? (
            <Badge tone="err" size="md">
              {t("loadtest.cancelledTag")}
            </Badge>
          ) : null}
        </div>
      ) : null}

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

// ---------------------------------------------------------------
// 这一轮的参数
// ---------------------------------------------------------------

/**
 * 归档里存下来的那八项配置。
 *
 * 导出的 HTML 报告一直有这一节（`loadtest/report.rs` 的 `html_config`），
 * 应用内的报告此前没有 —— 同一轮记录，导出到磁盘能看见「当时是多少并发打的」，
 * 点开报告反而看不见。这一节就是补上那个缺口。
 *
 * 取值全部来自**归档的快照**（`run.config`），不是上方那个实时配置区：
 * 历史轮次的意义正在于「当时配的是什么」，而上面那份随时会被改。
 */
function RunConfig({ run }: { run: LoadtestRun }) {
  const { t } = useI18n();
  const config = runConfigOf(run);

  // 一个键都没认出来 = 这不是我们存的那份快照（老记录 / 库被手改过）。
  // 整节不画，而不是画一排「—」：八行空值看起来像「这轮什么参数都没配」
  if (Object.keys(config).length === 0) return null;

  const httpVersionText = (v: HttpVersion) =>
    v === "http1"
      ? t("loadtest.httpVersionHttp1")
      : v === "http2"
        ? t("loadtest.httpVersionHttp2")
        : t("loadtest.httpVersionAuto");

  return (
    <Section title={t("loadtest.runConfig")}>
      <dl className="flex flex-wrap gap-x-4 gap-y-1 text-[10px] text-ink-faint">
        {config.concurrency !== undefined ? (
          <Meta label={t("loadtest.concurrency")} value={String(config.concurrency)} />
        ) : null}
        {config.total !== undefined ? (
          <Meta label={t("loadtest.requests")} value={String(config.total)} />
        ) : null}
        {/* 超时与间隔给**原始毫秒**：与上面配置区那两个输入框同单位。
            这一节回答的是「当时填的是几」，换个单位就得让人自己换算 */}
        {config.timeoutMs !== undefined ? (
          <Meta label={t("loadtest.timeout")} value={String(config.timeoutMs)} />
        ) : null}
        {config.thinkTimeMs !== undefined ? (
          <Meta label={t("loadtest.thinkTime")} value={String(config.thinkTimeMs)} />
        ) : null}
        {config.warmupRequests !== undefined ? (
          <Meta label={t("loadtest.warmup")} value={String(config.warmupRequests)} />
        ) : null}
        {config.httpVersion !== undefined ? (
          <Meta label={t("loadtest.httpVersion")} value={httpVersionText(config.httpVersion)} />
        ) : null}
        {config.keepAlive !== undefined ? (
          <Meta
            label={t("loadtest.keepAlive")}
            value={config.keepAlive ? t("loadtest.yes") : t("loadtest.no")}
          />
        ) : null}
        {config.followRedirects !== undefined ? (
          <Meta
            label={t("loadtest.followRedirects")}
            value={config.followRedirects ? t("loadtest.yes") : t("loadtest.no")}
          />
        ) : null}
      </dl>
    </Section>
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
 * 一个占比（`4 / 100` → `「4.0%」`）。
 *
 * 与 [`formatPercent`] 不同：那个吃的是已经是小数的 `successRate`，
 * 这个吃的是「几 / 共几」。两种入参混在一处最容易错，所以分成两个名字。
 */
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

/** 传输量：压测动辄上百 MB，比 `formatBytes` 多给一位有效数字 */
function formatBytesShort(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
