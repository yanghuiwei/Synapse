import { ChevronRight, Wrench } from "lucide-react";
import type { LucideIcon } from "lucide-react";

import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import { useUiStore } from "@/store/ui";

/**
 * 「工具」板块（侧栏）。
 *
 * ## 为什么侧栏里只有一行入口
 *
 * 与 `EnvManager` 同一个判断：侧栏最窄 180px，装不下任何有列的表格。
 * 实体本身在**右侧常驻栏**里（`features/tools/TokenToolPanel.tsx`）——
 * 不弹窗，因为解析 token 总是和「核对请求 / 响应」连着做，
 * 一层遮罩就把两边对照的路断了。
 *
 * ## 为什么用列表而不是一排按钮
 *
 * 需求是「以后还会有别的工具」。列表是会长的那一种形状：加一个工具就往
 * `TOOLS` 里加一项，这一层不用动。按钮排会随着数量增加先挤成两行、
 * 再挤到认不出来。
 */

interface ToolDef {
  id: string;
  labelKey: MessageKey;
  descKey: MessageKey;
  icon: LucideIcon;
  /** 点它做什么。目前只有「把右边那栏打开」这一种 */
  run: () => void;
}

export function ToolsPanel() {
  const { t } = useI18n();
  const setTokenToolOpen = useUiStore((s) => s.setTokenToolOpen);

  /** 加新工具时往这里加一项即可（栏体挂在 `App` 的布局里） */
  const tools: readonly ToolDef[] = [
    {
      id: "token",
      labelKey: "tools.token",
      descKey: "tools.tokenDesc",
      icon: Wrench,
      // 只开栏，不切板块：用户是想看这个 token，不是想离开当前的集合/历史
      run: () => setTokenToolOpen(true),
    },
  ];

  return (
    <div className="pt-0.5">
      <ul className="space-y-0.5">
        {tools.map((tool) => {
          const Icon = tool.icon;
          return (
            <li key={tool.id}>
              <button
                type="button"
                onClick={tool.run}
                className="flex w-full items-center gap-2 rounded-sm px-1.5 py-1.5 text-left transition-colors duration-100 hover:bg-surface-hover"
              >
                <Icon
                  size={14}
                  strokeWidth={1.9}
                  aria-hidden
                  className="shrink-0 text-ink-faint"
                />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-xs text-ink">{t(tool.labelKey)}</span>
                  <span className="block truncate text-[11px] text-ink-faint">
                    {t(tool.descKey)}
                  </span>
                </span>
                <ChevronRight
                  size={12}
                  strokeWidth={2}
                  aria-hidden
                  className="shrink-0 text-ink-faint"
                />
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
