import { useCallback, useState, type ReactNode } from "react";
import { Group, Separator, type Layout } from "react-resizable-panels";
import { cn } from "@/lib/cn";
import { LAYOUT_STORAGE_PREFIX } from "@/lib/constants";

export { Panel } from "react-resizable-panels";

export type SplitOrientation = "horizontal" | "vertical";

// ---------------------------------------------------------------
// 布局持久化
// react-resizable-panels v4 的原生做法：
//   <Group defaultLayout={布局} onLayoutChanged={(l) => 存起来}>
// 布局是 {panelId: 百分比} 的映射，所以 Panel 必须给稳定的 id。
// ---------------------------------------------------------------

export function loadLayout(storageKey: string): Layout | undefined {
  if (typeof localStorage === "undefined") return undefined;
  try {
    const raw = localStorage.getItem(storageKey);
    if (!raw) return undefined;
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return undefined;
    }
    const out: Layout = {};
    for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof value === "number" && Number.isFinite(value) && value > 0) {
        out[key] = value;
      }
    }
    return Object.keys(out).length > 0 ? out : undefined;
  } catch {
    // 存的东西坏了就当没有，别让启动流程挂在这
    return undefined;
  }
}

function saveLayout(storageKey: string, layout: Layout): void {
  if (typeof localStorage === "undefined") return;
  try {
    // 过滤掉 0：那是「面板被折叠」的瞬时状态，
    // 记下去会导致下次启动时该面板直接消失。
    const clean: Layout = {};
    for (const [key, value] of Object.entries(layout)) {
      if (typeof value === "number" && value > 0) clean[key] = value;
    }
    localStorage.setItem(storageKey, JSON.stringify(clean));
  } catch {
    /* 配额满了 / 隐私模式：忽略，布局记忆不是关键功能 */
  }
}

/** 清掉某个分栏的布局记忆（双击分隔条回到默认时用得上） */
export function resetLayout(id: string): void {
  try {
    localStorage.removeItem(LAYOUT_STORAGE_PREFIX + id);
  } catch {
    /* ignore */
  }
}

// ---------------------------------------------------------------
// Separator
// ---------------------------------------------------------------

export interface SplitHandleProps {
  orientation: SplitOrientation;
  className?: string;
}

/**
 * 分隔条。
 * 视觉上是一条 1px 线，但元素本身留了 8px 宽度作为拖拽命中区
 * ——1px 的命中区在实际使用中几乎抓不住。
 */
export function SplitHandle({ orientation, className }: SplitHandleProps) {
  const horizontal = orientation === "horizontal";
  return (
    <Separator
      className={cn(
        "group/sep relative z-10 outline-none",
        horizontal ? "w-2" : "h-2",
        className,
      )}
    >
      <span
        aria-hidden
        className={cn(
          "pointer-events-none absolute bg-line transition-colors duration-150",
          "group-hover/sep:bg-brand group-active/sep:bg-brand",
          horizontal
            ? "inset-y-0 left-1/2 w-px -translate-x-1/2"
            : "inset-x-0 top-1/2 h-px -translate-y-1/2",
        )}
      />
    </Separator>
  );
}

// ---------------------------------------------------------------
// SplitPane
// ---------------------------------------------------------------

export interface SplitPaneProps {
  /** 布局持久化标识。同名分栏会记住上次拖到的位置 */
  id: string;
  orientation?: SplitOrientation;
  className?: string;
  children: ReactNode;
}

/**
 * 可拖拽分栏容器。子元素应当是 <Panel> 与 <SplitHandle>。
 *
 * ⚠️ Panel 的 defaultSize 单位规则（v4）：
 *   数字   → 像素（defaultSize={240} 是 240px）
 *   字符串 → 百分比（defaultSize="22%" 是 22%）
 *   建议一律写显式单位，免得日后看代码猜。
 */
export function SplitPane({
  id,
  orientation = "horizontal",
  className,
  children,
}: SplitPaneProps) {
  const storageKey = LAYOUT_STORAGE_PREFIX + id;

  // 只在挂载时读一次：拖拽过程中不要再回头读 localStorage
  const [defaultLayout] = useState(() => loadLayout(storageKey));

  const handleLayoutChanged = useCallback(
    (layout: Layout) => {
      saveLayout(storageKey, layout);
    },
    [storageKey],
  );

  return (
    <Group
      id={id}
      orientation={orientation}
      defaultLayout={defaultLayout}
      onLayoutChanged={handleLayoutChanged}
      // 鼠标贴近分隔条 6px 内就开始算命中，比库默认值更跟手一点
      resizeTargetMinimumSize={{ fine: 6, coarse: 24 }}
      className={cn("h-full w-full", className)}
    >
      {children}
    </Group>
  );
}
