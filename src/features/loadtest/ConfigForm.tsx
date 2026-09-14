import { ChevronDown, RotateCcw } from "lucide-react";
import { useEffect, useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { HttpVersion } from "@/lib/types";
import { IconButton } from "@/features/ui/Button";
import { DEFAULT_LOADTEST_CONFIG, useLoadtestStore } from "@/store/loadtest";
import { FIELD_INPUT_CLASS, Field } from "./primitives";

/**
 * 上限。与 Rust `LoadtestConfig::validate()` 一一对应 ——
 * 这里挡在前面只是为了「别让用户敲进去一个注定被后端拒掉的数」，
 * 真正的把关仍然在后端（前端校验永远只是体验）。
 */
const LIMITS = {
  concurrency: { min: 1, max: 1_000 },
  total: { min: 1, max: 500_000 },
  timeoutMs: { min: 1, max: 600_000 },
  thinkTimeMs: { min: 0, max: 60_000 },
  warmupRequests: { min: 0, max: 10_000 },
} as const;

const HTTP_VERSIONS: readonly HttpVersion[] = ["auto", "http1", "http2"];

const HTTP_VERSION_LABEL = {
  auto: "loadtest.httpVersionAuto",
  http1: "loadtest.httpVersionHttp1",
  http2: "loadtest.httpVersionHttp2",
} as const;

/**
 * 数字输入框。
 *
 * 为什么不用「onChange 直接 clamp」（`SettingsForm` 里那种）：
 * 那个写法让「清空重输」变成不可能 —— 删到空会立刻被塞回最小值，
 * 于是想从 10 改成 500 必须全选覆盖，不能慢慢删。压测数字动辄四五位，
 * 手感差别很大。
 *
 * 这里的做法：输入期间保留用户敲的**原文本**（哪怕暂时不是合法数字），
 * 只在能解析时把 clamp 后的值提交上去；失焦时把文本对齐回真实值。
 * 于是「500000」这种数字可以一位一位删着改，而 store 里永远只有合法值。
 */
function NumberField({
  value,
  limit,
  onCommit,
  label,
  className,
}: {
  value: number;
  limit: { min: number; max: number };
  onCommit: (next: number) => void;
  label: string;
  className?: string;
}) {
  const [text, setText] = useState(String(value));
  // 是否正在输入。用它决定「外部改了值要不要覆盖输入框里的文本」
  const [typing, setTyping] = useState(false);

  // 「重置」按钮或切换历史轮次会从外部改值；没在输入时得跟上，
  // 在输入时就别抢 —— 那会把用户正敲的字符吃掉
  useEffect(() => {
    if (!typing) setText(String(value));
  }, [value, typing]);

  return (
    <input
      type="number"
      inputMode="numeric"
      aria-label={label}
      value={text}
      min={limit.min}
      max={limit.max}
      onFocus={() => setTyping(true)}
      onChange={(e) => {
        setText(e.target.value);
        const parsed = Number(e.target.value);
        if (e.target.value.trim() === "" || !Number.isFinite(parsed)) return;
        onCommit(Math.min(limit.max, Math.max(limit.min, Math.trunc(parsed))));
      }}
      onBlur={() => {
        setTyping(false);
        // 失焦时把文本对齐到真实值：输入了「3」而后端值是 1 的话，
        // 输入框里留着 3 就是在骗人
        setText(String(value));
      }}
      className={cn(FIELD_INPUT_CLASS, "tabular", className)}
    />
  );
}

function Checkbox({
  checked,
  onChange,
  label,
  hint,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  hint?: string;
}) {
  return (
    <label className="flex items-center gap-2" title={hint}>
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="h-3.5 w-3.5 cursor-pointer accent-[var(--brand)]"
      />
      <span className="text-[11px] text-ink">{label}</span>
    </label>
  );
}

/** 把配置按「常用 / 高级」两段摆开。advanced 默认收起 —— 那两项多数轮次用不到 */
export function ConfigForm({ disabled }: { disabled: boolean }) {
  const { t } = useI18n();
  const config = useLoadtestStore((s) => s.config);
  const patch = useLoadtestStore((s) => s.patchConfig);
  const reset = useLoadtestStore((s) => s.resetConfig);

  const [advanced, setAdvanced] = useState(false);

  // 「估算总量」提示：并发 × 请求间隔 决定了一轮至少多久
  const minSeconds =
    config.concurrency > 0
      ? Math.ceil((config.total * config.thinkTimeMs) / config.concurrency / 1000)
      : 0;

  return (
    <div className="flex flex-col gap-3">
      <div className="grid grid-cols-2 gap-x-4 gap-y-2.5">
        <Field label={t("loadtest.concurrency")} hint={t("loadtest.concurrencyHint")}>
          <NumberField
            label={t("loadtest.concurrency")}
            value={config.concurrency}
            limit={LIMITS.concurrency}
            onCommit={(concurrency) => patch({ concurrency })}
            className="w-20"
          />
        </Field>

        <Field label={t("loadtest.requests")} hint={t("loadtest.requestsHint")}>
          <NumberField
            label={t("loadtest.requests")}
            value={config.total}
            limit={LIMITS.total}
            onCommit={(total) => patch({ total })}
            className="w-24"
          />
        </Field>

        <Field label={t("loadtest.timeout")}>
          <NumberField
            label={t("loadtest.timeout")}
            value={config.timeoutMs}
            limit={LIMITS.timeoutMs}
            onCommit={(timeoutMs) => patch({ timeoutMs })}
            className="w-24"
          />
        </Field>

        <Field label={t("loadtest.thinkTime")} hint={t("loadtest.thinkTimeHint")}>
          <NumberField
            label={t("loadtest.thinkTime")}
            value={config.thinkTimeMs}
            limit={LIMITS.thinkTimeMs}
            onCommit={(thinkTimeMs) => patch({ thinkTimeMs })}
            className="w-20"
          />
        </Field>
      </div>

      <div className="flex flex-wrap items-center gap-x-5 gap-y-2">
        <Checkbox
          label={t("loadtest.followRedirects")}
          checked={config.followRedirects}
          onChange={(followRedirects) => patch({ followRedirects })}
        />
        <Checkbox
          label={t("loadtest.keepAlive")}
          hint={t("loadtest.keepAliveHint")}
          checked={config.keepAlive}
          onChange={(keepAlive) => patch({ keepAlive })}
        />

        <button
          type="button"
          onClick={() => setAdvanced((v) => !v)}
          aria-expanded={advanced}
          className="ml-auto flex items-center gap-1 text-[11px] text-ink-faint transition-colors duration-100 hover:text-ink-muted"
        >
          <ChevronDown
            size={12}
            strokeWidth={2.2}
            aria-hidden
            className={cn("transition-transform duration-150", advanced && "rotate-180")}
          />
          {t("loadtest.openAdvanced")}
        </button>
      </div>

      {advanced ? (
        <div className="grid grid-cols-2 gap-x-4 gap-y-2.5 rounded-sm border border-line bg-canvas-subtle px-2.5 py-2.5">
          <Field label={t("loadtest.httpVersion")}>
            <select
              aria-label={t("loadtest.httpVersion")}
              value={config.httpVersion}
              onChange={(e) => patch({ httpVersion: e.target.value as HttpVersion })}
              className={cn(FIELD_INPUT_CLASS, "w-28 font-sans")}
            >
              {HTTP_VERSIONS.map((v) => (
                <option key={v} value={v}>
                  {t(HTTP_VERSION_LABEL[v])}
                </option>
              ))}
            </select>
          </Field>

          <Field label={t("loadtest.warmup")} hint={t("loadtest.warmupHint")}>
            <NumberField
              label={t("loadtest.warmup")}
              value={config.warmupRequests}
              limit={LIMITS.warmupRequests}
              onCommit={(warmupRequests) => patch({ warmupRequests })}
              className="w-20"
            />
          </Field>
        </div>
      ) : null}

      {/* 底部一行：估算 + 重置。
          估算只在配了请求间隔时才有意义（否则耗时完全取决于服务端） */}
      <div className="flex items-center gap-2">
        {minSeconds > 0 ? (
          <span className="text-[10px] text-ink-faint">
            {t("loadtest.minDuration", { value: minSeconds })}
          </span>
        ) : null}
        <IconButton
          size="sm"
          label={t("loadtest.resetConfig")}
          disabled={disabled}
          onClick={reset}
          className="ml-auto"
        >
          <RotateCcw size={12} strokeWidth={2.2} />
        </IconButton>
      </div>

      {/* 默认值就在旁边说清楚，免得「重置」被当成清空 */}
      <p className="text-[10px] text-ink-faint/80">
        {t("loadtest.defaultHint", {
          concurrency: DEFAULT_LOADTEST_CONFIG.concurrency,
          total: DEFAULT_LOADTEST_CONFIG.total,
        })}
      </p>
    </div>
  );
}
