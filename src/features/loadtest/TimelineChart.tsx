import { useEffect, useMemo, useRef, useState } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { TimelinePoint } from "@/lib/types";
import { chartFont, useChartPalette } from "./chartPalette";

/**
 * 绘图区默认高度（不含标题与图例）。
 *
 * 报告里给 168px：那是要被反复看的图。
 * 实时区给矮一点（见 `LiveStats`）—— 那一屏还要放进度、四张卡和失败日志，
 * 而实时图看的是「刚才有没有抖」，不需要报告那么高。
 */
const DEFAULT_HEIGHT = 168;

/**
 * 时间线：左轴 P95（毫秒）、右轴每秒请求数。
 *
 * 两条线画一张图而不是上下两张，是因为**它们要一起看**：
 * 「RPS 上不去而 P95 在涨」和「RPS 上不去、P95 也很平」是两个完全不同的结论
 * （前者是服务端被压住了，后者是客户端没打出去）。分两张图并排，
 * 眼睛得来回对时间轴，很容易看错。
 *
 * 关于 uPlot 的三个坑，都踩过了：
 *
 * 1. **画布不认 CSS 变量**。颜色只能把 `--brand`/`--ok` 的**实际值**读出来，
 *    见 `chartPalette.ts`。因此主题一变必须**重建**这整个图 —— 改 opts 没用。
 * 2. **`resize` 得自己接**。uPlot 不会自动跟着容器变宽（它的 `setSize` 是命令式的）。
 *    这里用 `ResizeObserver` 顶住分栏拖拽。
 * 3. **数据必须等宽**。`[xs, p95[], rps[]]` 三个数组长度必须一致 ——
 *    少一个就会画出一段截断的线，而且**不报错**。
 */
export function TimelineChart({
  points,
  height = DEFAULT_HEIGHT,
  className,
}: {
  points: readonly TimelinePoint[];
  height?: number;
  className?: string;
}) {
  const { t } = useI18n();
  const palette = useChartPalette();

  const hostRef = useRef<HTMLDivElement>(null);
  const plotRef = useRef<uPlot | null>(null);
  const [width, setWidth] = useState(0);

  const data = useMemo<uPlot.AlignedData>(() => {
    const xs: number[] = [];
    const p95: number[] = [];
    const rps: number[] = [];
    for (const p of points) {
      xs.push(p.second);
      p95.push(p.p95Ms);
      rps.push(p.count);
    }
    return [xs, p95, rps];
  }, [points]);

  // 最新一份数据。用它给「重建图表」那条路径兜底：
  // 重建发生在宽度/主题变化时，而那一刻 `setData` 的 effect 不会重跑
  // （它的依赖 `data` 没变），不补这一手的话重建出来的图是空的
  const dataRef = useRef(data);

  useEffect(() => {
    dataRef.current = data;
  }, [data]);

  // 容器宽度。分栏可以拖，所以不能只在挂载时量一次
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const measure = () => setWidth(Math.floor(host.clientWidth));
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(host);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    const host = hostRef.current;
    if (!host || width <= 0) return;

    const font = chartFont(10);
    const axisBase = {
      stroke: palette.inkFaint,
      font,
      gap: 4,
      ticks: { stroke: palette.line, size: 4 },
    } as const;

    const plot = new uPlot(
      {
        width,
        height,
        // 顶部留一点，否则最高那条线会贴着边框；右/下由坐标轴自己撑开
        padding: [6, 0, 0, 0],
        legend: { show: false },
        // 关掉 y 向的十字准线：两条线在不同的量纲上，横向准线会误导
        cursor: { y: false, points: { show: false } },
        scales: {
          x: { time: false },
          y: { auto: true },
          rps: { auto: true },
        },
        axes: [
          {
            ...axisBase,
            size: 22,
            grid: { stroke: palette.line, width: 1 },
            // 横轴是「第几秒」，光秃秃的数字会被当成别的量纲
            values: (_u, splits) => splits.map((s) => `${s}s`),
          },
          {
            ...axisBase,
            scale: "y",
            side: 3,
            size: 40,
            grid: { stroke: palette.line, width: 1 },
          },
          {
            ...axisBase,
            scale: "rps",
            side: 1,
            size: 40,
            // 右轴不画网格：两套网格线叠在绘图区里会变成一张窗户纸
            grid: { show: false },
          },
        ],
        series: [
          {},
          {
            label: t("loadtest.seriesP95"),
            scale: "y",
            stroke: palette.brand,
            width: 1.8,
            points: { show: false },
          },
          {
            label: t("loadtest.seriesRps"),
            scale: "rps",
            stroke: palette.ok,
            width: 1.4,
            points: { show: false },
            // 虚线：两条实线在色觉障碍下不好分辨
            dash: [4, 3],
          },
        ],
      },
      dataRef.current,
      host,
    );

    plotRef.current = plot;
    return () => {
      plot.destroy();
      plotRef.current = null;
    };
    // 颜色进了 canvas，主题一变只能重建；文案进了 opts（series label），同理；
    // 尺寸也是构造期的参数（后面只改 width 的话，高度会停在旧值上）
  }, [width, height, palette, t]);

  useEffect(() => {
    plotRef.current?.setData(data, true);
  }, [data]);

  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      {/* 自己画图例：uPlot 的图例是一张 HTML 表，跟这个面板的排版风格对不上。
          左边那条必须写清是 **P95** 而不是笼统的「毫秒」—— 只写单位的话，
          用户会以为蓝线是平均延迟 */}
      <div className="flex items-center gap-3 text-[10px] text-ink-muted">
        <span className="flex items-center gap-1.5">
          <span aria-hidden className="h-[2px] w-4 rounded-full bg-brand" />
          {t("loadtest.seriesP95")} · {t("loadtest.chartYAxisLatency")}
        </span>
        <span className="flex items-center gap-1.5">
          <span aria-hidden className="h-0 w-4 border-t-2 border-dashed border-ok" />
          {t("loadtest.seriesRps")}
        </span>
      </div>

      <div className="relative">
        <div ref={hostRef} className="w-full" />
        {points.length === 0 ? (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-[11px] text-ink-faint">
            {t("loadtest.chartEmpty")}
          </div>
        ) : null}
      </div>
    </div>
  );
}
