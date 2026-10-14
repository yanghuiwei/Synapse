import { CircleCheck, RefreshCw } from "lucide-react";

import { cn } from "@/lib/cn";
import { formatSeconds } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import type { TranslateFn } from "@/lib/i18n";
import type { TokenUsage } from "@/lib/types";
import { InlineNotice } from "@/features/ui/InlineNotice";

export interface TokenUsageBannerProps {
  /** `undefined` = 本次没用到 Token（请求里没写 `{{token.*}}`，或没有可用来源） */
  usage: TokenUsage | undefined;
  className?: string;
}

/**
 * 「这一次发送到底用了什么 Token」的结果条。
 *
 * ## 它和「请求头」页那个徽标的区别
 *
 * 那里（Token 来源块里的状态徽标）回答的是「这个来源**现在**是什么状态」（随时可查）；
 * 这条回答的是「**刚刚那一次**发送发生了什么」—— 后者只有在执行期才知道
 * （缓存命中与真刷新在结果上一模一样，事后从库里读不出来），
 * 所以它由后端随响应带回来（`tokenUsage`）。
 *
 * ## 为什么「刷了」和「没刷」要用两种视觉
 *
 * 「本次已刷新」是有信息量的一条：发送前多打了一次取 Token 接口，
 * 而下面这个响应是用新 token 换来的。用户排查「改了配置但好像没生效」时，
 * 要的正是这一句。「复用了缓存」则是常态，安静地摆着就行 ——
 * 两者都是品牌色的话，真正有问题的那次反而看不出来了。
 *
 * 顺便：这两种说法**都不可能靠猜**。它们是执行期的事实，
 * 而不是「应该是这样」的推断。
 */
export function TokenUsageBanner({ usage, className }: TokenUsageBannerProps) {
  const { t } = useI18n();
  if (!usage) return null;

  const left = lifetimeLabel(usage, t);
  const refreshed = usage.refreshed;
  const Icon = refreshed ? RefreshCw : CircleCheck;

  // 「哪个接口」只在真刷新时才点出来：命中缓存的那次没有任何请求发生，
  // 写上去会让人以为它被调过。接口名读不出来时退回不提，不做猜测
  const endpoint = refreshed && usage.endpointName !== null ? usage.endpointName : null;

  return (
    <InlineNotice
      tone={refreshed ? "brand" : "info"}
      icon={Icon}
      className={className}
      title={refreshed ? t("token.usage.refreshedHint") : t("token.usage.cachedHint")}
    >
      <span className="font-medium">
        {refreshed ? t("token.usage.refreshed") : t("token.usage.cached")}
      </span>

      {endpoint === null ? null : (
        <span className="font-mono">{t("token.usage.via", { name: endpoint })}</span>
      )}

      {/* ⚠️ 次级文字用 `text-brand-on/90` 而不是 `/80`，也不是原色：
          加了 brand-subtle 底之后，原色 `text-brand/80` 在 5 套配色里有 3 套不达标
          （azure 3.84 / clay 4.11 / teal 3.56，AA 要 4.5）。
          `/90` 是逐套验过全过的最低档 —— 暗色+蓝紫在 /85 时只有 4.41，差 0.09。 */}
      <span className={cn("tabular", refreshed ? "text-brand-on/90" : "text-ink-faint")}>
        {left}
      </span>

      <span className={cn("truncate", refreshed ? "text-brand-on/90" : "text-ink-faint")}>
        {usage.sourceName}
      </span>

      {/* 关掉自动刷新时「过期也不刷」是预期行为，得说出来 ——
          否则下一次发送直接报错会被当成 bug */}
      {usage.autoRefresh ? null : (
        <span className={cn(refreshed ? "text-brand-on/90" : "text-warn-on")}>
          · {t("token.usage.autoRefreshOff")}
        </span>
      )}
    </InlineNotice>
  );
}

/**
 * 这个 token 还剩多久。
 *
 * `remainingSeconds` 是**响应带回来的那一刻**的值，不是「现在还剩多少」——
 * 面板可能已经摆了一分钟。所以它只用来描述这次执行的结果
 * （「换过来的 token 有效期还有 12 分钟」），不当作实时倒计时。
 * 实时倒计时在发送区那枚徽标上（那边会自己走秒）。
 */
function lifetimeLabel(usage: TokenUsage, t: TranslateFn): string {
  const seconds = usage.remainingSeconds;
  if (seconds === null) return t("token.usage.noExpiry");
  return seconds >= 0
    ? t("token.usage.left", { time: formatSeconds(seconds) })
    : t("token.usage.expired", { time: formatSeconds(seconds) });
}
