import { Braces } from "lucide-react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import type { BodyType } from "@/lib/types";
import { PlaceholderHint } from "./PlaceholderInput";

interface BodyTypeOption {
  value: BodyType;
  labelKey: MessageKey;
  /** 后端已经实现了没有 —— 没实现的在选项里明确标出来，而不是发出去才报错 */
  ready: boolean;
}

/**
 * 可选的请求体类型。
 *
 * 这个列表**必须与 Rust 侧 `build_body()` 的实现对齐**：
 * `none / json / raw / urlencoded` 已实现（另外 graphql 后端支持但 P2 不给入口），
 * `form`（multipart）和 `binary` 要读磁盘 / 拼流，涉及文件选择器，排在 P8。
 */
const BODY_TYPES: readonly BodyTypeOption[] = [
  { value: "none", labelKey: "request.bodyNone", ready: true },
  { value: "json", labelKey: "request.bodyRawJson", ready: true },
  { value: "raw", labelKey: "request.bodyRawText", ready: true },
  { value: "urlencoded", labelKey: "request.bodyFormUrlencoded", ready: true },
  { value: "form", labelKey: "request.bodyFormData", ready: false },
  { value: "binary", labelKey: "request.bodyBinary", ready: false },
];

export interface BodyEditorProps {
  bodyType: BodyType;
  body: string;
  onBodyTypeChange: (bodyType: BodyType) => void;
  onBodyChange: (body: string) => void;
  className?: string;
}

/**
 * 请求体编辑器。
 *
 * 只做一个纯文本区 + 类型选择，**不做**行号 / 折叠 / 内联校验提示条：
 * JSON 的合法性由后端在发送前本地校验（`E_BODY_NOT_JSON`），
 * 前端再实现一套校验器只会和 Rust 的规则慢慢长歪。
 */
export function BodyEditor({
  bodyType,
  body,
  onBodyTypeChange,
  onBodyChange,
  className,
}: BodyEditorProps) {
  const { t } = useI18n();
  const current = BODY_TYPES.find((item) => item.value === bodyType);
  const unsupported = current !== undefined && !current.ready;
  const locked = bodyType === "none" || unsupported;

  return (
    <div className={cn("flex min-h-0 flex-col", className)}>
      <div className="flex h-9 shrink-0 items-center gap-1.5 border-b border-line px-2.5">
        <Braces size={12} strokeWidth={2.2} aria-hidden className="text-ink-faint" />

        <select
          aria-label={t("request.bodyType")}
          value={bodyType}
          onChange={(e) => onBodyTypeChange(e.target.value as BodyType)}
          className={cn(
            "h-6 rounded-sm border border-line bg-surface px-1.5 text-xs text-ink",
            "outline-none transition-colors duration-100",
            "hover:border-line-strong focus:border-brand",
          )}
        >
          {BODY_TYPES.map((item) => (
            <option key={item.value} value={item.value}>
              {t(item.labelKey)}
              {item.ready ? "" : ` · ${t("common.comingSoon")}`}
            </option>
          ))}
        </select>

        {bodyType === "json" ? (
          <span className="ml-auto truncate text-[11px] text-ink-faint">
            {t("request.bodyJsonHint")}
          </span>
        ) : null}
      </div>

      <textarea
        value={body}
        onChange={(e) => onBodyChange(e.target.value)}
        disabled={locked}
        spellCheck={false}
        autoComplete="off"
        placeholder={
          unsupported
            ? t("request.unsupportedBody", {
                type: t(current?.labelKey ?? "request.bodyType"),
              })
            : t("request.bodyPlaceholder")
        }
        className={cn(
          "min-h-0 flex-1 resize-none bg-transparent p-2.5",
          "font-mono text-xs text-ink placeholder:font-sans placeholder:text-ink-faint",
          locked && "cursor-not-allowed text-ink-faint",
        )}
      />

      {/* 请求体是多行的，套不上那套「两层叠加 + 常量同步」的高亮（镜像层得
          同时复刻换行与两个方向的滚动，差一个字符就整段错位）。
          一行提示足够回答问题：「我这段 body 里的占位符写对了没有」。 */}
      <PlaceholderHint text={body} />
    </div>
  );
}