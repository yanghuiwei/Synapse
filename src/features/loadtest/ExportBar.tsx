import { FileCode, FileJson, FileSpreadsheet, FolderOpen } from "lucide-react";
import { useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import { useI18n } from "@/lib/i18n";
import * as ipc from "@/lib/ipc";
import type { ExportFormat } from "@/lib/types";
import { Button, IconButton } from "@/features/ui/Button";
import { useLoadtestStore } from "@/store/loadtest";
import { buildReportLabels } from "./reportLabels";
import { Notice } from "./primitives";

/**
 * 导出三种格式，各有各的用处 —— 所以是三个按钮而不是一个下拉：
 * 下拉会把「有哪几种」藏起来，而这个功能一共只有三种。
 *
 * | 格式 | 给谁 |
 * |---|---|
 * | HTML | 发给别人看（自带内联样式，双击就能开） |
 * | CSV  | 自己丢进 Excel / 脚本里算 |
 * | JSON | CI 里跟前一次比对，判断有没有退化 |
 */
const FORMATS: readonly { format: ExportFormat; labelKey: "loadtest.exportHtml" | "loadtest.exportCsv" | "loadtest.exportJson"; icon: typeof FileCode }[] = [
  { format: "html", labelKey: "loadtest.exportHtml", icon: FileCode },
  { format: "csv", labelKey: "loadtest.exportCsv", icon: FileSpreadsheet },
  { format: "json", labelKey: "loadtest.exportJson", icon: FileJson },
];

export function ExportBar({
  disabled,
  hasSummary,
}: {
  disabled: boolean;
  /**
   * 这一轮有没有概要。
   *
   * CSV 是**原始样本**，没有概要也能导；HTML / JSON 都要靠概要拼内容，
   * 后端会直接拒（`no_summary_error`）。与其让它去撞一个错误对话框，
   * 不如把按钮禁掉并说清为什么。
   */
  hasSummary: boolean;
}) {
  const { t } = useI18n();
  const exported = useLoadtestStore((s) => s.exported);
  const exportCurrent = useLoadtestStore((s) => s.exportCurrent);

  // 哪一个格式正在导出。**不能**用一个布尔量：三个按钮会一起转圈，
  // 用户就不知道点的是哪个了
  const [busy, setBusy] = useState<ExportFormat | null>(null);

  // 导出错误留在本地，**不**用 store 里那个共享的 `error` ——
  // 那个字段还承载着「打开某轮报告失败」，混用会让打开失败的文案
  // 变成「导出失败」。两条错误的来源不同，就不该共用一句提示。
  const [err, setErr] = useState<ipc.IpcError | null>(null);

  const run = async (format: ExportFormat) => {
    setBusy(format);
    setErr(null);
    try {
      await exportCurrent(format, buildReportLabels(t));
    } catch (e) {
      setErr(ipc.toIpcError(e));
    } finally {
      setBusy(null);
    }
  };

  const reveal = async () => {
    if (exported === null) return;
    if (!ipc.isTauriRuntime()) return;
    try {
      await revealItemInDir(exported.path);
    } catch (e) {
      // 打不开目录不影响「文件已经导出成功」这个事实，所以只留控制台线索
      console.warn("[reqly] 打开导出目录失败：", e);
    }
  };

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-1.5">
        {FORMATS.map(({ format, labelKey, icon: Icon }) => {
          const needsSummary = format !== "csv";
          const blocked = disabled || busy !== null || (needsSummary && !hasSummary);
          return (
            <Button
              key={format}
              size="sm"
              variant="secondary"
              disabled={blocked}
              title={needsSummary && !hasSummary ? t("loadtest.exportNeedsSummary") : undefined}
              onClick={() => void run(format)}
              icon={<Icon size={12} strokeWidth={2.2} />}
            >
              {t(labelKey)}
            </Button>
          );
        })}
      </div>

      {err !== null ? (
        <Notice tone="err">
          {t("loadtest.exportFailed")}：{err.localized}
        </Notice>
      ) : null}

      {exported !== null ? (
        <div className="flex items-center gap-1.5 rounded-sm border border-line bg-canvas-subtle px-2 py-1.5">
          {/* 路径是这一行里唯一重要的信息，让它能选中复制 */}
          <p className="min-w-0 flex-1 select-text break-all font-mono text-[10px] text-ink-muted">
            {t("loadtest.exported", { path: exported.path })}
          </p>
          <IconButton size="sm" label={t("loadtest.reveal")} onClick={() => void reveal()}>
            <FolderOpen size={12} strokeWidth={2.2} />
          </IconButton>
        </div>
      ) : null}
    </div>
  );
}
