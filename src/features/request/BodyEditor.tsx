import { AlignLeft, Braces } from "lucide-react";
import { useEffect, useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import { tryPrettyJson } from "@/lib/json";
import type { BodyType, KeyValueItem } from "@/lib/types";
import { Button } from "@/features/ui/Button";
import { JsonCodeArea } from "@/features/ui/JsonCodeArea";
import { KeyValueTable } from "./KeyValueTable";
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

/** 空 body 上点「美化」时给的骨架。比原地不动更有用 —— 那看起来像按钮坏了 */
const JSON_SKELETON = "{\n  \n}";

/** 提示自己消失的时间。够读完一句话，又不至于一直挂在那儿 */
const NOTICE_MS = 2200;

export interface BodyEditorProps {
  bodyType: BodyType;
  body: string;
  /** urlencoded 的键值行。只有这个类型用到，但始终透传 —— 见 `store/request` */
  bodyRows: KeyValueItem[];
  onBodyTypeChange: (bodyType: BodyType) => void;
  onBodyChange: (body: string) => void;
  onBodyRowsChange: (rows: KeyValueItem[]) => void;
  className?: string;
}

/**
 * 请求体编辑器。
 *
 * ## 两类编辑器，按类型分派
 *
 * `urlencoded` 是一张**键值表**（跟请求头长得一样）—— 那种接口的 body
 * 本来就是一组 `k=v`，让人手写 `a=1&b=2` 再自己数 `&` 是没道理的。
 * 其余类型是一块**文本区**。
 *
 * 两者写进的都是同一个 `body`：键值表在每次编辑后立刻拼好串（见
 * `store/request` 的 `setBodyRows`），所以后端、压测、历史只认 `body`
 * 这一条口径没有分叉。
 *
 * ## 为什么不做行号 / 折叠 / 内联校验波浪线
 *
 * 合法性仍然由后端在发送前判定（`E_BODY_NOT_JSON`），前端再实现一套校验器
 * 只会和 Rust 的规则慢慢长歪。这里只做两件事：**上色**（看得清结构）
 * 和**美化**（一键缩进），都是纯展示层的事。
 *
 * ## 上色不是「重新实现一遍编辑器」
 *
 * 高亮层是 `JsonCodeArea` 里 `textarea` 底下那层 `pre`，文字与光标各管各的。
 * 所以这里换掉一个 `<textarea>` 即可，不用接管输入、光标、选区。
 */
export function BodyEditor({
  bodyType,
  body,
  bodyRows,
  onBodyTypeChange,
  onBodyChange,
  onBodyRowsChange,
  className,
}: BodyEditorProps) {
  const { t } = useI18n();
  const current = BODY_TYPES.find((item) => item.value === bodyType);
  const unsupported = current !== undefined && !current.ready;
  const locked = bodyType === "none" || unsupported;

  const [notice, setNotice] = useState<string | null>(null);

  // 提示自己消失。依赖里放 notice，所以每次给出新提示都会重新计时
  useEffect(() => {
    if (notice === null) return;
    const timer = window.setTimeout(() => setNotice(null), NOTICE_MS);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const formatJson = () => {
    // 空 body 上补个骨架：原地不动会让人以为按钮坏了
    if (body.trim() === "") {
      onBodyChange(JSON_SKELETON);
      return;
    }

    const pretty = tryPrettyJson(body);
    if (pretty === null) {
      // 刻意**不清空也不改动**用户写的内容 —— 只是这次按不动而已
      setNotice(t("request.bodyFormatFailed"));
      return;
    }
    setNotice(null);
    onBodyChange(pretty);
  };

  return (
    <div className={cn("flex min-h-0 flex-col", className)}>
      <div className="flex h-9 shrink-0 items-center gap-1.5 border-b border-line px-2.5">
        <Braces size={12} strokeWidth={2.2} aria-hidden className="text-ink-faint" />

        <select
          aria-label={t("request.bodyType")}
          value={bodyType}
          onChange={(e) => onBodyTypeChange(e.target.value as BodyType)}
          className={cn(
            "h-6 shrink-0 rounded-sm border border-line bg-surface px-1.5 text-xs text-ink",
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
          <Button
            size="sm"
            variant="ghost"
            className="shrink-0"
            icon={<AlignLeft size={12} strokeWidth={2.2} />}
            onClick={formatJson}
          >
            {t("request.bodyFormat")}
          </Button>
        ) : null}

        {notice !== null ? (
          <span className="truncate text-[11px] text-err">{notice}</span>
        ) : null}

        <div className="min-w-0 flex-1" />

        {bodyType === "json" ? (
          <span className="truncate text-[11px] text-ink-faint">
            {t("request.bodyJsonHint")}
          </span>
        ) : bodyType === "urlencoded" ? (
          // 说清「表格里的东西会怎么变成一串」—— 否则用户不知道那些 `&` 是谁拼的
          <span className="truncate text-[11px] text-ink-faint">
            {t("request.bodyUrlencodedHint")}
          </span>
        ) : null}
      </div>

      {/* 滚动交给这一层：`JsonCodeArea` 的高度跟着内容长（见它的注释），
          所以这里只管把超出的部分滚出来 */}
      {bodyType === "urlencoded" ? (
        <div className="min-h-0 flex-1 overflow-auto">
          {/* 不传 `showDesc`：urlencoded 的「说明」没有归宿 ——
              序列化出去的是 `k=v`，注释无处安放，画出来只会让人以为它会生效 */}
          <KeyValueTable
            rows={bodyRows}
            onChange={onBodyRowsChange}
            keyPlaceholder={t("kv.key")}
            valuePlaceholder={t("kv.value")}
          />
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <JsonCodeArea
            value={body}
            onChange={onBodyChange}
            highlight={bodyType === "json"}
            disabled={locked}
            label={t("request.tab.body")}
            placeholder={
              unsupported
                ? t("request.unsupportedBody", {
                    type: t(current?.labelKey ?? "request.bodyType"),
                  })
                : t("request.bodyPlaceholder")
            }
          />
        </div>
      )}

      {/* 请求体是多行的，套不上那套「两层叠加 + 常量同步」的单行高亮。
          一行提示足够回答问题：「我这段 body 里的占位符写对了没有」 */}
      <PlaceholderHint text={body} />
    </div>
  );
}
