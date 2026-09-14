import { Folder as FolderIcon, Layers } from "lucide-react";
import { useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { Folder } from "@/lib/types";
import { Button } from "@/features/ui/Button";
import { Dialog } from "@/features/ui/Dialog";
import { childFolders } from "@/store/workspace";

/** 缩进基准（px），与集合树保持一致 */
const INDENT = 12;

/** 「根级」这一项用一个不可能与真实 id 撞的哨兵值 */
const ROOT = -1;

export interface MoveDialogProps {
  /** 正在被移动的节点名（放进标题里，让用户确认移的是哪个） */
  name: string;
  folders: Folder[];
  /** 当前所在位置；`null` = 未分类 */
  currentParentId: number | null;
  /**
   * 禁止选择的目标。
   *
   * 移动**文件夹**时是它自己 + 全部子孙 —— 选进去会形成环。
   * 移动**请求**时是空集（请求在哪都能放）。
   */
  forbidden: ReadonlySet<number>;
  /** 内部原因造成的不可选（比如目标在回收站里）—— 目前只有环这一种 */
  onConfirm: (parentId: number | null) => void;
  onClose: () => void;
}

/**
 * 移动目标选择器 —— **拖拽的键盘可达替代方案**。
 *
 * 拖拽再顺手也有一条硬伤：它只有鼠标能做。所以「移动到…」必须是
 * 菜单里的一项（菜单本身支持上下键 + 回车），而这个对话框用同一套
 * 上下键 + 回车就能完成移动。两条路径通向同一个后端命令。
 *
 * 用**单选列表**而不是原生 `<select>`：目标是一棵树，层级关系得看得见，
 * 而原生下拉在 Windows 上展开后的样式完全不受控。
 */
export function MoveDialog({
  name,
  folders,
  currentParentId,
  forbidden,
  onConfirm,
  onClose,
}: MoveDialogProps) {
  const { t } = useI18n();
  const [target, setTarget] = useState<number>(currentParentId ?? ROOT);

  const options: { id: number; label: string; depth: number }[] = [
    { id: ROOT, label: t("collections.moveToRoot"), depth: 0 },
  ];

  const walk = (parentId: number | null, depth: number) => {
    for (const folder of childFolders(folders, parentId)) {
      options.push({
        id: folder.id,
        label: folder.name,
        // 根级那一项占了一层，所以真实节点从 1 开始缩进
        depth: depth + 1,
      });
      walk(folder.id, depth + 1);
    }
  };
  walk(null, 0);

  return (
    <Dialog
      title={t("collections.moveDialogTitle", { name })}
      description={t("collections.moveDialogHint")}
      onClose={onClose}
      width={400}
      footer={
        <>
          <Button size="sm" variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant="primary"
            onClick={() => {
              onConfirm(target === ROOT ? null : target);
              onClose();
            }}
          >
            {t("common.confirm")}
          </Button>
        </>
      }
    >
      <div
        role="radiogroup"
        className="max-h-[300px] overflow-y-auto rounded-sm border border-line p-1"
      >
        {options.map((option) => {
          const disabled = forbidden.has(option.id);
          const selected = target === option.id;
          return (
            <button
              key={option.id}
              type="button"
              role="radio"
              aria-checked={selected}
              disabled={disabled}
              title={disabled ? t("collections.moveIntoSelf") : option.label}
              onClick={() => setTarget(option.id)}
              style={{ paddingLeft: option.depth * INDENT + 6 }}
              className={cn(
                "flex h-7 w-full items-center gap-1.5 rounded-sm pr-2 text-left text-xs",
                "transition-colors duration-75",
                disabled && "cursor-not-allowed opacity-40",
                !disabled && (selected ? "bg-brand/12 text-brand" : "hover:bg-surface-hover"),
              )}
            >
              {option.id === ROOT ? (
                <Layers size={13} strokeWidth={1.9} aria-hidden className="shrink-0 opacity-70" />
              ) : (
                <FolderIcon
                  size={13}
                  strokeWidth={1.9}
                  aria-hidden
                  className="shrink-0 opacity-70"
                />
              )}
              <span className="truncate">{option.label}</span>
            </button>
          );
        })}
      </div>
    </Dialog>
  );
}
