import { FileQuestion, Folder as FolderIcon, LoaderCircle, RotateCcw, Trash } from "lucide-react";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { cn } from "@/lib/cn";
import { methodTextClass } from "@/lib/http";
import { useI18n } from "@/lib/i18n";
import type { ApiRequest, Folder, HistoryEntry, PurgeTrashResult } from "@/lib/types";
import { ConfirmDialog } from "@/features/ui/Dialog";
import { EmptyState } from "@/features/ui/EmptyState";
import { useTrashStore } from "@/store/trash";
import { useWorkspaceStore } from "@/store/workspace";

/**
 * 回收站。
 *
 * 它是全应用**唯一**会永久销毁数据的地方，所以有三条硬规矩：
 *
 * 1. **彻底删除必须二次确认**，并且说清楚「删的是哪些东西」——
 *    分类要写明会连带子分类与接口（后端真的会连带删）。
 * 2. **清空回收站在这里，不在侧栏标题栏**。它影响的是整个回收站，
 *    按钮就该贴着那张受影响的总览，而且能顺便把三类计数写进确认文案。
 * 3. **还原之后不留残影**：还原动作会把数据搬回集合树，所以那句
 *    「上级分类若仍在回收站，还原后会落到根级」必须显示出来 ——
 *    否则用户会以为还原失败（东西没出现在原来的位置）。
 */
export function TrashBin() {
  const { t } = useI18n();

  const status = useTrashStore((s) => s.status);
  const error = useTrashStore((s) => s.error);
  const contents = useTrashStore((s) => s.contents);
  const load = useTrashStore((s) => s.load);

  /** 待确认的彻底删除（单条） */
  const [purging, setPurging] = useState<
    { kind: "folder" | "request" | "history"; id: number; name: string } | null
  >(null);
  const [purgeAllOpen, setPurgeAllOpen] = useState(false);
  /** 上一次清空的结果 —— 清空后列表必然是空的，正好用空态把结果说清楚 */
  const [lastPurge, setLastPurge] = useState<PurgeTrashResult | null>(null);

  // 依赖工作区 id 而不是「挂载一次」：启动序列还没回来时没有工作区 id，
  // `load()` 会直接返回 —— 只跑一次的话就会永远卡在转圈上（与历史列表同理）
  const workspaceId = useWorkspaceStore((s) => s.workspace?.id ?? null);
  useEffect(() => {
    if (workspaceId !== null) void load();
  }, [workspaceId, load]);

  const counts = useMemo(
    () => ({
      folders: contents.folders.length,
      requests: contents.requests.length,
      history: contents.history.length,
    }),
    [contents],
  );
  const totalItems = counts.folders + counts.requests + counts.history;

  const purgeOne = useCallback(() => {
    if (!purging) return;
    const store = useTrashStore.getState();
    if (purging.kind === "folder") void store.purgeFolder(purging.id);
    else if (purging.kind === "request") void store.purgeRequest(purging.id);
    else void store.purgeHistory(purging.id);
  }, [purging]);

  if (status === "idle" || (status === "loading" && totalItems === 0)) {
    return (
      <div className="mt-8 flex items-center justify-center gap-2 px-6 text-xs text-ink-faint">
        <LoaderCircle size={13} strokeWidth={2.2} className="animate-spin" />
        <span>{t("common.loading")}</span>
      </div>
    );
  }

  // 一条内容都没有 + 有错误 → 只说错误。
  // 「回收站是空的」在这种时候是**假话**：失败的那次加载什么都没读到，
  // 我们根本不知道里面有没有东西。
  if (totalItems === 0 && error) {
    return (
      <EmptyState
        icon={FileQuestion}
        title={t("trash.loadFailed")}
        description={error.localized}
        action={
          <button
            type="button"
            onClick={() => void load()}
            className="h-7 rounded-sm border border-line bg-surface px-2.5 text-xs text-ink hover:bg-surface-hover"
          >
            {t("common.retry")}
          </button>
        }
        className="mt-8"
      />
    );
  }

  if (totalItems === 0) {
    return lastPurge ? (
      <EmptyState
        icon={Trash}
        title={t("common.empty")}
        description={t("trash.purgeAllDone", {
          folders: lastPurge.folders,
          requests: lastPurge.requests,
          history: lastPurge.history,
        })}
        className="mt-8"
      />
    ) : (
      <EmptyState
        icon={Trash}
        title={t("common.empty")}
        description={t("trash.empty")}
        className="mt-8"
      />
    );
  }

  return (
    <>
      {/* 总览 + 清空 */}
      <div className="flex items-center gap-2 px-1.5 pb-1 pt-0.5">
        <span className="min-w-0 flex-1 truncate text-[10px] text-ink-faint">
          {t("trash.counts", counts)}
        </span>
        <button
          type="button"
          onClick={() => setPurgeAllOpen(true)}
          className={cn(
            "flex h-6 shrink-0 items-center gap-1 rounded-sm border border-line px-1.5",
            "text-[10px] text-ink-muted transition-colors duration-100",
            "hover:border-err hover:bg-err/10 hover:text-err",
          )}
        >
          <Trash size={11} strokeWidth={2} aria-hidden />
          {t("trash.emptyAction")}
        </button>
      </div>

      {/* 还原 / 彻底删除失败的提示。列表照常显示真实内容，只是多一行说明 ——
          否则用户会对着一个「点了没反应」的列表反复点 */}
      {error ? (
        <p className="mb-1 rounded-sm border border-err/40 bg-err/10 px-1.5 py-1 text-[10px] leading-relaxed text-err">
          {error.localized}
        </p>
      ) : null}

      {contents.folders.length > 0 ? (
        <Group titleKey="trash.groupFolders" count={contents.folders.length}>
          {contents.folders.map((folder) => (
            <FolderRow
              key={`f-${folder.id}`}
              folder={folder}
              onPurge={() =>
                setPurging({ kind: "folder", id: folder.id, name: folder.name })
              }
            />
          ))}
        </Group>
      ) : null}

      {contents.requests.length > 0 ? (
        <Group titleKey="trash.groupRequests" count={contents.requests.length}>
          {contents.requests.map((request) => (
            <RequestRow
              key={`r-${request.id}`}
              request={request}
              onPurge={() =>
                setPurging({ kind: "request", id: request.id, name: request.name })
              }
            />
          ))}
        </Group>
      ) : null}

      {contents.history.length > 0 ? (
        <Group titleKey="trash.groupHistory" count={contents.history.length}>
          {contents.history.map((entry) => (
            <HistoryRow
              key={`h-${entry.id}`}
              entry={entry}
              onPurge={() =>
                setPurging({ kind: "history", id: entry.id, name: entry.requestName })
              }
            />
          ))}
        </Group>
      ) : null}

      {purging ? (
        <ConfirmDialog
          danger
          title={t("trash.purgeConfirmTitle")}
          description={t(
            purging.kind === "folder"
              ? "trash.purgeFolderConfirmDesc"
              : "trash.purgeConfirmDesc",
            { name: purging.name },
          )}
          confirmLabel={t("trash.purgeTitle")}
          onConfirm={purgeOne}
          onClose={() => setPurging(null)}
        />
      ) : null}

      {purgeAllOpen ? (
        <ConfirmDialog
          danger
          title={t("trash.purgeAllConfirmTitle")}
          description={t("trash.purgeAllConfirmDesc", counts)}
          confirmLabel={t("trash.emptyAction")}
          onConfirm={() => {
            void useTrashStore.getState().purgeAll().then(setLastPurge);
          }}
          onClose={() => setPurgeAllOpen(false)}
        />
      ) : null}
    </>
  );
}

// ---------------------------------------------------------------
// 分组与行
// ---------------------------------------------------------------

function Group({
  titleKey,
  count,
  children,
}: {
  titleKey: "trash.groupFolders" | "trash.groupRequests" | "trash.groupHistory";
  count: number;
  children: ReactNode;
}) {
  const { t } = useI18n();
  return (
    <div className="mb-1">
      <p className="px-1.5 pb-0.5 pt-2 text-[10px] font-medium tracking-wide text-ink-faint">
        {t(titleKey)}
        <span className="ml-1 tabular text-ink-faint/70">{count}</span>
      </p>
      <ul className="flex flex-col gap-px">{children}</ul>
    </div>
  );
}

/**
 * 行容器。
 *
 * 动作按钮**常驻可见**（不做悬停才出现）：回收站里只有「还原」和
 * 「彻底删除」两件事可做，把它们藏起来等于让界面变成只读列表 ——
 * 用户得先猜到这里能悬停。淡色 + 悬停加深，噪声换可达性，值。
 */
const ROW =
  "group flex h-[26px] items-center gap-1.5 rounded-sm px-1.5 text-xs " +
  "transition-colors duration-75 hover:bg-surface-hover";
const ROW_ACTION =
  "flex h-5 w-5 shrink-0 items-center justify-center rounded-sm text-ink-faint/70 " +
  "transition-colors duration-100 hover:bg-surface-hover hover:text-ink " +
  "focus-visible:text-ink";

function RowActions({
  onRestore,
  onPurge,
  purgeHint,
}: {
  onRestore: () => void;
  onPurge: () => void;
  purgeHint: string;
}) {
  const { t } = useI18n();
  return (
    <>
      <button
        type="button"
        title={t("common.restore")}
        aria-label={t("common.restore")}
        onClick={onRestore}
        className={ROW_ACTION}
      >
        <RotateCcw size={12} strokeWidth={2} aria-hidden />
      </button>
      <button
        type="button"
        title={`${t("trash.purgeTitle")}（${purgeHint}）`}
        aria-label={t("trash.purgeTitle")}
        onClick={onPurge}
        className={cn(ROW_ACTION, "hover:text-err")}
      >
        <Trash size={12} strokeWidth={2} aria-hidden />
      </button>
    </>
  );
}

function FolderRow({ folder, onPurge }: { folder: Folder; onPurge: () => void }) {
  const { t } = useI18n();
  return (
    <li>
      <div className={ROW} title={t("trash.restoreFolderHint")}>
        <FolderIcon size={12} strokeWidth={2} aria-hidden className="shrink-0 text-ink-faint" />
        <span className="min-w-0 flex-1 truncate text-ink">{folder.name}</span>
        {folder.isTokenSource ? (
          <span className="shrink-0 rounded-sm bg-brand/12 px-1 text-[10px] text-brand">
            {t("auth.tokenSource")}
          </span>
        ) : null}
        <RowActions
          onRestore={() => void useTrashStore.getState().restoreFolder(folder.id)}
          onPurge={onPurge}
          purgeHint={t("trash.purgeFolderHint")}
        />
      </div>
    </li>
  );
}

function RequestRow({ request, onPurge }: { request: ApiRequest; onPurge: () => void }) {
  const { t } = useI18n();
  return (
    <li>
      <div className={ROW} title={request.url}>
        <span
          className={cn(
            "w-[40px] shrink-0 font-mono text-[10px] font-semibold tracking-tight",
            methodTextClass(request.method),
          )}
        >
          {request.method}
        </span>
        <span className="min-w-0 flex-1 truncate text-ink-muted">{request.name}</span>
        <RowActions
          onRestore={() => void useTrashStore.getState().restoreRequest(request.id)}
          onPurge={onPurge}
          purgeHint={t("trash.purgeHint")}
        />
      </div>
    </li>
  );
}

function HistoryRow({ entry, onPurge }: { entry: HistoryEntry; onPurge: () => void }) {
  const { t } = useI18n();
  const method = typeof entry.snapshot["method"] === "string" ? entry.snapshot["method"] : "GET";
  return (
    <li>
      <div className={ROW}>
        <span
          className={cn(
            "w-[40px] shrink-0 font-mono text-[10px] font-semibold tracking-tight",
            methodTextClass(method),
          )}
        >
          {method}
        </span>
        <span className="min-w-0 flex-1 truncate text-ink-muted">{entry.requestName}</span>
        <RowActions
          onRestore={() => void useTrashStore.getState().restoreHistory(entry.id)}
          onPurge={onPurge}
          purgeHint={t("trash.purgeHint")}
        />
      </div>
    </li>
  );
}
