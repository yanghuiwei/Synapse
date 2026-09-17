import { Trash } from "lucide-react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { KeyValueItem } from "@/lib/types";
import { IconButton } from "@/features/ui/Button";
import { PlaceholderInput } from "./PlaceholderInput";

export interface KeyValueTableProps {
  rows: readonly KeyValueItem[];
  onChange: (rows: KeyValueItem[]) => void;
  /** 是否显示「说明」列（查询参数不需要，请求头有时想写一句备注） */
  showDesc?: boolean;
  keyPlaceholder?: string;
  valuePlaceholder?: string;
  className?: string;
}

/** 空行判定：什么都没写、且仍是启用状态。文件行由调用方另行判定（见 `FormFieldTable`） */
export function isBlankRow(row: KeyValueItem | undefined): boolean {
  return !row || (row.key === "" && row.value === "" && row.enabled);
}

/**
 * 编辑器末尾永远留一个空行，让用户边打字边「长」出新行。
 *
 * 这一行是**渲染时补出来的**，不在 `rows` 里 —— 否则每次保存都会往库里
 * 塞一行空的。一旦用户在上面敲了字，它就被下面 `patchRow` 的
 * 「改整个 view」方式实体化进数组，空行随即在末尾重新出现。
 *
 * 条件是「**末尾那行不是空的**就补一个」。
 * 反过来写（只在末尾已经为空时才补）会同时出两个毛病：
 *   * 末行非空时不给空行 —— 界面上就没有可下笔的地方，加不了新行。
 *     从外部灌入的行（urlencoded 从 `body` 解析出来的、导入器写进去的）
 *     恰好全都不以空行结尾，所以那条路径上是必然踩到。
 *   * 末行已空时再补一个 —— 末尾会并排出现两个空行。
 *
 * `isBlank` 可替换：multipart 的表格里「只有文件路径、没有文本值」的行
 * **不是空行**，照通用的判定会被补出一个空行并排在它下面。
 */
export function withTrailingBlank(
  rows: readonly KeyValueItem[],
  isBlank: (row: KeyValueItem | undefined) => boolean = isBlankRow,
): KeyValueItem[] {
  const list = rows.slice();
  if (list.length === 0 || !isBlank(list[list.length - 1])) {
    list.push({ key: "", value: "", enabled: true });
  }
  return list;
}

// 刻意**不写** `outline-none`：表格里的输入框没有边框，全局的
// `:focus-visible` 焦点环就是它唯一的键盘焦点指示（见 global.css 的注释）
const CELL =
  "w-full bg-transparent px-1.5 py-1 font-mono text-xs " +
  "placeholder:font-sans placeholder:text-ink-faint";

/**
 * 键值对表格 —— 查询参数与请求头共用。
 *
 * 用原生 `<table>` 而不是 flex 拼的假表格：列宽由 `<colgroup>` 统一控制，
 * 不会出现两行宽度对不上；读屏也能正确念出「键 / 值」的表头。
 */
export function KeyValueTable({
  rows,
  onChange,
  showDesc = false,
  keyPlaceholder,
  valuePlaceholder,
  className,
}: KeyValueTableProps) {
  const { t } = useI18n();
  const view = withTrailingBlank(rows);

  const patchRow = (index: number, patch: Partial<KeyValueItem>) => {
    onChange(view.map((row, i) => (i === index ? { ...row, ...patch } : row)));
  };

  const removeRow = (index: number) => {
    onChange(view.filter((_, i) => i !== index));
  };

  return (
    <div className={cn("min-h-0", className)}>
      {/* `table-fixed`：列宽只认 `<colgroup>`，**不再由内容反推**。
          少了它，一个 64 字符的 `scope` 值就能把「值」列撑到一千多像素宽，
          把「键」列挤到只剩两三个字符（实测：`client_id` 显示成 `cl`），
          同时整张表横向溢出、值列反而永远不需要折行 ——
          那一格再窄也折不出第二行。 */}
      <table className="w-full table-fixed border-collapse">
        <colgroup>
          <col style={{ width: 30 }} />
          <col style={{ width: showDesc ? "30%" : "42%" }} />
          <col />
          {showDesc ? <col style={{ width: "26%" }} /> : null}
          <col style={{ width: 30 }} />
        </colgroup>

        <thead>
          <tr className="border-b border-line text-left text-[11px] text-ink-faint">
            <th />
            <th className="px-1.5 py-1 font-normal">{t("kv.key")}</th>
            <th className="px-1.5 py-1 font-normal">{t("kv.value")}</th>
            {showDesc ? (
              <th className="px-1.5 py-1 font-normal">{t("kv.description")}</th>
            ) : null}
            <th />
          </tr>
        </thead>

        <tbody>
          {view.map((row, index) => {
            const blank = isBlankRow(row);
            const dim = row.enabled ? "text-ink" : "text-ink-faint";
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
                    placeholder={keyPlaceholder}
                    onChange={(e) => patchRow(index, { key: e.target.value })}
                    className={cn(CELL, dim)}
                  />
                </td>

                <td>
                  {/* 值这一列走 `PlaceholderInput`：`?x={{env.baseUrl}}`、
                      `Authorization: {{token.access}}` 是最常见的两处占位符，
                      在表格里当场看出写没写对，比发送后翻响应体便宜得多。
                      **键那一列不接** —— 键名从来不是模板。 */}
                  <PlaceholderInput
                    dense
                    multiline
                    value={row.value}
                    onChange={(value) => patchRow(index, { value })}
                    placeholder={valuePlaceholder}
                    label={t("kv.value")}
                    tone={row.enabled ? "normal" : "muted"}
                  />
                </td>

                {showDesc ? (
                  <td>
                    <input
                      value={row.desc ?? ""}
                      spellCheck={false}
                      autoComplete="off"
                      onChange={(e) => patchRow(index, { desc: e.target.value })}
                      className={cn(CELL, "font-sans text-ink-muted")}
                    />
                  </td>
                ) : null}

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
