import { ChevronDown, KeyRound, Plus, TriangleAlert } from "lucide-react";
import { useMemo, type ReactNode } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import type { ApiRequest, ExpiryUnit, TokenSource } from "@/lib/types";
import { Button } from "@/features/ui/Button";
import { useUiStore } from "@/store/ui";

/**
 * Token 源的可编辑字段。
 *
 * 与 `TokenSource` 的差别：没有 id / 时间戳（新建时还没有），
 * 也没有 `workspaceId`（由调用方塞进去）。用 `Omit` 而不是重写一遍，
 * 是为了让「后端加了字段」时这里会编译报错，而不是静默漏掉。
 */
export type TokenSourceDraft = Omit<TokenSource, "id" | "createdAt" | "updatedAt">;

/**
 * 新建时的初值。
 *
 * 四个路径刻意与 design.md §4.3 的示例一致：绝大多数 OAuth 接口就是这个形状，
 * 让常见情况开箱可用，而不是让用户从空白开始填。
 *
 * `refreshRequestId` 恒为 `null` —— 「获取 Token」与「刷新 Token」在本产品里
 * 是同一个接口，界面上已经没有第二个请求可配了（后端仍支持分开配，
 * 老库里配过的那种照常走 `refresh_request_id`，只是新建时不会再有值）。
 */
export function emptyTokenSourceDraft(folderId: number): TokenSourceDraft {
  return {
    workspaceId: 0, // 由调用方覆盖
    folderId,
    name: "",
    fetchRequestId: 0,
    accessPath: "$.access_token",
    refreshPath: "$.refresh_token",
    expiresInPath: "$.expires_in",
    expiresInUnit: "seconds",
    refreshRequestId: null,
    autoRefresh: true,
    skewSeconds: 30,
  };
}

/** 从已有配置反推一份草稿（编辑时用） */
export function draftFromSource(source: TokenSource): TokenSourceDraft {
  const { id: _id, createdAt: _createdAt, updatedAt: _updatedAt, ...rest } = source;
  return rest;
}

const UNIT_LABELS: Record<ExpiryUnit, MessageKey> = {
  seconds: "token.unit.seconds",
  millis: "token.unit.millis",
  absolute: "token.unit.absolute",
};

export interface TokenSourceFormProps {
  value: TokenSourceDraft;
  onChange: (next: TokenSourceDraft) => void;
  /**
   * 可选的 Token 接口。**由调用方决定给哪些** —— 现在是系统分类「token刷新」
   * 下的全部请求。表单自己不去全局捞：那样「这个接口到底放哪儿」这件事
   * 又变成用户要猜的东西了。
   */
  requests: readonly ApiRequest[];
  className?: string;
}

/**
 * Token 来源配置表单。
 *
 * ## 为什么只留 6 组字段
 *
 * 早先还有「所属分类」和「刷新 Token 的请求」两项，现在都去掉了：
 * 分类固定成系统分类「token刷新」（见 `folder::ensure_system_folder`），
 * 而获取与刷新本来就是同一个接口 —— 多配一次只是多一次配错的机会。
 *
 * ## 布局为什么是一列
 *
 * 剩下这几项里「过期时间路径」+「单位」必须挨着读（单位是给路径的解释），
 * 强行两列会把它们拆开。一列 + 分组标题读起来更省事。
 */
export function TokenSourceForm({
  value,
  onChange,
  requests,
  className,
}: TokenSourceFormProps) {
  const { t } = useI18n();

  const patch = (next: Partial<TokenSourceDraft>) => onChange({ ...value, ...next });

  /**
   * 下拉选项。
   *
   * 当前值可能指向一个**已经被删掉**的请求。这种情况不补一个占位项的话，
   * `<select>` 找不到匹配的 option 会退回去显示第一项「选择一个请求…」——
   * 界面在撒谎，而发送时后端读的仍是那个旧 id（`load_fetch_spec` 直接
   * `NotFound`，请求整个失败）。宁可写「已失效」，让用户自己动手改。
   */
  const options = useMemo(() => {
    const base = [
      { value: "0", label: t("token.pickRequest") },
      ...requests.map((r) => ({ value: String(r.id), label: `${r.method} ${r.name}` })),
    ];

    const current = value.fetchRequestId;
    if (current === 0 || requests.some((r) => r.id === current)) return base;
    return [{ value: String(current), label: t("token.fetchRequestGone") }, ...base];
  }, [requests, value.fetchRequestId, t]);

  const accessError = pathError(value.accessPath, true, t);
  const refreshError = pathError(value.refreshPath, false, t);
  const expiresError = pathError(value.expiresInPath, false, t);

  return (
    <div className={cn("space-y-3", className)}>
      <Field label={t("token.name")}>
        <input
          value={value.name}
          onChange={(e) => patch({ name: e.target.value })}
          placeholder={t("token.namePlaceholder")}
          spellCheck={false}
          autoComplete="off"
          className={INPUT}
        />
      </Field>

      <Field label={t("token.fetchRequest")} hint={t("token.fetchRequestHint")}>
        {requests.length === 0 ? (
          <div className="space-y-2">
            <p className="flex items-start gap-1.5 py-1 text-[11px] text-warn">
              <TriangleAlert size={12} strokeWidth={2.2} aria-hidden className="mt-0.5 shrink-0" />
              {t("token.noRequestInFolder")}
            </p>
            {/* 光说「还没有接口」是个死路：用户会去左栏找「把刚才那个登录接口挪过来」
                这个动作，而那里只有拖拽。直接给一个能在该分类下建接口的入口。 */}
            <Button
              size="sm"
              variant="secondary"
              icon={<Plus size={11} strokeWidth={2.2} />}
              onClick={() => useUiStore.getState().openCreateDialog("request", value.folderId)}
            >
              {t("token.createFetchRequest")}
            </Button>
          </div>
        ) : (
          <Select
            value={String(value.fetchRequestId)}
            onChange={(raw) => patch({ fetchRequestId: Number(raw) })}
            options={options}
          />
        )}
      </Field>

      <Group title={t("token.extractTitle")} icon={KeyRound}>
        <Field label={t("token.accessPath")}>
          <PathInput
            value={value.accessPath}
            onChange={(accessPath) => patch({ accessPath })}
            error={accessError}
          />
        </Field>

        <Field label={t("token.refreshPath")} hint={t("token.refreshPathHint")}>
          <PathInput
            value={value.refreshPath}
            onChange={(refreshPath) => patch({ refreshPath })}
            error={refreshError}
            optional
          />
        </Field>

        <Field label={t("token.expiresInPath")} hint={t("token.expiresInPathHint")}>
          <div className="flex items-center gap-1.5">
            <PathInput
              value={value.expiresInPath}
              onChange={(expiresInPath) => patch({ expiresInPath })}
              error={expiresError}
              optional
              className="flex-1"
            />
            <Select
              className="w-[132px] shrink-0"
              value={value.expiresInUnit}
              onChange={(raw) => patch({ expiresInUnit: raw as ExpiryUnit })}
              options={(Object.keys(UNIT_LABELS) as ExpiryUnit[]).map((u) => ({
                value: u,
                label: t(UNIT_LABELS[u]),
              }))}
              disabled={value.expiresInPath.trim() === ""}
            />
          </div>
        </Field>
      </Group>

      <Group title={t("token.autoRefresh")}>
        <label className="flex items-start gap-2 text-xs">
          <input
            type="checkbox"
            checked={value.autoRefresh}
            onChange={(e) => patch({ autoRefresh: e.target.checked })}
            className="mt-0.5 h-3.5 w-3.5 cursor-pointer accent-[var(--brand)]"
          />
          <span className="min-w-0">
            <span className="block text-ink">{t("token.autoRefresh")}</span>
            <span className="mt-0.5 block text-[11px] leading-relaxed text-ink-faint">
              {t("token.autoRefreshHint")}
            </span>
          </span>
        </label>

        <Field label={t("token.skew")} hint={t("token.skewHint")}>
          <input
            type="number"
            min={0}
            step={5}
            value={value.skewSeconds}
            // 负数由后端拒（`E_INVALID_INPUT`），这里先夹住 —— 没必要为一次
            // 输入法的手滑走一趟 IPC 再弹一个红框
            onChange={(e) =>
              patch({ skewSeconds: Math.max(0, Math.trunc(Number(e.target.value)) || 0) })
            }
            className={cn(INPUT, "w-24 tabular")}
          />
        </Field>
      </Group>
    </div>
  );
}

/**
 * 输入框的公共外形。
 *
 * ⚠️ **刻意不含文字色与边框色** —— 它们是变体，见下面的常量。
 * `cn` 不做冲突消解（见 `lib/cn.ts`）：同时给出 `border-line` 和 `border-err`
 * 时，最终生效的那个取决于构建时生成的 CSS 顺序，而不是书写顺序。
 * 这不是「大概能work」的问题，是「今天对明天错」的问题。
 */
const FIELD_BASE =
  "rounded-sm border bg-surface font-mono text-xs outline-none transition-colors duration-100 disabled:opacity-45";

/** 常规输入框 */
const INPUT = cn(
  FIELD_BASE,
  "h-8 w-full px-2 text-ink placeholder:font-sans placeholder:text-ink-faint",
  "border-line hover:border-line-strong focus:border-brand",
);

/** 校验不通过时的输入框 */
const INPUT_ERROR = cn(
  FIELD_BASE,
  "h-8 w-full px-2 text-ink placeholder:font-sans placeholder:text-ink-faint",
  "border-err",
);

/** 下拉框：`appearance-none` 去掉原生箭头，右边留出我们自己画的 ChevronDown 的位置 */
const SELECT = cn(
  FIELD_BASE,
  "h-8 w-full appearance-none pl-2 pr-7 text-ink",
  "border-line hover:border-line-strong focus:border-brand",
);

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <label className="block space-y-1">
      <span className="block text-[11px] text-ink-muted">{label}</span>
      {children}
      {hint ? (
        <span className="block text-[11px] leading-relaxed text-ink-faint">{hint}</span>
      ) : null}
    </label>
  );
}

function Group({
  title,
  icon: Icon,
  children,
}: {
  title: string;
  icon?: typeof KeyRound;
  children: ReactNode;
}) {
  return (
    <fieldset className="space-y-2 rounded-md border border-line p-2.5">
      <legend className="flex items-center gap-1 px-1 text-[11px] font-medium text-ink-muted">
        {Icon ? <Icon size={11} strokeWidth={2.2} aria-hidden /> : null}
        {title}
      </legend>
      {children}
    </fieldset>
  );
}

function Select({
  value,
  onChange,
  options,
  disabled,
  className,
}: {
  value: string;
  onChange: (value: string) => void;
  options: readonly { value: string; label: string }[];
  disabled?: boolean;
  className?: string;
}) {
  return (
    <div className={cn("relative", className)}>
      <select
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
        className={SELECT}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
      <ChevronDown
        size={13}
        strokeWidth={2.4}
        aria-hidden
        className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 text-ink-faint"
      />
    </div>
  );
}

/**
 * JSONPath 输入框。
 *
 * 就地校验「要以 `$` 开头」：这个子集是我们自己手写的解析器
 * （见 `src-tauri/src/template/jsonpath.rs`），不支持通配符与递归下降。
 * 与其让用户配完一个不支持的路径、发送时拿到 `E_TOKEN_EXTRACT`，
 * 不如在输入框上就标出来。
 */
function PathInput({
  value,
  onChange,
  error,
  optional = false,
  className,
}: {
  value: string;
  onChange: (value: string) => void;
  /** 错误文案；`null` = 合法 */
  error: string | null;
  optional?: boolean;
  className?: string;
}) {
  const { t } = useI18n();
  return (
    <div className={cn("space-y-1", className)}>
      <input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={optional ? t("token.refreshPathHint") : "$.access_token"}
        spellCheck={false}
        autoComplete="off"
        aria-invalid={error !== null}
        className={error ? INPUT_ERROR : INPUT}
      />
      {error ? (
        <span className="block text-[11px] text-err">{error}</span>
      ) : null}
    </div>
  );
}

/** 路径合法性。`$` 开头是硬要求（手写解析器只认这个子集） */
function pathError(
  path: string,
  required: boolean,
  t: (key: MessageKey, params?: Record<string, string | number>) => string,
): string | null {
  const trimmed = path.trim();
  if (trimmed === "") return required ? t("token.pathMustStartWithDollar") : null;
  return trimmed.startsWith("$") ? null : t("token.pathMustStartWithDollar");
}
