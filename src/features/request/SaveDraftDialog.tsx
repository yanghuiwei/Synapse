import { FolderPlus } from "lucide-react";
import { useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import * as ipc from "@/lib/ipc";
import { validateName } from "@/lib/name";
import { useRequestStore } from "@/store/request";
import { useUiStore } from "@/store/ui";
import { childFolders, useWorkspaceStore } from "@/store/workspace";
import { FolderPicker } from "@/features/collections/FolderPicker";
import { Button } from "@/features/ui/Button";
import { Dialog } from "@/features/ui/Dialog";

/**
 * 「保存到分类」对话框。
 *
 * ## 它补的是哪条死路
 *
 * 草稿（`sourceId === null`）没有归属分类，而 Token 来源**必须**挂在分类上。
 * 于是「粘个地址就发，再用 `{{token.*}}`」这条路在第二步就断了：
 * 发送时会报「没有可用 Token 源」，而左侧并没有「把这份内容收进某个分类」
 * 这个动作 —— 选中分类只是展开/折叠它。
 *
 * ## 为什么分两层
 *
 * 里面那层持有全部表单 state。外层用「不打开就返回 null」控制挂载，
 * 于是**每次打开都是一份全新的表单**：名字预填、分类默认选中第一项的
 * 逻辑写在 `useState` 初始值里就够了。合成一层的话，得靠 `useEffect`
 * 去同步这些值 —— 那会在打开的那一帧先闪一下「上一次填的内容」。
 */
export function SaveDraftDialog() {
  const open = useUiStore((s) => s.saveDraftOpen);
  if (!open) return null;
  return <SaveDraftForm />;
}

function SaveDraftForm() {
  const { t } = useI18n();
  const close = useUiStore((s) => s.closeSaveDraft);
  const openCreateDialog = useUiStore((s) => s.openCreateDialog);

  const draft = useRequestStore((s) => s.draft);
  const sourceName = useRequestStore((s) => s.sourceName);
  const folders = useWorkspaceStore((s) => s.folders);

  const [name, setName] = useState(() => sourceName ?? nameFromUrl(draft.url));
  const [folderId, setFolderId] = useState<number | null>(
    () => childFolders(folders, null)[0]?.id ?? null,
  );
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  const error = validateName(name, t);
  // 空名字不该在刚打开时就飘红 —— 那只是「还没填」。
  // 与 `PromptDialog` 同样处理：错误文案只在用户真的敲了东西之后才给
  const shownError = name.trim() === "" ? null : error;
  // 必须选一个分类：选「未分类」等于没存 —— Token 来源照样配不了，
  // 用户会以为这个按钮坏了。所以列表里压根不给「未分类」这一项
  const canSubmit = error === null && folderId !== null && !busy;

  const submit = async () => {
    if (!canSubmit) return;
    setBusy(true);
    setFailed(false);
    try {
      await useRequestStore.getState().saveAs(folderId, name.trim());
      close();
    } catch (e) {
      console.warn("[reqly] 保存到分类失败：", ipc.toIpcError(e).debug);
      setFailed(true);
      setBusy(false);
    }
  };

  return (
    <Dialog
      title={t("request.saveAsTitle")}
      description={t("request.saveAsHint")}
      onClose={close}
      width={400}
      footer={
        <>
          <Button size="sm" variant="secondary" disabled={busy} onClick={close}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant="primary"
            disabled={!canSubmit}
            onClick={() => void submit()}
          >
            {t("common.confirm")}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <label className="block space-y-1.5">
          <span className="text-[11px] font-medium text-ink-muted">{t("request.saveAsName")}</span>
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void submit();
              }
            }}
            spellCheck={false}
            autoComplete="off"
            aria-invalid={shownError !== null}
            className={cn(
              "h-8 w-full rounded-sm border bg-surface px-2.5",
              "text-xs text-ink placeholder:text-ink-faint",
              "transition-colors duration-100",
              shownError ? "border-err" : "border-line hover:border-line-strong focus:border-brand",
            )}
          />
        </label>
        {shownError !== null ? (
          <p className="-mt-2 text-[11px] text-err">{shownError}</p>
        ) : null}
        {failed ? <p className="-mt-2 text-[11px] text-err">{t("request.saveAsFailed")}</p> : null}

        <div className="space-y-1.5">
          <span className="block text-[11px] font-medium text-ink-muted">{t("token.folder")}</span>
          <FolderPicker
            folders={folders}
            value={folderId}
            onChange={setFolderId}
            emptyHint={t("request.saveAsNoFolder")}
            emptyAction={
              <Button
                size="sm"
                variant="secondary"
                icon={<FolderPlus size={13} strokeWidth={2.2} />}
                onClick={() => {
                  close();
                  openCreateDialog("folder", null);
                }}
              >
                {t("sidebar.newFolder")}
              </Button>
            }
          />
        </div>
      </div>
    </Dialog>
  );
}

/**
 * 从 URL 抠一个能当默认名的片段。
 *
 * 只求「比空白强」—— 用户多半会改掉它。草稿还没填完时 `new URL` 会抛，
 * 那就留空让用户自己填。
 */
function nameFromUrl(url: string): string {
  try {
    const parsed = new URL(url);
    const last = parsed.pathname.split("/").filter(Boolean).pop();
    return last ?? parsed.hostname;
  } catch {
    return "";
  }
}
