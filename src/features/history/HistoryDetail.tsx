import { Clock, CornerUpLeft, FileBraces, Info, LoaderCircle, Timer } from "lucide-react";
import { useState } from "react";

import { cn } from "@/lib/cn";
import { formatBytes, formatMs } from "@/lib/format";
import { methodTextClass } from "@/lib/http";
import { useI18n } from "@/lib/i18n";
import * as ipc from "@/lib/ipc";
import { zhCN } from "@/lib/i18n/zh-CN";
import type { HistoryEntry, HttpResponsePayload } from "@/lib/types";
import { ResponseViewer, type ResponseBodyTab } from "@/features/response/ResponseViewer";
import { Button } from "@/features/ui/Button";
import { EmptyState } from "@/features/ui/EmptyState";
import { TabStrip, type TabItem } from "@/features/ui/TabStrip";
import { useRequestStore } from "@/store/request";
import { useUiStore } from "@/store/ui";
import { hasResponse } from "./hasResponse";
import { methodOf, specOf, statusClassOf, statusLabelOf, urlOf } from "./HistoryList";

/**
 * 历史主视图的**右侧那一半**：选中那一条的详情。
 *
 * ## 与旧弹窗的区别
 *
 * 之前它是 `HistoryResponseDialog`（一个 780px 的模态框）。改成常驻面板之后，
 * 左边滚列表、右边看响应可以同时进行 —— 而核对「三天前那次返回什么」
 * 本来就是要对着几条记录来回比的，模态框每换一条都得关一次再开一次。
 *
 * 搬过来时**保留**了弹窗那版的全部判断，一处没改：
 * `timeline` 页签仍然不给（010 没存 ttfb / 下载耗时，拿 0 去画是假数据）、
 * 二进制响应仍然由这里说「当时没留」而不是走 `ResponseViewer` 的 binary 分支
 * （那句讲的是「临时区写不进去」，两件事）。
 *
 * ## 三个状态
 *
 * 未选中 / 选中了但这次没有响应 / 选中了且有响应 —— 三种都要有交代，
 * 第三种才是最常见的。
 */

/**
 * 详情里给出的三个页签。
 *
 * `timeline` **刻意不给**（尽管 `ResponseViewer` 支持它）：那一页画的是
 * ttfb / 下载两段耗时，而 010 没有存它们。拿 0 去画会得到一张
 * 「首字节 0ms、下载 0ms」的图 —— 那不是「没有数据」，是**假数据**，
 * 而且看起来还挺像真的。
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
  return {
    status: entry.status ?? 0,
    statusText: "",
    httpVersion: "",
    headers: entry.respHeaders ?? [],
    url: urlOf(entry),
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

export function HistoryDetail({ entry }: { entry: HistoryEntry | null }) {
  const { t } = useI18n();

  if (entry === null) {
    return (
      <EmptyState
        icon={Clock}
        title={t("history.detailEmpty")}
        description={t("history.detailEmptyHint")}
        className="h-full"
      />
    );
  }

  return <Detail ready={entry} />;
}

function Detail({ ready }: { ready: HistoryEntry }) {
  const { t } = useI18n();
  const [tab, setTab] = useState<HistoryBodyTab>("body");

  const entry = ready;
  const method = methodOf(entry);
  const url = urlOf(entry);
  const canFill = specOf(entry) !== null;
  const hasAnythingToShow = hasResponse(entry);
  const errorLabel =
    entry.error !== null && ipc.isErrorCode(entry.error)
      ? zhCN[ipc.ERROR_KEYS[entry.error]]
      : entry.error;

  /**
   * 回填编辑器。与右键菜单里那一项是**同一个动作**，
   * 所以同样会切到「集合」—— 否则点了没有任何可见的变化。
   */
  const fill = () => {
    const spec = specOf(entry);
    if (!spec) return;
    useUiStore.getState().setActiveRequestId(null);
    useUiStore.getState().setSection("collections");
    useRequestStore.getState().openSpec(spec, entry.requestName);
  };

  return (
    <section className="flex h-full min-h-0 flex-col">
      {/* 头部：这一条是什么 */}
      <header className="flex shrink-0 flex-col gap-1.5 border-b border-line px-3 py-2">
        <div className="flex flex-wrap items-center gap-2">
          <span
            className={cn(
              "shrink-0 font-mono text-[11px] font-semibold tracking-tight",
              methodTextClass(method),
            )}
          >
            {method}
          </span>
          <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-ink" title={url}>
            {url || t("common.unsaved")}
          </span>
          <span className={cn("shrink-0 font-mono text-[11px]", statusClassOf(entry))}>
            {statusLabelOf(entry, t)}
          </span>
          <Button
            size="sm"
            variant="secondary"
            icon={<CornerUpLeft size={13} strokeWidth={2} />}
            disabled={!canFill}
            onClick={fill}
          >
            {t("history.fill")}
          </Button>
        </div>

        {/* 指标行。状态用与响应面板同一套着色（2xx 绿、4xx 黄、5xx 红） */}
        <dl className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px] text-ink-faint">
          <Metric
            icon={FileBraces}
            label={t("response.status")}
            value={entry.status === null ? t("history.failed") : String(entry.status)}
            className={statusClassOf(entry)}
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
          <div className="flex items-center gap-1">
            <dt>{t("history.executedAt")}</dt>
            <dd className="tabular font-mono text-ink-muted">{executedAt(entry.executedAt)}</dd>
          </div>
          {/* 环境。`null` 是真实状态（不使用环境），不是缺数据 */}
          <div className="flex items-center gap-1">
            <dt>{t("env.title")}</dt>
            <dd className="text-ink-muted">{entry.environment ?? t("env.none")}</dd>
          </div>
          {/* 请求名只在它和地址不一样时才有信息量 —— 否则这一行会重复上面的地址 */}
          {entry.requestName !== "" && entry.requestName !== url ? (
            <div className="flex items-center gap-1">
              <dt>{t("history.requestName")}</dt>
              <dd className="truncate text-ink-muted">{entry.requestName}</dd>
            </div>
          ) : null}
        </dl>
      </header>

      {hasAnythingToShow ? (
        <>
          <p className="flex shrink-0 items-start gap-1.5 border-b border-line bg-canvas-subtle px-3 py-1.5 text-[11px] leading-relaxed text-ink-faint">
            <Info size={12} strokeWidth={2} className="mt-px shrink-0" aria-hidden />
            <span>{t("history.responseSnapshotHint")}</span>
          </p>

          <div className="flex h-8 shrink-0 items-center border-b border-line pl-1.5">
            <TabStrip items={TABS} value={tab} onChange={setTab} />
          </div>

          <div className="min-h-0 flex-1 overflow-hidden">
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
          className="min-h-0 flex-1"
        />
      )}
    </section>
  );
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
    <div className="flex items-center gap-1">
      <Icon size={11} strokeWidth={2.2} aria-hidden />
      <dt className="sr-only">{label}</dt>
      <dd className={cn("tabular", className ?? "text-ink-muted")}>{value}</dd>
    </div>
  );
}

/** `YYYY-MM-DD HH:mm:ss`。解析不了就原样显示，别显示成 `Invalid Date` */
function executedAt(iso: string): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return iso;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${pad(at.getHours())}:${pad(at.getMinutes())}:${pad(at.getSeconds())}`;
}

/** 详情区在等历史列表首屏时用的加载态 */
export function HistoryDetailLoading() {
  const { t } = useI18n();
  return (
    <div className="flex h-full items-center justify-center gap-2 text-xs text-ink-faint">
      <LoaderCircle size={13} strokeWidth={2.2} className="animate-spin" />
      <span>{t("common.loading")}</span>
    </div>
  );
}
