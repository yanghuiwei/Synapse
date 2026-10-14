import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "@/lib/cn";

/**
 * 状态徽标。语义色的**实心字 + 淡底**。
 */
export type BadgeTone = "neutral" | "brand" | "ok" | "warn" | "err" | "info";
export type BadgeSize = "sm" | "md";

/**
 * tone → 类。
 *
 * 写法上刻意**不用** `bg-ok/10`：那是拿透明度去糊，10% 的语义色压在
 * `canvas` 上和压在 `surface` 上是两个色，压在深色主题里又是另一套 ——
 * 同一句「未保存」在两个面板里会长得不一样，而这种差异没有任何报错。
 * `--x-subtle`（x 取 ok/warn/err…）是 theme.css 里的一组**不透明实色**，
 * 底色因此是唯一的。
 *
 * ⚠️ 文字取 `text-x-on` 而不是 `text-x`。这不是审美偏好，是**实测的硬约束**：
 * 徽标文字只有 10~11px，WCAG AA 要 4.5:1，而
 *   --ok  (#16a34a) on --ok-subtle  (#e8f6ec) = 2.95  ✗
 *   --err (#dc2626) on --err-subtle (#fdecec) = 4.23 ✗
 * 把底色一路调浅也救不回来（浅到 #f9fdfa 时 ok 也只有 3.21）——
 * 底色与文字同色相，一起往白跑不会改善比值。
 * 换句话说：**是「加底色」这个动作本身弄坏了达标状态**
 * （加之前文字压白底，--ok 是 4.83）。
 * 补救只能是给「底色之上的文字」单独压深一档，即 `--x-on`。
 * 深色主题下 `--x-on` 等于 `--x` 原色（那边本来就够亮，压深反而是错的）。
 *
 * ⚠️ 别把纯文字态也换成 `text-x-on`（列表行内、状态栏那些）：
 * 那会把整个界面的语义色都压深，改动面远超「给徽标加个底」这件事本身。
 *
 * ⚠️ 这套实色底**不适用于可交互元素**（按钮、hover 态）：
 * 实色底 + hover 底互相盖不住，悬停时警告底会消失。
 * 交互态继续用半透明写法 —— 那里要的就是「透出下面的东西」。
 * 参见 `request/TokenGate.tsx` 里的同类注释。
 *
 * `neutral` 走 `canvas-subtle` + `text-ink-muted`：中性标签不需要色相，
 * 用背景层次区分就够了，硬塞一个语义色反而是噪音。
 */
const TONE_CLASSES: Record<BadgeTone, string> = {
  neutral: "bg-canvas-subtle text-ink-muted",
  brand: "bg-brand-subtle text-brand-on",
  ok: "bg-ok-subtle text-ok-on",
  warn: "bg-warn-subtle text-warn-on",
  err: "bg-err-subtle text-err-on",
  info: "bg-info-subtle text-info-on",
};

const SIZE_CLASSES: Record<BadgeSize, string> = {
  sm: "h-[17px] gap-1 rounded-xs px-1.5 text-[10px]",
  md: "h-5 gap-1.5 rounded-xs px-2 text-[11px]",
};

export interface BadgeProps {
  tone?: BadgeTone;
  size?: BadgeSize;
  /**
   * 左侧图标，**传 Lucide 组件**（`icon={CircleCheck}`）。
   *
   * ⚠️ 刻意**不**做成「组件或元素都行」的联合类型 —— 那样 TS 收窄不了
   * `ReactNode`（`LucideIcon` 本身也是可调用对象，与 ReactNode 的元素类型
   * 不兼容），最后只能在 JSX 里强行 `as ReactNode` 断言糊过去。
   * 拆成两个具名 prop 比在类型上花招更省事：
   * 需要自定义节点时用 `iconNode`，尺寸由调用方自己定。
   */
  icon?: LucideIcon;
  /** 自定义图标节点（自带尺寸与类名）。与 `icon` 二选一，`iconNode` 优先 */
  iconNode?: ReactNode;
  /** 图标转圈（加载中）。**只**转图标，不转文字 —— 见下方注释 */
  spinning?: boolean;
  /** 悬停说明。原生 `title` —— 徽标文案都很短，自绘浮层不值得 */
  title?: string;
  className?: string;
  children: ReactNode;
}

export function Badge({
  tone = "neutral",
  size = "sm",
  icon: Icon,
  iconNode,
  spinning = false,
  title,
  className,
  children,
}: BadgeProps) {
  const iconEl =
    iconNode ??
    (Icon ? (
      <Icon
        size={size === "sm" ? 10 : 11}
        strokeWidth={2.3}
        aria-hidden
        className={spinning ? "animate-spin" : undefined}
      />
    ) : null);

  return (
    <span
      title={title}
      className={cn(
        "inline-flex max-w-full shrink-0 items-center font-medium whitespace-nowrap",
        SIZE_CLASSES[size],
        TONE_CLASSES[tone],
        className,
      )}
    >
      {iconEl}
      <span className="truncate">{children}</span>
    </span>
  );
}
