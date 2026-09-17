import { Copy, FileDown, FolderOpen, TextWrap } from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import { cn } from "@/lib/cn";
import { formatBytes, formatMs } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import * as ipc from "@/lib/ipc";
import {
  HIGHLIGHT_LIMIT,
  JSON_TOKEN_CLASS,
  PRETTY_LIMIT,
  tokenizeJson,
  tryPrettyJson,
} from "@/lib/json";
import type { BodyFile, HttpResponsePayload, SavedFile } from "@/lib/types";
import { Button, IconButton } from "@/features/ui/Button";

export type ResponseTab = "body" | "headers" | "cookies" | "timeline" | "curl";

/**
 * [`ResponseViewer`] 负责的那几个页签。
 *
 * `curl` 被排除在外：它**不依赖响应**（这条请求还没跑过时照样要看），
 * 所以由 `CurlPanel` 单独挂在响应区里，而不是塞进这个「读响应」的组件。
 * 排除之后这里的 `switch` 就是穷尽的，加新页签时漏一处会被编译器挡住。
 */
export type ResponseBodyTab = Exclude<ResponseTab, "curl">;

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

/**
 * 「文件型」响应的下载卡。
 *
 * 后端在执行时就把**完整字节**落到了自己的临时区（见 `http::execute` 的
 * `spill_response_file`），所以这里只负责把它另存到用户选的位置 ——
 * 界面全程不用碰内容本身。
 *
 * 为什么不在这里「顺便」直接用 base64 过 IPC：一个 8MB 的 Excel 过一趟
 * 序列化不但慢，还会在渲染进程里留一份副本。临时区那一跳把这两件事都省了。
 */
function FileBodyCard({
  file,
  onPreview,
  className,
}: {
  file: BodyFile;
  /** `null` = 这份内容本来就不是文本（真二进制），没有可预览的东西 */
  onPreview: (() => void) | null;
  className?: string;
}) {
  const { t } = useI18n();
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState<SavedFile | null>(null);
  const [err, setErr] = useState<ipc.IpcError | null>(null);

  const save = async () => {
    setBusy(true);
    setErr(null);
    try {
      const result = await ipc.files.saveResponseBody(file.path, file.name);
      // `null` = 用户在对话框里取消了。取消是正常操作，不该弹错误
      if (result !== null) setSaved(result);
    } catch (e) {
      setErr(ipc.toIpcError(e));
    } finally {
      setBusy(false);
    }
  };

  const reveal = async () => {
    if (saved === null || !ipc.isTauriRuntime()) return;
    try {
      await revealItemInDir(saved.path);
    } catch (e) {
      // 打不开目录不影响「已经存下来了」这个事实，留个控制台线索就够
      console.warn("[reqly] 打开保存目录失败：", e);
    }
  };

  return (
    <div
      className={cn(
        "flex h-full flex-col items-center justify-center gap-3 p-6 text-center",
        className,
      )}
    >
      <FileDown size={26} strokeWidth={1.6} aria-hidden className="text-ink-faint" />

      {/* 文件名是这张卡上最有用的信息 —— 用户靠它确认「导出的对不对」 */}
      <div className="max-w-full min-w-0">
        <p className="truncate font-mono text-[13px] text-ink" title={file.name}>
          {file.name}
        </p>
        <p className="mt-0.5 text-xs text-ink-faint">{formatBytes(file.size)}</p>
      </div>

      <p className="max-w-[420px] text-[11px] leading-relaxed text-ink-faint">
        {t("response.fileHint")}
      </p>

      <div className="flex items-center gap-1.5">
        <Button
          size="sm"
          variant="primary"
          disabled={busy}
          onClick={() => void save()}
          icon={<FileDown size={12} strokeWidth={2.2} />}
        >
          {t("response.saveToFile")}
        </Button>

        {onPreview !== null ? (
          <Button size="sm" variant="ghost" onClick={onPreview}>
            {t("response.preview")}
          </Button>
        ) : null}
      </div>

      {err !== null ? (
        <p className="max-w-[420px] text-[11px] text-err">
          {t("response.saveFailed")}：{err.localized}
        </p>
      ) : null}

      {saved !== null ? (
        <div className="flex w-full max-w-[460px] items-center gap-1.5 rounded-sm border border-line bg-canvas-subtle px-2 py-1.5">
          {/* 路径要能选中复制 —— 用户下一步往往就是把它粘到别处 */}
          <p className="min-w-0 flex-1 select-text text-left font-mono text-[10px] break-all text-ink-muted">
            {t("response.savedTo", { path: saved.path })}
          </p>
          <IconButton size="sm" label={t("response.openFolder")} onClick={() => void reveal()}>
            <FolderOpen size={12} strokeWidth={2.2} />
          </IconButton>
        </div>
      ) : null}
    </div>
  );
}

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
  /** 文件型响应默认不展开文本；能展开的（CSV 这类）由用户自己点进来 */
  const [preview, setPreview] = useState(false);

  const file = response.bodyFile;
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

  // 文件型响应默认给下载卡：服务端标了 `attachment`、或者内容根本不是文本时，
  // 用户要的是那个文件，不是一块乱码。但 CSV / 纯文本导出确实还读得出来，
  // 那就留一个「预览」入口 —— 小文件想扫一眼是很常见的需求。
  if (file !== undefined && !preview) {
    return (
      <FileBodyCard
        file={file}
        onPreview={response.bodyEncoding === "binary" ? null : () => setPreview(true)}
        className={className}
      />
    );
  }

  // 走到这里只可能是「文件型但后端没落上盘」（磁盘写不进去）——
  // 那时没有可下载的东西，只能把「为什么看不到内容」说清楚
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

        {file !== undefined ? (
          // 从预览回到下载卡。用图标而不是文字：这一行已经挤了三个按钮
          <IconButton
            size="sm"
            label={t("response.backToFile")}
            onClick={() => setPreview(false)}
            className="text-brand"
          >
            <FileDown size={13} strokeWidth={2} />
          </IconButton>
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
          {/* 「已存到文件」这半句只在真存下来了的时候说 —— 没有它就说得像有 */}
          {t(file !== undefined ? "response.truncatedSaved" : "response.truncated", {
            size: formatBytes(response.body.length),
          })}
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
                <span key={i} className={JSON_TOKEN_CLASS[token.kind]}>
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
  tab: ResponseBodyTab;
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
