import { ChevronDown, Info } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "@/lib/cn";

/**
 * 压测面板里反复出现的几种小块。
 *
 * 收在一个文件里而不是各写各的：压测界面上的「卡片 / 分段 / 行 / 提示条」
 * 加起来有七八处，分散到各组件里迟早会出现两种圆角、两种字号。
 */

// ---------------------------------------------------------------
// 卡片
// ---------------------------------------------------------------

export type StatTone = "default" | "ok" | "err" | "brand" | "warn";

const TONE_VALUE: Record<StatTone, string> = {
  default: "text-ink",
  ok: "text-ok",
  err: "text-err",
  brand: "text-brand",
  warn: "text-warn",
};

export interface StatCardProps {
  label: string;
  value: string;
  /** 单位。与 value 分开，好让数字本身右对齐等宽 */
  unit?: string;
  tone?: StatTone;
  /** 悬停解释。指标口径是这个界面上最容易误解的地方，值得说明 */
  hint?: string;
  className?: string;
}

/**
 * 一枚指标卡。
 *
 * `hint` 走原生 `title` 而不是自绘 tooltip：这个面板里的说明都是
 * 「看一眼口径」级别的短句，原生提示零成本、键盘和读屏都能拿到，
 * 不值得为此引一套浮层。
 */
export function StatCard({ label, value, unit, tone = "default", hint, className }: StatCardProps) {
  return (
    <div
      title={hint}
      className={cn(
        "flex min-w-0 flex-col gap-1 rounded-md border border-line bg-surface px-2.5 py-2",
        className,
      )}
    >
      <span className="flex items-center gap-1 text-[10px] leading-none text-ink-faint">
        <span className="truncate">{label}</span>
        {hint ? <Info size={9} strokeWidth={2.2} aria-hidden className="shrink-0 opacity-70" /> : null}
      </span>
      <span className="flex items-baseline gap-1">
        <span className={cn("tabular truncate font-mono text-[17px] leading-tight", TONE_VALUE[tone])}>
          {value}
        </span>
        {unit ? <span className="shrink-0 text-[10px] text-ink-faint">{unit}</span> : null}
      </span>
    </div>
  );
}

// ---------------------------------------------------------------
// 分段
// ---------------------------------------------------------------

export interface SectionProps {
  title: string;
  /** 标题右侧（比如一个「导出」按钮） */
  aside?: ReactNode;
  children: ReactNode;
  className?: string;
  /** 可折叠：标题变成按钮，展开/收起由调用方控制 */
  collapsible?: boolean;
  open?: boolean;
  onToggle?: () => void;
  /**
   * 收起时在标题右边顶上来的一行紧凑信息。
   *
   * 存在的理由：收起配置区是为了给报告腾地方，但**不能顺便把参数藏掉** ——
   * 「这一轮打的是什么参数」是解读报告数字的前提。一行摘要两头都顾上。
   */
  summary?: ReactNode;
}

/** 报告里的一个小节。用 `h3` 而不是 `h2`：`h2` 是侧栏/面板标题那一级 */
export function Section({
  title,
  aside,
  children,
  className,
  collapsible = false,
  open = true,
  onToggle,
  summary,
}: SectionProps) {
  const expanded = !collapsible || open;

  return (
    <section className={cn("flex flex-col gap-2", className)}>
      <header className="flex items-center gap-2">
        {collapsible ? (
          <button
            type="button"
            onClick={onToggle}
            aria-expanded={open}
            className={cn(
              "flex min-w-0 shrink-0 items-center gap-1.5",
              "text-[11px] font-semibold tracking-wide text-ink-muted",
              "transition-colors duration-100 hover:text-ink",
            )}
          >
            <ChevronDown
              size={12}
              strokeWidth={2.4}
              aria-hidden
              className={cn("shrink-0 transition-transform duration-150", !open && "-rotate-90")}
            />
            <span className="truncate">{title}</span>
          </button>
        ) : (
          <h3 className="text-[11px] font-semibold tracking-wide text-ink-muted">{title}</h3>
        )}

        {!expanded && summary !== undefined ? (
          <span className="tabular min-w-0 truncate font-mono text-[10px] text-ink-faint">
            {summary}
          </span>
        ) : null}

        <div className="h-px min-w-0 flex-1 bg-line" />
        {aside}
      </header>
      {expanded ? children : null}
    </section>
  );
}

// ---------------------------------------------------------------
// 表单行
// ---------------------------------------------------------------

export interface FieldProps {
  label: string;
  hint?: string;
  /** 控件。宽度由这里决定 —— 有的要填满，有的只有几十像素 */
  children: ReactNode;
  /**
   * 标签固定宽度，让一列数字框对齐。
   *
   * 118 是量出来的，不是拍的：最长的标签「单请求超时（毫秒）」是 10 个汉字，
   * 11px 字号下约 110px。宽度给到 96 时它会被 `truncate` 切成
   * 「单请求超时（毫…」——**构建不报错、只在像素上看得出来**。
   */
  labelWidth?: number;
}

export function Field({ label, hint, children, labelWidth = 118 }: FieldProps) {
  return (
    <label className="flex items-center gap-2" title={hint ?? label}>
      <span
        style={{ width: labelWidth }}
        className="shrink-0 truncate text-[11px] text-ink-muted"
      >
        {label}
      </span>
      {children}
    </label>
  );
}

/** 面板里数字/文本输入框的统一长相 */
export const FIELD_INPUT_CLASS = cn(
  "h-7 rounded-sm border border-line bg-surface px-2",
  "font-mono text-xs text-ink outline-none",
  "transition-colors duration-100 hover:border-line-strong focus:border-brand",
);

// ---------------------------------------------------------------
// 横向条形（状态码分布 / 错误分布 / 直方图共用）
// ---------------------------------------------------------------

export interface BarRowProps {
  /** 左列文字（状态码、错误码、区间） */
  label: ReactNode;
  /** 右列文字（次数、占比） */
  value: ReactNode;
  /** 0 ~ 1 的填充比例 */
  ratio: number;
  /** 条的颜色类（`bg-ok` / `bg-err` …）。默认中性 */
  barClass?: string;
  onSelect?: () => void;
  selected?: boolean;
}

/**
 * 一行「标签 + 条 + 数值」。
 *
 * 条用绝对定位的 div 而不是 `<progress>` / `<meter>`：后两者的默认外观
 * 在 WebView 里各平台不一致，且要改色得写一堆 `::-webkit-*` 前缀。
 */
export function BarRow({
  label,
  value,
  ratio,
  barClass = "bg-ink-faint/45",
  onSelect,
  selected = false,
}: BarRowProps) {
  const pct = Math.max(0, Math.min(1, ratio)) * 100;
  const interactive = onSelect !== undefined;

  const inner = (
    <>
      <span className="flex w-full items-center gap-2 text-[11px]">
        <span className="shrink-0 font-mono text-ink-muted">{label}</span>
        <span className="tabular ml-auto shrink-0 font-mono text-[10px] text-ink-faint">{value}</span>
      </span>
      <span className="relative block h-1.5 w-full overflow-hidden rounded-full bg-canvas-subtle">
        <span
          aria-hidden
          style={{ width: `${pct}%` }}
          className={cn("absolute inset-y-0 left-0 rounded-full", barClass)}
        />
      </span>
    </>
  );

  if (!interactive) {
    return <div className="flex w-full flex-col gap-1">{inner}</div>;
  }

  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      className={cn(
        "flex w-full flex-col gap-1 rounded-sm px-1 py-0.5 text-left transition-colors duration-75 hover:bg-surface-hover",
        selected && "bg-surface-hover",
      )}
    >
      {inner}
    </button>
  );
}

// ---------------------------------------------------------------
// 提示条
// ---------------------------------------------------------------

export type NoticeTone = "warn" | "err" | "info";

const NOTICE_TONE: Record<NoticeTone, string> = {
  warn: "border-warn/35 bg-warn/8 text-warn",
  err: "border-err/35 bg-err/8 text-err",
  info: "border-line bg-canvas-subtle text-ink-muted",
};

/**
 * 一句必须被看到的话。
 *
 * 「这一轮被取消了」「明细已被清理」「动态占位符被冻结」这三条都是
 * **会改变数字解读方式**的信息 —— 不能藏在 tooltip 里。
 */
export function Notice({
  tone = "info",
  children,
  className,
}: {
  tone?: NoticeTone;
  children: ReactNode;
  className?: string;
}) {
  return (
    <p
      role={tone === "info" ? undefined : "note"}
      className={cn(
        "rounded-sm border px-2 py-1.5 text-[11px] leading-relaxed",
        NOTICE_TONE[tone],
        className,
      )}
    >
      {children}
    </p>
  );
}
