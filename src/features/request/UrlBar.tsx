import { useCallback, useRef } from "react";

import { cn } from "@/lib/cn";

/** 占位符 `{{...}}`；中间不允许再出现花括号，免得一层贪婪匹配把两个占位符连成一个 */
const PLACEHOLDER_RE = /\{\{[^{}]*\}\}/g;

interface Segment {
  text: string;
  isPlaceholder: boolean;
}

/** 把一段文本切成「普通片段 / 占位符片段」 */
function splitPlaceholders(text: string): Segment[] {
  const out: Segment[] = [];
  let cursor = 0;

  for (const match of text.matchAll(PLACEHOLDER_RE)) {
    const start = match.index ?? 0;
    if (start > cursor) {
      out.push({ text: text.slice(cursor, start), isPlaceholder: false });
    }
    out.push({ text: match[0], isPlaceholder: true });
    cursor = start + match[0].length;
  }

  if (cursor < text.length) {
    out.push({ text: text.slice(cursor), isPlaceholder: false });
  }
  return out;
}

export interface UrlBarProps {
  value: string;
  onChange: (value: string) => void;
  /** 回车即发送（和 IDE 里回车跑测试的肌肉记忆一致） */
  onSubmit?: () => void;
  placeholder?: string;
  className?: string;
}

/**
 * 地址栏，带 `{{占位符}}` 高亮。
 *
 * ## 实现方式：两层叠加，而不是 contentEditable
 *
 * 结构是「底层一个只画颜色的镜像 div + 上层一个文字透明的 `<input>`」。
 * 为什么不用 `contentEditable` 自己画高亮：那要自己处理光标、
 * 选区、粘贴、撤销、中文输入法的组合事件 —— 每一个都是坑，
 * 而且坑在最核心的输入路径上。原生 `<input>` 这些全都是免费的。
 *
 * 代价是两层必须**像素级对齐**：字体、字号、内边距、滚动位置都要同步。
 * 所以这里有两条纪律：
 * 1. 镜像和 input 的 `font-*` / `px-*` 必须一起改
 * 2. input 横向滚动时要同步 `scrollLeft`（长 URL 一滚就露馅）
 */
export function UrlBar({
  value,
  onChange,
  onSubmit,
  placeholder,
  className,
}: UrlBarProps) {
  const inputRef = useRef<HTMLInputElement>(null);
  const mirrorRef = useRef<HTMLDivElement>(null);

  const syncScroll = useCallback(() => {
    const input = inputRef.current;
    const mirror = mirrorRef.current;
    if (input && mirror) mirror.scrollLeft = input.scrollLeft;
  }, []);

  const segments = splitPlaceholders(value);

  return (
    // 注：背景和边框画在**外层**而不是 input 上。input 必须背景透明，
    // 否则它会把自己下面那层高亮盖住 —— 这是这套方案最容易被改错的地方。
    <div
      className={cn(
        "relative min-w-0 flex-1 rounded-md border border-line bg-surface",
        "transition-colors duration-100 hover:border-line-strong focus-within:border-brand",
        className,
      )}
    >
      <div
        ref={mirrorRef}
        aria-hidden
        className={cn(
          "pointer-events-none absolute inset-0 flex items-center overflow-hidden",
          "px-2.5 font-mono text-xs whitespace-pre",
        )}
      >
        {segments.map((seg, i) =>
          seg.isPlaceholder ? (
            <span key={i} className="bg-err/12 text-err">
              {seg.text}
            </span>
          ) : (
            <span key={i} className="text-ok">
              {seg.text}
            </span>
          ),
        )}
        {" "}
      </div>

      <input
        ref={inputRef}
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onScroll={syncScroll}
        onKeyDown={(e) => {
          if (e.key === "Enter" && onSubmit) {
            e.preventDefault();
            onSubmit();
          }
        }}
        spellCheck={false}
        autoComplete="off"
        placeholder={placeholder}
        className={cn(
          "relative h-8 w-full bg-transparent px-2.5 outline-none",
          "font-mono text-xs placeholder:font-sans placeholder:text-ink-faint",
          // 文字透明 + 光标单独设色 —— 可见的字全部来自镜像层
          "text-info caret-brand selection:bg-brand/25",
        )}
      />
    </div>
  );
}
