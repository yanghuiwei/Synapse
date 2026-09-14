import { FileUp, LoaderCircle, TriangleAlert } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import * as ipc from "@/lib/ipc";
import type {
  ConflictMode,
  Folder,
  ImportFormat,
  ImportPreview,
  ImportWarningKind,
} from "@/lib/types";
import { useEnvStore } from "@/store/env";
import { useUiStore } from "@/store/ui";
import { useWorkspaceStore } from "@/store/workspace";
import { Button } from "@/features/ui/Button";
import { Dialog } from "@/features/ui/Dialog";
import { CountsBar, EnvPreviewRow, ImportPreviewTree } from "./ImportPreview";

/**
 * 文件体积上限（KB）。超了直接在**读之前**拒掉 ——
 * 一份 20MB 的集合拉进 textarea 会让界面卡住好几秒，
 * 而那种文件本来就该先在编辑器里处理。
 */
const MAX_FILE_KB = 4096;

/** 输入停下多久才去问后端。打字时每敲一个字符跑一次预览没必要 */
const PREVIEW_DEBOUNCE_MS = 250;

const FORMAT_LABEL: Record<ImportFormat, MessageKey> = {
  curl: "import.sourceCurl",
  postman: "import.sourcePostman",
  apipost: "import.sourceApipost",
  reqly: "import.sourceReqly",
};

const CONFLICT_LABEL: Record<ConflictMode, MessageKey> = {
  merge: "import.conflictMerge",
  duplicate: "import.conflictDuplicate",
  overwrite: "import.conflictOverwrite",
};

const CONFLICT_HINT: Record<ConflictMode, MessageKey> = {
  merge: "import.conflictMergeHint",
  duplicate: "import.conflictDuplicateHint",
  overwrite: "import.conflictOverwriteHint",
};

const CONFLICT_ORDER: ConflictMode[] = ["merge", "duplicate", "overwrite"];

const WARNING_LABEL: Record<ImportWarningKind, MessageKey> = {
  unknownArgument: "import.warning.unknownArgument",
  missingArgumentValue: "import.warning.missingArgumentValue",
  unsupportedField: "import.warning.unsupportedField",
  renamedDuplicate: "import.warning.renamedDuplicate",
  truncatedName: "import.warning.truncatedName",
  generatedName: "import.warning.generatedName",
  skippedItem: "import.warning.skippedItem",
};

/**
 * 导入对话框。
 *
 * ## 为什么预览是「边打字边算」而不是一个按钮
 *
 * 导入这一步唯一有价值的动作就是**在落库前看清会变成什么**。
 * 让它变成一个要主动点的按钮，用户就会直接点「导入」——
 * 于是预览等于不存在。所以：输入停下 250ms 就静默重算，
 * 结果永远摆在按钮上方。
 *
 * ## 为什么冲突模式一变就重算
 *
 * 差异树**随模式变**：副本模式把同名的算成新增、覆盖模式多出一批
 * 「移除」。模式切换时重算，用户才能看到「换成覆盖模式会删掉 12 条」
 * 这件事 —— 这正是他按下按钮前最该知道的。
 *
 * ## 竞态
 *
 * 每次预览都带一个自增序号，只有最新的那次结果才允许写回 state。
 * 少了这一步，快速切换模式时先发的慢响应会盖掉后发的快响应，
 * 界面上就会停在「上一个模式」的树上，而按钮已经按新模式走了。
 */
export function ImportDialog({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();

  const workspaceId = useWorkspaceStore((s) => s.workspace?.id);
  const folders = useWorkspaceStore((s) => s.folders);

  const [text, setText] = useState("");
  const [fileName, setFileName] = useState<string | null>(null);
  const [targetId, setTargetId] = useState<number | null>(null);
  const [conflict, setConflict] = useState<ConflictMode>("merge");
  const [includeEnvs, setIncludeEnvs] = useState(true);

  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [previewError, setPreviewError] = useState<ipc.IpcError | null>(null);
  const [parsing, setParsing] = useState(false);

  const [committing, setCommitting] = useState(false);
  const [commitError, setCommitError] = useState<ipc.IpcError | null>(null);
  const [doneCount, setDoneCount] = useState<number | null>(null);

  const [fileError, setFileError] = useState<string | null>(null);

  // 占位文案先取成字符串再进 effect 的依赖表：`t` 的引用会随渲染变，
  // 把它写进依赖会让防抖预览反复重启
  const untitledFolder = t("import.untitledFolder");
  const untitledRequest = t("import.untitledRequest");
  const untitledCollection = t("import.untitledCollection");

  const seq = useRef(0);
  const trimmed = text.trim();

  useEffect(() => {
    if (trimmed === "" || workspaceId === undefined) {
      setPreview(null);
      setPreviewError(null);
      setParsing(false);
      return;
    }

    const ticket = ++seq.current;
    setParsing(true);
    const timer = window.setTimeout(() => {
      void ipc.imports
        .preview(
          {
            text: trimmed,
            workspaceId,
            targetFolderId: targetId,
            fileName: fileName ?? undefined,
            labels: { untitledFolder, untitledRequest, untitledCollection },
          },
          conflict,
        )
        .then((result) => {
          // 只认最新那一张票：过期响应直接丢掉
          if (ticket !== seq.current) return;
          setPreview(result);
          setPreviewError(null);
        })
        .catch((e: unknown) => {
          if (ticket !== seq.current) return;
          setPreview(null);
          setPreviewError(ipc.toIpcError(e));
        })
        .finally(() => {
          if (ticket === seq.current) setParsing(false);
        });
    }, PREVIEW_DEBOUNCE_MS);

    return () => window.clearTimeout(timer);
  }, [
    trimmed,
    workspaceId,
    targetId,
    conflict,
    fileName,
    untitledFolder,
    untitledRequest,
    untitledCollection,
  ]);

  const pickFile = useCallback(
    async (file: File) => {
      setFileError(null);
      if (file.size > MAX_FILE_KB * 1024) {
        setFileError(t("import.fileTooBig", { max: MAX_FILE_KB }));
        return;
      }
      try {
        const content = await file.text();
        setText(content);
        setFileName(file.name);
      } catch (e) {
        setFileError(e instanceof Error ? e.message : String(e));
      }
    },
    [t],
  );

  const submit = async () => {
    if (!preview || workspaceId === undefined) return;
    setCommitting(true);
    setCommitError(null);
    try {
      const outcome = await ipc.imports.commit(
        {
          text: trimmed,
          workspaceId,
          targetFolderId: targetId,
          fileName: fileName ?? undefined,
          labels: { untitledFolder, untitledRequest, untitledCollection },
        },
        conflict,
        includeEnvs,
      );
      // 集合树与环境列表都可能被这次导入改了 —— 两块都刷新，
      // 不做「往本地列表里塞几条」的乐观更新（覆盖模式还会删东西）
      await useWorkspaceStore.getState().refresh();
      if (outcome.environments > 0) void useEnvStore.getState().load();
      setDoneCount(outcome.counts.created + outcome.counts.updated);
    } catch (e) {
      setCommitError(ipc.toIpcError(e));
    } finally {
      setCommitting(false);
    }
  };

  const folderOptions = useMemo(() => flattenFolders(folders), [folders]);
  const hasEnvs = (preview?.environments.length ?? 0) > 0;

  // ---- 导入完成 ----
  if (doneCount !== null) {
    return (
      <Dialog
        title={t("import.title")}
        onClose={onClose}
        width={460}
        footer={
          <Button size="sm" variant="primary" data-autofocus onClick={onClose}>
            {t("common.close")}
          </Button>
        }
      >
        <p className="text-xs leading-relaxed text-ink">{t("import.done", { count: doneCount })}</p>
      </Dialog>
    );
  }

  return (
    <Dialog
      title={t("import.title")}
      onClose={onClose}
      width={640}
      // 粘了一屏内容之后误点遮罩关掉太亏
      dismissOnOverlay={false}
      footer={
        <>
          <Button size="sm" variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant="primary"
            disabled={!preview || committing}
            onClick={() => void submit()}
          >
            {committing ? t("import.committing") : t("import.confirm")}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        {/* ---- 来源 ---- */}
        <div className="space-y-1.5">
          <div className="flex items-center gap-2">
            <span className="text-[11px] text-ink-muted">{t("import.source")}</span>
            <FilePickerButton onPick={(file) => void pickFile(file)} />
            {fileName ? (
              <span className="truncate text-[11px] text-ink-faint" title={fileName}>
                {fileName}
              </span>
            ) : null}
            {/* 格式徽标：识别出来才显示。它是「这段东西我们认不认」的唯一反馈 */}
            {preview ? (
              <span className="ml-auto shrink-0 rounded-sm border border-line px-1.5 py-0.5 text-[10px] text-ink-muted">
                {t(FORMAT_LABEL[preview.format])}
              </span>
            ) : null}
          </div>

          <textarea
            value={text}
            onChange={(e) => {
              setText(e.target.value);
              setFileName(null);
            }}
            placeholder={t("import.curlPlaceholder")}
            spellCheck={false}
            data-autofocus
            aria-label={t("import.title")}
            className={cn(
              "h-32 w-full resize-y rounded-sm border border-line bg-surface px-2.5 py-2",
              "font-mono text-[11px] leading-relaxed text-ink placeholder:text-ink-faint",
              "transition-colors duration-100 hover:border-line-strong focus:border-brand",
            )}
          />
          {fileError ? <p className="text-[11px] text-err">{fileError}</p> : null}
        </div>

        {/* ---- 落点 ---- */}
        <label className="flex items-center gap-2">
          <span className="shrink-0 text-[11px] text-ink-muted">{t("import.targetFolder")}</span>
          <select
            value={targetId === null ? "" : String(targetId)}
            onChange={(e) => setTargetId(e.target.value === "" ? null : Number(e.target.value))}
            className={cn(
              "h-7 min-w-0 flex-1 rounded-sm border border-line bg-surface px-1.5",
              "text-xs text-ink transition-colors duration-100 hover:border-line-strong",
            )}
          >
            <option value="">{t("import.root")}</option>
            {folderOptions.map(({ folder, depth }) => (
              <option key={folder.id} value={folder.id}>
                {"　".repeat(depth)}
                {folder.name}
              </option>
            ))}
          </select>
        </label>

        {/* ---- 冲突模式 ---- */}
        <fieldset className="space-y-1">
          <legend className="mb-1 text-[11px] text-ink-muted">{t("import.conflict")}</legend>
          <div className="flex flex-wrap gap-x-4 gap-y-1">
            {CONFLICT_ORDER.map((mode) => (
              <label key={mode} className="flex cursor-pointer items-center gap-1.5">
                <input
                  type="radio"
                  name="import-conflict"
                  checked={conflict === mode}
                  onChange={() => setConflict(mode)}
                  className="accent-brand"
                />
                <span className="text-xs text-ink">{t(CONFLICT_LABEL[mode])}</span>
              </label>
            ))}
          </div>
          {/* 说明只跟着**当前选中**的那条走：三条都摊开会变成一段没人读的说明文 */}
          <p className="text-[11px] leading-relaxed text-ink-faint">{t(CONFLICT_HINT[conflict])}</p>
        </fieldset>

        {/* ---- 环境 ---- */}
        {hasEnvs ? (
          <div className="space-y-1 rounded-sm border border-line bg-canvas-subtle px-2.5 py-2">
            <label className="flex cursor-pointer items-center gap-1.5">
              <input
                type="checkbox"
                checked={includeEnvs}
                onChange={(e) => setIncludeEnvs(e.target.checked)}
                className="accent-brand"
              />
              <span className="text-xs text-ink">{t("import.envImport")}</span>
            </label>
            <ul className={cn("space-y-0.5 pl-4", includeEnvs ? "" : "opacity-45")}>
              {preview?.environments.map((env) => (
                <EnvPreviewRow key={env.name} env={env} />
              ))}
            </ul>
          </div>
        ) : null}

        {/* ---- 预览 ---- */}
        <div className="space-y-1.5">
          <div className="flex items-center gap-2">
            <span className="text-[11px] text-ink-muted">{t("import.preview")}</span>
            {parsing ? (
              <LoaderCircle size={11} strokeWidth={2.4} className="animate-spin text-ink-faint" />
            ) : null}
          </div>

          {previewError ? (
            <p className="text-[11px] leading-relaxed text-err">{previewError.localized}</p>
          ) : preview ? (
            <div className="space-y-2">
              <CountsBar counts={preview.counts} />
              {preview.nodes.length === 0 ? (
                <p className="text-[11px] text-ink-faint">{t("import.nothingToImport")}</p>
              ) : (
                <div className="max-h-64 overflow-y-auto rounded-sm border border-line px-1.5 py-1">
                  <ImportPreviewTree nodes={preview.nodes} />
                </div>
              )}
              <WarningList preview={preview} />
            </div>
          ) : (
            <p className="text-[11px] text-ink-faint">{t("import.emptyHint")}</p>
          )}
        </div>

        {commitError ? <p className="text-[11px] text-err">{commitError.localized}</p> : null}
      </div>
    </Dialog>
  );
}

/**
 * 警告列表。
 *
 * **不是错误**：有警告也照样导得进来。所以它排在预览树**下面**、
 * 用警示色而不是错误色 —— 位置上就说明「主体已经没问题了，
 * 这些是顺带告诉你的」。
 */
function WarningList({ preview }: { preview: ImportPreview }) {
  const { t } = useI18n();
  if (preview.warnings.length === 0) return null;

  return (
    <div className="space-y-1">
      <div className="flex items-center gap-1.5">
        <TriangleAlert size={11} strokeWidth={2.2} aria-hidden className="text-warn" />
        <span className="text-[11px] text-ink-muted">
          {t("import.warnings", { count: preview.warnings.length })}
        </span>
      </div>
      <ul className="max-h-28 space-y-0.5 overflow-y-auto pl-4">
        {preview.warnings.map((warning, index) => (
          <li
            // 同一份文件里同一类警告会重复出现，细节又不适合当 key（可能很长）
            key={`${warning.kind}-${index}`}
            className="text-[11px] leading-relaxed text-ink-faint"
            title={warning.detail}
          >
            {warning.subject ? <span className="text-ink-muted">{warning.subject}：</span> : null}
            {t(WARNING_LABEL[warning.kind])}
          </li>
        ))}
      </ul>
    </div>
  );
}

/** 选文件。隐藏的 input + 一个按钮：原生 input 的样式在各平台上都没法统一 */
function FilePickerButton({ onPick }: { onPick: (file: File) => void }) {
  const { t } = useI18n();
  const inputRef = useRef<HTMLInputElement>(null);

  return (
    <>
      <Button
        size="sm"
        variant="secondary"
        icon={<FileUp size={12} strokeWidth={2.1} />}
        onClick={() => inputRef.current?.click()}
      >
        {t("import.pickFile")}
      </Button>
      <input
        ref={inputRef}
        type="file"
        accept=".json,.txt,.sh,.bash,.zsh,.ps1,.cmd,.bat,.har,application/json,text/plain"
        className="hidden"
        onChange={(e) => {
          const file = e.target.files?.[0];
          // 立刻清空：不清的话「选同一个文件第二次」不会触发 change
          e.target.value = "";
          if (file) onPick(file);
        }}
      />
    </>
  );
}

/**
 * 分类树 → 带缩进的平铺列表，供 `<select>` 用。
 *
 * `<select>` 里的 option 不支持真正的层级，只能用全角空格做视觉缩进。
 * 用全角而不是半角：半角空格在 option 里会被折叠，看起来跟没缩进一样。
 */
function flattenFolders(folders: Folder[]): Array<{ folder: Folder; depth: number }> {
  const byParent = new Map<number | null, Folder[]>();
  for (const folder of folders) {
    const list = byParent.get(folder.parentId);
    if (list) list.push(folder);
    else byParent.set(folder.parentId, [folder]);
  }

  const out: Array<{ folder: Folder; depth: number }> = [];
  const walk = (parentId: number | null, depth: number) => {
    for (const folder of byParent.get(parentId) ?? []) {
      // 库里有环（导入来的脏数据）时不能无限递归 —— 深度封顶
      if (depth > 32) continue;
      out.push({ folder, depth });
      walk(folder.id, depth + 1);
    }
  };
  walk(null, 0);
  return out;
}

/** 侧栏那个按钮点开的就是它 */
export function ConnectedImportDialog() {
  const importOpen = useUiStore((s) => s.importOpen);
  const closeImport = useUiStore((s) => s.closeImport);
  if (!importOpen) return null;
  return <ImportDialog onClose={closeImport} />;
}
