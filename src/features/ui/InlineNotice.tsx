import { CircleCheck, Info, TriangleAlert } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "@/lib/cn";

/**
 * 横贯面板的一条提示带（未解析占位符 / Token 使用结果 / 响应过大 / curl 失败…）。
 *
 * ## 为什么值得单独抽一个组件
 *
 * 这类提示带在本项目里出现得**很早、也很多** —— 请求区 4 条、响应区 4 条、
 * 压测区若干条。抽出来之前它们各自写着：
 *
 * ```
 * border-b border-warn/40 bg-warn/10   ← UnresolvedBanner
 * border-b border-warn/10 bg-warn/8    ← ResponseViewer
 * border-b border-line  bg-warn/8      ← HeadersTab
 * border-b border-line  bg-warn/10     ← PlaceholderInput
 * ```
 *
 * 底色透明度在 `/8 /10 /15` 三个值之间飘、边框透明度在 `/30 /35 /40` 之间飘，
 * 而这些条**挨在一起显示**（请求区上方一条、响应区上方一条、中间还可能夹一条）。
 * 三种透明度压在各自的 `bg-canvas` / `bg-surface` 上，
 * 于是「同一个警告」在屏幕上呈现出**三个略微不同的黄**。
 * 这类差异构建期不报错、review 也看不出来，但用户能感觉到「这界面好像不太齐」。
 *
 * 底色一律走 `--x-subtle` 实色令牌后，条与条之间的差异只剩语义（warn / err / info），
 * 视觉上就齐了。
 */
export type NoticeTone = "warn" | "err" | "info" | "ok" | "brand";

/**
 * tone → 类。
 *
 * 底色走 `--x-subtle` **实色**令牌，不写 `bg-warn/8`：
 * 原来这里是半透明的，于是压到什么底上就变成什么色 —— 压测报告区底色是
 * `canvas`，而同样的 `<InlineNotice tone="warn">` 若被放进 `bg-surface`
 * 的卡片里，底色会明显偏浅。同一条提示在两个位置长得不一样，且**不报任何错**。
 *
 * ⚠️ 文字取 `text-x-on`：横幅正文是 11px，AA 要 4.5:1，
 * 而 --ok 直接压在 --ok-subtle 上只有 2.95。理由与数据见 `Badge.tsx` 同名注释。
 *
 * ⚠️ `info` 的底色是 canvas-subtle 而不是 info-subtle：
 * 这条带子出现时，用户**不需要**为它紧张 —— 它是陈述（curl 预览模式、
 * 响应体积超限的提醒），不是「出了问题」。用 info 的青底会把它渲染成告警。
 *
 * 边框保留半透明（`border-x/35`）是有意的：边框只负责「圈出这一块」，
 * 不该跟着底色一起变实，否则小字块会显得比正文还重。
 */
const TONE_CLASSES: Record<NoticeTone, string> = {
  info: "border-line bg-canvas-subtle text-ink-muted",
  brand: "border-brand/30 bg-brand-subtle text-brand-on",
  ok: "border-ok/30 bg-ok-subtle text-ok-on",
  warn: "border-warn/35 bg-warn-subtle text-warn-on",
  err: "border-err/35 bg-err-subtle text-err-on",
};

const TONE_ICONS: Record<NoticeTone, LucideIcon> = {
  info: Info,
  brand: Info,
  ok: CircleCheck,
  warn: TriangleAlert,
  err: TriangleAlert,
};

export interface InlineNoticeProps {
  tone?: NoticeTone;
  /** 覆盖默认图标。不传 `null` 之外的图标时用 tone 对应的 */
  icon?: LucideIcon;
  /** 不画图标（内容自带图标时用） */
  noIcon?: boolean;
  /** 悬停说明。原生 `title` —— 见下方注释 */
  title?: string;
  className?: string;
  children: ReactNode;
}

/**
 * 一条提示带。
 *
 * 刻意**没有**退场动画，也没有高度动画：这类条的出现经常是「请求刚发完、
 * 响应刚回来」这种同一拍里的事，动画只会让密集界面显得晃。
 */
export function InlineNotice({
  tone = "info",
  icon,
  noIcon = false,
  title,
  className,
  children,
}: InlineNoticeProps) {
  const Icon = icon ?? TONE_ICONS[tone];

  return (
    <div
      // warn / err 用 role="status" 而不是 role="alert"：
      // alert 会打断读屏当前的朗读（assertive），而这些条并不紧急 ——
      // 用户不处理它，请求照样发得出去、响应照样看得到。
      role={tone === "info" || tone === "brand" ? undefined : "status"}
      title={title}
      className={cn(
        "flex shrink-0 flex-wrap items-center gap-x-2 gap-y-0.5 border-b px-2.5 py-1.5 text-[11px]",
        TONE_CLASSES[tone],
        className,
      )}
    >
      {noIcon ? null : <Icon size={12} strokeWidth={2.2} aria-hidden className="shrink-0" />}
      {children}
    </div>
  );
}
