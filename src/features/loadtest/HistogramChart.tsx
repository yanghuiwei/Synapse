import { cn } from "@/lib/cn";
import { formatMs } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import type { HistogramBucket } from "@/lib/types";
import { useChartPalette } from "./chartPalette";

/**
 * 延迟分布直方图。
 *
 * 为什么不用 uPlot 画：uPlot 的柱状图是「按时间轴排布」的，
 * 而这个直方图的横轴是**桶区间**（2 的幂），桶宽不等 —— 用 uPlot 得把
 * 区间中点当坐标硬塞，于是「1-2ms」和「512-1024ms」在屏幕上一样宽，
 * 一眼看上去会以为两边样本量可比。手绘 SVG 反而更短也更诚实。
 *
 * 分位数（P50/P95/P99）回答「大部分人有多快」，直方图回答
 * 「快慢是怎么分布的」—— 同一条 P95 可能是「全都 95ms」也可能是
 * 「95% 的请求 1ms + 5% 的请求 2s」。两个数必须一起看。
 */

// 视图坐标系。用固定 viewBox + `w-full h-auto` 等比缩放，
// 这样容器一改宽度图表就跟着变，不需要 ResizeObserver
const VB_W = 600;
const VB_H = 160;
const PLOT_TOP = 16;
const PLOT_BOTTOM = 134;
const PLOT_LEFT = 34;
const PLOT_RIGHT = 596;

/** 横轴标签在桶多的时候隔一个显示，否则会糊成一片 */
const LABEL_CROWD_THRESHOLD = 10;

export function HistogramChart({
  buckets,
  className,
}: {
  buckets: readonly HistogramBucket[];
  className?: string;
}) {
  const { t } = useI18n();
  const palette = useChartPalette();

  if (buckets.length === 0) {
    return (
      <p className={cn("py-6 text-center text-[11px] text-ink-faint", className)}>
        {t("loadtest.noHistogram")}
      </p>
    );
  }

  const total = buckets.reduce((sum, b) => sum + b.count, 0);
  const peak = buckets.reduce((max, b) => Math.max(max, b.count), 0);
  // 峰值是 0 时（整轮全失败、没有一条成功样本）所有柱子高度都会是 0，
  // 除以 0 会得到 NaN 并让整张 SVG 的 path 变成非法值
  const scaleMax = peak === 0 ? 1 : peak;

  const slot = (PLOT_RIGHT - PLOT_LEFT) / buckets.length;
  const barW = Math.max(2, Math.min(slot - 2, slot * 0.72));
  const plotH = PLOT_BOTTOM - PLOT_TOP;
  const showEveryLabel = buckets.length <= LABEL_CROWD_THRESHOLD ? 1 : 2;

  return (
    <div className={cn("flex flex-col gap-1", className)}>
      <svg
        viewBox={`0 0 ${VB_W} ${VB_H}`}
        role="img"
        aria-label={t("loadtest.chartHistogram")}
        className="h-auto w-full overflow-visible"
      >
        {/* 峰值刻度线。只有一条 —— 分布图的目的是看形状，不是读精确值 */}
        <line
          x1={PLOT_LEFT}
          x2={PLOT_RIGHT}
          y1={PLOT_TOP}
          y2={PLOT_TOP}
          stroke={palette.line}
          strokeWidth={1}
        />
        <text x={PLOT_LEFT - 4} y={PLOT_TOP + 3} textAnchor="end" fontSize={9} fill={palette.inkFaint}>
          {peak}
        </text>

        {/* 基线 */}
        <line
          x1={PLOT_LEFT}
          x2={PLOT_RIGHT}
          y1={PLOT_BOTTOM}
          y2={PLOT_BOTTOM}
          stroke={palette.line}
          strokeWidth={1}
        />

        {buckets.map((bucket, i) => {
          const h = (bucket.count / scaleMax) * plotH;
          const x = PLOT_LEFT + slot * i + (slot - barW) / 2;
          const y = PLOT_BOTTOM - h;
          const share = total === 0 ? 0 : bucket.count / total;
          const last = i === buckets.length - 1;

          return (
            <g key={`${bucket.lowerMs}-${bucket.upperMs}`}>
              {/* 命中区铺满整个槽位：柱子本身可能只有 2px 宽，鼠标根本点不上去 */}
              <rect
                x={PLOT_LEFT + slot * i}
                y={PLOT_TOP}
                width={slot}
                height={plotH}
                fill="transparent"
              >
                <title>
                  {`${formatMs(bucket.lowerMs)} – ${last ? "+" : formatMs(bucket.upperMs)}：${bucket.count} (${(share * 100).toFixed(1)}%)`}
                </title>
              </rect>
              <rect
                x={x}
                y={y}
                width={barW}
                height={Math.max(h, bucket.count > 0 ? 1 : 0)}
                rx={1}
                fill={palette.brand}
                opacity={0.85}
              />
              {i % showEveryLabel === 0 ? (
                <text
                  x={PLOT_LEFT + slot * i + slot / 2}
                  y={PLOT_BOTTOM + 12}
                  textAnchor="middle"
                  fontSize={9}
                  fill={palette.inkFaint}
                >
                  {formatMs(bucket.lowerMs)}
                </text>
              ) : null}
            </g>
          );
        })}
      </svg>

      <p className="text-[10px] text-ink-faint">{t("loadtest.histogramHint", { count: buckets.length })}</p>
    </div>
  );
}
