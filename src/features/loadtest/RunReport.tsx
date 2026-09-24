import { FileQuestion, LoaderCircle } from "lucide-react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { useLoadtestStore } from "@/store/loadtest";
import { EmptyState } from "@/features/ui/EmptyState";
import { Notice } from "./primitives";
import { ReportView } from "./ReportView";

/**
 * 「历史压测」页签的**右边那一半**：选中轮次的报告。
 *
 * ## 为什么不用 `LoadTestPanel`
 *
 * 一开始这里直接复用了「新压测」页那个面板（报告区本来就住在它里面），
 * 结果两页共享同一份 `detail`：切回「新压测」时它还顶着
 * 「正在看第 #5 轮」和一份别人的报告 —— 页签只切了容器，没切数据。
 * 于是「新压测」页为了把这个矛盾藏起来，还得用 `viewing` 去收配置区、
 * 换按钮文案，越补越绕。
 *
 * 现在两页各有各的东西：
 *
 * - **新压测** → `LoadTestPanel`：目标、配置、实时统计，**没有历史报告**
 * - **历史压测** → 这里：只画报告，**没有目标行、没有配置表单**
 *
 * 「打新一轮」的入口因此只在「新压测」页 —— 这反而更对：看历史的人
 * 是在读数据，不是在准备下一次。
 *
 * ## 三态
 *
 * 加载中 / 报告（有概要或没跑完，由 `ReportView` 自己分辨）/ 读不出来。
 * 没选中任何轮次的那一态由调用方兜住（自动选最新一轮）。
 */
export function RunReport({ className }: { className?: string }) {
  const { t } = useI18n();

  const detail = useLoadtestStore((s) => s.detail);
  const chart = useLoadtestStore((s) => s.chart);
  const detailLoading = useLoadtestStore((s) => s.detailLoading);
  const error = useLoadtestStore((s) => s.error);

  if (detailLoading) {
    return (
      <div
        className={cn(
          "flex h-full items-center justify-center gap-2 text-xs text-ink-faint",
          className,
        )}
      >
        <LoaderCircle size={13} strokeWidth={2.2} className="animate-spin" />
        <span>{t("common.loading")}</span>
      </div>
    );
  }

  // 读某一轮失败（记录被删了 / 库出错）。与「这一段没有报告」分开：
  // 前者是出错了，后者是事实。错误优先，否则用户会以为记录本来就是空的
  if (error !== null && detail === null) {
    return (
      <div className={cn("p-3", className)}>
        <Notice tone="err">{error.localized}</Notice>
      </div>
    );
  }

  if (detail === null) {
    return (
      <EmptyState
        icon={FileQuestion}
        title={t("loadtest.pickRun")}
        description={t("loadtest.pickRunHint")}
        className="h-full"
      />
    );
  }

  return (
    <div className={cn("h-full overflow-auto px-3 py-3", className)}>
      {/* 图表是**第二个**请求（概要先到）。这段间隙里图表还是空的，
          不说明的话「暂无数据」会被读成「这一轮没采到数据」 */}
      {chart === null ? (
        <p className="mb-3 flex items-center gap-2 text-[11px] text-ink-faint">
          <LoaderCircle size={12} strokeWidth={2.2} className="animate-spin" />
          {t("loadtest.chartLoading")}
        </p>
      ) : null}

      <ReportView detail={detail} chart={chart} />
    </div>
  );
}
