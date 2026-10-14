import { Copy, Eye, EyeOff, RefreshCw, Terminal } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import * as ipc from "@/lib/ipc";
import type { HistoryEntry } from "@/lib/types";
import { draftToSpec, useRequestStore } from "@/store/request";
import { Badge, type BadgeTone } from "@/features/ui/Badge";
import { Button, IconButton } from "@/features/ui/Button";
import { InlineNotice } from "@/features/ui/InlineNotice";

/**
 * 面板里这条命令的来源。
 *
 * 三种来源必须一眼分清，因为它们的**可信度不一样**：
 * `sent` / `history` 是发生过的事实（当时发出去的就是这一串），
 * `preview` 是按现在的定义算出来的推演。用户打开这个面板的动机是解释一个
 * 已经发生的 401，把推演看成事实会让人去怀疑一个根本不存在的 bug。
 */
interface CurlSource {
  kind: "sent" | "history" | "preview";
  plain: string;
  masked: string;
  /** 只有 `history` 有 */
  executedAt?: string;
  /** 只有 `preview` 有：没能解析的占位符数量 */
  unresolvedCount?: number;
  /** 只有 `preview` 有：这次解析顺带刷新了 Token */
  tokenRefreshed?: boolean;
}

/** curl 来源 → 徽标色调。走 Badge 的 tone，底色由 `--x-subtle` 令牌给 */
const SOURCE_TONE: Record<CurlSource["kind"], BadgeTone> = {
  sent: "ok",
  history: "info",
  // 推演用 warn 而不是中性色：它是三种来源里唯一「不是事实」的那种
  preview: "warn",
};

/** `MM-DD HH:mm`。历史列表那边有它自己的一套日期分组口径，这里只要精确到分钟 */
function shortTime(iso: string): string {
  const at = new Date(iso);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${pad(at.getHours())}:${pad(at.getMinutes())}`;
}

/**
 * 顶部说明条该说哪一句。
 *
 * `sent` 那一支要看**这次到底用没用上 Token 源**（`tokenUsage` 有没有）：
 * 「Token 已注入」是一句无条件说出口就会变成假话的断言，而它出现的位置
 * 恰好是用户核对一个 401 的时候 —— 一句假话会把人送去找不存在的 bug。
 */
function hintKeyOf(kind: CurlSource["kind"], usedToken: boolean): MessageKey {
  if (kind === "history") return "curl.sourceHistoryHint";
  if (kind === "preview") return "curl.sourcePreviewHint";
  return usedToken ? "curl.sourceSentHint" : "curl.sourceSentHintNoToken";
}

function CopyCommandButton({ text }: { text: string }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);

  const copy = () => {
    void navigator.clipboard
      .writeText(text)
      .then(() => {
        setCopied(true);
        window.setTimeout(() => setCopied(false), 1500);
      })
      .catch((e: unknown) => {
        // 剪贴板要用户手势 + 权限，失败没什么可补救的，留个线索
        console.warn("[reqly] 复制失败：", e);
      });
  };

  return (
    <IconButton
      size="sm"
      label={copied ? t("common.copied") : t("curl.copy")}
      onClick={copy}
    >
      <Copy size={13} strokeWidth={2} className={copied ? "text-ok" : undefined} />
    </IconButton>
  );
}

/**
 * 「查看 curl」面板。
 *
 * ## 数据从哪来（按优先级）
 *
 * 1. **本次响应带的** —— 发送那一刻后端算好的，最可信
 * 2. **这条请求最近一次带 curl 的历史** —— 也是事实，只不过是上一次的
 * 3. **用户点「生成」后的推演结果** —— 这条请求还没发过时的退路
 *
 * 前两者由后端在发送时落库（占位符已展开、Token 已注入），所以「上次跑的
 * 到底是什么样」这个问题事后也能回答；拿编辑框里的草稿是答不了的，那里全是
 * `{{token.access}}` 这样的原文。
 */
export function CurlPanel({ className }: { className?: string }) {
  const { t } = useI18n();
  const response = useRequestStore((s) => s.response);
  const draft = useRequestStore((s) => s.draft);
  const sourceId = useRequestStore((s) => s.sourceId);

  /** 默认明文：核对 401 时看不到 token 本身就等于没看。打码是给人截图用的 */
  const [masked, setMasked] = useState(false);
  const [preview, setPreview] = useState<CurlSource | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ipc.IpcError | null>(null);
  const [history, setHistory] = useState<HistoryEntry | null>(null);

  // 发了新请求、或者换了另一条请求 → 之前那份推演就过期了，让位给事实
  useEffect(() => {
    setPreview(null);
  }, [response, sourceId]);

  // 「上次跑的是什么」按请求 id 查。没保存过的草稿没有 id，也就无从查起
  useEffect(() => {
    if (sourceId === null) {
      setHistory(null);
      return;
    }

    let alive = true;
    void ipc.history
      .latestCurl(sourceId)
      .then((entry) => {
        if (alive) setHistory(entry);
      })
      .catch(() => {
        // 查不到不是错误（这条请求可能从没跑过）→ 退化成「没有上次」，
        // 不弹错、也不留一个红条去吓人
        if (alive) setHistory(null);
      });

    return () => {
      alive = false;
    };
  }, [sourceId]);

  const source = useMemo<CurlSource | null>(() => {
    if (preview !== null) return preview;

    if (response?.curlPlain !== undefined) {
      return {
        kind: "sent",
        plain: response.curlPlain,
        // 打码那份理论上一定有，缺了就退回明文 —— 显示得不准比显示不出来强
        masked: response.curl ?? response.curlPlain,
      };
    }

    // `curl_masked` 可能是 null 而 `curl` 有值（两个字段是同一次写入给的，
    // 但老库、外部写入方不受这个约束）
    if (history?.curl != null) {
      return {
        kind: "history",
        plain: history.curl,
        masked: history.curlMasked ?? history.curl,
        executedAt: history.executedAt,
      };
    }

    return null;
  }, [preview, response, history]);

  const generate = async () => {
    setBusy(true);
    setError(null);
    try {
      const result = await ipc.requests.previewCurl(draftToSpec(draft));
      setPreview({
        kind: "preview",
        plain: result.curlPlain,
        masked: result.curl,
        unresolvedCount: result.unresolvedPlaceholders.length,
        tokenRefreshed: result.tokenUsage?.refreshed === true,
      });
    } catch (e) {
      setError(ipc.toIpcError(e));
    } finally {
      setBusy(false);
    }
  };

  const generateButton = (
    <Button
      size="sm"
      variant={source === null ? "primary" : "ghost"}
      disabled={busy}
      onClick={() => void generate()}
      icon={<RefreshCw size={12} strokeWidth={2.2} />}
    >
      {t(source?.kind === "preview" ? "curl.regenerate" : "curl.generate")}
    </Button>
  );

  const errorLine =
    error === null ? null : (
      <InlineNotice tone="err" className="py-1">
        {t("curl.failed")}：{error.localized}
      </InlineNotice>
    );

  if (source === null) {
    return (
      <div
        className={cn(
          "flex h-full flex-col items-center justify-center gap-3 p-6 text-center",
          className,
        )}
      >
        <Terminal size={22} strokeWidth={1.6} aria-hidden className="text-ink-faint" />
        <div className="space-y-1">
          <p className="text-[13px] text-ink-muted">{t("curl.empty")}</p>
          <p className="max-w-[380px] text-xs text-ink-faint">{t("curl.emptyHint")}</p>
        </div>
        {generateButton}
        {errorLine}
      </div>
    );
  }

  const text = masked ? source.masked : source.plain;

  return (
    <div className={cn("flex min-h-0 flex-col", className)}>
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-line px-1.5">
        <Badge tone={SOURCE_TONE[source.kind]}>
          {source.executedAt !== undefined
            ? t("curl.sourceHistory", { time: shortTime(source.executedAt) })
            : t(source.kind === "sent" ? "curl.sourceSent" : "curl.sourcePreview")}
        </Badge>

        <span className="truncate text-[11px] text-ink-faint">
          {t(hintKeyOf(source.kind, response?.tokenUsage != null))}
        </span>

        <div className="flex-1" />

        <IconButton
          size="sm"
          label={t("curl.mask")}
          onClick={() => setMasked((v) => !v)}
        >
          {/* 图标表示「点下去会变成什么」：现在是明文，点一下就打码。

              ⚠️ 状态色画在**图标**上，不是画在 `IconButton` 的 className 上。
              `cn()` 是纯拼接、没有 tailwind-merge（见 `lib/cn.ts` 的注释），
              而 ghost 变体自己就带着 `text-ink-muted` —— 从外面再塞一个
              `text-brand` 会让两个类同时挂在 button 上，谁赢由样式表里的
              先后顺序决定，实测是 `text-ink-muted` 赢：开关点了没有任何视觉反馈。
              画在 svg 上是另一个元素，没有竞争，`stroke="currentColor"` 直接跟着走。 */}
          {masked ? (
            <EyeOff size={13} strokeWidth={2} className="text-brand" />
          ) : (
            <Eye size={13} strokeWidth={2} />
          )}
        </IconButton>

        {generateButton}

        <CopyCommandButton text={text} />
      </div>

      {errorLine}

      {source.unresolvedCount !== undefined && source.unresolvedCount > 0 ? (
        <InlineNotice tone="warn" className="py-1">
          {t("curl.unresolved", { count: String(source.unresolvedCount) })}
        </InlineNotice>
      ) : null}

      {source.tokenRefreshed === true ? (
        <InlineNotice tone="info" className="py-1">
          {t("curl.refreshHint")}
        </InlineNotice>
      ) : null}

      <div className="min-h-0 flex-1 overflow-auto">
        <pre
          className={cn(
            "min-h-full p-2.5 font-mono text-xs leading-relaxed",
            "whitespace-pre-wrap break-all",
          )}
        >
          {text}
        </pre>
      </div>
    </div>
  );
}
