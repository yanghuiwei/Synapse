import { Clock } from "lucide-react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { useHistoryStore } from "@/store/history";
import { HistoryDetail } from "./HistoryDetail";
import { HistoryList } from "./HistoryList";

/**
 * 历史板块的**主视图**。
 *
 * 左边是快照流（`HistoryList`），右边是选中那条的详情。
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
  const selected = items.find((i) => i.id === selectedId) ?? null;

  return (
    <section className={cn("flex h-full min-h-0 flex-col bg-canvas", className)}>
      {/* 标题行。与 `RequestPane` 的地址栏同为 h-11，切板块时中间区不会跳高 */}
      <header className="flex h-11 shrink-0 items-center gap-2 border-b border-line px-2.5">
        <Clock size={13} strokeWidth={2.2} aria-hidden className="shrink-0 text-ink-faint" />
        <h2 className="text-[13px] font-semibold tracking-tight text-ink">
          {t("history.title")}
        </h2>
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
    </section>
  );
}
