import { Clock, FileBraces, Info, Timer } from "lucide-react";
import { useState } from "react";

import { cn } from "@/lib/cn";
import { formatBytes, formatMs } from "@/lib/format";
import { statusTextClass } from "@/lib/http";
import { useI18n } from "@/lib/i18n";
import * as ipc from "@/lib/ipc";
import { zhCN } from "@/lib/i18n/zh-CN";
import type { HistoryEntry, HttpResponsePayload } from "@/lib/types";
import { ResponseViewer, type ResponseBodyTab } from "@/features/response/ResponseViewer";
import { Dialog } from "@/features/ui/Dialog";
import { EmptyState } from "@/features/ui/EmptyState";
import { TabStrip, type TabItem } from "@/features/ui/TabStrip";

/**
 * 历史详情里给出的三个页签。
 *
 * `timeline` **刻意不给**（尽管 `ResponseViewer` 支持它）：那一页画的是
 * ttfb / 下载两段耗时，而 010 没有存它们。拿 0 去画会得到一张「首字节 0ms、
 * 下载 0ms」的图 —— 那不是「没有数据」，是**假数据**，而且看起来还挺像真的。
 */
type HistoryBodyTab = Extract<ResponseBodyTab, "body" | "headers" | "cookies">;

const TABS: readonly TabItem<HistoryBodyTab>[] = [
  { key: "body", labelKey: "response.tab.body" },
  { key: "headers", labelKey: "response.tab.headers" },
  { key: "cookies", labelKey: "response.tab.cookies" },
];

/**
 * 把历史行拼成一份 `ResponseViewer` 认得的响应。
 *
 * ⚠️ 这是**显式降级**而不是伪装 —— 010 只存了状态、响应头、响应体三样。
 * 所以：
 *
 * - `ttfbMs` / `downloadMs` 填 0，同时**不提供 timeline 页签**（见上面的注释）；
 * - `bodyEncoding` 恒为 `"text"`：010 对二进制响应存的是 `null`（内容本来就在
 *   后端临时区，随那次会话结束就没了）。但那种情况**由调用方显式处理**，
 *   不交给 `BodyView` 的 binary 分支 —— 它说的是「临时区写不进去」，
 *   而历史里没内容是「当时没留」，两句原因不一样，不能混用。
 */
function payloadOf(entry: HistoryEntry): HttpResponsePayload {
  const url = typeof entry.snapshot["url"] === "string" ? entry.snapshot["url"] : "";

  return {
    status: entry.status ?? 0,
    statusText: "",
    httpVersion: "",
    headers: entry.respHeaders ?? [],
    url,
    redirected: false,
    body: entry.respBody ?? "",
    bodyEncoding: "text",
    bodySize: entry.respSize ?? 0,
    truncated: entry.respBodyTruncated === true,
    ttfbMs: 0,
    downloadMs: 0,
    totalMs: entry.durationMs ?? 0,
    unresolvedPlaceholders: [],
  };
}

/** 一枚指标：图标 + 值。与响应面板那排胶囊同一个读法 */
function Metric({
  icon: Icon,
  label,
  value,
  className,
}: {
  icon: typeof Clock;
  label: string;
  value: string;
  className?: string;
}) {
  return (
    <span className="flex items-center gap-1 text-[11px] text-ink-faint">
      <Icon size={11} strokeWidth={2.2} aria-hidden />
      <span className="sr-only">{label}</span>
      <span className={cn("tabular", className ?? "text-ink-muted")}>{value}</span>
    </span>
  );
}

function ExecutedAt({ iso }: { iso: string }) {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return <span className="text-ink-faint">{iso}</span>;
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    <span className="tabular text-ink-muted">
      {`${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${pad(at.getHours())}:${pad(at.getMinutes())}:${pad(at.getSeconds())}`}
    </span>
  );
}

/**
 * 这条历史有没有响应可看。
 *
 * 单独一个函数是因为它有两个调用方，而它们的判断**必须一致**：
 * `HistoryList` 用它决定菜单项置不置灰，本文件用它决定显示响应还是
 * 「这次没有响应」。两边不一致的话，会出现「能点开、点开是空的」。
 */
export function hasResponse(entry: HistoryEntry): boolean {
  return entry.status !== null || entry.respBody !== null;
}

/**
 * 「查看当时的响应」。
 *
 * 历史在此之前只有状态码、耗时、体积三个标量 —— 能看出「200 · 143ms · 1.2KB」，
 * 但看不到那 1.2KB 是什么。隔天想复核一个返回结构，唯一的办法是再发一次，
 * 而那已经是**另一次执行**了（环境变了、Token 换了、服务端发过版）。
 *
 * 为什么另开一个对话框而不是把响应灌回下半部分的响应区：历史存的是快照，
 * 与当前编辑器里那条请求没有关系。灌回去会让「上半部分显示 A、下半部分显示
 * 三天前 A 的响应」变成一种看起来正常的状态 —— 而这正是最容易被误读的一种。
 */
export function HistoryResponseDialog({
  entry,
  onClose,
}: {
  entry: HistoryEntry;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const [tab, setTab] = useState<HistoryBodyTab>("body");

  const hasAnythingToShow = hasResponse(entry);
  const errorLabel =
    entry.error !== null && ipc.isErrorCode(entry.error)
      ? zhCN[ipc.ERROR_KEYS[entry.error]]
      : entry.error;

  return (
    <Dialog
      title={t("history.responseTitle", { name: entry.requestName })}
      onClose={onClose}
      width={780}
    >
      {/* 指标行。状态用与响应面板同一套着色（2xx 绿、4xx 黄、5xx 红） */}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-line pb-2">
        <Metric
          icon={FileBraces}
          label={t("response.status")}
          value={entry.status === null ? t("history.failed") : String(entry.status)}
          className={entry.status === null ? "text-err" : statusTextClass(entry.status)}
        />
        <Metric
          icon={Clock}
          label={t("response.time")}
          value={entry.durationMs === null ? "—" : formatMs(entry.durationMs)}
        />
        <Metric
          icon={Timer}
          label={t("response.size")}
          value={entry.respSize === null ? "—" : formatBytes(entry.respSize)}
        />
        <span className="ml-auto flex items-center gap-1 text-[11px] text-ink-faint">
          <span>{t("history.executedAt")}</span>
          <ExecutedAt iso={entry.executedAt} />
        </span>
      </div>

      {/* 环境。`null` 是真实状态（不使用环境），不是缺数据 */}
      <p className="mt-1.5 flex items-center gap-1 text-[11px] text-ink-faint">
        <span>{t("env.title")}</span>
        <span className="text-ink-muted">
          {entry.environment ?? t("env.none")}
        </span>
      </p>

      {hasAnythingToShow ? (
        <>
          <p className="mt-2 flex items-start gap-1.5 rounded-sm bg-canvas-subtle px-2 py-1.5 text-[11px] leading-relaxed text-ink-faint">
            <Info size={12} strokeWidth={2} className="mt-px shrink-0" aria-hidden />
            <span>{t("history.responseSnapshotHint")}</span>
          </p>

          <div className="mt-2 flex h-8 items-center border-b border-line">
            <TabStrip items={TABS} value={tab} onChange={setTab} />
          </div>

          <div className="h-[400px] overflow-hidden">
            {tab === "body" && entry.respBody === null ? (
              // 010 对二进制 / 文件型响应存的是 `null`（内容只在那次执行里存在）。
              // 这句话得由历史自己说 —— `ResponseViewer` 的 binary 分支讲的是
              // 「临时区写不进去」，那是另一个原因。
              <p className="p-4 text-xs leading-relaxed text-ink-faint">
                {t("history.responseBodyMissing", {
                  size: formatBytes(entry.respSize ?? 0),
                })}
              </p>
            ) : (
              <ResponseViewer response={payloadOf(entry)} tab={tab} className="h-full" />
            )}
          </div>
        </>
      ) : (
        <EmptyState
          icon={Clock}
          title={t("history.responseNone")}
          description={
            errorLabel === null
              ? t("history.responseNoneHint")
              : `${t("history.responseNoneHint")} ${errorLabel}`
          }
          className="py-10"
        />
      )}
    </Dialog>
  );
}
