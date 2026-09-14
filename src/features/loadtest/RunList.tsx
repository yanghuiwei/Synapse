import { Gauge, LoaderCircle, Trash } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { LoadtestConfig, LoadtestRun } from "@/lib/types";
import { ContextMenu, type MenuItem } from "@/features/ui/ContextMenu";
import { ConfirmDialog } from "@/features/ui/Dialog";
import { EmptyState } from "@/features/ui/EmptyState";
import { useLoadtestStore } from "@/store/loadtest";

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
      <div className="flex items-baseline gap-2 px-1.5 pb-1 pt-0.5">
        <span className="text-[10px] text-ink-faint">
          {t("loadtest.historyRuns")} · {runs.length}
        </span>
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

  const cfg = configOf(run);
  const target = targetOf(run);
  const unfinished = run.finishedAt === null;

  const hint = [target, t("loadtest.openRun")].filter((s) => s !== "").join("\n");

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
        {unfinished ? (
          <span className="shrink-0 rounded-full bg-warn/12 px-1.5 text-[9px] leading-4 text-warn">
            {t("loadtest.unfinished")}
          </span>
        ) : null}
      </span>

      <span className="flex w-full items-center gap-1.5 text-[10px] text-ink-faint">
        <span className="tabular shrink-0">{stamp(run.startedAt)}</span>
        {cfg.concurrency !== undefined && cfg.total !== undefined ? (
          <span className="tabular shrink-0">
            {t("loadtest.runMeta", { concurrency: cfg.concurrency, total: cfg.total })}
          </span>
        ) : null}
      </span>
    </button>
  );
}

// ---------------------------------------------------------------
// 取值
// ---------------------------------------------------------------

/** 快照是 `{ target: { method, url }, config: LoadtestConfig }`；缺了就当没有 */
function configOf(run: LoadtestRun): Partial<LoadtestConfig> {
  const raw = run.config["config"];
  if (typeof raw !== "object" || raw === null) return {};
  const cfg = raw as Record<string, unknown>;
  return {
    concurrency: typeof cfg["concurrency"] === "number" ? cfg["concurrency"] : undefined,
    total: typeof cfg["total"] === "number" ? cfg["total"] : undefined,
  };
}

function targetOf(run: LoadtestRun): string {
  const raw = run.config["target"];
  if (typeof raw === "object" && raw !== null) {
    const { method, url } = raw as Record<string, unknown>;
    if (typeof method === "string" && typeof url === "string") return `${method} ${url}`;
  }
  return run.requestName !== "" ? run.requestName : `#${run.id}`;
}

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
