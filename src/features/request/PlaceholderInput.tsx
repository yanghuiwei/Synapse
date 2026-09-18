import { TriangleAlert } from "lucide-react";
import { useCallback, useMemo, useRef, type ClipboardEvent } from "react";

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
 *
 * ## 多行模式（`multiline`）
 *
 * 把上层换成 `<textarea>`、镜像层从 `absolute` 改成参与布局，其余一模一样 ——
 * 字体、内边距、换行规则仍从同一张表里取，两层对齐这个不变量没有被破坏。
 * 单元格于是能随着值变长而长高，长值一屏看全，也不用在一条窄缝里左右滚。
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
   * 多行模式：值长了自动折行、行高跟着内容长，而不是横向滚。
   *
   * 给表格里的「值」用 —— 那里最窄，而 `scope=SAP Media Message …` 这种
   * 空格分隔的长值恰恰最常见。单行模式下它只能横着滚，滚到一半既看不全
   * 也不好改，`Home` / `End` 之外没有别的办法。
   *
   * 实现上镜像层从 `absolute` 改成**参与布局**（见下面的注释），
   * 让 CSS 自己算高度 —— 不需要测量、不需要 `ResizeObserver`。
   */
  multiline?: boolean;
  /**
   * 普通文本的颜色。
   *
   * 高亮的字全部来自镜像层，所以「这一行被停用了」这种视觉状态
   * 必须在**镜像层**上生效 —— 给 `<input>` 加 `text-ink-faint` 是没用的，
   * 它的文字本来就是透明的。
   */
  tone?: "normal" | "muted";
  /**
   * 粘贴拦截。返回 `null` = 不干预，走浏览器默认行为。
   *
   * 存在的理由只有一条：从网站上复制 token 时，页面上写的往往是
   * `Bearer eyJ…`，而这一行里已经写着 `Bearer ` —— 直接粘下去就变成
   * `Bearer Bearer eyJ…`。这个值会**原样发出去**，服务端必然回 401，
   * 而界面上看不出哪里不对（两个词都是白的，值也不短）。
   *
   * 规则由调用方给（见 `HeadersTab`），这个组件不做任何判断 ——
   * 它只负责在「用户没有选中文本」时把粘贴的路子让出来。
   */
  onPasteText?: (pasted: string, current: string) => string | null;
}

const TONE_CLASS = { normal: "text-ink", muted: "text-ink-faint" } as const;

/**
 * 镜像层与 input 的**共享**类（内边距部分）。
 *
 * 抽成常量不是为了少打字，而是为了让「必须一起改」这件事在代码里可见 ——
 * 之前它们各写一份，改了一处就会出现重影。
 */
const SHARED_BASE = "font-mono text-xs";

/**
 * 换行方式。
 *
 * 单行控件横着滚、多行控件折行 —— 镜像层必须**与承载它的那个元素的原生行为一致**，
 * 否则两层换行位置不同，高亮就整体错位。所以两边都从这一张表里取。
 *
 * 不能靠「后写的类覆盖先写的」来切换：两个 `white-space` 工具类同层同特异性，
 * 谁生效取决于 CSS 的生成顺序，那是不可依赖的。
 */
const WRAP = {
  single: "whitespace-pre",
  multi: "whitespace-pre-wrap break-words",
} as const;

const PADDING = { normal: "px-2.5", dense: "px-1.5" } as const;

/**
 * 多行模式的纵向内边距 —— 取「单行控件高度 − 一行文字高度」的一半。
 *
 * 这样只写一行时两层的文字位置与单行模式逐像素相同，表格行高也就不会因为
 * 换成多行控件而变化；多出来的每一行是真正的文字行。
 */
const VPADDING = { normal: "py-2", dense: "py-1.5" } as const;

const HEIGHT = { normal: "h-8", dense: "h-7" } as const;

/** 多行模式的最小高度 = 单行控件高度，空值时不会塌成一条缝 */
const MIN_HEIGHT = { normal: "min-h-8", dense: "min-h-7" } as const;

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
  multiline = false,
  tone = "normal",
  onPasteText,
}: PlaceholderInputProps) {
  const { t } = useI18n();
  const inputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const mirrorRef = useRef<HTMLDivElement>(null);

  const size = dense ? "dense" : "normal";
  const shared = cn(PADDING[size], SHARED_BASE, WRAP[multiline ? "multi" : "single"]);

  const syncScroll = useCallback(() => {
    const input = inputRef.current;
    const mirror = mirrorRef.current;
    if (input && mirror) mirror.scrollLeft = input.scrollLeft;
  }, []);

  /**
   * 粘贴的转发。有选区时一律不干预 —— 那时用户想的是「换掉这一段」，
   * 替他去重前缀只会得到一个不是他想要的拼接结果。
   *
   * 光标位置在 `onChange` 之后会回到末尾（值被整体替换了）。对「粘一段
   * token 进来」这个动作来说，末尾正是接下来的落点，所以不额外做选区恢复 ——
   * 那是给 contentEditable 才需要的复杂度。
   */
  const handlePaste = useCallback(
    (e: ClipboardEvent<HTMLInputElement | HTMLTextAreaElement>) => {
      if (!onPasteText) return;
      if (e.currentTarget.selectionStart !== e.currentTarget.selectionEnd) return;

      const pasted = e.clipboardData.getData("text/plain");
      if (pasted === "") return;

      const next = onPasteText(pasted, value);
      if (next === null) return;

      e.preventDefault();
      onChange(next);
    },
    [onPasteText, value, onChange],
  );

  // 单行控件里，镜像层必须和 `<input>` 一样先把换行剥掉。
  // 浏览器对 `type=text` 会 strip newlines（HTML 规范），而这里是
  // `whitespace-pre` 的 div —— 不剥就会把值里的 `\n` 渲染成真换行，
  // 于是单行框里冒出两行、光标与可见文字整体错位。
  //
  // 多行模式**不剥**：`<textarea>` 本来就显示换行，剥掉反而让两层不一致；
  // 顺带也意味着往单元格里粘一段多行文本时，内容不会被悄悄改掉。
  const shown = multiline ? value : flattenNewlines(value);
  const segments = useMemo(() => splitSegments(shown), [shown]);

  /** 两层的文字内容完全一样，只有颜色和可见性不同 —— 抽出来免得两个分支改岔 */
  const renderSegments = () =>
    segments.map((seg, i) =>
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
    );

  // 背景和边框由**调用方**画在外层（见 className）。两层都得背景透明，
  // 否则上层会把下面那层高亮盖住 —— 这是这套方案最容易被改错的地方。
  const shell = cn("relative min-w-0", className);

  if (multiline) {
    return (
      <div className={shell}>
        {/* 多行模式下镜像层**参与布局**（不是 `absolute`）：它多高这一格就多高，
            `textarea` 再绝对定位铺满它。高度交给 CSS 自己算 —— 不必在 `onChange`
            里量 `scrollHeight`，也不必挂 `ResizeObserver`；窗口缩放、拖列宽引起的
            重新折行同样自动跟上。 */}
        <div
          ref={mirrorRef}
          aria-hidden
          className={cn(
            // `pointer-events-none` 与单行那份保持一致：这一层只负责画颜色，
            // 命中测试一律交给上层（否则悬停会落在镜像的文字上）
            "pointer-events-none w-full overflow-hidden",
            shared,
            VPADDING[size],
            MIN_HEIGHT[size],
          )}
        >
          {renderSegments()}
        </div>

        <textarea
          ref={textareaRef}
          rows={1}
          value={value}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
          onPaste={handlePaste}
          // 回车留给换行（`onEnter` 是单行控件的约定：那儿的回车没有别的事可做）
          spellCheck={false}
          autoComplete="off"
          aria-label={label}
          placeholder={placeholder}
          className={cn(
            // `inset-0` 而不是 `h-full`：高度完全听镜像层的
            "absolute inset-0 w-full resize-none overflow-hidden border-0 bg-transparent outline-none",
            VPADDING[size],
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

  return (
    <div className={shell}>
      <div
        ref={mirrorRef}
        aria-hidden
        className={cn(
          "pointer-events-none absolute inset-0 flex items-center overflow-hidden",
          shared,
        )}
      >
        {renderSegments()}
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
        onPaste={handlePaste}
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
          HEIGHT[size],
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

/**
 * 单行控件显示之前把换行抹掉。
 *
 * 对齐 `<input type="text">` 的规范行为（strip newlines）—— 是**删掉**而不是
 * 换成空格，这一点拿无头浏览器量过：`"a\nb"` 在 input 里渲染成 `ab`。
 * 镜像层若自己拿主意（比如 `whitespace-pre` 照原样换行），两层就不一致了。
 *
 * 只有单行模式走这里：`<textarea>` 会把换行显示出来，抹掉就成了「两层不一致」
 * 外加「粘进来的内容被悄悄改了」。
 */
function flattenNewlines(text: string): string {
  return text.replace(/[\r\n]+/g, "");
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
