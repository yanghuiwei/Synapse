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

/**
 * 指标数字的颜色。
 *
 * ⚠️ 只有 `ok` / `err` 两档带底色，`brand` / `warn` 刻意**不带**：
 * 理由是「底色 = 这一格值得多看一眼」。
 * 压测报告里成功/失败是两个对立的事实，给底色能把它们一眼分开；
 * 而 P95 是「所有分位数里被引用最多的一个」，真正该突出的是**它** ——
 * 但一屏里到处是淡蓝底就会失去强调意义，所以 P95 仍靠品牌色的数字本身。
 * （真要强调 P95，正确做法是换布局，不是换底色。）
 *
 * 带底色的两档用 `text-x-on`（压深一档），没底色的用 `text-x` 原色。
 * 数字是 17px，AA 大字门槛 3:1 下 `text-ok` 其实也够（2.95 ~ 临界），
 * 但**同屏里两套绿**（带底色的压深、没底色的原色）本身就是不一致，
 * 视觉上会看出「这两个绿不是同一个绿」。所以带底色的一律用 on 档。
 */
const TONE_VALUE: Record<StatTone, string> = {
  default: "text-ink",
  ok: "text-ok-on",
  err: "text-err-on",
  brand: "text-brand",
  warn: "text-warn",
};

/**
 * 哪些档位带底色。
 *
 * 单独一张表而不是从 TONE_VALUE 里 `includes("bg-")` 嗅探：
 * 那种写法在有人把某个 tone 的类串改一下时就静默失效（底色跟着边框一起没了，
 * 构建不报错），而这里是显式的「带 / 不带」清单。
 */
const TONE_FILLED: Record<StatTone, boolean> = {
  default: false,
  ok: true,
  err: true,
  brand: false,
  warn: false,
};

/** 带底色那两档的边框。实色边框 + 实色底，边界才不会靠透明度猜 */
const TONE_FILLED_SURFACE: Record<"ok" | "err", string> = {
  ok: "bg-ok-subtle border-ok/25",
  err: "bg-err-subtle border-err/25",
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
        "flex min-w-0 flex-col gap-1 rounded-md border px-2.5 py-2",
        TONE_FILLED[tone]
          ? cn("bg-surface", TONE_FILLED_SURFACE[tone as "ok" | "err"])
          : "border-line bg-surface",
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

/**
 * 提示条的底色走 `--x-subtle`（x 取 ok/warn/err…）那档实色令牌，**不写** `bg-warn/8`。
 *
 * 原来这里是 `bg-warn/8` + `border-warn/35` —— 8% 的品牌色是**半透明**的，
 * 于是它压到什么底上就变成什么色。压测报告区底色是 `canvas`，
 * 而同样的 `<Notice tone="warn">` 若被放进 `bg-surface` 的卡片里，
 * 底色会明显偏浅 —— 同一条提示在两个位置长得不一样，且**不报任何错**。
 * 实色令牌让底色与它下面的东西无关。
 *
 * 边框保留半透明（`border-x/35`）是有意的：边框只负责「圈出这一块」，
 * 不该跟着底色一起变实，否则小字块会显得比正文还重。
 */
const NOTICE_TONE: Record<NoticeTone, string> = {
  warn: "border-warn/35 bg-warn-subtle text-warn-on",
  err: "border-err/35 bg-err-subtle text-err-on",
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
