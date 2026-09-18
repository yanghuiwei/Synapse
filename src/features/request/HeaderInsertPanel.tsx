import { ChevronDown, Plus, TriangleAlert, X } from "lucide-react";
import { useMemo, useState } from "react";

import { cn } from "@/lib/cn";
import { headerNameChoices } from "@/lib/headerNames";
import { useI18n } from "@/lib/i18n";
import type { KeyValueItem } from "@/lib/types";
import { useWorkspaceStore } from "@/store/workspace";
import { Button } from "@/features/ui/Button";

/**
 * 「值从哪来」—— 面板的第二段。
 *
 * 这一层刻意**不是**「授权类型」。它不落库、不参与发送语义，只决定
 * 「点插入时往值那一格写什么字符串」。插完这一行就是一条普通请求头，
 * 和用户手打的一模一样 —— 这是整个重构的核心：
 * 类型一旦落库就会和头名打架（`type: "apiKey"` + `key: "X-Auth-Token"`），
 * 而一条字符串没有第二种解释。
 */
type ValueShape = "tokenSource" | "paste" | "basic" | "env" | "empty";

const SHAPE_ORDER: readonly ValueShape[] = ["tokenSource", "paste", "basic", "env", "empty"];

const SHAPE_LABEL = {
  tokenSource: "headers.insert.shapeTokenSource",
  paste: "headers.insert.shapePaste",
  basic: "headers.insert.shapeBasic",
  env: "headers.insert.shapeEnv",
  empty: "headers.insert.shapeEmpty",
} as const;

/** 自定义头名在下拉里的哨兵值 */
const CUSTOM = "\u0000custom";

export interface HeaderInsertPanelProps {
  /** 当前请求已有的头行 —— 只用来判「这次会不会覆盖」 */
  rows: readonly KeyValueItem[];
  /** 有 Token 源时填 `{{token.access}}`；`null` = 工作区里还没有 Token 源 */
  tokenRef: string | null;
  onInsert: (row: KeyValueItem) => void;
}

/**
 * 快捷插入条 / 面板（设计 §3.2）。
 *
 * ## 为什么头名按「你用过的次数」排，而不是按 RFC
 *
 * 第一版设计这里是硬编码的 `Authorization` / `X-Api-Key` 三件套 —— 那是按
 * RFC 想的。查真库之后发现实际在用的鉴权头是 `X-Access-Token`（15 处，
 * 值是**裸 token** 不带前缀），三个 RFC 按钮一条都覆盖不到。
 * 所以候选头名从工作区的 headers 里数出来（见 `lib/headerNames.ts`），
 * 下拉底部永远留「自定义…」。
 *
 * ## 前缀为什么是自动判断的
 *
 * `Authorization: Bearer xxx` 里的 `Bearer ` 是**值的一部分**（它不是一个
 * 独立字段）。让用户每次自己敲这七个字符，就是每次给一次拼错的机会 ——
 * 而拼错的表现是 401，看不出原因。所以插入时按「头名 + 值形态」替他补上，
 * 并且**补完仍然可改**（它是普通文本）。
 */
export function HeaderInsertPanel({ rows, tokenRef, onInsert }: HeaderInsertPanelProps) {
  const { t } = useI18n();
  const requests = useWorkspaceStore((s) => s.requests);

  const [open, setOpen] = useState(false);
  const [name, setName] = useState<string>("");
  const [customName, setCustomName] = useState("");
  const [shape, setShape] = useState<ValueShape>("tokenSource");
  const [pasted, setPasted] = useState("");
  const [envKey, setEnvKey] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  /** `null` = 还没被用户改过，跟着头名与形态自动走 */
  const [prefixOverride, setPrefixOverride] = useState<boolean | null>(null);

  const choices = useMemo(() => headerNameChoices(requests), [requests]);

  // 第一次展开时预选「用过的第一个」—— 绝大多数时候它就是用户想要的那个
  const headerName = name === CUSTOM ? customName.trim() : name === "" ? (choices[0]?.name ?? "") : name;

  const defaultPrefix = prefixFor(headerName, shape);
  const prefix = prefixOverride ?? defaultPrefix !== "";
  const prefixText = prefix ? "Bearer " : "";

  /**
   * `{{$basic …}}` 展开后**自带** `Basic ` 前缀（Rust 的 `functions::basic`）。
   * 再补一个就成了 `Basic Basic xxx` —— 那是个必然 401 的值，而且两个词
   * 都是白的，界面上看不出来。
   *
   * ⚠️ 设计文档 §3.2 的对照表里写着「Authorization + `{{$basic …}}` → 补
   * Basic」，与同一份文档 §4.2 的示例写法（`Authorization: {{$basic "u" "p"}}`，
   * 没有额外前缀）冲突。以 §4.2 与实现为准 —— 函数自己会写。
   */
  const prefixLocked = shape === "basic";

  const willOverwrite = useMemo(() => {
    const trimmed = headerName.trim().toLowerCase();
    return trimmed !== "" && rows.some((row) => row.key.trim().toLowerCase() === trimmed);
  }, [rows, headerName]);

  const canInsert = headerName.trim() !== "";

  const reset = () => {
    setOpen(false);
    setPasted("");
    setEnvKey("");
    setUsername("");
    setPassword("");
    setCustomName("");
    setPrefixOverride(null);
    setShape("tokenSource");
  };

  const submit = () => {
    if (!canInsert) return;
    onInsert({ key: headerName.trim(), value: composeValue(), enabled: true });
    reset();
  };

  /** 把「头名 + 值形态 + 前缀」拼成那一行最终写进去的字符串 */
  function composeValue(): string {
    switch (shape) {
      case "tokenSource":
        // 没有 Token 源时仍然写占位符：它会被标成「未解析」并原样发出去，
        // 而那是**看得见**的失败（发送后有一行提示），比写个空串好
        return prefixText + (tokenRef ?? "{{token.access}}");
      case "paste":
        return prefixText + stripBearer(pasted.trim());
      case "basic":
        return `{{$basic ${quoteOrPlaceholder(username, t("headers.insert.username"))} ${quoteOrPlaceholder(password, t("headers.insert.password"))}}}`;
      case "env":
        return prefixText + `{{env.${envKey.trim()}}}`;
      default:
        return "";
    }
  }

  if (!open) {
    return (
      <div className="flex min-w-0 items-center gap-2">
        <Button
          size="sm"
          variant="secondary"
          icon={<Plus size={12} strokeWidth={2.4} />}
          onClick={() => setOpen(true)}
        >
          {t("headers.insert.open")}
        </Button>
        <span className="truncate text-[11px] text-ink-faint">{t("headers.insert.hint")}</span>
      </div>
    );
  }

  return (
    <div className="w-full space-y-2 rounded-md border border-line bg-surface p-2">
      <div className="flex items-center gap-1.5">
        <span className="text-[11px] font-medium text-ink-muted">{t("headers.insert.title")}</span>
        <span className="flex-1" />
        <Button size="sm" variant="ghost" icon={<X size={12} strokeWidth={2.2} />} onClick={reset}>
          {t("common.cancel")}
        </Button>
        <Button size="sm" variant="primary" disabled={!canInsert} onClick={submit}>
          {t(willOverwrite ? "headers.insert.overwrite" : "headers.insert.insert")}
        </Button>
      </div>

      {/* 第一段：头名 */}
      <div className="flex items-center gap-2">
        <span className="w-[64px] shrink-0 text-xs text-ink-muted">{t("headers.insert.name")}</span>
        <div className="relative min-w-0 flex-1">
          <select
            aria-label={t("headers.insert.name")}
            value={name === "" ? (choices[0]?.name ?? CUSTOM) : name}
            onChange={(e) => setName(e.target.value)}
            className={cn(
              "h-7 w-full appearance-none rounded-sm border border-line bg-surface",
              "pl-2 pr-7 font-mono text-xs text-ink outline-none transition-colors duration-100",
              "hover:border-line-strong focus:border-brand",
            )}
          >
            {/* 用过的在前（次数是提示，不是语义），内置兜底在后，自定义永远垫底 */}
            {choices.map((choice) => (
              <option key={choice.name} value={choice.name}>
                {choice.count > 0 ? `${choice.name}  ·  ${choice.count}` : choice.name}
              </option>
            ))}
            <option value={CUSTOM}>{t("headers.insert.nameCustom")}</option>
          </select>
          <ChevronDown
            size={13}
            strokeWidth={2.4}
            aria-hidden
            className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 text-ink-faint"
          />
        </div>

        {name === CUSTOM ? (
          <input
            value={customName}
            onChange={(e) => setCustomName(e.target.value)}
            placeholder={t("headers.insert.nameCustomPlaceholder")}
            spellCheck={false}
            autoComplete="off"
            aria-label={t("headers.insert.nameCustom")}
            className={cn(
              "h-7 w-[180px] shrink-0 rounded-sm border border-line bg-surface px-2",
              "font-mono text-xs text-ink placeholder:font-sans placeholder:text-ink-faint",
              "outline-none transition-colors duration-100 hover:border-line-strong focus:border-brand",
            )}
          />
        ) : null}
      </div>

      {/* 第二段：值的形态 */}
      <div className="flex items-start gap-2">
        <span className="w-[64px] shrink-0 pt-0.5 text-xs text-ink-muted">
          {t("headers.insert.shape")}
        </span>
        <div className="min-w-0 flex-1 space-y-1">
          {SHAPE_ORDER.map((option) => (
            <label key={option} className="flex cursor-pointer items-center gap-1.5 text-xs">
              <input
                type="radio"
                name="reqly-insert-shape"
                checked={shape === option}
                onChange={() => {
                  setShape(option);
                  setPrefixOverride(null);
                }}
                className="h-3.5 w-3.5 cursor-pointer accent-[var(--brand)]"
              />
              <span className={shape === option ? "text-ink" : "text-ink-muted"}>
                {t(SHAPE_LABEL[option])}
              </span>
              {option === "tokenSource" ? (
                <code className="font-mono text-[11px] text-brand">{tokenRef ?? "{{token.access}}"}</code>
              ) : null}
            </label>
          ))}
        </div>
      </div>

      {/* 形态各自的输入 */}
      {shape === "paste" ? (
        <PlainInput
          value={pasted}
          onChange={setPasted}
          placeholder={t("headers.insert.pastePlaceholder")}
          label={t("headers.insert.shapePaste")}
        />
      ) : null}

      {shape === "basic" ? (
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <span className="w-[64px] shrink-0 text-xs text-ink-muted">
              {t("headers.insert.username")}
            </span>
            <PlainInput
              value={username}
              onChange={setUsername}
              label={t("headers.insert.username")}
            />
          </div>
          <div className="flex items-center gap-2">
            <span className="w-[64px] shrink-0 text-xs text-ink-muted">
              {t("headers.insert.password")}
            </span>
            <PlainInput
              value={password}
              onChange={setPassword}
              label={t("headers.insert.password")}
            />
          </div>
        </div>
      ) : null}

      {shape === "env" ? (
        <PlainInput
          value={envKey}
          onChange={setEnvKey}
          placeholder={t("headers.insert.envPlaceholder")}
          label={t("headers.insert.shapeEnv")}
        />
      ) : null}

      {/* 前缀：自动判断，但永远可以改回去 */}
      {prefixLocked ? (
        <p className="pl-[72px] text-[11px] leading-relaxed text-ink-faint">
          {t("headers.insert.prefixLocked")}
        </p>
      ) : (
        <label className="flex items-center gap-1.5 pl-[72px] text-[11px] text-ink-muted">
          <input
            type="checkbox"
            checked={prefix}
            onChange={(e) => setPrefixOverride(e.target.checked)}
            className="h-3.5 w-3.5 cursor-pointer accent-[var(--brand)]"
          />
          {t("headers.insert.prefix", { prefix: "Bearer" })}
        </label>
      )}

      {shape === "basic" ? (
        <p className="text-[11px] leading-relaxed text-ink-faint">
          {t("headers.insert.basicHint")}
        </p>
      ) : null}

      {shape === "tokenSource" && tokenRef === null ? (
        <p className="flex items-start gap-1.5 text-[11px] leading-relaxed text-warn">
          <TriangleAlert size={12} strokeWidth={2.2} aria-hidden className="mt-0.5 shrink-0" />
          {t("headers.insert.tokenSourceMissing")}
        </p>
      ) : null}
    </div>
  );
}

function PlainInput({
  value,
  onChange,
  placeholder,
  label,
}: {
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  label: string;
}) {
  return (
    <input
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      spellCheck={false}
      autoComplete="off"
      aria-label={label}
      className={cn(
        "h-7 min-w-0 flex-1 rounded-sm border border-line bg-surface px-2",
        "font-mono text-xs text-ink placeholder:font-sans placeholder:text-ink-faint",
        "outline-none transition-colors duration-100 hover:border-line-strong focus:border-brand",
      )}
    />
  );
}

/**
 * 前缀的默认判断 —— 就是设计 §3.2 那张对照表。
 *
 * 判据只有「头名 + 值形态」两项，**不看值的具体内容**：
 * 设计原文里那句「仅当值不是裸 JWT 时不加」是关于 `X-Access-Token` 的，
 * 而它已经由头名覆盖了（那个头名一律不加）。按值的内容去猜会引入
 * 一个看不出规律的行为 —— 用户改一个字符前缀就不见了，找不到原因。
 */
function prefixFor(headerName: string, shape: ValueShape): string {
  if (shape === "empty" || shape === "basic") return "";
  // 只有标准头名补 `Bearer `：真库里 X-Access-Token 的 15 处都是裸 token，
  // 给它补前缀会把每一处都改成 401
  return headerName.trim().toLowerCase() === "authorization" ? "Bearer " : "";
}

/** 从网站上复制的 token 常自带 `Bearer ` 前缀。这里剥掉，避免拼成两个 */
function stripBearer(value: string): string {
  return value.replace(/^Bearer\s+/i, "");
}

/**
 * Basic 的两个参数：填了就用字面量，没填留一个中文占位让人一眼看到要改。
 *
 * 占位符里**不能有引号**（`split_two_quoted` 只认最外层的两个引号），
 * 所以这里先把用户输入里的双引号换成单引号，而不是去转义 ——
 * 密码里带双引号时，转义在模板语法里没有对应的表示法。
 */
function quoteOrPlaceholder(value: string, fallback: string): string {
  const trimmed = value.trim();
  if (trimmed === "") return `"${fallback}"`;
  return `"${trimmed.replace(/"/g, "'")}"`;
}
