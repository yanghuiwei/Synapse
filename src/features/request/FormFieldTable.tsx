import { FileUp, Trash } from "lucide-react";
import { useEffect, useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import * as ipc from "@/lib/ipc";
import type { KeyValueItem } from "@/lib/types";
import { InlineNotice } from "@/features/ui/InlineNotice";
import { IconButton } from "@/features/ui/Button";
import { isBlankRow, withTrailingBlank } from "./KeyValueTable";
import { PlaceholderInput } from "./PlaceholderInput";

/** 提示自己消失的时间。与请求体编辑器的一致 */
const NOTICE_MS = 2200;

/**
 * 这一行是不是文件字段。
 *
 * `undefined` = 文本行；**空串** = 用户选了「文件」但还没挑具体文件。
 * 两者必须分开：合成一个 `""` 的话，切到文件类型会因为「路径是空的」
 * 又被当成文本行，那个下拉框会自己弹回去。
 */
function isFileRow(row: KeyValueItem): boolean {
  return row.file !== undefined;
}

/**
 * 行是不是空的。
 *
 * 不能直接用 `KeyValueTable` 那份判定：文件行**没有 `value`**，
 * 照那份算就是空行 → 末尾会补一个空行并排在它下面。
 */
function isBlankFormRow(row: KeyValueItem | undefined): boolean {
  if (!row || !row.enabled) return false;
  if (isFileRow(row)) return false;
  return isBlankRow(row);
}

// 与 `KeyValueTable` 同一个常量：两张表并排出现在同一个页签里，
// 字号/内边距不一致一眼就能看出来
const CELL =
  "w-full bg-transparent px-1.5 py-1 font-mono text-xs " +
  "placeholder:font-sans placeholder:text-ink-faint";

export interface FormFieldTableProps {
  rows: readonly KeyValueItem[];
  onChange: (rows: KeyValueItem[]) => void;
  className?: string;
}

/**
 * `multipart/form-data` 的字段表。
 *
 * ## 与 `KeyValueTable` 的差别只有两点
 *
 * 1. 每行多一个**类型**（文本 / 文件）。选文件时值那一列换成「路径 + 选择文件」。
 * 2. 文件路径**不在编辑期读盘**。只有发送那一刻才读（见 Rust 的 `build_multipart`）——
 *    在这里读意味着几十 MB 的内容先进内存、再走一遍 IPC，而它只是要发出去。
 *
 * ## 为什么不复用 `KeyValueTable` 加个 `files` 开关
 *
 * 那张表服务 params / headers / urlencoded 三处，是**最热**的一条界面路径。
 * 给它加一个只有 multipart 用得上的分支，等于让每次渲染都多背一份复杂度 ——
 * 而这两张表的共同部分（末尾空行、删除按钮、勾选框）本来就只有几十行。
 */
export function FormFieldTable({ rows, onChange, className }: FormFieldTableProps) {
  const { t } = useI18n();
  const view = withTrailingBlank(rows, isBlankFormRow);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    if (notice === null) return;
    const timer = window.setTimeout(() => setNotice(null), NOTICE_MS);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const patchRow = (index: number, patch: Partial<KeyValueItem>) => {
    onChange(view.map((row, i) => (i === index ? { ...row, ...patch } : row)));
  };

  const removeRow = (index: number) => {
    onChange(view.filter((_, i) => i !== index));
  };

  /** 切换一行的类型。值刻意留着 —— 切到文件再切回来，原来写的文本还在 */
  const setRowKind = (index: number, kind: "text" | "file") => {
    const row = view[index];
    if (row === undefined) return;
    if (kind === "file" && !isFileRow(row)) {
      patchRow(index, { file: "" });
      return;
    }
    if (kind === "text" && isFileRow(row)) {
      patchRow(index, { file: undefined });
    }
  };

  const browse = async (index: number) => {
    try {
      const picked = await ipc.files.pickFile();
      // `null` = 用户在对话框里点了取消。**不是错误**，什么都不做
      if (picked === null) return;
      patchRow(index, { file: picked });
    } catch (e) {
      // 走 `localized` 而不是 `message` —— 后者是给控制台看的
      // （`E_NOT_TAURI: window.__TAURI_INTERNALS__ is missing` 这种），
      // 直接摆到界面上就是半中半英的乱码
      setNotice(ipc.toIpcError(e).localized);
    }
  };

  return (
    <div className={cn("min-h-0", className)}>
      {notice !== null ? (
        <InlineNotice tone="err" className="py-1">
          {notice}
        </InlineNotice>
      ) : null}

      {/* `table-fixed`：理由与 `KeyValueTable` 那份相同（列宽只认 `<colgroup>`），
          这里还多一条 —— 「值」列右侧挂着「选择文件」按钮，
          被内容撑宽的话按钮会被顶出可视区 */}
      <table className="w-full table-fixed border-collapse">
        <colgroup>
          <col style={{ width: 30 }} />
          <col style={{ width: "30%" }} />
          <col style={{ width: 92 }} />
          <col />
          <col style={{ width: 30 }} />
        </colgroup>

        <thead>
          <tr className="border-b border-line text-left text-[11px] text-ink-faint">
            <th />
            <th className="px-1.5 py-1 font-normal">{t("kv.key")}</th>
            <th className="px-1.5 py-1 font-normal">{t("kv.fieldType")}</th>
            <th className="px-1.5 py-1 font-normal">{t("kv.value")}</th>
            <th />
          </tr>
        </thead>

        <tbody>
          {view.map((row, index) => {
            const blank = isBlankFormRow(row);
            const dim = row.enabled ? "text-ink" : "text-ink-faint";
            const file = isFileRow(row);

            return (
              <tr key={index} className="group border-b border-line/60">
                <td className="pl-1.5">
                  <input
                    type="checkbox"
                    checked={row.enabled}
                    onChange={(e) => patchRow(index, { enabled: e.target.checked })}
                    aria-label={t("kv.enable")}
                    className="h-3.5 w-3.5 cursor-pointer accent-[var(--brand)]"
                  />
                </td>

                <td>
                  <input
                    value={row.key}
                    spellCheck={false}
                    autoComplete="off"
                    placeholder={t("kv.key")}
                    onChange={(e) => patchRow(index, { key: e.target.value })}
                    className={cn(CELL, dim)}
                  />
                </td>

                <td>
                  <select
                    value={file ? "file" : "text"}
                    aria-label={t("kv.fieldType")}
                    onChange={(e) => setRowKind(index, e.target.value as "text" | "file")}
                    disabled={blank}
                    className={cn(
                      "w-full rounded-sm bg-transparent px-1 py-0.5 text-xs",
                      "outline-none transition-colors duration-100",
                      dim,
                    )}
                  >
                    <option value="text">{t("kv.fieldTypeText")}</option>
                    <option value="file">{t("kv.fieldTypeFile")}</option>
                  </select>
                </td>

                <td>
                  {file ? (
                    <div className="flex items-center gap-0.5">
                      {/* 路径不接占位符高亮：它几乎总是一串绝对路径，
                          高亮层要的两层像素对齐在这里只有成本没有收益 */}
                      <input
                        value={row.file ?? ""}
                        spellCheck={false}
                        autoComplete="off"
                        placeholder={t("kv.filePlaceholder")}
                        title={row.file === "" ? undefined : row.file}
                        onChange={(e) => patchRow(index, { file: e.target.value })}
                        className={cn(CELL, dim)}
                      />
                      <IconButton
                        size="sm"
                        label={t("kv.pickFile")}
                        onClick={() => void browse(index)}
                        className="shrink-0 text-ink-faint hover:text-brand"
                      >
                        <FileUp size={13} strokeWidth={2} />
                      </IconButton>
                    </div>
                  ) : (
                    <PlaceholderInput
                      dense
                      multiline
                      value={row.value}
                      onChange={(value) => patchRow(index, { value })}
                      placeholder={t("kv.value")}
                      label={t("kv.value")}
                      tone={row.enabled ? "normal" : "muted"}
                    />
                  )}
                </td>

                <td className="pr-1 text-center">
                  {blank ? null : (
                    <IconButton
                      size="sm"
                      label={t("kv.remove")}
                      onClick={() => removeRow(index)}
                      className="text-ink-faint opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                    >
                      <Trash size={13} strokeWidth={2} />
                    </IconButton>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>

      {rows.length === 0 ? (
        <p className="px-3 py-2 text-[11px] text-ink-faint">{t("kv.empty")}</p>
      ) : null}
    </div>
  );
}
