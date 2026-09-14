import { useMemo, type UIEvent } from "react";

import { cn } from "@/lib/cn";
import { HIGHLIGHT_LIMIT, JSON_TOKEN_CLASS, tokenizeJson } from "@/lib/json";

/**
 * 两层的公共排版。
 *
 * ⚠️ 高亮层（`pre`）与输入层（`textarea`）**必须逐像素对齐** —— 同一个字、
 * 同一行、同一个折行点。任何一条排版属性只写在一边，表现就是
 * 「光标和看到的字对不上」，而且**不会有任何报错**。
 *
 * 所以这两层用**同一个常量**，而不是把一串类名抄两遍：抄的那份迟早会被只改一处。
 * 尤其注意 `whitespace-pre-wrap` + `break-words`：它们决定软换行的断点，
 * 两边差一个词，整段就往上错位一行。
 */
const SHARED =
  "block w-full p-2.5 font-mono text-xs leading-relaxed whitespace-pre-wrap break-words";

export interface JsonCodeAreaProps {
  value: string;
  onChange: (value: string) => void;
  /**
   * 上不上色。非 JSON 的请求体（raw / urlencoded）也走这个组件，
   * 图的是它与高亮模式共用同一套布局行为 —— 不然两种模式一个自己滚、
   * 一个由外层滚，光标落点的手感会不一样。
   */
  highlight?: boolean;
  disabled?: boolean;
  placeholder?: string;
  /** 无障碍名。没有可见 label 时必填 */
  label: string;
  className?: string;
}

/**
 * 带语法高亮的代码输入区。
 *
 * ## 怎么做到「又高亮又能编辑」
 *
 * `textarea` 本身没法给单个 token 上色。所以叠两层：
 * 下面一层 `pre` 渲染高亮的同一段文本（只是没有光标），
 * 上面一层 `textarea` 文字设成透明、只留下光标。
 *
 * ## 为什么高度是「跟着内容长」而不是固定 `flex-1`
 *
 * 两层各自的滚动位置必须同步，而 `textarea` 在光标移出可视区时会**自己**
 * 改 `scrollTop` —— 它不通知谁，高亮层就跟丢了，表现为「文字整体错位」。
 * 与其去追这个同步，不如让它根本没有滚动条：容器高度等于内容高度，
 * 滚动统一交给外层容器（`min-h-full` 保证内容少时也占满一屏，
 * 这样点空白处照样能聚焦打字）。
 */
export function JsonCodeArea({
  value,
  onChange,
  highlight = false,
  disabled = false,
  placeholder,
  label,
  className,
}: JsonCodeAreaProps) {
  // 超大的 body 不做高亮：几十万个 `<span>` 会把浏览器拖死，
  // 而那正是「我粘了一段 5MB 的 JSON 进来」这种最容易发生的场景
  const colorize = highlight && !disabled && value.length <= HIGHLIGHT_LIMIT;
  const tokens = useMemo(
    () => (colorize ? tokenizeJson(value) : null),
    [colorize, value],
  );

  /**
   * 把 `textarea` 自己的滚动归零。
   *
   * 浏览器偶尔会因为光标位置「纠正」一下 `scrollTop`（哪怕内容并没有溢出），
   * 那一下之后高亮层就与输入层错开了，而且再也不会自己回来。
   * 高度已经等于内容高度，本来就不该滚动 —— 归零是安全且幂等的。
   */
  const resetScroll = (event: UIEvent<HTMLTextAreaElement>) => {
    const el = event.currentTarget;
    if (el.scrollTop !== 0) el.scrollTop = 0;
    if (el.scrollLeft !== 0) el.scrollLeft = 0;
  };

  return (
    <div className={cn("relative min-h-full", className)}>
      <pre
        aria-hidden
        className={cn(
          SHARED,
          "pointer-events-none",
          disabled ? "text-ink-faint" : "text-ink",
        )}
      >
        {tokens
          ? tokens.map((token, i) => (
              <span key={i} className={JSON_TOKEN_CLASS[token.kind]}>
                {token.text}
              </span>
            ))
          : value}
        {/* 末尾补一个换行：内容以空行结尾时，`pre` 会忽略最后那个换行，
            高度就比 `textarea` 矮一行，滚动到末尾会露馅 */}
        {"\n"}
      </pre>

      <textarea
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onScroll={resetScroll}
        disabled={disabled}
        spellCheck={false}
        autoComplete="off"
        aria-label={label}
        placeholder={placeholder}
        className={cn(
          SHARED,
          "absolute inset-0 resize-none overflow-hidden bg-transparent",
          // 文字透明、只剩光标：真正给用户看的是下面那层 `pre`
          "text-transparent caret-ink outline-none",
          "placeholder:font-sans placeholder:text-ink-faint",
          disabled && "cursor-not-allowed",
        )}
      />
    </div>
  );
}
