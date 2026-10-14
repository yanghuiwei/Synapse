import {
  ArrowDownToLine,
  Ban,
  Clock,
  FileBraces,
  LoaderCircle,
  Timer,
  TriangleAlert,
} from "lucide-react";
import { useState, type ReactNode } from "react";

import { cn } from "@/lib/cn";
import { formatBytes, formatMs } from "@/lib/format";
import { statusTextClass } from "@/lib/http";
import { useI18n } from "@/lib/i18n";
import type { IpcError } from "@/lib/ipc";
import { useRequestStore } from "@/store/request";
import { Badge } from "@/features/ui/Badge";
import { Button } from "@/features/ui/Button";
import { EmptyState } from "@/features/ui/EmptyState";
import { TabStrip, type TabItem } from "@/features/ui/TabStrip";
import { CurlPanel } from "./CurlPanel";
import { ResponseViewer, type ResponseTab } from "./ResponseViewer";
import { TokenUsageBanner } from "./TokenUsageBanner";
import { UnresolvedBanner } from "./UnresolvedBanner";

const TABS: readonly TabItem<ResponseTab>[] = [
  { key: "body", labelKey: "response.tab.body" },
  { key: "headers", labelKey: "response.tab.headers" },
  { key: "cookies", labelKey: "response.tab.cookies" },
  { key: "timeline", labelKey: "response.tab.timeline" },
  { key: "curl", labelKey: "response.tab.curl" },
];

/** 一枚指标胶囊：状态 / 耗时 / 大小 */
function Metric({
  icon: Icon,
  label,
  value,
  valueClass,
}: {
  icon: typeof Clock;
  label: string;
  value: string;
  valueClass?: string;
}) {
  return (
    <span className="flex items-center gap-1 text-[11px] text-ink-faint">
      <Icon size={11} strokeWidth={2.2} aria-hidden />
      <span className="sr-only">{label}</span>
      <span className={cn("tabular", valueClass ?? "text-ink-muted")}>{value}</span>
    </span>
  );
}

/** 发送中。带上取消按钮 —— 界面本身就该是「这个请求卡住了」的出口 */
function SendingPanel({ cancelling }: { cancelling: boolean }) {
  const { t } = useI18n();
  const cancel = useRequestStore((s) => s.cancel);

  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
      <LoaderCircle size={22} strokeWidth={2} className="animate-spin text-brand" />
      <p className="text-[13px] text-ink-muted">{t("request.sending")}</p>
      <Button
        size="sm"
        variant="secondary"
        disabled={cancelling}
        onClick={() => void cancel()}
      >
        {t("request.cancel")}
      </Button>
    </div>
  );
}

function ErrorPanel({ error }: { error: IpcError }) {
  const { t } = useI18n();
  const send = useRequestStore((s) => s.send);

  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
      <div className="flex h-11 w-11 items-center justify-center rounded-lg border border-err/30 bg-err/8 text-err">
        <TriangleAlert size={20} strokeWidth={1.6} />
      </div>

      <div className="space-y-1">
        {/* debug 里带着完整 URL 和底层原因，放 title 里让开发者能顺手看到，
            但**不作为正文** —— 那是给排查用的，不是给用户读的 */}
        <p className="max-w-[320px] text-[13px] font-medium text-ink" title={error.debug}>
          {error.localized}
        </p>
        <p className="font-mono text-[11px] text-ink-faint">{error.code}</p>
      </div>

      {error.retryable ? (
        <Button size="sm" variant="secondary" onClick={() => void send()}>
          {t("common.retry")}
        </Button>
      ) : null}
    </div>
  );
}

/**
 * 响应面板（工作区下半部分）。
 *
 * 状态优先级刻意是「有响应就先显示响应」：
 * 一次失败或取消之后 `response` 会被清空，所以这几条分支不会互相打架。
 */
export function ResponsePane() {
  const { t } = useI18n();
  const [tab, setTab] = useState<ResponseTab>("body");

  const response = useRequestStore((s) => s.response);
  const error = useRequestStore((s) => s.error);
  const cancelled = useRequestStore((s) => s.cancelled);
  const runId = useRequestStore((s) => s.runId);
  const cancelling = useRequestStore((s) => s.cancelling);

  const running = runId !== null;

  const status = response
    ? `${response.status} ${response.statusText}`.trim()
    : running
      ? t("status.sending")
      : "—";

  let content: ReactNode;
  // 「cURL」排在最前面判：它**不看有没有响应** —— 恰恰是「这条请求还没跑过、
  // 我想知道它打出去会是什么样」时最需要它。其余分支都要求有响应。
  if (tab === "curl") {
    content = <CurlPanel className="h-full" />;
  } else if (response) {
    content = <ResponseViewer response={response} tab={tab} className="h-full" />;
  } else if (error) {
    content = <ErrorPanel error={error} />;
  } else if (running) {
    content = <SendingPanel cancelling={cancelling} />;
  } else if (cancelled) {
    content = (
      <EmptyState
        icon={Ban}
        title={t("response.cancelled")}
        description={t("response.noResponseHint")}
        className="h-full"
      />
    );
  } else {
    content = (
      <EmptyState
        icon={ArrowDownToLine}
        title={t("response.noResponse")}
        description={t("response.noResponseHint")}
        className="h-full"
      />
    );
  }

  return (
    <section className="flex h-full min-h-0 flex-col bg-canvas">
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line pl-1.5 pr-2">
        <TabStrip items={TABS} value={tab} onChange={setTab} />

        <div className="flex-1" />

        <div className="flex items-center gap-3">
          <Metric
            icon={FileBraces}
            label={t("response.status")}
            value={status}
            valueClass={
              response
                ? statusTextClass(response.status)
                : running
                  ? "text-brand"
                  : undefined
            }
          />
          <Metric
            icon={Clock}
            label={t("response.time")}
            value={response ? formatMs(response.totalMs) : "—"}
          />
          <Metric
            icon={Timer}
            label={t("response.size")}
            value={response ? formatBytes(response.bodySize) : "—"}
          />
        </div>

        {response?.redirected ? (
          <Badge tone="info" size="md">
            {t("response.redirectedNote")}
          </Badge>
        ) : null}
      </div>

      {/* 未解析的占位符挂在**响应上**（它就是这一次发送的属性），
          所以横幅也画在响应区里 —— 而不是飘在请求区或弹一个 toast。
          排在最上面：它说的是「你发出去的东西可能不是你想要的」，
          优先级高于下面那条「这次用了什么 Token」的信息。

          `cURL` 页签下不画这两条：它们讲的是「上一次响应」的元信息，
          而那个页签里显示的可能是**另一份**命令（历史的、或按当前内容推演的）——
          两件事并排放会让人把它们当成同一份东西的属性。那页面板自己会说出
          更要紧的（没解析的占位符、顺带刷了 Token）。 */}
      {tab === "curl" ? null : (
        <>
          <UnresolvedBanner placeholders={response?.unresolvedPlaceholders ?? []} />

          {/* 「这一次到底刷没刷 Token」只有执行期知道（缓存命中与真刷新在结果上
              一模一样，事后从库里读不出来），所以它随响应一起回来，也画在这里。
              没有它的话，用户改完 Token 配置只能靠「发出去看 401」来验证 */}
          <TokenUsageBanner usage={response?.tokenUsage} />
        </>
      )}

      <div className="min-h-0 flex-1 overflow-hidden">{content}</div>
    </section>
  );
}
