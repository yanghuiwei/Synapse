import { useMemo } from "react";

import { useTheme } from "@/lib/useTheme";

/**
 * canvas 用的调色板。
 *
 * ⚠️ 这里是「禁止硬编码色值」这条铁律唯一需要额外解释的地方 —— 不是例外，
 * 而是**换了一种取法**：uPlot 把图画在 `<canvas>` 上，拿不到 Tailwind 类，
 * 所以只能读 `theme.css` 里那些自定义属性的**实际值**。
 * 色值的唯一来源仍然是 `theme.css`，下面这些只是运行时把它们取出来。
 *
 * 由此带来一个后果：**画布不会跟着CSS 变量自动重绘**。主题一变，
 * 画布上还是旧颜色 —— 所以 `useChartPalette` 会订阅主题（明暗 **与** 配色），
 * 让调用方带着新的 palette 重建图表（见 `TimelineChart` 的 effect 依赖）。
 */
export interface ChartPalette {
  brand: string;
  ok: string;
  err: string;
  warn: string;
  info: string;
  ink: string;
  inkMuted: string;
  inkFaint: string;
  line: string;
  surface: string;
}

/**
 * 取不到令牌时的兜底。
 *
 * 只在「DOM 还没挂」或「令牌名写错了」时生效。写错了不该静默变成透明 ——
 * 透明线画出来是「图是空的」，比颜色不对更难查，所以这里给的是可见的中性灰。
 */
const FALLBACK: ChartPalette = {
  brand: "#2563eb",
  ok: "#16a34a",
  err: "#dc2626",
  warn: "#d97706",
  info: "#0891b2",
  ink: "#0f172a",
  inkMuted: "#475569",
  inkFaint: "#94a3b8",
  line: "#e2e8f0",
  surface: "#ffffff",
};

/** 令牌名 → palette 字段 */
const TOKENS: Record<keyof ChartPalette, string> = {
  brand: "--brand",
  ok: "--ok",
  err: "--err",
  warn: "--warn",
  info: "--info",
  ink: "--ink",
  inkMuted: "--ink-muted",
  inkFaint: "--ink-faint",
  line: "--line",
  surface: "--surface",
};

function readToken(name: string, fallback: string): string {
  if (typeof document === "undefined") return fallback;
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return raw === "" ? fallback : raw;
}

/**
 * 当前主题下的画布调色板。
 *
 * 依赖 `resolved` **与** `accent`：
 * 前者是因为 `mode === "system"` 时切系统主题画布也得跟着换；
 * 后者是因为配色（accent）是与明暗正交的第二个维度 —— 只盯 resolved
 * 会漏掉「深色下把靛青换成藕粉」这种情况，而画布上还是旧颜色。
 * 少一个依赖的表现是**颜色不对但不报错**：图表照画，只是配色像上一套。
 */
export function useChartPalette(): ChartPalette {
  const { resolved, accent } = useTheme();
  return useMemo(() => {
    const out = {} as ChartPalette;
    for (const key of Object.keys(TOKENS) as (keyof ChartPalette)[]) {
      out[key] = readToken(TOKENS[key], FALLBACK[key]);
    }
    return out;
  }, [resolved, accent]);
}

/**
 * 图表用的字体。与 `--type-mono` 同源，但 canvas 只认 CSS 的 font 简写，
 * 且**不会**解析 var() —— 所以这里必须展开成实际字体名。
 */
export function chartFont(sizePx: number): string {
  const stack = readToken("--type-mono", "ui-monospace, monospace");
  return `${sizePx}px ${stack}`;
}
