import { Gauge, LoaderCircle, Trash } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { LoadtestRun } from "@/lib/types";
import { Badge } from "@/features/ui/Badge";
import { ContextMenu, type MenuItem } from "@/features/ui/ContextMenu";
import { ConfirmDialog } from "@/features/ui/Dialog";
import { EmptyState } from "@/features/ui/EmptyState";
import { useLoadtestStore } from "@/store/loadtest";
import { formatDuration, formatPercent } from "./format";
import { runConfigOf, runSummaryOf, runTargetOf } from "./runSnapshot";

/**
 * 侧栏里的历史压测列表。
 *
 * 与「历史请求」那条列表的区别：那边是**快照流**（点一下回填编辑器），
 * 这边是**结果集**（点一下打开报告）。所以这里不做自动续页：
 * 压测一轮动辄几十秒到几分钟，一天攒不下几十条，一页 50 条够用很久。
 *
 * 被选中态跟着 `store.detail` 走，而不是本地 state：报告可能来自
 * 「刚跑完」而不是「点了列表」，本地 state 会漏掉那一种。
 */
export function RunList() {
  const { t } = useI18n();

  const runs = useLoadtestStore((s) => s.runs);
  const loading = useLoadtestStore((s) => s.runsLoading);
  const refreshRuns = useLoadtestStore((s) => s.refreshRuns);
  const openRun = useLoadtestStore((s) => s.openRun);
  const deleteRun = useLoadtestStore((s) => s.deleteRun);
  const activeId = useLoadtestStore((s) => s.detail?.run.id ?? null);
  const running = useLoadtestStore((s) => s.phase === "running");

  const [menu, setMenu] = useState<{ x: number; y: number; run: LoadtestRun } | null>(null);
  const [pendingDelete, setPendingDelete] = useState<LoadtestRun | null>(null);

  // 进这个板块就刷一次。列表里可能有别的窗口/上一次会话留下的记录
  useEffect(() => {
    void refreshRuns();
  }, [refreshRuns]);

  const menuItems = useMemo((): MenuItem[] => {
    if (!menu) return [];
    return [
      {
        key: "delete",
        label: t("loadtest.deleteRunTitle"),
        icon: Trash,
        danger: true,
        onSelect: () => setPendingDelete(menu.run),
      },
    ];
  }, [menu, t]);

  if (loading && runs.length === 0) {
    return (
      <div className="mt-8 flex items-center justify-center gap-2 px-6 text-xs text-ink-faint">
        <LoaderCircle size={13} strokeWidth={2.2} className="animate-spin" />
        <span>{t("common.loading")}</span>
      </div>
    );
  }

  if (runs.length === 0) {
    // 正在跑但还没有历史（第一轮）时，「还没有记录」是事实但没意义 ——
    // 换成一句更贴切的话
    return (
      <EmptyState
        icon={Gauge}
        title={t("common.empty")}
        description={running ? t("loadtest.firstRunRunning") : t("loadtest.noRuns")}
        className="mt-8"
      />
    );
  }

  return (
    <>
      {/* 计数行。**不再写「历史压测」** —— 页签上已经有了，再说一遍是同一句话两次。
          左边留空则整行只剩「右键管理」右对齐，所以保留计数、只换文案 */}
      <div className="flex items-baseline gap-2 px-1.5 pb-1 pt-0.5">
        <span className="text-[10px] text-ink-faint">
          {t("history.total", { count: runs.length })}
        </span>
        {/* 右键菜单没有任何视觉痕迹，得明说一句 —— 否则这个功能等于不存在 */}
        <span className="ml-auto shrink-0 text-[10px] text-ink-faint/70">
          {t("history.rightClickHint")}
        </span>
      </div>

      <ul className="flex flex-col gap-px">
        {runs.map((run) => (
          <li key={run.id}>
            <RunRow
              run={run}
              active={run.id === activeId}
              onOpen={() => void openRun(run.id)}
              onMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setMenu({ x: e.clientX, y: e.clientY, run });
              }}
            />
          </li>
        ))}
      </ul>

      {menu ? (
        <ContextMenu items={menuItems} x={menu.x} y={menu.y} onClose={() => setMenu(null)} />
      ) : null}

      {/* 删除压测记录是**不可撤销**的（明细不在回收站里）--> 用红色确认框 */}
      {pendingDelete ? (
        <ConfirmDialog
          danger
          title={t("loadtest.deleteRunTitle")}
          description={t("loadtest.deleteRunDesc", { id: pendingDelete.id })}
          confirmLabel={t("common.delete")}
          onConfirm={() => void deleteRun(pendingDelete.id)}
          onClose={() => setPendingDelete(null)}
        />
      ) : null}
    </>
  );
}

function RunRow({
  run,
  active,
  onOpen,
  onMenu,
}: {
  run: LoadtestRun;
  active: boolean;
  onOpen: () => void;
  onMenu: (e: React.MouseEvent) => void;
}) {
  const { t } = useI18n();

  const config = runConfigOf(run);
  const target = runTargetOf(run);
  /**
   * 这一轮**跑成什么样**。
   *
   * 没有就不画第三行，而不是画一排零 ——「0% 成功率」和「还没跑」是相反的两件事，
   * 而这一列的意义正是「不用点开就能判断哪几轮值得看」。
   */
  const outcome = runSummaryOf(run);
  const unfinished = run.finishedAt === null;

  /**
   * 悬停提示：把行里放不下的数字补齐。
   *
   * 侧栏最窄能拖到 180px，一行塞四个指标就会换行；而「成功/失败各多少」
   * 恰恰是判断这一轮值不值得点开的关键 —— 成功率 99% 里那 1% 是一个还是十个，
   * 结论完全不同。
   */
  const hint = [
    target,
    outcome !== null
      ? [
          `${t("loadtest.succeeded")} ${outcome.succeeded}`,
          `${t("loadtest.failed")} ${outcome.failed}`,
          `${t("loadtest.cardAvgRps")} ${outcome.throughput.avgRps.toFixed(1)}`,
        ].join(" · ")
      : "",
    t("loadtest.openRun"),
  ]
    .filter((s) => s !== "")
    .join("\n");

  return (
    <button
      type="button"
      onClick={onOpen}
      onContextMenu={onMenu}
      title={hint}
      className={cn(
        "flex w-full flex-col gap-0.5 rounded-sm px-1.5 py-1 text-left transition-colors duration-75",
        active ? "bg-surface-hover" : "hover:bg-surface-hover",
      )}
    >
      <span className="flex w-full items-center gap-1.5">
        <span
          className={cn(
            "shrink-0 font-mono text-[10px] font-semibold",
            active ? "text-brand" : "text-ink-muted",
          )}
        >
          #{run.id}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-ink">{target}</span>
        {/* 被取消的那一轮概要是**残缺**的（只代表跑过的那一段），
            所以这个标签要和「未完成」一样显眼 —— 不然那些数字会被当成完整结果引用 */}
        {outcome !== null && outcome.cancelled ? (
          <RowTag>{t("loadtest.cancelledTag")}</RowTag>
        ) : null}
        {unfinished ? <RowTag>{t("loadtest.unfinished")}</RowTag> : null}
      </span>

      <span className="flex w-full items-center gap-1.5 text-[10px] text-ink-faint">
        <span className="tabular shrink-0">{stamp(run.startedAt)}</span>
        {config.concurrency !== undefined && config.total !== undefined ? (
          <span className="tabular shrink-0">
            {t("loadtest.runMeta", { concurrency: config.concurrency, total: config.total })}
          </span>
        ) : null}
      </span>

      {outcome !== null ? (
        <span className="flex w-full flex-wrap items-center gap-x-2 gap-y-px text-[10px]">
          <span className={cn("tabular shrink-0", rateClass(outcome.successRate))}>
            {t("loadtest.successRate")} {formatPercent(outcome.successRate)}
          </span>
          <span className="tabular shrink-0 text-ink-faint">
            {t("loadtest.p95")} {outcome.latency.p95Ms} ms
          </span>
          <span className="tabular shrink-0 text-ink-faint">
            {t("loadtest.elapsedShort")} {formatDuration(outcome.elapsedMs)}
          </span>
        </span>
      ) : null}
    </button>
  );
}

/** 行内的小标签。`未完成` / `已取消` 共用，省得两处各写一套圆角字号 */
function RowTag({ children }: { children: string }) {
  return (
    <Badge tone="warn" size="sm">
      {children}
    </Badge>
  );
}

/**
 * 成功率的三档配色。
 *
 * 与报告里「失败数」那格同一套语义：**全对才算 ok** —— 压测里
 * 「99.9% 成功」和「100% 成功」是两种结论，前者意味着服务端已经在丢请求了。
 */
function rateClass(rate: number): string {
  if (rate >= 1) return "text-ok";
  return rate > 0 ? "text-warn" : "text-err";
}

// ---------------------------------------------------------------
// 取值 / 格式化
// ---------------------------------------------------------------

// 「这一轮打了什么 / 打出来什么」全部来自 `./runSnapshot` —— 与报告页
// 共用同一份读取逻辑，免得两处对同一个 JSON 契约各理解一次。

/**
 * 时间戳。同一天只给 `HH:mm`，更早给 `MM-DD HH:mm`。
 *
 * 不用相对时间（「3 分钟前」）：压测记录是按「几点跑的」来对事的 ——
 * 用户多半在跟另一台机器上的日志对时间，绝对时刻比「3 分钟前」有用。
 */
function stamp(iso: string): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return "";
  const p = (n: number) => String(n).padStart(2, "0");
  const clock = `${p(at.getHours())}:${p(at.getMinutes())}`;
  const now = new Date();
  const sameDay =
    at.getFullYear() === now.getFullYear() &&
    at.getMonth() === now.getMonth() &&
    at.getDate() === now.getDate();
  return sameDay ? clock : `${p(at.getMonth() + 1)}-${p(at.getDate())} ${clock}`;
}
