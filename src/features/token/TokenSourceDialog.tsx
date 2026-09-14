import { useEffect, useState } from "react";

import { useI18n } from "@/lib/i18n";
import type { TokenSource } from "@/lib/types";
import { useTokenStore } from "@/store/token";
import { useWorkspaceStore } from "@/store/workspace";
import { Button } from "@/features/ui/Button";
import { Dialog } from "@/features/ui/Dialog";
import { draftFromSource, emptyTokenSourceDraft, TokenSourceForm, type TokenSourceDraft } from "./TokenSourceForm";

export interface TokenSourceDialogProps {
  /** 新配置挂在哪个分类下 */
  folderId: number;
  /** `null` = 新建 */
  source: TokenSource | null;
  onClose: () => void;
}

/**
 * Token 来源的新建 / 编辑对话框。
 *
 * ## 为什么保存按钮的可用性在这里算，而不是在表单里
 *
 * 表单只管「这 9 个字段长什么样」，而「能不能保存」取决于**跨字段**的约束
 * （名字非空 + 选了获取请求 + access 路径合法）。把它放在表单里，
 * 表单就得暴露一个 `onValidityChange` 回调给调用方 —— 一个组件两个出口，
 * 还不如让持有提交动作的那一层自己判断。
 */
export function TokenSourceDialog({ folderId, source, onClose }: TokenSourceDialogProps) {
  const { t } = useI18n();
  const folders = useWorkspaceStore((s) => s.folders);
  const create = useTokenStore((s) => s.create);
  const save = useTokenStore((s) => s.save);

  const [draft, setDraft] = useState<TokenSourceDraft | null>(null);
  const [saving, setSaving] = useState(false);

  // 初值在**挂载时**算一次（惰性初始化）。写成依赖 `source` 的 effect
  // 会把用户已经敲进去的内容在父组件重渲染时冲掉
  useEffect(() => {
    setDraft(
      source ? draftFromSource(source) : emptyTokenSourceDraft(folderId),
    );
  }, [source, folderId]);

  if (draft === null) return null;

  const pathOk = (path: string, required: boolean) => {
    const trimmed = path.trim();
    if (trimmed === "") return !required;
    return trimmed.startsWith("$");
  };

  const canSave =
    draft.name.trim() !== "" &&
    draft.fetchRequestId !== 0 &&
    pathOk(draft.accessPath, true) &&
    pathOk(draft.refreshPath, false) &&
    pathOk(draft.expiresInPath, false) &&
    !saving;

  const submit = async () => {
    setSaving(true);
    // 两边都送整份草稿：新建这条路径下 store 会先建行、再把剩下 8 个字段
    // （包括「刷新 Token 的请求」）补一次全量保存
    const done = source
      ? await save({ ...source, ...draft, name: draft.name.trim() })
      : await create({ ...draft, name: draft.name.trim() });
    setSaving(false);
    if (done) onClose();
  };

  return (
    <Dialog
      title={source ? t("token.title") : t("token.new")}
      onClose={onClose}
      width={560}
      dismissOnOverlay={false}
      footer={
        <>
          <Button size="sm" variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button size="sm" variant="primary" disabled={!canSave} onClick={() => void submit()}>
            {saving ? t("common.loading") : t("common.save")}
          </Button>
        </>
      }
    >
      <TokenSourceForm
        value={draft}
        onChange={setDraft}
        folders={folders}
        // 编辑时锁住分类：挪分类等于换了一整条继承链，
        // 而「这个 Token 到底是给谁用的」恰恰是这类配置最容易搞混的地方
        lockFolder={source !== null}
      />
    </Dialog>
  );
}
