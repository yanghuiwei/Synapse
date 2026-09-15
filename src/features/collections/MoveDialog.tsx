import { useState } from "react";

import { useI18n } from "@/lib/i18n";
import type { Folder } from "@/lib/types";
import { FolderPicker } from "@/features/collections/FolderPicker";
import { Button } from "@/features/ui/Button";
import { Dialog } from "@/features/ui/Dialog";

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
 * 选择列表本身住在 `FolderPicker` —— 「保存到分类」用的是同一个组件，
 * 同一个动作在两处出现时必须长得一样。
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
  const [target, setTarget] = useState<number | null>(currentParentId);

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
              onConfirm(target);
              onClose();
            }}
          >
            {t("common.confirm")}
          </Button>
        </>
      }
    >
      <FolderPicker
        folders={folders}
        value={target}
        onChange={setTarget}
        includeRoot
        forbidden={forbidden}
      />
    </Dialog>
  );
}
