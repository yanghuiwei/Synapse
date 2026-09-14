import { TriangleAlert } from "lucide-react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import type { UnresolvedPlaceholder, UnresolvedReason } from "@/lib/types";

/**
 * 未解析原因 → 文案 key。
 *
 * 写成 `Record<UnresolvedReason, MessageKey>` 而不是拼字符串：
 * 后端在 `UnresolvedReason::as_str()` 里加一个变体时，这里会**编译报错**
 * （少一个键），而不是在界面上显示成 `placeholder.reason.xyz` 这种生 key。
 */
const REASON_KEYS: Record<UnresolvedReason, MessageKey> = {
  unknownNamespace: "placeholder.reason.unknownNamespace",
  badFunction: "placeholder.reason.badFunction",
  envNotFound: "placeholder.reason.envNotFound",
  noTokenSource: "placeholder.reason.noTokenSource",
  tokenUnavailable: "placeholder.reason.tokenUnavailable",
  noResponse: "placeholder.reason.noResponse",
  respPathMiss: "placeholder.reason.respPathMiss",
  notSupported: "placeholder.reason.notSupported",
};

export interface UnresolvedBannerProps {
  placeholders: readonly UnresolvedPlaceholder[];
  className?: string;
}

/**
 * 「这次请求里有些占位符没解析出来」的横幅。
 *
 * ## 为什么这条提示必须显眼
 *
 * 占位符没解析出来**不会**让请求失败 —— 它的原文被原样发了出去，
 * 所以用户看到的是一个 404 或一段莫名其妙的响应，而不是「你变量名拼错了」。
 * 没有这条横幅，那个 404 会被归因到接口本身上，而真正的原因
 * （`{{env.baseUrl}}` 里 baseUrl 拼错了）完全看不出来。
 *
 * 颜色用 `warn` 而不是 `err`：请求确实发出去了、也确实拿到了响应，
 * 这只是一条「你发出去的东西可能不是你以为的」警告。
 */
export function UnresolvedBanner({ placeholders, className }: UnresolvedBannerProps) {
  const { t } = useI18n();
  if (placeholders.length === 0) return null;

  return (
    <div
      role="status"
      className={cn(
        "shrink-0 border-b border-warn/40 bg-warn/10 px-2.5 py-1.5 text-[11px] text-warn",
        className,
      )}
    >
      <div className="flex items-center gap-1.5">
        <TriangleAlert size={12} strokeWidth={2.2} aria-hidden className="shrink-0" />
        <span className="font-medium">
          {t("placeholder.unresolvedTitle", { count: placeholders.length })}
        </span>
      </div>

      <ul className="mt-1 space-y-0.5 pl-[18px]">
        {placeholders.map((item) => (
          <li key={item.raw} className="flex flex-wrap items-baseline gap-x-1.5">
            <code className="font-mono">{item.raw}</code>
            <span className="text-warn/80">{t(REASON_KEYS[item.reason])}</span>
          </li>
        ))}
      </ul>

      <p className="mt-1 pl-[18px] text-warn/80">{t("placeholder.unresolvedHint")}</p>
    </div>
  );
}
