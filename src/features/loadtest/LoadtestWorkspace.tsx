import { Gauge } from "lucide-react";
import { useEffect, useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { TabStrip, type TabItem } from "@/features/ui/TabStrip";
import { useLoadtestStore } from "@/store/loadtest";
import { LoadTestPanel } from "./LoadTestPanel";
import { RunList } from "./RunList";
import { RunReport } from "./RunReport";

/**
 * 压测板块的**主视图**。两块内容用页签分开：
 *
 * - **新压测**：配参数、打一轮、看实时统计与报告（`LoadTestPanel`）
 * - **历史压测**：轮次列表 + 选中那轮的报告（`RunList`）
 *
 * ## 为什么要分页签
 *
 * 上一版把 `RunList` 铺在主视图左侧、`LoadTestPanel` 放在右边，结果：
 * 进板块第一眼看到的是一列历史记录，而「配参数 → 打一轮」才是这个功能的主线，
 * 却被挤在右边、还得先有草稿地址才能按开始。两块并排还各自要宽度
 * （列表要放 URL 加六个指标，配置要放 8 个字段），主区分成两半后都嫌窄。
 *
 * 分开之后每一种状态都能占满整块：历史列表从 320px 变成整宽，
 * 地址不用再截断成认不出来的样子。
 *
 * ## 默认落在「新压测」
 *
 * 点压测图标的人，绝大多数是来打一轮的；回看历史是个有明确目标的动作
 * （找某一轮），值得多一次点击。
 *
 * ## 侧栏在这个板块被让出去了
 *
 * 「压测」板块现在**不显示侧栏** —— 它本该放的东西（历史轮次列表）已经全在
 * 这个页签里了，留着那条空栏只会占掉 240px 并让人以为还有内容没加载出来。
 * 见 `App.tsx` 里 `section === "loadtest"` 那条分支。
 */
export function LoadtestWorkspace({ className }: { className?: string }) {
  const { t } = useI18n();
  const [tab, setTab] = useState<LoadtestTab>("new");

  /**
   * 上次看到过的轮次 id。
   *
   * ⚠️ 只在「打开的轮次**变了**」的那一刻切页签，而不是「当前有轮次打开就切」。
   * 后者会让用户手动切回「新压测」时被立刻拽回去 —— 因为那时 `detail` 还在。
   */
  const [seenId, setSeenId] = useState<number | null>(null);

  const openedId = useLoadtestStore((s) => s.detail?.run.id ?? null);
  if (openedId !== null && openedId !== seenId) {
    // 渲染期同步派生状态，是 React 官方给的写法：比 useEffect 少一帧闪烁
    // （effect 要等这帧画完才跑，用户会先看到「新压测」再跳到历史）
    setSeenId(openedId);
    setTab("history");
  }

  return (
    <section className={cn("flex h-full min-h-0 flex-col bg-canvas", className)}>
      <header className="flex h-11 shrink-0 items-center gap-2 border-b border-line px-2.5">
        <Gauge size={13} strokeWidth={2.2} aria-hidden className="shrink-0 text-ink-faint" />
        <h2 className="shrink-0 text-[13px] font-semibold tracking-tight text-ink">
          {t("nav.loadtest")}
        </h2>
        <span className="mx-1 h-4 w-px shrink-0 bg-line" aria-hidden />
        <TabStrip items={TABS} value={tab} onChange={setTab} className="px-0" />
      </header>

      <div className="min-h-0 flex-1">{tab === "new" ? <NewTab /> : <HistoryTab />}</div>
    </section>
  );
}

type LoadtestTab = "new" | "history";

const TABS: readonly TabItem<LoadtestTab>[] = [
  { key: "new", labelKey: "loadtest.tabNew" },
  { key: "history", labelKey: "loadtest.tabHistory" },
];

/** 「新压测」页。整块交给面板自己排（目标 → 配置 → 实时 → 报告） */
function NewTab() {
  return <LoadTestPanel className="h-full" />;
}

/**
 * 「历史压测」页：左边轮次列表，右边选中那轮的报告。
 *
 * 右边用的是 `RunReport`（**只画报告**），不是「新压测」页那个面板 ——
 * 复用面板会让两页共享同一份 `detail`，切回「新压测」时它还顶着
 * 「正在看第 #5 轮」。细节见 `RunReport` 的注释。
 */
function HistoryTab() {
  const runs = useLoadtestStore((s) => s.runs);
  const detail = useLoadtestStore((s) => s.detail);
  const openRun = useLoadtestStore((s) => s.openRun);
  const loading = useLoadtestStore((s) => s.runsLoading);

  /**
   * 一进这一页就自动打开最新一轮。
   *
   * 不留空白：右边空着会让人以为「历史是空的」或「点左边才出内容」，
   * 而最新一轮正是绝大多数时候要看的那一份。等列表加载完再选，
   * 否则启动那一刻 `runs` 还是空的。
   */
  useEffect(() => {
    if (loading || runs.length === 0 || detail !== null) return;
    const latest = runs[0];
    if (latest) void openRun(latest.id);
  }, [loading, runs, detail, openRun]);

  // 一轮都没有时分左右两栏没有意义：右边那半会是空的，还把「还没有压测记录」
  // 这句空态挤在左窄栏里。整块交给 `RunList` 自己的空态
  if (runs.length === 0) {
    return (
      <div className="h-full overflow-y-auto px-3 py-2">
        <RunList />
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0">
      <div className="min-h-0 w-[320px] min-w-[280px] shrink-0 overflow-y-auto border-r border-line px-1.5 pb-2 pt-2">
        <RunList />
      </div>

      {/* `min-w-0` 不能少：报告里的长 URL 与图表会把这一栏顶宽 */}
      <div className="min-h-0 min-w-0 flex-1">
        <RunReport />
      </div>
    </div>
  );
}
