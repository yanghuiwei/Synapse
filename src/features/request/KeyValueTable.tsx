import { Trash } from "lucide-react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { KeyValueItem } from "@/lib/types";
import { IconButton } from "@/features/ui/Button";

export interface KeyValueTableProps {
  rows: readonly KeyValueItem[];
  onChange: (rows: KeyValueItem[]) => void;
  /** 是否显示「说明」列（查询参数不需要，请求头有时想写一句备注） */
  showDesc?: boolean;
  keyPlaceholder?: string;
  valuePlaceholder?: string;
  className?: string;
}

function isBlankRow(row: KeyValueItem | undefined): row is undefined | never {
  return !row || (row.key === "" && row.value === "" && row.enabled);
}

/**
 * 编辑器末尾永远留一个空行，让用户边打字边「长」出新行。
 *
 * 这一行是**渲染时补出来的**，不在 `rows` 里 —— 否则每次保存都会往库里
 * 塞一行空的。一旦用户在上面敲了字，它就被下面 `patchRow` 的
 * 「改整个 view」方式实体化进数组，空行随即在末尾重新出现。
 */
function withTrailingBlank(rows: readonly KeyValueItem[]): KeyValueItem[] {
  const list = rows.slice();
  if (isBlankRow(list[list.length - 1]).valueOf() || list.length === 0) {
    list.push({ key: "", value: "", enabled: true });
  }
  return list;
}

const CELL =
  "w-full bg-transparent px-1.5 py-1 font-mono text-xs outline-none " +
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
      <table className="w-full border-collapse">
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
            const blank = isBlankRow(row).valueOf();
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
                  <input
                    value={row.value}
                    spellCheck={false}
                    autoComplete="off"
                    placeholder={valuePlaceholder}
                    onChange={(e) => patchRow(index, { value: e.target.value })}
                    className={cn(CELL, dim)}
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
