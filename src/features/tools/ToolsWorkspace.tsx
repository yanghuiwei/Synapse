import { Wrench } from "lucide-react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { useUiStore } from "@/store/ui";
import { TokenToolPanel } from "./TokenToolPanel";

/**
 * 工具板块的**主视图**（中间那块）。
 *
 * ## 为什么不再挂在最右一列
 *
 * 第一版做成「请求区右边再挤一列常驻栏」，那是错的：左栏那六个图标是
 * **并列的功能区**，点进「工具」时中间却还是请求编辑器 —— 等于点进来
 * 什么也没变。真正该切的是中间那块。所以这里铺满整个工作区。
 *
 * ## 以后加第二个工具时
 *
 * `TOOLS` 是个数组，现在只有一项所以不画选择器。加到两项以上时，
 * 在这一层加一条子页签（`TabStrip`）即可 —— 工具列表住在侧栏、
 * 主视图只负责当前那一个，与「集合」板块里侧栏树 / 中间请求的关系一致。
 */

interface ToolDef {
  id: string;
  labelKey: "tools.token";
  icon: typeof Wrench;
}

const TOOLS: readonly ToolDef[] = [
  { id: "token", labelKey: "tools.token", icon: Wrench },
];

export function ToolsWorkspace({ className }: { className?: string }) {
  const { t } = useI18n();
  const open = useUiStore((s) => s.tokenToolOpen);

  return (
    <section className={cn("flex h-full min-h-0 flex-col bg-canvas", className)}>
      {/* 标题行。与 `RequestPane` 的地址栏同为 h-11，切板块时中间区不会跳高 */}
      <header className="flex h-11 shrink-0 items-center gap-2 px-2.5">
        {TOOLS.map((tool) => {
          const Icon = tool.icon;
          return (
            <span key={tool.id} className="flex items-center gap-1.5 text-[13px] text-ink">
              <Icon size={14} strokeWidth={1.9} aria-hidden className="text-ink-faint" />
              {t(tool.labelKey)}
            </span>
          );
        })}
      </header>

      {open ? (
        <TokenToolPanel className="min-h-0 flex-1" />
      ) : (
        // 侧栏点「Token 解析」会把它打开，所以这个空态几乎只在
        // 「关掉后再切回来」时出现 —— 给一条回到打开状态的路
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 px-6 text-center">
          <div className="flex h-11 w-11 items-center justify-center rounded-lg border border-line bg-canvas-subtle text-ink-faint">
            <Wrench size={20} strokeWidth={1.6} />
          </div>
          <p className="text-[13px] font-medium text-ink-muted">{t("tools.emptyWorkspace")}</p>
        </div>
      )}
    </section>
  );
}
