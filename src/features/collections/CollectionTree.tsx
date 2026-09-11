import { ChevronRight, Folder as FolderIcon, FolderOpen, Layers } from "lucide-react";
import { useMemo, useState } from "react";

import { cn } from "@/lib/cn";
import { methodTextClass } from "@/lib/http";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import type { ApiRequest, Folder } from "@/lib/types";
import { EmptyState } from "@/features/ui/EmptyState";
import { useUiStore } from "@/store/ui";
import { childFolders, requestsInFolder, useWorkspaceStore } from "@/store/workspace";

/**
 * 集合树。
 *
 * 两个实现选择：
 * 1. **先拍平成行再渲染**（而不是递归组件）。递归渲染在折叠/筛选时会不断
 *    重建整棵 React 子树；拍平之后平铺 map，每一行的 key 稳定，
 *    折叠一个文件夹只影响它后面那段。行数上千时差别很明显。
 * 2. **折叠状态存 id 集合**（而不是节点上的 boolean）。后端返回的 Folder
 *    没有 UI 字段，且 `load()` 之后是新对象 —— 状态挂在 id 上才不会被刷新冲掉。
 */
export function CollectionTree() {
  const { t } = useI18n();
  const folders = useWorkspaceStore((s) => s.folders);
  const requests = useWorkspaceStore((s) => s.requests);
  const activeRequestId = useUiStore((s) => s.activeRequestId);
  const setActiveRequestId = useUiStore((s) => s.setActiveRequestId);

  const [collapsed, setCollapsed] = useState<ReadonlySet<number>>(
    () => new Set<number>(),
  );

  const toggle = (id: number) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const rows = useMemo(() => flatten(folders, requests, collapsed), [
    folders,
    requests,
    collapsed,
  ]);

  if (rows.length === 0) {
    return (
      <EmptyState
        icon={Layers}
        title={t("common.empty")}
        description={t("sidebar.emptyCollections")}
        className="mt-8"
      />
    );
  }

  return (
    <ul role="tree" className="flex flex-col gap-px">
      {rows.map((row) => {
        if (row.kind === "label") {
          return (
            <li
              key={row.key}
              className="px-2 pb-0.5 pt-2 text-[10px] font-medium tracking-wide text-ink-faint"
            >
              {t(row.labelKey)}
            </li>
          );
        }
        return row.kind === "folder" ? (
          <FolderRow
            key={`f-${row.folder.id}`}
            folder={row.folder}
            depth={row.depth}
            itemCount={row.itemCount}
            collapsed={collapsed.has(row.folder.id)}
            onToggle={() => toggle(row.folder.id)}
          />
        ) : (
          <RequestRow
            key={`r-${row.request.id}`}
            request={row.request}
            depth={row.depth}
            active={row.request.id === activeRequestId}
            onSelect={() => setActiveRequestId(row.request.id)}
          />
        );
      })}
    </ul>
  );
}

// ---------------------------------------------------------------
// 拍平
// ---------------------------------------------------------------

type Row =
  | { kind: "folder"; folder: Folder; depth: number; itemCount: number }
  | { kind: "request"; request: ApiRequest; depth: number }
  | { kind: "label"; key: string; labelKey: MessageKey };

/** 缩进基准（px）。深度乘它 —— 比写 6 层 `pl-*` 类干净 */
const INDENT = 11;

function flatten(
  folders: Folder[],
  requests: ApiRequest[],
  collapsed: ReadonlySet<number>,
): Row[] {
  const out: Row[] = [];

  const walk = (parentId: number | null, depth: number) => {
    for (const folder of childFolders(folders, parentId)) {
      const subFolders = childFolders(folders, folder.id);
      const own = requestsInFolder(requests, folder.id);
      out.push({
        kind: "folder",
        folder,
        depth,
        itemCount: subFolders.length + own.length,
      });

      if (collapsed.has(folder.id)) continue;

      // 子文件夹在前、请求在后 —— 跟文件管理器的习惯一致
      walk(folder.id, depth + 1);
      for (const r of own) out.push({ kind: "request", request: r, depth: depth + 1 });
    }
  };

  walk(null, 0);

  // 未归档的请求排在整棵树之后。**只有当上面有文件夹时才加分组标题** ——
  // 否则「未归档」会贴在空列表里，看起来像个莫名其妙的分类。
  const unfiled = requestsInFolder(requests, null);
  if (unfiled.length > 0 && folders.length > 0) {
    out.push({ kind: "label", key: "label-unfiled", labelKey: "collections.unfiled" });
  }
  for (const r of unfiled) {
    out.push({ kind: "request", request: r, depth: 0 });
  }

  return out;
}

// ---------------------------------------------------------------
// 行
// ---------------------------------------------------------------

const ROW_BASE =
  "flex h-[26px] w-full items-center gap-1.5 rounded-sm pr-1.5 text-left text-xs " +
  "transition-colors duration-75 select-none";

function FolderRow({
  folder,
  depth,
  itemCount,
  collapsed,
  onToggle,
}: {
  folder: Folder;
  depth: number;
  itemCount: number;
  collapsed: boolean;
  onToggle: () => void;
}) {
  const { t } = useI18n();
  const Icon = collapsed ? FolderIcon : FolderOpen;

  return (
    <li role="treeitem" aria-expanded={!collapsed}>
      <button
        type="button"
        onClick={onToggle}
        aria-label={t(collapsed ? "collections.expand" : "collections.collapse")}
        style={{ paddingLeft: depth * INDENT + 4 }}
        className={cn(ROW_BASE, "hover:bg-surface-hover")}
      >
        <ChevronRight
          size={12}
          strokeWidth={2.4}
          aria-hidden
          className={cn(
            "shrink-0 text-ink-faint transition-transform duration-100",
            !collapsed && "rotate-90",
          )}
        />
        <Icon
          size={13}
          strokeWidth={1.9}
          aria-hidden
          className="shrink-0 text-ink-faint"
        />
        <span className="truncate font-medium text-ink">{folder.name}</span>

        {folder.isTokenSource ? (
          <span
            className="shrink-0 rounded-sm bg-brand/12 px-1 text-[10px] font-medium text-brand"
            title={t("auth.tokenSource")}
          >
            {t("auth.tokenSource")}
          </span>
        ) : null}

        <span className="flex-1" />
        {/* 数字给视觉，完整说法给屏幕阅读器 */}
        <span
          aria-label={t("collections.itemCount", { count: itemCount })}
          className="tabular shrink-0 text-[10px] text-ink-faint"
        >
          {itemCount > 0 ? itemCount : ""}
        </span>
      </button>
    </li>
  );
}

function RequestRow({
  request,
  depth,
  active,
  onSelect,
}: {
  request: ApiRequest;
  depth: number;
  active: boolean;
  onSelect: () => void;
}) {
  return (
    <li role="treeitem" aria-selected={active}>
      <button
        type="button"
        onClick={onSelect}
        style={{ paddingLeft: depth * INDENT + 4 }}
        className={cn(
          ROW_BASE,
          active ? "bg-brand/12 text-ink" : "hover:bg-surface-hover",
        )}
      >
        {/* 固定宽度 + 等宽字体：方法名长短不一时，右边名称仍然对齐成一列 */}
        <span
          className={cn(
            "w-[40px] shrink-0 font-mono text-[10px] font-semibold tracking-tight",
            methodTextClass(request.method),
          )}
        >
          {request.method}
        </span>
        <span className={cn("truncate", active ? "text-ink" : "text-ink-muted")}>
          {request.name}
        </span>
      </button>
    </li>
  );
}
