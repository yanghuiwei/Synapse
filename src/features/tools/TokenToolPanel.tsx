import { Eraser, ScanSearch, TriangleAlert, X } from "lucide-react";
import { useMemo, useState } from "react";

import { cn } from "@/lib/cn";
import { formatClock, formatSeconds } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import { describeClaims, parseToken, type ClaimRow, type ParsedToken } from "@/lib/tokenTool";
import { useUiStore } from "@/store/ui";
import { Button, IconButton } from "@/features/ui/Button";

/**
 * Token 解析（右侧常驻栏）。
 *
 * ## 为什么是栏而不是弹窗
 *
 * 解析 token 从来不是一个孤立的动作：拿到一个值 → 想看它写了什么 →
 * 回到请求头 / 响应里核对。弹窗把这条路断了三件事：遮罩挡住后面的内容、
 * 关掉就丢（模板是局部 state）、也没法一边摆着一边滚下面的响应。
 * 常驻栏让「看 token」和「看请求」同时成立。
 *
 * ## 为什么宽度是硬的
 *
 * 表格三列（字段 / 含义 / 值），窄于 300px 时「值」那一列会被压到看不出内容
 * —— 与 `EnvManager` 不把变量表塞进侧栏是同一个判断。
 * 所以给一个 `min-w`，宁可让用户去拖分隔条调宽，也不让它在窄宽下退化。
 *
 * ## ⚠️ 只解码，不验签
 *
 * 这个面板回答的是「这段文本自己写着什么」，不是「这个 token 能不能用」。
 * 签名可能无效、`alg` 可能是 `none`、`exp` 可能是伪造的 —— 判据在服务端。
 * 所以界面上不出现「有效 / 安全」这类结论性的字眼。
 */
export function TokenToolPanel({ className }: { className?: string }) {
  const { t } = useI18n();
  const setOpen = useUiStore((s) => s.setTokenToolOpen);
  const [input, setInput] = useState("");
  const [submitted, setSubmitted] = useState("");

  /** 解析只在提交后发生一次。`undefined` = 还没解析过，`null` = 认不出来 */
  const parsed = useMemo(
    () => (submitted.trim() === "" ? undefined : parseToken(submitted)),
    [submitted],
  );

  return (
    <section
      className={cn(
        // 铺满主视图时不再画左边框（左边没有东西可分隔），
        // 也不再设 min-w —— 主视图的宽度由布局给，不该被这里顶出横向滚动
        "flex h-full min-h-0 flex-col bg-canvas-subtle",
        className,
      )}
    >
      <header className="flex h-9 shrink-0 items-center gap-2 border-b border-line pl-2.5 pr-1.5">
        <h2 className="min-w-0 flex-1 truncate text-[13px] font-semibold tracking-tight text-ink">
          {t("tools.token")}
        </h2>
        <IconButton
          size="sm"
          label={t("tools.close")}
          onClick={() => setOpen(false)}
        >
          <X size={14} strokeWidth={2} />
        </IconButton>
      </header>

      <div className="flex min-h-0 flex-1 flex-col gap-2.5 overflow-auto p-2.5">
        <p className="text-[11px] leading-relaxed text-ink-faint">{t("tools.tokenHint")}</p>

        <div className="space-y-1.5">
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              // Ctrl+Enter 解析。单行输入框用回车提交，这里粘进来的是多行，
              // 回车得留给换行
              if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                e.preventDefault();
                setSubmitted(input);
              }
            }}
            placeholder={t("tools.inputPlaceholder")}
            spellCheck={false}
            autoComplete="off"
            aria-label={t("tools.inputLabel")}
            rows={3}
            className={cn(
              "w-full resize-y rounded-sm border border-line bg-surface px-2 py-1.5",
              "font-mono text-[11px] leading-relaxed text-ink placeholder:font-sans placeholder:text-ink-faint",
              "outline-none transition-colors duration-100",
              "hover:border-line-strong focus:border-brand",
            )}
          />
          <div className="flex items-center gap-2">
            <Button
              size="sm"
              variant="primary"
              icon={<ScanSearch size={13} strokeWidth={2.2} />}
              disabled={input.trim() === ""}
              onClick={() => setSubmitted(input)}
            >
              {t("tools.decode")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              icon={<Eraser size={13} strokeWidth={2} />}
              disabled={input === ""}
              onClick={() => {
                setInput("");
                // 结果也要跟着清掉：只清输入框会留下上一次的解析结果，
                // 看着像是「空输入却解析出了东西」
                setSubmitted("");
              }}
            >
              {t("tools.clear")}
            </Button>
          </div>
        </div>

        {parsed === undefined ? (
          <p className="py-6 text-center text-[11px] text-ink-faint">{t("tools.tokenEmpty")}</p>
        ) : parsed === null ? (
          <NotAToken />
        ) : (
          <Result parsed={parsed} />
        )}
      </div>

      <footer className="shrink-0 border-t border-line px-2.5 py-1.5 text-[11px] leading-relaxed text-ink-faint">
        {t("tools.noVerify")}
      </footer>
    </section>
  );
}

/**
 * 认不出来。**不说「这不是 JWT」** —— 它可能是别的形状的凭据
 * （opaque token、签名串），说死会误导。只说这里认没认出来。
 */
function NotAToken() {
  const { t } = useI18n();
  return (
    <div className="flex items-start gap-2 rounded-sm border border-warn/35 bg-warn-subtle px-2 py-1.5 text-[11px] text-warn-on">
      <TriangleAlert size={13} strokeWidth={2.2} aria-hidden className="mt-px shrink-0" />
      <div className="space-y-0.5">
        <p className="font-medium">{t("tools.notAToken")}</p>
        <p className="leading-relaxed text-warn/85">{t("tools.notATokenHint")}</p>
      </div>
    </div>
  );
}

function Result({ parsed }: { parsed: ParsedToken }) {
  const { t } = useI18n();
  const headerRows = useMemo(
    () => (parsed.header.json ? describeClaims(parsed.header.json) : []),
    [parsed],
  );
  const payloadRows = useMemo(
    () => (parsed.payload.json ? describeClaims(parsed.payload.json) : []),
    [parsed],
  );

  return (
    <div className="space-y-3">
      <Section title={t("tools.header")}>
        <ClaimList rows={headerRows} />
      </Section>

      <Section title={t("tools.payload")}>
        <ClaimList rows={payloadRows} />
      </Section>

      <Section title={t("tools.signature")}>
        <div className="space-y-1">
          {parsed.signature === "" ? (
            <p className="text-[11px] text-warn">{t("tools.signatureEmpty")}</p>
          ) : (
            <p className="flex flex-wrap items-baseline gap-x-2 text-[11px] text-ink-muted">
              <span className="tabular">
                {t("tools.signatureLength", { count: parsed.signature.length })}
              </span>
              <span className="text-ink-faint">{t("tools.signatureNote")}</span>
            </p>
          )}
        </div>
      </Section>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="space-y-1.5">
      <h3 className="text-xs font-semibold tracking-tight text-ink">{title}</h3>
      {children}
    </section>
  );
}

/**
 * claim 列表。
 *
 * ## 为什么不是表格
 *
 * 弹窗那版用的是三列表格，因为 760px 摆得下。搬到 300~460px 的窄栏之后
 * 「字段 / 含义 / 值」三列会把「值」压成一条 —— 而值恰恰是唯一必须读全的东西。
 * 换成上下两层：第一行「键名 + 中文说明」（都是短词，一行放得下），
 * 第二行是值（独占整行宽度）。同一个信息的两种排法，窄的时候只有这一种读得下去。
 *
 * 键名与说明之间用等宽字体和着色区分：`sub 主体` 这样两段挨着，
 * 不给点视觉差别会读成一个词。
 */
function ClaimList({ rows }: { rows: ClaimRow[] }) {
  const { t } = useI18n();
  if (rows.length === 0) {
    return <p className="text-[11px] text-ink-faint">{t("common.empty")}</p>;
  }

  return (
    <ul className="space-y-1.5">
      {rows.map((row) => (
        <li key={row.key} className="rounded-sm border border-line bg-surface px-2 py-1.5">
          <div className="flex flex-wrap items-baseline gap-x-2">
            <span className="font-mono text-xs break-all text-syn-key">{row.key}</span>
            {row.label ? (
              <span className="text-[11px] text-ink-faint">{row.label}</span>
            ) : null}
          </div>
          {row.block ? (
            <pre className="mt-0.5 max-h-32 overflow-auto whitespace-pre-wrap font-mono text-[11px] leading-relaxed break-all text-ink-muted">
              {row.value}
            </pre>
          ) : (
            <ClaimValue row={row} />
          )}
        </li>
      ))}
    </ul>
  );
}

/**
 * 一个 claim 的值。时间类的额外渲染出本地时刻与相对量。
 *
 * ⚠️ 三种状态，不是两种：**还没生效**与**已过期**都会让「这个 token 现在没用」，
 * 但它们是不同的事，文案不能混用。第一版只分了「过去 / 未来」两支，
 * 于是 `nbf` 落在未来时显示成「642302 小时 14 分后过期」，还带着表示正常的灰字 ——
 * 一眼看过去像是「这个 token 很久以后才过期」，
 * 而事实正好相反：它现在还没生效。这种错比没有这行字危险。
 */
function ClaimValue({ row }: { row: ClaimRow }) {
  const { t } = useI18n();
  if (row.at === null) {
    return <p className="mt-0.5 text-xs leading-relaxed break-all text-ink">{row.value}</p>;
  }

  const at = new Date(row.at);
  const seconds = Math.trunc((at.getTime() - Date.now()) / 1000);

  /** 生效时刻（`nbf`）在未来 → 它还没生效 */
  const pending = row.key === "nbf" && seconds > 0;
  /**
   * 过期时刻（`exp`）已经过去 → 它过期了。
   * 时钟偏差留 2 秒的余量：`exp` 常与签发同秒，把「正好现在」显示成「已过期」是虚报。
   */
  const expired = row.key === "exp" && seconds < -2;
  /** 其余的过去时刻（`iat` 等）只是「发生过」，不是问题，照实说 */
  const past = !pending && !expired && seconds < 0;

  const tone = pending || expired ? "text-warn" : "text-ink-faint";
  const label = pending
    ? t("tools.notYetValid")
    : expired
      ? t("tools.expired")
      : past
        ? t("token.expiredAgo", { time: formatSeconds(seconds) })
        : t("token.expiresIn", { time: formatSeconds(seconds) });

  return (
    <p className="mt-0.5 flex flex-wrap items-baseline gap-x-2">
      <span className="tabular text-xs text-ink">{formatClock(row.at)}</span>
      <span className={cn("text-[11px]", tone)}>{label}</span>
    </p>
  );
}
