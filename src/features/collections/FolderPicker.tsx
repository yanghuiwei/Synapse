import { Folder as FolderIcon, Layers } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { Folder } from "@/lib/types";
import { childFolders } from "@/store/workspace";

/** 缩进基准（px），与集合树保持一致 */
const INDENT = 12;

/** 「根级 / 未分类」这一项用一个不可能与真实 id 撞的哨兵值 */
const ROOT = -1;

export interface FolderPickerProps {
  folders: Folder[];
  /** 当前选中项；`null` = 根级（未分类） */
  value: number | null;
  onChange: (id: number | null) => void;
  /**
   * 是否给一个「未分类」选项。
   *
   * 移动时要（允许把东西移回根级），「保存到分类」时**不要** ——
   * 那个动作的目的就是离开未分类（Token 来源要求有分类）。
   */
  includeRoot?: boolean;
  /**
   * 禁止选择的目标。
   *
   * 移动**文件夹**时是它自己 + 全部子孙（选进去会形成环）；
   * 移动**请求**时是空集（请求放哪都行）。
   */
  forbidden?: ReadonlySet<number>;
  /** 一个分类都没有时的提示语 */
  emptyHint?: string;
  /** 一个分类都没有时给的补救动作（比如「新建分类」） */
  emptyAction?: ReactNode;
  className?: string;
}

/**
 * 分类选择器 —— 单选列表。
 *
 * 用列表而不是原生 `<select>`：目标是一棵树，层级关系得看得见，
 * 而原生下拉在 Windows 上展开后的样式完全不受控。
 *
 * 从 `MoveDialog` 抽出来是因为「保存到分类」需要**一模一样**的选择体验 ——
 * 两个入口选同一棵树，长得不一样只会让人怀疑选中的不是同一个东西。
 */
export function FolderPicker({
  folders,
  value,
  onChange,
  includeRoot = false,
  forbidden,
  emptyHint,
  emptyAction,
  className,
}: FolderPickerProps) {
  const { t } = useI18n();

  const options: { id: number; label: string; depth: number }[] = [];
  if (includeRoot) {
    options.push({ id: ROOT, label: t("collections.moveToRoot"), depth: 0 });
  }

  const walk = (parentId: number | null, depth: number) => {
    for (const folder of childFolders(folders, parentId)) {
      options.push({
        id: folder.id,
        label: folder.name,
        // 根级那一项占了一层，所以真实节点从 1 开始缩进
        depth: includeRoot ? depth + 1 : depth,
      });
      walk(folder.id, depth + 1);
    }
  };
  walk(null, 0);

  if (options.length === 0) {
    return (
      <div
        className={cn(
          "rounded-sm border border-line px-3 py-3 text-center",
          className,
        )}
      >
        <p className="text-xs leading-relaxed text-ink-faint">
          {emptyHint ?? t("collections.noFolders")}
        </p>
        {emptyAction ? <div className="mt-2.5 flex justify-center">{emptyAction}</div> : null}
      </div>
    );
  }

  return (
    <div
      role="radiogroup"
      className={cn(
        "max-h-[300px] overflow-y-auto rounded-sm border border-line p-1",
        className,
      )}
    >
      {options.map((option) => {
        const disabled = forbidden?.has(option.id) ?? false;
        const selected = (value ?? ROOT) === option.id;
        return (
          <button
            key={option.id}
            type="button"
            role="radio"
            aria-checked={selected}
            disabled={disabled}
            title={disabled ? t("collections.moveIntoSelf") : option.label}
            onClick={() => onChange(option.id === ROOT ? null : option.id)}
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
              <FolderIcon size={13} strokeWidth={1.9} aria-hidden className="shrink-0 opacity-70" />
            )}
            <span className="truncate">{option.label}</span>
          </button>
        );
      })}
    </div>
  );
}
