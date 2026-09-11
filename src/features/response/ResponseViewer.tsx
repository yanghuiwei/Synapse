import { Copy, TextWrap } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";

import { cn } from "@/lib/cn";
import { formatBytes, formatMs } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import type { HttpResponsePayload } from "@/lib/types";
import { IconButton } from "@/features/ui/Button";

export type ResponseTab = "body" | "headers" | "cookies" | "timeline";

/**
 * 超过这个大小就不再「美化」。
 *
 * `JSON.parse` + `JSON.stringify` 在 2MB 上已经是几百毫秒的卡顿，
 * 而且美化后的字符串体积是原来的两三倍 —— 卡的是主线程，用户看到的是界面直接僵住。
 */
const PRETTY_LIMIT = 2 * 1024 * 1024;

/** 再往上是连语法高亮都别做：每个 token 一个 `<span>`，几十万个节点会把浏览器拖死 */
const HIGHLIGHT_LIMIT = 400 * 1024;

/** 试着重排成缩进 JSON；不是合法 JSON 就返回 null（不是错误） */
function tryPrettyJson(body: string): string | null {
  try {
    return JSON.stringify(JSON.parse(body), null, 2);
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------
// JSON 语法高亮
// ---------------------------------------------------------------

type TokenKind = "key" | "str" | "num" | "bool" | "null" | "punct" | "ws";

interface JsonToken {
  text: string;
  kind: TokenKind;
}

/**
 * 一次正则扫完整段 JSON。
 *
 * 为什么不引 highlight.js / prism：为了一个只高亮 JSON 的场景，
 * 那是几十 KB 的依赖 + 一套主题系统；而 `--syn-*` 设计令牌本来就在 theme.css 里备好了。
 * 这个正则只做**分类**，不做校验 —— 输入已经是 `JSON.stringify` 的输出，
 * 所以不必考虑畸形输入。
 */
const JSON_TOKEN_RE =
  /("(?:\\.|[^"\\])*")(\s*:)?|\b(true|false)\b|\bnull\b|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|([{}[\],:])/g;

const TOKEN_CLASS: Record<TokenKind, string> = {
  key: "text-syn-key",
  str: "text-syn-str",
  num: "text-syn-num",
  bool: "text-syn-bool",
  null: "text-syn-null",
  punct: "text-syn-punct",
  ws: "",
};

function tokenizeJson(text: string): JsonToken[] {
  const out: JsonToken[] = [];
  let cursor = 0;

  for (const match of text.matchAll(JSON_TOKEN_RE)) {
    const start = match.index ?? 0;
    const raw = match[0];
    if (start > cursor) {
      out.push({ text: text.slice(cursor, start), kind: "ws" });
    }

    const quoted = match[1];
    const colon = match[2];
    const literal = match[3];
    const number = match[4];
    const punct = match[5];

    if (quoted !== undefined) {
      // 引号后面跟着冒号 = 对象的键，否则是字符串值
      out.push({ text: quoted, kind: colon !== undefined ? "key" : "str" });
      if (colon !== undefined) out.push({ text: colon, kind: "punct" });
    } else if (literal !== undefined) {
      out.push({ text: raw, kind: "bool" });
    } else if (number !== undefined) {
      out.push({ text: raw, kind: "num" });
    } else if (raw === "null") {
      out.push({ text: raw, kind: "null" });
    } else if (punct !== undefined) {
      out.push({ text: raw, kind: "punct" });
    }

    cursor = start + raw.length;
  }

  if (cursor < text.length) {
    out.push({ text: text.slice(cursor), kind: "ws" });
  }
  return out;
}

/** 复制按钮 —— 带 1.5 秒的「已复制」反馈 */
function CopyButton({ text }: { text: string }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      // 剪贴板要用户手势 + 权限，失败没什么可补救的，留个线索
      console.warn("[reqly] 复制失败：", e);
    }
  };

  return (
    <IconButton
      size="sm"
      label={copied ? t("common.copied") : t("response.copyBody")}
      onClick={() => void copy()}
      className={copied ? "text-ok" : undefined}
    >
      <Copy size={13} strokeWidth={2} />
    </IconButton>
  );
}

// ---------------------------------------------------------------
// 各 Tab
// ---------------------------------------------------------------

function BodyView({
  response,
  className,
}: {
  response: HttpResponsePayload;
  className?: string;
}) {
  const { t } = useI18n();
  /** 默认给「美化」；用户点了原文之后记住选择 */
  const [raw, setRaw] = useState(false);
  const [wrap, setWrap] = useState(true);

  const tooBig = response.bodySize > PRETTY_LIMIT;
  const pretty = useMemo(
    () => (raw || tooBig ? null : tryPrettyJson(response.body)),
    [response.body, raw, tooBig],
  );

  const text = pretty ?? response.body;
  const highlight = pretty !== null && text.length <= HIGHLIGHT_LIMIT;
  const tokens = useMemo(
    () => (highlight ? tokenizeJson(text) : null),
    [highlight, text],
  );

  if (response.bodyEncoding === "binary") {
    return (
      <div className={cn("flex h-full flex-col items-center justify-center gap-1.5 p-6 text-center", className)}>
        <p className="text-[13px] text-ink-muted">
          {t("response.binaryBody", { size: formatBytes(response.bodySize) })}
        </p>
        <p className="text-xs text-ink-faint">{t("response.binaryHint")}</p>
      </div>
    );
  }

  if (text.length === 0) {
    return (
      <p className={cn("p-4 text-xs text-ink-faint", className)}>
        {t("response.emptyBody")}
      </p>
    );
  }

  return (
    <div className={cn("flex min-h-0 flex-col", className)}>
      <div className="flex h-8 shrink-0 items-center gap-1 border-b border-line px-1.5">
        {pretty !== null ? (
          <div className="flex items-center gap-0.5 rounded-sm bg-canvas-subtle p-0.5">
            <FormatToggle active={!raw} onClick={() => setRaw(false)}>
              {t("response.pretty")}
            </FormatToggle>
            <FormatToggle active={raw} onClick={() => setRaw(true)}>
              {t("response.raw")}
            </FormatToggle>
          </div>
        ) : (
          <span className="px-1 text-[11px] text-ink-faint">
            {t("response.raw")}
            {tooBig ? ` · ${formatBytes(response.bodySize)}` : ""}
          </span>
        )}

        <div className="flex-1" />

        {response.bodyEncoding === "lossy" ? (
          <span className="truncate text-[11px] text-warn">
            {t("response.lossyBody")}
          </span>
        ) : null}

        <IconButton
          size="sm"
          label={t("response.wrap")}
          onClick={() => setWrap((v) => !v)}
          className={wrap ? "text-brand" : undefined}
        >
          <TextWrap size={13} strokeWidth={2} />
        </IconButton>

        <CopyButton text={response.body} />
      </div>

      {response.truncated ? (
        <p className="shrink-0 border-b border-line bg-warn/8 px-2.5 py-1 text-[11px] text-warn">
          {t("response.truncated", { size: formatBytes(response.body.length) })}
        </p>
      ) : null}

      <div className="min-h-0 flex-1 overflow-auto">
        <pre
          className={cn(
            "min-h-full p-2.5 font-mono text-xs leading-relaxed",
            wrap ? "whitespace-pre-wrap break-all" : "whitespace-pre",
          )}
        >
          {tokens
            ? tokens.map((token, i) => (
                <span key={i} className={TOKEN_CLASS[token.kind]}>
                  {token.text}
                </span>
              ))
            : text}
        </pre>
      </div>
    </div>
  );
}

function FormatToggle({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "rounded-[4px] px-2 py-0.5 text-[11px] transition-colors duration-100",
        active
          ? "bg-surface text-ink shadow-[var(--shadow-1)]"
          : "text-ink-faint hover:text-ink-muted",
      )}
    >
      {children}
    </button>
  );
}

function HeadersView({
  response,
  className,
}: {
  response: HttpResponsePayload;
  className?: string;
}) {
  const { t } = useI18n();

  if (response.headers.length === 0) {
    return (
      <p className={cn("p-4 text-xs text-ink-faint", className)}>
        {t("response.headersEmpty")}
      </p>
    );
  }

  return (
    <div className={cn("flex min-h-0 flex-col", className)}>
      <div className="flex h-8 shrink-0 items-center justify-end border-b border-line px-1.5">
        <CopyButton
          text={response.headers.map((h) => `${h.name}: ${h.value}`).join("\n")}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-full border-collapse">
          <colgroup>
            <col style={{ width: "34%" }} />
            <col />
          </colgroup>
          <tbody>
            {response.headers.map((header, i) => (
              <tr key={i} className="border-b border-line/60 align-top">
                <td className="px-2.5 py-1.5 font-mono text-xs break-all text-syn-key">
                  {header.name}
                </td>
                <td className="px-2.5 py-1.5 font-mono text-xs break-all text-ink-muted">
                  {header.value}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

/** `Set-Cookie` 的值拆成「名字 / 值 / 属性」三段（属性原样保留，不解析语义） */
function splitCookie(raw: string): { name: string; value: string; attrs: string } {
  const parts = raw.split(";");
  const first = parts[0] ?? "";
  const eq = first.indexOf("=");
  return {
    name: eq === -1 ? first.trim() : first.slice(0, eq).trim(),
    value: eq === -1 ? "" : first.slice(eq + 1).trim(),
    attrs: parts.slice(1).map((s) => s.trim()).join("; "),
  };
}

function CookiesView({
  response,
  className,
}: {
  response: HttpResponsePayload;
  className?: string;
}) {
  const { t } = useI18n();

  // 直接用原始响应头解析，不再跑一遍 cookie jar ——
  // 用户想看的是「服务端到底设了什么」，而不是「客户端最后存成了什么」。
  const cookies = response.headers
    .filter((h) => h.name.toLowerCase() === "set-cookie")
    .map((h) => splitCookie(h.value));

  if (cookies.length === 0) {
    return (
      <p className={cn("p-4 text-xs text-ink-faint", className)}>
        {t("common.empty")}
      </p>
    );
  }

  return (
    <div className={cn("min-h-0 overflow-auto", className)}>
      <ul className="divide-y divide-line/60">
        {cookies.map((cookie, i) => (
          <li key={i} className="px-2.5 py-2">
            <div className="flex items-baseline gap-1.5 font-mono text-xs">
              <span className="text-syn-key">{cookie.name}</span>
              <span className="text-ink-faint">=</span>
              <span className="break-all text-ink">{cookie.value}</span>
            </div>
            {cookie.attrs ? (
              <p className="mt-0.5 font-mono text-[11px] break-all text-ink-faint">
                {cookie.attrs}
              </p>
            ) : null}
          </li>
        ))}
      </ul>
    </div>
  );
}

interface Phase {
  labelKey: MessageKey;
  /** null = 上游库没暴露这个观测点 */
  ms: number | null;
}

function TimelineView({
  response,
  className,
}: {
  response: HttpResponsePayload;
  className?: string;
}) {
  const { t } = useI18n();

  const phases: Phase[] = [
    { labelKey: "response.timeline.dns", ms: null },
    { labelKey: "response.timeline.connect", ms: null },
    { labelKey: "response.timeline.tls", ms: null },
    { labelKey: "response.timeline.ttfb", ms: response.ttfbMs },
    { labelKey: "response.timeline.download", ms: response.downloadMs },
  ];

  // 条形长度按总耗时归一 —— 用实测的 totalMs 而不是各段之和，
  // 这样多出来的「组装 / 解析」时间不会凭空消失
  const scale = Math.max(response.totalMs, 1);

  return (
    <div className={cn("min-h-0 overflow-auto p-2.5", className)}>
      <table className="w-full border-collapse text-xs">
        <colgroup>
          <col style={{ width: 92 }} />
          <col />
          <col style={{ width: 84 }} />
        </colgroup>
        <thead>
          <tr className="border-b border-line text-left text-[11px] text-ink-faint">
            <th className="py-1 font-normal">{t("response.timelinePhase")}</th>
            <th className="py-1 font-normal" />
            <th className="py-1 text-right font-normal">
              {t("response.timelineDuration")}
            </th>
          </tr>
        </thead>
        <tbody>
          {phases.map((phase) => (
            <tr key={phase.labelKey} className="border-b border-line/60">
              <td className="py-1.5 text-ink-muted">{t(phase.labelKey)}</td>
              <td className="py-1.5 pr-2">
                {phase.ms === null ? (
                  <span className="text-ink-faint">—</span>
                ) : (
                  <span
                    className="block h-1.5 rounded-full bg-brand"
                    style={{ width: `${Math.max((phase.ms / scale) * 100, 0.5)}%` }}
                  />
                )}
              </td>
              <td className="tabular py-1.5 text-right text-ink-muted">
                {phase.ms === null ? (
                  <span title={t("response.timelineUnavailable")}>—</span>
                ) : (
                  formatMs(phase.ms)
                )}
              </td>
            </tr>
          ))}

          <tr>
            <td className="py-1.5 font-medium text-ink">
              {t("response.timelineTotal")}
            </td>
            <td />
            <td className="tabular py-1.5 text-right font-medium text-ink">
              {formatMs(response.totalMs)}
            </td>
          </tr>
        </tbody>
      </table>

      <p className="mt-2 text-[11px] leading-relaxed text-ink-faint">
        {t("response.timelineHint")}
      </p>

      <dl className="mt-3 space-y-1.5 border-t border-line pt-2.5 text-xs">
        <div className="flex gap-2">
          <dt className="w-[92px] shrink-0 text-ink-faint">
            {t("response.finalUrl")}
          </dt>
          <dd className="min-w-0 font-mono break-all text-ink-muted">
            {response.url}
            {response.redirected ? (
              <span className="ml-1.5 font-sans text-[11px] text-info">
                ({t("response.redirectedNote")})
              </span>
            ) : null}
          </dd>
        </div>
        <div className="flex gap-2">
          <dt className="w-[92px] shrink-0 text-ink-faint">
            {t("response.protocol")}
          </dt>
          <dd className="font-mono text-ink-muted">{response.httpVersion}</dd>
        </div>
      </dl>
    </div>
  );
}

export interface ResponseViewerProps {
  response: HttpResponsePayload;
  tab: ResponseTab;
  className?: string;
}

/** 响应内容区。外层的标签栏与指标胶囊由 `ResponsePane` 负责 */
export function ResponseViewer({ response, tab, className }: ResponseViewerProps) {
  switch (tab) {
    case "body":
      return <BodyView response={response} className={className} />;
    case "headers":
      return <HeadersView response={response} className={className} />;
    case "cookies":
      return <CookiesView response={response} className={className} />;
    case "timeline":
      return <TimelineView response={response} className={className} />;
  }
}
