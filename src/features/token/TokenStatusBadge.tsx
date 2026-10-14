import { CircleCheck, CircleHelp, LoaderCircle, RefreshCw, TriangleAlert } from "lucide-react";
import type { LucideIcon } from "lucide-react";

import { cn } from "@/lib/cn";
import { formatSeconds } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import type { TokenStatus } from "@/lib/types";
import { Badge, type BadgeTone } from "@/features/ui/Badge";
import { IconButton } from "@/features/ui/Button";

/**
 * Token 有效期的四态。
 *
 * 「有缓存但永不过期」（`expiresAt === null`）是**第 4 态**，不能并进「有效」：
 * 用户在 Token 源里把过期时间路径留空，是显式选择了「不自动刷新」，
 * 界面上得能看出来 —— 否则他会奇怪「为什么这个 Token 一直不刷新」。
 */
type BadgeState = "unknown" | "never" | "valid" | "expired";

interface StateMeta {
  icon: LucideIcon;
  /** 图标 / 纯文字态下的颜色 */
  className: string;
  /** 有底色徽标时的 tone。`null` = 走纯文字（紧凑模式） */
  tone: BadgeTone;
  spin?: boolean;
}

const STATE_META: Record<BadgeState, StateMeta> = {
  // 状态还没查回来。**不**默认显示成「有效」—— PR 里最常见的一类错误
  unknown: { icon: CircleHelp, className: "text-ink-faint", tone: "neutral" },
  // 「永不过期」用 ok 的色调但**不带警示图标**（CircleCheck 已足够）
  never: { icon: CircleCheck, className: "text-ok", tone: "ok" },
  valid: { icon: CircleCheck, className: "text-ok", tone: "ok" },
  expired: { icon: TriangleAlert, className: "text-warn", tone: "warn" },
};

export interface TokenStatusBadgeProps {
  /** `null` = 还没查到状态 */
  status: TokenStatus | null;
  /** 正在刷新（按钮转圈 + 文案换成「刷新中…」） */
  refreshing?: boolean;
  /** 传了才显示「立即刷新」按钮 */
  onRefresh?: () => void;
  /** 额外的类（外边距之类） */
  className?: string;
  /** 紧凑模式：只显示一个点 + 文字，不显示刷新按钮 */
  compact?: boolean;
}

/**
 * Token 有效期的徽标。
 *
 * ## 为什么文案由**后端**给的 `expired` 决定
 *
 * 「还有多少秒过期」取决于 `skewSeconds` 与 `expiresAt`，而判定逻辑
 * （`http::token::is_expired`）只有一份。前端自己拿两个字段再算一遍，
 * 两边迟早会在边界上分叉，表现为「徽标说还有 5 秒，后端说已经过期了」。
 *
 * 所以这里的职责只是**把已经算好的结论翻译成人话**，
 * `remainingSeconds` 也只用来显示「多久之后 / 之前」。
 */
export function TokenStatusBadge({
  status,
  refreshing = false,
  onRefresh,
  className,
  compact = false,
}: TokenStatusBadgeProps) {
  const { t } = useI18n();
  const state = resolveState(status, refreshing);
  const meta = STATE_META[state];
  const Icon = meta.icon;

  return (
    <div className={cn("flex min-w-0 items-center gap-1.5", className)}>
      {/*
        紧凑与非紧凑走**两套渲染**，不是同一个徽标换个尺寸：
        紧凑模式出现在侧栏行内、历史列表行内 —— 那些位置一行本来就窄，
        再加个底色块，读起来像「这一行出错了」，而实际语义只是「这个 Token 还有 42 秒过期」。
        底色的意义是「值得多看一眼」，密集列表里到处是底色就没有一处值得多看。
      */}
      {compact ? (
        <span className={cn("flex min-w-0 items-center gap-1 text-[11px]", meta.className)}>
          {refreshing ? (
            <LoaderCircle size={12} strokeWidth={2.2} aria-hidden className="animate-spin" />
          ) : (
            <Icon size={12} strokeWidth={2.2} aria-hidden />
          )}
          <span className="truncate">{label(t, state, status)}</span>
        </span>
      ) : (
        <Badge tone={meta.tone} icon={refreshing ? LoaderCircle : Icon} spinning={refreshing}>
          {label(t, state, status)}
        </Badge>
      )}

      {onRefresh && !compact ? (
        <IconButton
          size="sm"
          label={refreshing ? t("token.refreshing") : t("token.refresh")}
          disabled={refreshing}
          onClick={onRefresh}
        >
          <RefreshCw size={12} strokeWidth={2.2} />
        </IconButton>
      ) : null}
    </div>
  );
}

function resolveState(status: TokenStatus | null, refreshing: boolean): BadgeState {
  if (refreshing) return "valid"; // 转圈时颜色不重要，图标已经说明了
  if (!status) return "unknown";
  if (!status.hasCache) return "expired"; // 从没取过 ≡ 对使用者而言不可用
  if (status.expiresAt === null) return "never";
  return status.expired ? "expired" : "valid";
}

function label(
  t: (key: MessageKey, params?: Record<string, string | number>) => string,
  state: BadgeState,
  status: TokenStatus | null,
): string {
  switch (state) {
    case "unknown":
      return t("common.loading");
    case "never":
      return t("token.noExpiry");
    case "expired": {
      if (!status?.hasCache) return t("token.neverFetched");
      // 负数剩余 = 已经过去多久。取绝对值，用「已过期 X」表达
      const past = status.remainingSeconds;
      return past === null
        ? t("auth.tokenExpired")
        : t("token.expiredAgo", { time: formatSeconds(past) });
    }
    default: {
      const left = status?.remainingSeconds;
      return left === null || left === undefined
        ? t("auth.tokenValid", { time: "—" })
        : t("token.expiresIn", { time: formatSeconds(left) });
    }
  }
}

/** 只需要一个「多少秒之后过期」的小圆点时用这个 */
export function TokenCountdownDot({ expired }: { expired: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        "h-1.5 w-1.5 shrink-0 rounded-full",
        expired ? "bg-warn" : "bg-ok",
      )}
    />
  );
}

/** 图标色的语义别处也要用（比如历史列表里标出「这次请求刷过 Token」） */
export const TOKEN_EXPIRED_COLOR = STATE_META.expired.className;
export const TOKEN_VALID_COLOR = STATE_META.valid.className;
