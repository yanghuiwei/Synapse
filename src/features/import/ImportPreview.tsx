import { FilePlus2, Folder, RefreshCw, Trash2, Undo2 } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import type { DiffStatus, EnvPreview, ImportCounts, PreviewNode } from "@/lib/types";

/**
 * 差异状态 → 图标 + 颜色 + 文案。
 *
 * 颜色刻意**不**用语义色（成功/危险/警告）里的红绿：红绿在这套界面里
 * 已经被「删除/危险」「激活/成功」占住了，再用一次会让用户
 * 把「这条会被移除」读成「这条出错了」。四个中性色 + 一个警示色，
 * 靠**形状**区分（加号 / 波浪 / 等号 / 减号）才是这类 diff 的通行读法。
 */
const STATUS_META: Record<
  DiffStatus,
  { icon: LucideIcon; labelKey: MessageKey; className: string }
> = {
  new: { icon: FilePlus2, labelKey: "import.status.new", className: "text-ok" },
  update: { icon: RefreshCw, labelKey: "import.status.update", className: "text-brand" },
  skip: { icon: Undo2, labelKey: "import.status.skip", className: "text-ink-faint" },
  keep: { icon: Undo2, labelKey: "import.status.keep", className: "text-ink-faint" },
  remove: { icon: Trash2, labelKey: "import.status.remove", className: "text-warn" },
};

/**
 * 计数条：`+12 新增 / ~3 更新 / =5 跳过 / -2 移除`。
 *
 * 只有非零的项才画 —— 一行「=0 跳过」除了占地方没有任何信息量，
 * 而**零本身**恰恰是「这次导入什么都没被跳过」这个结论的证据，
 * 但那个结论由「其他几项都在」表达得更清楚。
 */
export function CountsBar({ counts }: { counts: ImportCounts }) {
  const { t } = useI18n();
  const items: Array<[number, MessageKey]> = [
    [counts.created, "import.count.created"],
    [counts.updated, "import.count.updated"],
    [counts.skipped, "import.count.skipped"],
    [counts.removed, "import.count.removed"],
  ];
  const shown = items.filter(([count]) => count > 0);
  if (shown.length === 0) return null;

  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
      {shown.map(([count, key]) => (
        <span key={key} className="text-ink-muted">
          {t(key, { count })}
        </span>
      ))}
    </div>
  );
}

/** 一个环境的预览行 */
export function EnvPreviewRow({ env }: { env: EnvPreview }) {
  const { t } = useI18n();
  return (
    <li className="flex items-center gap-2 text-xs">
      <span className="truncate text-ink">{env.name}</span>
      <span className="shrink-0 text-ink-faint">{t("import.envVars", { count: env.varCount })}</span>
      <span className="shrink-0 text-ink-faint">
        {env.merged ? t("import.envMerged") : t("import.envNew")}
      </span>
    </li>
  );
}

/**
 * 差异树。
 *
 * 默认**全部展开**（而不是折叠成一层）：导入前这一次检查的全部意义就是
 * 「让我在落库前把该看的都看到」。折叠状态下用户只会点「导入」，
 * 于是预览等于没做。节点多了确实会滚动，但滚动比漏看便宜。
 *
 * 分类节点可以手动折叠 —— 层级深的时候需要这个动作来聚焦。
 */
export function ImportPreviewTree({ nodes }: { nodes: PreviewNode[] }) {
  if (nodes.length === 0) return null;
  return (
    <ul className="space-y-px">
      {nodes.map((node, index) => (
        // 名字会重（副本模式专门制造重名），所以下标是这里唯一稳的 key
        <Node key={`${node.kind}-${index}-${node.name}`} node={node} depth={0} />
      ))}
    </ul>
  );
}

function Node({ node, depth }: { node: PreviewNode; depth: number }) {
  const { t } = useI18n();
  const [open, setOpen] = useState(true);
  const meta = STATUS_META[node.status];
  const Icon = meta.icon;
  const children = node.children ?? [];
  const isFolder = node.kind === "folder";

  return (
    <li>
      <div
        className="flex items-center gap-1.5 rounded-sm py-0.5 pr-1 hover:bg-surface-hover"
        // 缩进用内边距而不是嵌套 ul：嵌套会让每层的行高与 hover 底色对不齐
        style={{ paddingLeft: depth * 12 + 4 }}
      >
        {isFolder ? (
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            aria-expanded={open}
            aria-label={node.name}
            className="flex h-4 w-3.5 shrink-0 items-center justify-center text-ink-faint"
          >
            <span
              className={cn(
                "text-[9px] leading-none transition-transform duration-100",
                open ? "rotate-90" : "",
              )}
            >
              ▶
            </span>
          </button>
        ) : null}

        <Icon size={11} strokeWidth={2.2} aria-hidden className={cn("shrink-0", meta.className)} />

        {isFolder ? (
          <Folder size={12} strokeWidth={1.9} aria-hidden className="shrink-0 text-ink-faint" />
        ) : null}

        <span className="min-w-0 truncate text-xs text-ink" title={node.url ?? node.name}>
          {node.name}
        </span>

        {node.method ? (
          <span className="shrink-0 font-mono text-[10px] text-ink-faint">{node.method}</span>
        ) : null}

        <span className={cn("ml-auto shrink-0 text-[11px]", meta.className)}>
          {t(meta.labelKey)}
        </span>
      </div>

      {isFolder && open && children.length > 0 ? (
        <ul>
          {children.map((child, index) => (
            <Node key={`${child.kind}-${index}-${child.name}`} node={child} depth={depth + 1} />
          ))}
        </ul>
      ) : null}
    </li>
  );
}
