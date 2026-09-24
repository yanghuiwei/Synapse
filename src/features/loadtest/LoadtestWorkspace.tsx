import { Gauge } from "lucide-react";
import { useEffect } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { useLoadtestStore } from "@/store/loadtest";
import { RunList } from "./RunList";
import { RunReport } from "./RunReport";

/**
 * 压测板块的主视图：**只有历史**。
 *
 * ## 为什么这里没有「打一轮」
 *
 * 打一轮的入口在**请求区的「压测」页签**（`RequestPane` → `LoadTestPanel`）。
 * 这不是随便挑的位置 —— 压测压的是**当前编辑器里的草稿**
 * （`store/loadtest.ts` 的 `start()` 直接读 `request.draft`），所以配置和执行
 * 必须紧挨着它们作用的那个请求：「改完地址点开始，打的就是这个地址」。
 *
 * 曾经这里有个「新压测」页签，把配置表单搬了过来 —— 那等于把动作和它作用的
 * 对象拆到两个地方：在这个板块改不了要打的请求，在请求区又看不到正在跑什么。
 * 已撤回。**这个板块现在只回答一个问题：以前跑过的那几轮，结果是什么样。**
 *
 * ## 自动选中最新一轮
 *
 * 进来看见一张空白页会让人以为「历史是空的」或「还得先点左边」，
 * 而最新一轮正是绝大多数时候要看的那份。等列表加载完再选，
 * 否则启动那一刻 `runs` 还是空的。
 */
export function LoadtestWorkspace({ className }: { className?: string }) {
  const { t } = useI18n();

  const runs = useLoadtestStore((s) => s.runs);
  const detail = useLoadtestStore((s) => s.detail);
  const openRun = useLoadtestStore((s) => s.openRun);
  const loading = useLoadtestStore((s) => s.runsLoading);

  useEffect(() => {
    if (loading || runs.length === 0 || detail !== null) return;
    const latest = runs[0];
    if (latest) void openRun(latest.id);
  }, [loading, runs, detail, openRun]);

  return (
    <section className={cn("flex h-full min-h-0 flex-col bg-canvas", className)}>
      {/* 标题行。与 `RequestPane` 的地址栏同为 h-11，切板块时中间区不会跳高 */}
      <header className="flex h-11 shrink-0 items-center gap-2 border-b border-line px-2.5">
        <Gauge size={13} strokeWidth={2.2} aria-hidden className="shrink-0 text-ink-faint" />
        <h2 className="shrink-0 text-[13px] font-semibold tracking-tight text-ink">
          {t("loadtest.historyTitle")}
        </h2>
      </header>

      {/* 一轮都没有时不分左右两栏：右边会是空的，还把「还没有压测记录」
          这句空态挤在左窄栏里。整块交给 `RunList` 自己的空态 */}
      {runs.length === 0 ? (
        <div className="min-h-0 flex-1 overflow-y-auto px-3 py-2">
          <RunList />
        </div>
      ) : (
        <div className="flex min-h-0 flex-1">
          <div className="min-h-0 w-[320px] min-w-[280px] shrink-0 overflow-y-auto border-r border-line px-1.5 pb-2 pt-2">
            <RunList />
          </div>

          {/* `min-w-0` 不能少：报告里的长 URL 与图表会把这一栏顶宽 */}
          <div className="min-h-0 min-w-0 flex-1">
            <RunReport />
          </div>
        </div>
      )}
    </section>
  );
}
