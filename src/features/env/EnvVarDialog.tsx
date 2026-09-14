import { Asterisk, Eye, EyeOff, Plus, Trash } from "lucide-react";
import { useEffect, useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { EnvVar, Environment } from "@/lib/types";
import { useEnvStore } from "@/store/env";
import { Button, IconButton } from "@/features/ui/Button";
import { Dialog } from "@/features/ui/Dialog";

/** 变量名长度上限。**必须**与后端 `commands::env::MAX_VAR_KEY_LEN` 一致 */
const MAX_KEY_LEN = 128;

/** 新建一行时的初值 —— id/sortOrder 由后端重排，这里给什么都不作数 */
function blankRow(environmentId: number): EnvVar {
  return {
    id: 0,
    environmentId,
    key: "",
    value: "",
    isSecret: false,
    enabled: true,
    sortOrder: 0,
  };
}

export interface EnvVarDialogProps {
  environment: Environment;
  onClose: () => void;
}

/**
 * 环境变量表。
 *
 * ## 编辑对象是**草稿**，不是 store 里的数据
 *
 * 关掉对话框时没保存的改动就该消失 —— 这与「自动保存」的请求编辑区
 * 刻意不同：变量表是**整表提交**语义（design.md §6.3），
 * 边打字边落库等于每敲一个字符就删一遍表再插一遍。
 *
 * ## 密钥列的「显示」只在界面上打码
 *
 * P4 **不接** keyring（见 `docs/plan.md` §4 的决策）：`is_secret`
 * 现在只影响展示（打码 + 需要点一下才显示）。加密存储是 P8 的事，
 * 但字段与交互先就位，免得将来既要改库又要改一套 UI。
 */
export function EnvVarDialog({ environment, onClose }: EnvVarDialogProps) {
  const { t } = useI18n();
  const replaceVars = useEnvStore((s) => s.replaceVars);

  const [rows, setRows] = useState<EnvVar[] | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [saving, setSaving] = useState(false);
  /** 被临时「看一眼」的密钥行下标 */
  const [revealed, setRevealed] = useState<ReadonlySet<number>>(new Set());

  useEffect(() => {
    let cancelled = false;
    setRows(null);
    setLoadError(false);

    void useEnvStore
      .getState()
      .vars(environment.id)
      .then((vars) => {
        // StrictMode 下 effect 会跑两遍：第一遍的结果回来时已经换了环境，
        // 直接丢弃，否则会把上一个环境的变量灌进这个对话框
        if (!cancelled) setRows(vars.length > 0 ? vars : [blankRow(environment.id)]);
      })
      .catch((e) => {
        if (cancelled) return;
        console.warn("[reqly] 变量加载失败：", e);
        setLoadError(true);
        setRows([]);
      });

    return () => {
      cancelled = true;
    };
  }, [environment.id]);

  if (rows === null) {
    return (
      <Dialog
        title={t("env.editTitle", { name: environment.name })}
        onClose={onClose}
        width={620}
      >
        <p className="py-6 text-center text-xs text-ink-faint">{t("common.loading")}</p>
      </Dialog>
    );
  }

  const patchRow = (index: number, next: Partial<EnvVar>) =>
    setRows(rows.map((row, i) => (i === index ? { ...row, ...next } : row)));

  const removeRow = (index: number) => setRows(rows.filter((_, i) => i !== index));

  /** 有键名的行才算「有效」，用来决定保存按钮能不能点 */
  const usable = rows.filter((row) => row.key.trim() !== "");
  const overlong = usable.find((row) => row.key.trim().length > MAX_KEY_LEN);
  const canSave = usable.length > 0 && overlong === undefined && !saving;

  const save = async () => {
    setSaving(true);
    // 空白键由后端丢弃（不报错）—— 这里也一并去掉，免得「保存后行数变少了」
    const saved = await replaceVars(
      environment.id,
      rows.filter((row) => row.key.trim() !== ""),
    );
    setSaving(false);
    if (saved) onClose();
  };

  return (
    <Dialog
      title={t("env.editTitle", { name: environment.name })}
      description={t("env.editDesc")}
      onClose={onClose}
      width={620}
      // 有未保存的改动，点遮罩就丢太容易了
      dismissOnOverlay={false}
      footer={
        <>
          <span className="mr-auto text-[11px] text-ink-faint">
            {overlong
              ? t("env.tooManyVars", { max: MAX_KEY_LEN })
              : t("env.varCount", { count: usable.length })}
          </span>
          <Button size="sm" variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button size="sm" variant="primary" disabled={!canSave} onClick={() => void save()}>
            {saving ? t("common.loading") : t("common.save")}
          </Button>
        </>
      }
    >
      {loadError ? (
        <p className="py-4 text-xs text-err">{t("env.varsLoadFailed")}</p>
      ) : (
        <div className="space-y-1.5">
          <div className="flex items-center gap-1.5 text-[11px] text-ink-faint">
            <span className="w-6" />
            <span className="flex-1">{t("env.keyPlaceholder")}</span>
            <span className="flex-1">{t("env.valuePlaceholder")}</span>
            {/* 三个图标槽：标记为密钥 / 看一眼 / 删除。表头也要占位，
                否则上面的标题会比下面的输入框往右偏一格 */}
            <span className="w-[26px]" />
            <span className="w-[26px]" />
            <span className="w-[26px]" />
          </div>

          {rows.map((row, index) => (
            <VarRow
              key={index}
              row={row}
              revealed={revealed.has(index)}
              onToggleReveal={() =>
                setRevealed((prev) => {
                  const next = new Set(prev);
                  if (next.has(index)) next.delete(index);
                  else next.add(index);
                  return next;
                })
              }
              onPatch={(next) => patchRow(index, next)}
              onRemove={() => removeRow(index)}
            />
          ))}

          <Button
            size="sm"
            variant="ghost"
            icon={<Plus size={13} strokeWidth={2.2} />}
            onClick={() => setRows([...rows, blankRow(environment.id)])}
          >
            {t("env.addVariable")}
          </Button>
        </div>
      )}
    </Dialog>
  );
}

function VarRow({
  row,
  revealed,
  onToggleReveal,
  onPatch,
  onRemove,
}: {
  row: EnvVar;
  revealed: boolean;
  onToggleReveal: () => void;
  onPatch: (next: Partial<EnvVar>) => void;
  onRemove: () => void;
}) {
  const { t } = useI18n();
  const dim = row.enabled ? "text-ink" : "text-ink-faint";
  const masked = row.isSecret && !revealed;

  return (
    <div className="flex items-center gap-1.5">
      <input
        type="checkbox"
        checked={row.enabled}
        onChange={(e) => onPatch({ enabled: e.target.checked })}
        aria-label={t("kv.enable")}
        className="h-3.5 w-6 shrink-0 cursor-pointer accent-[var(--brand)]"
      />

      <input
        value={row.key}
        onChange={(e) => onPatch({ key: e.target.value })}
        placeholder={t("env.keyPlaceholder")}
        spellCheck={false}
        autoComplete="off"
        aria-label={t("env.keyPlaceholder")}
        className={cn(FIELD, "flex-1", dim)}
      />

      {/* 密钥行的值用 `type=password` 而不是「画几个圆点」：
          前者能挡住截屏之外的偷看（浏览器不会把它放进自动填充的候选里），
          后者只是一层视觉障眼法 */}
      <input
        type={masked ? "password" : "text"}
        value={row.value}
        onChange={(e) => onPatch({ value: e.target.value })}
        placeholder={t("env.valuePlaceholder")}
        spellCheck={false}
        autoComplete="off"
        aria-label={t("env.valuePlaceholder")}
        className={cn(FIELD, "flex-1", dim)}
      />

      <IconButton
        size="sm"
        label={t("env.secret")}
        onClick={() => onPatch({ isSecret: !row.isSecret })}
        className={row.isSecret ? "text-warn" : "text-ink-faint"}
      >
        <Asterisk size={13} strokeWidth={2} aria-hidden />
      </IconButton>

      {/* 「看一眼」只在标记为密钥的行上有意义。不标记时也画出来但禁用 ——
          行与行之间的图标位置才不会左右跳 */}
      <IconButton
        size="sm"
        label={revealed ? t("env.hideSecret") : t("env.revealSecret")}
        disabled={!row.isSecret}
        onClick={onToggleReveal}
        className="text-ink-faint"
      >
        {revealed && row.isSecret ? (
          <EyeOff size={13} strokeWidth={2} aria-hidden />
        ) : (
          <Eye size={13} strokeWidth={2} aria-hidden />
        )}
      </IconButton>

      <IconButton size="sm" label={t("kv.remove")} onClick={onRemove}>
        <Trash size={13} strokeWidth={2} />
      </IconButton>
    </div>
  );
}

/**
 * 输入框外形。**不含文字色** —— 「这一行被停用」会把文字色换成 `text-ink-faint`，
 * 而 `cn` 不做冲突消解（见 `lib/cn.ts`），两个 `text-*` 撞在一起时
 * 生效的是构建顺序靠后的那个，不是书写顺序靠后的那个。
 */
const FIELD = cn(
  "h-7 min-w-0 rounded-sm border border-line bg-surface px-2",
  "font-mono text-xs placeholder:font-sans placeholder:text-ink-faint",
  "outline-none transition-colors duration-100",
  "hover:border-line-strong focus:border-brand",
);
