import { cn } from "@/lib/cn";
import { statusTextClass } from "@/lib/http";
import { formatSeconds } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import type { FailureLine, LoadtestConfig, LoadtestProgress, TimelinePoint } from "@/lib/types";
import type { LivePoint } from "@/store/loadtest";
import { StatCard } from "./primitives";
import { TimelineChart } from "./TimelineChart";

/**
 * 跑着的时候看的那一块。
 *
 * 两个刻意的选择：
 *
 * 1. **进度条按「已发送 / 总数」而不是按时间**。压测没有已知时长（那是
 *    「持续时间」模型的产物，design.md §7.1 已经否掉了），而请求数是确定的 ——
 *    所以进度是**真的**能算出来的百分比，不是一个猜出来的动画。
 * 2. **失败日志必须在跑的时候就看得见**，不能等报告。压测里最常见的现场是
 *    「RPS 上不去，因为一半请求在超时」—— 等 5 分钟跑完才知道，这 5 分钟就白等了。
 */
export function LiveStats({
  config,
  progress,
  livePoints,
}: {
  config: LoadtestConfig;
  progress: LoadtestProgress | null;
  livePoints: readonly LivePoint[];
}) {
  const { t } = useI18n();

  const sent = progress?.sent ?? 0;
  const succeeded = progress?.succeeded ?? 0;
  const pct = config.total === 0 ? 0 : Math.min(100, (sent / config.total) * 100);
  // 后端 `StatsAccumulator::record` 先 `sent += 1` 再按状态分到 succeeded / failed，
  // 所以 **`sent === succeeded + failed`**（是「已拿到结果」的意思，不是「已派发」）。
  // 分母用它而不是另算一个和：加法的口径一旦漂了，进度和成功率会对不上
  const successRate = sent === 0 ? null : (succeeded / sent) * 100;

  const timeline: TimelinePoint[] = livePoints.map((p) => ({
    second: p.second,
    count: Math.round(p.rps),
    p95Ms: p.p95Ms,
  }));

  const failures = progress?.recentFailures ?? [];

  return (
    <div className="flex flex-col gap-4">
      {/* 进度条 + 已用时 */}
      <div className="flex flex-col gap-1.5">
        <div className="flex items-baseline gap-2 text-[11px]">
          <span className="tabular font-mono text-ink">
            {t("loadtest.progress", { sent, total: config.total })}
          </span>
          <span className="tabular font-mono text-[10px] text-ink-faint">
            {pct.toFixed(1)}%
          </span>
          {progress !== null ? (
            <span className="ml-auto text-[10px] text-ink-faint">
              {t("loadtest.elapsed", { value: formatSeconds(progress.elapsedMs / 1000) })}
            </span>
          ) : null}
        </div>
        <div
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={config.total}
          aria-valuenow={sent}
          className="relative h-1.5 w-full overflow-hidden rounded-full bg-canvas-subtle"
        >
          <span
            aria-hidden
            style={{ width: `${pct}%` }}
            className="absolute inset-y-0 left-0 rounded-full bg-brand transition-[width] duration-200 ease-out"
          />
        </div>
      </div>

      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        <StatCard label={t("loadtest.cardDone")} value={String(sent)} />
        <StatCard
          label={t("loadtest.cardSuccessRate")}
          value={successRate === null ? "—" : successRate.toFixed(1)}
          unit={successRate === null ? undefined : "%"}
          tone={successRate === null ? "default" : successRate >= 99 ? "ok" : "warn"}
        />
        <StatCard
          label={t("loadtest.cardRps")}
          value={(progress?.currentRps ?? 0).toFixed(1)}
          tone="brand"
        />
        <StatCard
          label={t("loadtest.cardP95")}
          value={String(progress?.p95Ms ?? 0)}
          unit="ms"
        />
      </div>

      {/* 失败日志排在图表**前面**：跑着的时候，先看到「正在坏什么」
          比先看到趋势曲线有用。而且它正常情况只有一行字（没有失败），
          真有失败时它在第一屏而不是要往下滚 */}
      <FailureLog failures={failures} />

      <TimelineChart points={timeline} height={132} />
    </div>
  );
}

/**
 * 失败日志环。
 *
 * 只显示**最近 50 条**（后端就推这么多）。刻意不做「加载全部」：
 * 跑着的时候要的是「现在正在坏什么」，全量那是报告的事。
 */
function FailureLog({ failures }: { failures: readonly FailureLine[] }) {
  const { t } = useI18n();

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2">
        <h4 className="text-[11px] font-semibold tracking-wide text-ink-muted">
          {t("loadtest.failureLog")}
        </h4>
        {failures.length > 0 ? (
          <span className="tabular rounded-full bg-err/10 px-1.5 text-[10px] leading-4 text-err">
            {failures.length}
          </span>
        ) : null}
        <span className="ml-auto text-[10px] text-ink-faint">{t("loadtest.failureLogHint")}</span>
      </div>

      {failures.length === 0 ? (
        <p className="rounded-sm border border-line bg-canvas-subtle px-2 py-1.5 text-[11px] text-ink-faint">
          {t("loadtest.failureLogEmpty")}
        </p>
      ) : (
        // max-h 压到 112px（约 5 行）：这一屏还要留给下面的折线图。
        // 滚动看更多失败是能接受的，图表被挤没是不能接受的
        <ul className="flex max-h-28 flex-col gap-px overflow-y-auto rounded-sm border border-line">
          {/* 后端按发生顺序推，最新的在最前面读起来才顺 —— 这里翻一次，
              而不是让用户滚到底 */}
          {failures
            .slice()
            .reverse()
            .map((f) => (
              <li
                key={f.seq}
                className={cn(
                  "flex items-center gap-2 px-2 py-1 font-mono text-[10px]",
                  "border-b border-line/60 last:border-b-0",
                )}
              >
                <span className="tabular w-10 shrink-0 text-ink-faint">#{f.seq}</span>
                <span className="w-[92px] shrink-0 truncate">
                  {f.status === null ? (
                    <span className="text-ink-faint">{t("loadtest.noResponse")}</span>
                  ) : (
                    <span className={statusTextClass(f.status)}>{f.status}</span>
                  )}
                </span>
                <span className="min-w-0 flex-1 truncate font-sans text-err">
                  {f.code ?? ""}
                </span>
                <span className="tabular shrink-0 text-ink-muted">{f.durationMs} ms</span>
                <span className="tabular shrink-0 text-ink-faint">@{f.offsetMs} ms</span>
              </li>
            ))}
        </ul>
      )}
    </div>
  );
}
