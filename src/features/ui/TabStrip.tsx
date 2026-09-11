import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";

export interface TabItem<K extends string> {
  key: K;
  labelKey: MessageKey;
  /** 右侧小角标，比如「请求头 3」；为 0 时不显示 */
  count?: number;
}

export interface TabStripProps<K extends string> {
  items: readonly TabItem<K>[];
  value: K;
  onChange: (key: K) => void;
  className?: string;
}

/**
 * 下划线式标签栏。
 * 用 aria-selected + role="tab" 让键盘和读屏能正确识别，
 * 视觉上的下划线用绝对定位的 span，避免 border 影响行高。
 */
export function TabStrip<K extends string>({
  items,
  value,
  onChange,
  className,
}: TabStripProps<K>) {
  const { t } = useI18n();

  return (
    <div
      role="tablist"
      className={cn("flex items-center gap-1 px-1.5", className)}
    >
      {items.map((item) => {
        const active = item.key === value;
        return (
          <button
            key={item.key}
            type="button"
            role="tab"
            aria-selected={active}
            onClick={() => onChange(item.key)}
            className={cn(
              "relative inline-flex h-8 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-sm px-2.5",
              "text-xs font-medium transition-colors duration-100",
              active ? "text-ink" : "text-ink-faint hover:text-ink-muted",
            )}
          >
            {t(item.labelKey)}
            {item.count ? (
              <span className="tabular rounded-full bg-canvas-subtle px-1.5 text-[10px] leading-4 text-ink-faint">
                {item.count}
              </span>
            ) : null}
            <span
              aria-hidden
              className={cn(
                "absolute inset-x-1.5 -bottom-px h-[2px] rounded-full transition-opacity duration-150",
                active ? "bg-brand opacity-100" : "opacity-0",
              )}
            />
          </button>
        );
      })}
    </div>
  );
}
