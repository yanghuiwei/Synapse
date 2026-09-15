import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cn } from "@/lib/cn";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md";

const VARIANT_CLASSES: Record<ButtonVariant, string> = {
  primary:
    "bg-brand text-brand-fg border-transparent hover:bg-brand-hover active:bg-brand-hover",
  secondary:
    "bg-surface text-ink border-line hover:bg-surface-hover hover:border-line-strong",
  ghost:
    "bg-transparent text-ink-muted border-transparent hover:bg-surface-hover hover:text-ink",
  danger:
    "bg-transparent text-err border-transparent hover:bg-err/10 active:bg-err/15",
};

const SIZE_CLASSES: Record<ButtonSize, string> = {
  sm: "h-7 px-2 text-xs gap-1.5 rounded-sm",
  md: "h-8 px-3 text-[13px] gap-2 rounded-md",
};

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** 左侧图标（传一个 lucide 图标元素即可） */
  icon?: ReactNode;
  /** 右侧图标 */
  trailingIcon?: ReactNode;
  /** 只显示图标，无文字 —— 此时会强制成正方形 */
  iconOnly?: boolean;
}

export function Button({
  variant = "secondary",
  size = "md",
  icon,
  trailingIcon,
  iconOnly = false,
  className,
  children,
  type = "button",
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={cn(
        "inline-flex select-none items-center justify-center whitespace-nowrap",
        "border font-medium transition-colors duration-100",
        "disabled:pointer-events-none disabled:opacity-45",
        SIZE_CLASSES[size],
        VARIANT_CLASSES[variant],
        iconOnly && (size === "sm" ? "w-7 px-0" : "w-8 px-0"),
        className,
      )}
      {...rest}
    >
      {icon}
      {!iconOnly && children}
      {/* 排在 children 之后 = 按钮最右侧。
          ⚠️ 这一行原先漏了：prop 声明了、参数也解构了，就是没画出来 ——
          传 `trailingIcon` 的地方会静默什么都看不到，且不报任何错。
          「新建 ▾」那个下拉箭头是它的第一个真实使用者，才把这个洞暴露出来。 */}
      {trailingIcon}
    </button>
  );
}

export interface IconButtonProps
  extends Omit<ButtonProps, "icon" | "trailingIcon" | "iconOnly"> {
  /** 无障碍标签，必填 —— 没有文字时屏幕阅读器只能靠它 */
  label: string;
  children: ReactNode;
}

export function IconButton({
  label,
  children,
  variant = "ghost",
  size = "md",
  className,
  ...rest
}: IconButtonProps) {
  return (
    <Button
      variant={variant}
      size={size}
      iconOnly
      aria-label={label}
      title={label}
      className={className}
      {...rest}
    >
      {children}
    </Button>
  );
}
