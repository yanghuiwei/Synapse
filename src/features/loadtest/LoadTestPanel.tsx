import { Ban, Gauge, LoaderCircle, Play, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";

import { cn } from "@/lib/cn";
import { methodTextClass } from "@/lib/http";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import { Button } from "@/features/ui/Button";
import { useLoadtestStore, type LoadtestPhase } from "@/store/loadtest";
import { useRequestStore } from "@/store/request";
import { ConfigForm } from "./ConfigForm";
import { LiveStats } from "./LiveStats";
import { ReportView } from "./ReportView";import { ScopeNote } from "./ScopeNote";
import { Notice, Section } from "./primitives";

/**
 * 「压测」页签的根。
 *
 * 一块屏幕从上到下只有三段：**打什么 → 怎么打 → 打出来什么**。
 * 不做子页签（配置/结果/图表）：压测的价值在于「改一个数字，重跑，看数字怎么动」，
 * 而分页会让这条回路多两次点击，还会把配置藏起来 —— 重跑时最需要看着它。
 *
 * 一句话说清一个设计选择：**配置区在跑的时候不隐藏，只禁用**。
 * 隐藏会让人在等结果的时候忘记「这一轮打的是什么参数」，
 * 而那份参数恰恰是解读数字的前提。
 */
export function LoadTestPanel({ className }: { className?: string }) {
  const { t } = useI18n();

  const phase = useLoadtestStore((s) => s.phase);
  const config = useLoadtestStore((s) => s.config);
  const progress = useLoadtestStore((s) => s.progress);
  const livePoints = useLoadtestStore((s) => s.livePoints);
  const detail = useLoadtestStore((s) => s.detail);
  const chart = useLoadtestStore((s) => s.chart);
  const detailLoading = useLoadtestStore((s) => s.detailLoading);
  const cancelling = useLoadtestStore((s) => s.cancelling);
  const error = useLoadtestStore((s) => s.error);
  const start = useLoadtestStore((s) => s.start);
  const cancel = useLoadtestStore((s) => s.cancel);

  // 目标来自**当前的草稿**，不是某一轮的快照 —— 这个头部回答的是
  // 「按下去会打哪里」，而那个问题永远是现在时
  const draft = useRequestStore((s) => s.draft);
  const running = phase === "running";
  const canStart = draft.url.trim() !== "" && !running;

  /**
   * 配置区的展开态。
   *
   * 跑起来之后**自动收起**，不是为了省事，是因为空间真的不够：
   * 请求面板默认占纵向 56%（约 525px），减去地址栏 + 页签栏，
   * 内容区约 450px；而配置表单自己就要 230px。收起来这 230px 正好是
   * 「概要 + 实时图表」那一屏 —— 不收的话报告基本看不见。
   *
   * 收起**不等于藏起来**：标题右边会顶上 `50 并发 · 500 请求` 这行摘要，
   * 而「这一轮打的是什么参数」正是解读报告数字的前提。
   *
   * 用户手动展开后不会被抢回去 —— 这个 effect 只在 phase **变化**时动手。
   */
  const [configOpen, setConfigOpen] = useState(true);
  useEffect(() => {
    // 待配置（idle）和跑失败（failed）时展开：那两种状态下用户就是来改参数的
    setConfigOpen(phase === "idle" || phase === "failed");
  }, [phase]);

  return (
    <section className={cn("flex h-full min-h-0 flex-col bg-canvas", className)}>
      {/* 顶部：目标 + 唯一的那个动作 */}
      <div className="flex h-11 shrink-0 items-center gap-2 border-b border-line px-2.5">
        <Gauge size={13} strokeWidth={2.2} aria-hidden className="shrink-0 text-ink-faint" />

        <span className="flex min-w-0 flex-1 items-center gap-1.5" title={t("loadtest.targetHint")}>
          <span
            className={cn(
              "shrink-0 font-mono text-[11px] font-semibold",
              methodTextClass(draft.method),
            )}
          >
            {draft.method}
          </span>
          <span className="min-w-0 truncate font-mono text-[11px] text-ink-muted">
            {draft.url.trim() === "" ? t("loadtest.needUrl") : draft.url}
          </span>
        </span>

        <PhaseTag phase={phase} />

        {running ? (
          <Button
            variant="danger"
            size="md"
            className="shrink-0"
            icon={<Ban size={13} strokeWidth={2.2} />}
            disabled={cancelling}
            // 取消语义挂在这里而不是实时面板里：用户真正需要这句话的一刻，
            // 就是鼠标移到「停止」上的那一刻。放在下面既占地方又赶不上时候
            title={t("loadtest.stoppingHint")}
            onClick={() => void cancel()}
          >
            {cancelling ? t("loadtest.stopping") : t("loadtest.stop")}
          </Button>
        ) : (
          <Button
            variant="primary"
            size="md"
            className="shrink-0"
            disabled={!canStart}
            title={canStart ? undefined : t("loadtest.needUrl")}
            icon={<Play size={13} strokeWidth={2.2} />}
            onClick={() => void start()}
          >
            {t("loadtest.start")}
          </Button>
        )}
      </div>

      <div className="min-h-0 flex-1 space-y-5 overflow-auto px-3 py-3">
        <Section
          title={t("loadtest.config")}
          collapsible
          open={configOpen}
          onToggle={() => setConfigOpen((v) => !v)}
          summary={t("loadtest.runMeta", {
            concurrency: config.concurrency,
            total: config.total,
          })}
        >
          <ConfigForm disabled={running} />
        </Section>

        {running || progress !== null ? (
          <Section title={t("loadtest.liveTitle")}>
            <LiveStats config={config} progress={progress} livePoints={livePoints} />
          </Section>
        ) : null}

        {/* 被取消在**预热阶段**：后端把那一轮的记录删掉了，所以没有报告可给。
            这不是错误，但必须说清「为什么什么都没有」*/}
        {phase === "stopped" ? <Notice tone="info">{t("loadtest.stoppedNoReport")}</Notice> : null}

        {phase === "failed" && error !== null ? (
          <Notice tone="err">
            <span className="inline-flex items-center gap-1">
              <TriangleAlert size={11} strokeWidth={2.2} aria-hidden />
              {error.localized}
            </span>
            {error.debug !== "" ? (
              <span className="mt-1 block select-text font-mono text-[10px] opacity-80">
                {error.debug}
              </span>
            ) : null}
          </Notice>
        ) : null}

        {/* 打开历史轮次失败。与上面那条分开，因为语义不同：
            一条是「刚才那一轮没能跑起来」，一条是「这条记录读不出来」*/}
        {phase !== "failed" && error !== null ? (
          <Notice tone="err">{error.localized}</Notice>
        ) : null}

        {detailLoading ? (
          <p className="flex items-center justify-center gap-2 py-4 text-[11px] text-ink-faint">
            <LoaderCircle size={12} strokeWidth={2.2} className="animate-spin" />
            {t("common.loading")}
          </p>
        ) : detail === null ? (
          // 没有报告时**什么都不画**，而不是放一个「还没有报告」的空态。
          // 那个空态和侧栏「还没有压测记录」说的是同一件事，而且它会占掉
          // 150px —— 正好把下面那块「能力边界」顶出可视区。而能力边界
          // （design.md §7.5）是**必须在这个界面上看得见**的东西：
          // 用户正要配 500 并发的时候，最该知道这工具做不到分布式加压。
          null
        ) : (
          <>
            {/* 图表是**第二个**请求（概要先到）。这段间隙里图表还是空的，
                不说明的话「暂无数据」会被读成「这一轮没采到数据」*/}
            {chart === null ? (
              <p className="flex items-center gap-2 text-[11px] text-ink-faint">
                <LoaderCircle size={12} strokeWidth={2.2} className="animate-spin" />
                {t("loadtest.chartLoading")}
              </p>
            ) : null}
            <ReportView detail={detail} chart={chart} />
          </>
        )}

        <ScopeNote />
      </div>
    </section>
  );
}

/** 阶段标签。只做「现在是什么状态」这一件事，动作在右边那个按钮上 */
const PHASE_TAG: Record<LoadtestPhase, { labelKey: MessageKey; className: string }> = {
  idle: { labelKey: "loadtest.phaseIdle", className: "text-ink-faint" },
  running: { labelKey: "loadtest.phaseRunning", className: "text-brand" },
  done: { labelKey: "loadtest.phaseDone", className: "text-ok" },
  stopped: { labelKey: "loadtest.phaseStopped", className: "text-warn" },
  failed: { labelKey: "loadtest.phaseFailed", className: "text-err" },
};

function PhaseTag({ phase }: { phase: LoadtestPhase }) {
  const { t } = useI18n();
  const { labelKey, className } = PHASE_TAG[phase];

  return <span className={cn("shrink-0 font-mono text-[10px]", className)}>{t(labelKey)}</span>;
}
