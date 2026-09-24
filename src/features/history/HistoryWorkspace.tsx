import { Clock, Search, Trash, X } from "lucide-react";
import { useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { useHistoryStore } from "@/store/history";
import { IconButton } from "@/features/ui/Button";
import { ConfirmDialog } from "@/features/ui/Dialog";
import { HistoryDetail } from "./HistoryDetail";
import { HistoryList } from "./HistoryList";

/**
 * 历史板块的**主视图**。
 *
 * 左边是快照流（`HistoryList`），右边是选中那条的详情。
 *
 * ## 搜索与「清空」搬到这里来了
 *
 * 这两样原本住在**侧栏**（`Sidebar` 的标题区 + 搜索框）。侧栏被撤掉之后
 * 得有个去处，而主视图的标题行正是它们该在的位置：搜索框紧挨着它要筛的那份
 * 列表，而不是隔着一根分隔条在另一列里。
 *
 * ## 为什么左侧那一列的宽度是硬的
 *
 * 历史行要放「方法 + 名称 + 状态 + 地址 + 耗时/体积/时间」六样东西，
 * 窄于 300px 时地址会被截成认不出来。所以给 `min-w`，宁可让用户去拖分隔条，
 * 也不让它在窄宽下退化成一行看不出内容的文字。
 */
export function HistoryWorkspace({ className }: { className?: string }) {
  const { t } = useI18n();
  const selectedId = useHistoryStore((s) => s.selectedId);
  const items = useHistoryStore((s) => s.items);
  const keyword = useHistoryStore((s) => s.keyword);
  const setKeyword = useHistoryStore((s) => s.setKeyword);
  const selected = items.find((i) => i.id === selectedId) ?? null;

  const [clearOpen, setClearOpen] = useState(false);

  return (
    <section className={cn("flex h-full min-h-0 flex-col bg-canvas", className)}>
      {/* 标题行。与 `RequestPane` 的地址栏同为 h-11，切板块时中间区不会跳高 */}
      <header className="flex h-11 shrink-0 items-center gap-2 border-b border-line px-2.5">
        <Clock size={13} strokeWidth={2.2} aria-hidden className="shrink-0 text-ink-faint" />
        <h2 className="shrink-0 text-[13px] font-semibold tracking-tight text-ink">
          {t("history.title")}
        </h2>

        {/* 搜索框占满中间：它就是这一页的主筛选器，没必要挤在窄栏里 */}
        <div className="relative mx-1 min-w-0 flex-1">
          <Search
            size={13}
            strokeWidth={2}
            aria-hidden
            className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-ink-faint"
          />
          <input
            type="search"
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            placeholder={t("history.searchPlaceholder")}
            className={cn(
              "h-7 w-full rounded-sm border border-line bg-surface pl-7 pr-7",
              "text-xs text-ink placeholder:text-ink-faint",
              "transition-colors duration-100 hover:border-line-strong focus:border-brand",
            )}
          />
          {/* 有内容时给个清除按钮。`type=search` 自带的那个 ✕ 在
              WebView 里不一定画出来，不能指望它 */}
          {keyword !== "" ? (
            <button
              type="button"
              aria-label={t("common.clear")}
              onClick={() => setKeyword("")}
              className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded-sm p-0.5 text-ink-faint hover:bg-surface-hover hover:text-ink"
            >
              <X size={12} strokeWidth={2.2} />
            </button>
          ) : null}
        </div>

        {/* 清空历史是**能撤**的（记录进回收站），所以不套红色按钮 ——
            红色留给真正不可撤销的那几个操作 */}
        <IconButton
          size="sm"
          variant="danger"
          label={t("history.clearAll")}
          onClick={() => setClearOpen(true)}
        >
          <Trash size={14} strokeWidth={2} />
        </IconButton>
      </header>

      <div className="flex min-h-0 flex-1">
        <div className="min-h-0 w-[320px] min-w-[280px] shrink-0 overflow-y-auto border-r border-line px-1.5 pb-2">
          <HistoryList />
        </div>

        {/* `min-w-0` 不能少：没有它，右侧里那块长 URL / 响应体
            会把这一栏顶宽，左侧列被挤扁 */}
        <div className="min-h-0 min-w-0 flex-1">
          <HistoryDetail entry={selected} />
        </div>
      </div>

      {clearOpen ? (
        <ConfirmDialog
          title={t("history.clearConfirmTitle")}
          description={t("history.clearConfirm")}
          confirmLabel={t("history.clearAll")}
          onConfirm={() => void useHistoryStore.getState().clearAll()}
          onClose={() => setClearOpen(false)}
        />
      ) : null}
    </section>
  );
}
