import { TriangleAlert } from "lucide-react";
import { useCallback, useMemo, useRef } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import { classifyPlaceholder, parsePlaceholders, type PlaceholderKind } from "@/lib/placeholder";

/**
 * 单行输入框，带 `{{占位符}}` 高亮。
 *
 * ## 实现方式：两层叠加，而不是 contentEditable
 *
 * 结构是「底层一个只画颜色的镜像 div + 上层一个文字透明的 `<input>`」。
 * 为什么不用 `contentEditable` 自己画高亮：那要自己处理光标、选区、粘贴、
 * 撤销、中文输入法的组合事件 —— 每一个都是坑，而且坑在最核心的输入路径上。
 * 原生 `<input>` 这些全都是免费的。
 *
 * 代价是两层必须**像素级对齐**：字体、字号、内边距、滚动位置都要同步。
 * 所以这里有两条纪律：
 * 1. 镜像和 input 的 `font-*` / `px-*` 必须一起改（都写在下面的常量里）
 * 2. input 横向滚动时要同步 `scrollLeft`（长 URL 一滚就露馅）
 *
 * ## 三种颜色
 *
 * * 普通文本 → `text-ink`
 * * 写法正确的占位符 → `text-brand` + 淡蓝底
 * * 写坏的 / 本阶段不支持的 → `text-warn` + 淡黄底，并把 `title` 换成原因
 *
 * 第三种是这个组件存在的**主要理由**：一个写错的占位符不会让请求失败，
 * 它会原样发出去，于是用户看到的是 404 而不是「变量名拼错了」。
 */
export interface PlaceholderInputProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  /** 无障碍标签（没有可见 label 时必须给） */
  label?: string;
  /** 回车回调（地址栏用它发送） */
  onEnter?: () => void;
  /** 外层的类（边框、背景、宽度都归调用方） */
  className?: string;
  /** 额外加在 input 上的类 */
  inputClassName?: string;
  disabled?: boolean;
  /**
   * 紧凑模式：内边距与行高更小，给表格里的单元格用。
   *
   * 为什么是「模式」而不是让调用方自己传 padding 类：镜像层与 input 的
   * 内边距**必须一致**（否则高亮会整体偏移）。让调用方各传一份，
   * 就等于把这个不变量交给两个调用点去维持。
   */
  dense?: boolean;
  /**
   * 普通文本的颜色。
   *
   * 高亮的字全部来自镜像层，所以「这一行被停用了」这种视觉状态
   * 必须在**镜像层**上生效 —— 给 `<input>` 加 `text-ink-faint` 是没用的，
   * 它的文字本来就是透明的。
   */
  tone?: "normal" | "muted";
}

const TONE_CLASS = { normal: "text-ink", muted: "text-ink-faint" } as const;

/**
 * 镜像层与 input 的**共享**类（内边距部分）。
 *
 * 抽成常量不是为了少打字，而是为了让「必须一起改」这件事在代码里可见 ——
 * 之前它们各写一份，改了一处就会出现重影。
 */
const SHARED_BASE = "font-mono text-xs whitespace-pre";

const PADDING = { normal: "px-2.5", dense: "px-1.5" } as const;
const HEIGHT = { normal: "h-8", dense: "h-7" } as const;

const KIND_CLASS: Record<PlaceholderKind, string> = {
  resolved: "rounded-[3px] bg-brand/12 text-brand",
  // 淡黄底 + 警示色文字。深浅主题下都由令牌决定，不硬编码色值
  unknown: "rounded-[3px] bg-warn/15 text-warn",
  unsupported: "rounded-[3px] bg-warn/15 text-warn",
};

const KIND_HINT: Record<PlaceholderKind, MessageKey> = {
  resolved: "placeholder.title",
  unknown: "placeholder.unknownHint",
  unsupported: "placeholder.notSupported",
};

export function PlaceholderInput({
  value,
  onChange,
  placeholder,
  label,
  onEnter,
  className,
  inputClassName,
  disabled = false,
  dense = false,
  tone = "normal",
}: PlaceholderInputProps) {
  const { t } = useI18n();
  const inputRef = useRef<HTMLInputElement>(null);
  const mirrorRef = useRef<HTMLDivElement>(null);

  const shared = cn(PADDING[dense ? "dense" : "normal"], SHARED_BASE);

  const syncScroll = useCallback(() => {
    const input = inputRef.current;
    const mirror = mirrorRef.current;
    if (input && mirror) mirror.scrollLeft = input.scrollLeft;
  }, []);

  const segments = useMemo(() => splitSegments(value), [value]);

  return (
    // 注：背景和边框由**调用方**画在外层（见 className）。input 必须背景透明，
    // 否则它会把自己下面那层高亮盖住 —— 这是这套方案最容易被改错的地方。
    <div className={cn("relative min-w-0", className)}>
      <div
        ref={mirrorRef}
        aria-hidden
        className={cn(
          "pointer-events-none absolute inset-0 flex items-center overflow-hidden",
          shared,
        )}
      >
        {segments.map((seg, i) =>
          seg.kind === "text" ? (
            <span key={i} className={TONE_CLASS[tone]}>
              {seg.text}
            </span>
          ) : (
            <span
              key={i}
              title={t(KIND_HINT[seg.kind], { raw: seg.text })}
              className={KIND_CLASS[seg.kind]}
            >
              {seg.text}
            </span>
          ),
        )}
        {/* 末尾一个空格：光标停在最后一个字符后面时镜像层也得有内容，
            否则长文本滚到底会缺一截 */}
        {" "}
      </div>

      <input
        ref={inputRef}
        type="text"
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
        onScroll={syncScroll}
        onKeyDown={(e) => {
          if (e.key === "Enter" && onEnter) {
            e.preventDefault();
            onEnter();
          }
        }}
        spellCheck={false}
        autoComplete="off"
        aria-label={label}
        placeholder={placeholder}
        className={cn(
          "relative w-full bg-transparent outline-none",
          HEIGHT[dense ? "dense" : "normal"],
          shared,
          "placeholder:font-sans placeholder:text-ink-faint",
          // 文字透明 + 光标单独设色 —— 可见的字全部来自镜像层
          "text-transparent caret-brand selection:bg-brand/25",
          inputClassName,
        )}
      />
    </div>
  );
}

interface Segment {
  text: string;
  kind: TextSegmentKind;
}

type TextSegmentKind = "text" | PlaceholderKind;

function splitSegments(text: string): Segment[] {
  const out: Segment[] = [];
  let cursor = 0;

  for (const parsed of parsePlaceholders(text)) {
    const start = text.indexOf(parsed.raw, cursor);
    if (start < 0) continue;
    if (start > cursor) {
      out.push({ text: text.slice(cursor, start), kind: "text" });
    }
    out.push({ text: parsed.raw, kind: classifyPlaceholder(parsed) });
    cursor = start + parsed.raw.length;
  }

  if (cursor < text.length) {
    out.push({ text: text.slice(cursor), kind: "text" });
  }
  return out;
}

export interface PlaceholderHintProps {
  /** 要检查的文本（可以是多行 —— 请求体用） */
  text: string;
  className?: string;
}

/**
 * 多行输入（请求体）的占位符提示条。
 *
 * 为什么不给 `<textarea>` 也套两层高亮：镜像层得**同时**同步横向和纵向滚动，
 * 还要复刻换行算法 —— 一个字符差一点就整段错位，而请求体本来就不像 URL
 * 那样需要「一眼看出哪段是变量」。一行提示足够回答问题
 * 「我这段 body 里的占位符写对了没有」。
 */
export function PlaceholderHint({ text, className }: PlaceholderHintProps) {
  const { t } = useI18n();
  const broken = useMemo(
    () => parsePlaceholders(text).filter((p) => classifyPlaceholder(p) !== "resolved"),
    [text],
  );

  if (broken.length === 0) return null;

  return (
    <div
      className={cn(
        "flex shrink-0 items-start gap-1.5 border-t border-warn/40 bg-warn/10 px-2.5 py-1.5",
        "text-[11px] text-warn",
        className,
      )}
    >
      <TriangleAlert size={12} strokeWidth={2.2} aria-hidden className="mt-0.5 shrink-0" />
      <span className="min-w-0 flex-wrap break-all">
        {broken.map((p) => p.raw).join("  ")}
        {" — "}
        {t("placeholder.unknownHint", { raw: broken[0]?.raw ?? "" })}
      </span>
    </div>
  );
}
