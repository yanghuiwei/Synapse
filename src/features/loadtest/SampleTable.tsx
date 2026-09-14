import { cn } from "@/lib/cn";
import { formatBytes } from "@/lib/format";
import { statusTextClass } from "@/lib/http";
import { useI18n } from "@/lib/i18n";
import * as ipc from "@/lib/ipc";
import { zhCN } from "@/lib/i18n/zh-CN";
import type { LoadtestSample } from "@/lib/types";

/**
 * 样本表。「最慢的请求」和「失败的请求」共用 —— 两边的差别只有两列，
 * 各写一份的话列宽、字号、对齐方式迟早会漂。
 *
 * 用真 `<table>` 而不是 div 网格：这是**表格数据**，表格语义能让
 * 读屏软件报出「第 3 行第 2 列，状态 500」。成本几乎为零。
 */
export type SampleTableKind = "slowest" | "failures";

export function SampleTable({
  rows,
  kind,
  emptyText,
  className,
}: {
  rows: readonly LoadtestSample[];
  kind: SampleTableKind;
  emptyText: string;
  className?: string;
}) {
  const { t } = useI18n();

  if (rows.length === 0) {
    return (
      <p className={cn("py-4 text-center text-[11px] text-ink-faint", className)}>{emptyText}</p>
    );
  }

  const cell = "px-2 py-1 text-left align-middle";

  return (
    <div className={cn("overflow-x-auto rounded-sm border border-line", className)}>
      <table className="w-full border-collapse">
        <thead>
          <tr className="bg-canvas-subtle text-[10px] text-ink-faint">
            <th className={cn(cell, "w-[52px] font-medium")}>{t("loadtest.seq")}</th>
            <th className={cn(cell, "w-[72px] font-medium")}>{t("loadtest.statusCode")}</th>
            <th className={cn(cell, "w-[88px] text-right font-medium")}>
              {t("loadtest.durationMs")}
            </th>
            <th className={cn(cell, "w-[96px] text-right font-medium")}>{t("loadtest.offsetMs")}</th>
            {kind === "slowest" ? (
              <th className={cn(cell, "w-[80px] text-right font-medium")}>{t("loadtest.bytes")}</th>
            ) : (
              <th className={cn(cell, "font-medium")}>{t("loadtest.code")}</th>
            )}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.id} className="border-t border-line/70 hover:bg-surface-hover">
              <td className={cn(cell, "tabular font-mono text-[11px] text-ink-faint")}>{row.seq}</td>
              <td className={cn(cell, "font-mono text-[11px]")}>
                {row.status === null ? (
                  <span className="text-ink-faint">{t("loadtest.noResponse")}</span>
                ) : (
                  <span className={statusTextClass(row.status)}>{row.status}</span>
                )}
              </td>
              <td className={cn(cell, "tabular text-right font-mono text-[11px] text-ink")}>
                {row.durationMs}
              </td>
              <td
                className={cn(cell, "tabular text-right font-mono text-[11px] text-ink-muted")}
              >
                {row.startedOffsetMs}
              </td>
              {kind === "slowest" ? (
                <td
                  className={cn(cell, "tabular text-right font-mono text-[11px] text-ink-muted")}
                >
                  {row.bytes === null ? "—" : formatBytes(row.bytes)}
                </td>
              ) : (
                <td className={cn(cell, "text-[11px] text-err")}>{errorText(row.error)}</td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/**
 * 错误码 → 给人看的文案。
 *
 * 带占位符的模板（「连接被拒绝：{host}」）在这里补不上参数：样本里只存了
 * 错误码，**没存参数**。露出一个 `{host}` 比不给原因更糟，所以退回错误码本身。
 * 排查时 `E_TIMEOUT` 这四个字母已经够用。
 */
function errorText(code: string | null): string {
  if (code === null) return "—";
  if (!ipc.isErrorCode(code)) return code;

  const template = zhCN[ipc.ERROR_KEYS[code]];
  return template.includes("{") ? code : template;
}
