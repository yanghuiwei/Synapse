import { cn } from "@/lib/cn";
import { PlaceholderInput } from "./PlaceholderInput";

export interface UrlBarProps {
  value: string;
  onChange: (value: string) => void;
  /** 回车即发送（和 IDE 里回车跑测试的肌肉记忆一致） */
  onSubmit?: () => void;
  placeholder?: string;
  className?: string;
}

/**
 * 地址栏。
 *
 * 高亮本身在 [`PlaceholderInput`] 里 —— 这里只负责「地址栏该长什么样」：
 * 一整个圆角框、悬停与聚焦时的边框色。
 *
 * 为什么把两层对齐的细节抽出去：参数表与请求体的值输入框要用同一套高亮。
 * 三处各写一遍的话，字体/内边距/滚动同步迟早只有两处是对的，
 * 表现就是「某几个输入框里高亮歪了」这种改了 A 坏 B 的问题。
 */
export function UrlBar({ value, onChange, onSubmit, placeholder, className }: UrlBarProps) {
  return (
    <PlaceholderInput
      value={value}
      onChange={onChange}
      onEnter={onSubmit}
      placeholder={placeholder}
      className={cn(
        "min-w-0 flex-1 rounded-md border border-line bg-surface",
        "transition-colors duration-100 hover:border-line-strong focus-within:border-brand",
        className,
      )}
    />
  );
}
