import { cn } from "@/lib/cn";
import { LoadTestPanel } from "./LoadTestPanel";
import { RunList } from "./RunList";

/**
 * 压测板块的**主视图**。
 *
 * 左边是历史轮次（`RunList`），右边是「打一轮 + 看报告」（`LoadTestPanel`）——
 * 那个面板原本是**请求区里的一个页签**。搬出来有两个理由：
 *
 * 1. 左栏那六个图标是并列的功能区，点进「压测」中间却还是请求编辑器，
 *    等于点进来什么也没变；
 * 2. 「改一个数字，重跑，看数字怎么动」这条回路里，最有用的参照物是
 *    **上一次的结果**。两者同屏之后，跑完一轮就能直接跟左边历史里的
 *    上一轮对比 —— 而在页签形态下，报告和历史列表从来没有同时出现过。
 */
export function LoadtestWorkspace({ className }: { className?: string }) {
  return (
    <section className={cn("flex h-full min-h-0 flex-col bg-canvas", className)}>
      <div className="flex min-h-0 flex-1">
        <div className="min-h-0 w-[320px] min-w-[280px] shrink-0 overflow-y-auto border-r border-line px-1.5 pb-2 pt-2">
          <RunList />
        </div>

        {/* `min-w-0` 不能少：报告里的长 URL 与图表会把这一栏顶宽 */}
        <div className="min-h-0 min-w-0 flex-1">
          <LoadTestPanel className="h-full" />
        </div>
      </div>
    </section>
  );
}
