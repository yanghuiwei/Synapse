import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/lib/cn";

export interface EmptyStateProps {
  icon: LucideIcon;
  title: string;
  description?: string;
  /** 底部操作区（按钮等） */
  action?: ReactNode;
  className?: string;
}

/** 列表/面板的空态。统一尺寸与留白，避免各页面各写一套 */
export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
  className,
}: EmptyStateProps) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-3 px-6 py-10 text-center",
        className,
      )}
    >
      <div className="flex h-11 w-11 items-center justify-center rounded-lg border border-line bg-canvas-subtle text-ink-faint">
        <Icon size={20} strokeWidth={1.6} />
      </div>
      <div className="space-y-1">
        <p className="text-[13px] font-medium text-ink-muted">{title}</p>
        {description ? (
          <p className="max-w-[240px] text-xs leading-relaxed text-ink-faint">
            {description}
          </p>
        ) : null}
      </div>
      {action ? <div className="mt-1">{action}</div> : null}
    </div>
  );
}
