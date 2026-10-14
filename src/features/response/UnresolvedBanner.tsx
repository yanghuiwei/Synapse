import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import type { UnresolvedPlaceholder, UnresolvedReason } from "@/lib/types";
import { InlineNotice } from "@/features/ui/InlineNotice";

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
    <InlineNotice tone="warn" className={cn("block", className)}>
      <div className="flex items-center gap-1.5">
        <span className="font-medium">
          {t("placeholder.unresolvedTitle", { count: placeholders.length })}
        </span>
      </div>

      <ul className="mt-1 space-y-0.5 pl-[18px]">
        {placeholders.map((item) => (
          <li key={item.raw} className="flex flex-wrap items-baseline gap-x-1.5">
            <code className="font-mono">{item.raw}</code>
            {/* ⚠️ 这里原先是 `text-warn/80`，加底色之后就不达标了：
                --warn 以 80% 透明度压在 --warn-subtle (#fdf2e3) 上只有 2.32:1。
                改造前它压的是白底（那时 3.4:1，11px 小字勉强），
                换成实色底之后必须跟着改 —— 加底色会改变底的亮度，
                任何叠在上面的半透明文字都得重算。
                改成 `text-warn-on/90`：7.06:1 达标，且仍比上面的标题弱一层。 */}
            <span className="text-warn-on/90">{t(REASON_KEYS[item.reason])}</span>
          </li>
        ))}
      </ul>

      <p className="mt-1 pl-[18px] text-warn-on/90">{t("placeholder.unresolvedHint")}</p>
    </InlineNotice>
  );
}
