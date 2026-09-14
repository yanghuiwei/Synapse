import { Check, X } from "lucide-react";

import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import { Section } from "./primitives";

/**
 * 能力边界（`design.md` §7.5 要求**在界面上**明说，不能只写在文档里）。
 *
 * 为什么这块值得占屏幕：压测是最容易被拿去下结论的功能 ——
 * 「P95 200ms，接口没问题」这句话如果是在一台 4 核笔记本上、用固定并发
 * 打出来的，它和用分布式压测机跑出来的结论完全是两回事。
 * 把做不到的事写清楚，比多画一张图有用。
 */

const CAN: readonly MessageKey[] = [
  "loadtest.can1",
  "loadtest.can2",
  "loadtest.can3",
  "loadtest.can4",
];

const CANNOT: readonly MessageKey[] = [
  "loadtest.cannot1",
  "loadtest.cannot2",
  "loadtest.cannot3",
  "loadtest.cannot4",
];

export function ScopeNote() {
  const { t } = useI18n();

  return (
    <Section title={t("loadtest.scope")}>
      <div className="grid gap-3 rounded-sm border border-line bg-canvas-subtle px-3 py-2.5 sm:grid-cols-2">
        <ul className="flex flex-col gap-1.5">
          <li className="text-[10px] font-medium tracking-wide text-ok">{t("loadtest.canDo")}</li>
          {CAN.map((key) => (
            <li key={key} className="flex items-start gap-1.5 text-[11px] leading-snug text-ink-muted">
              <Check size={11} strokeWidth={2.6} aria-hidden className="mt-[3px] shrink-0 text-ok" />
              {t(key)}
            </li>
          ))}
        </ul>

        <ul className="flex flex-col gap-1.5">
          <li className="text-[10px] font-medium tracking-wide text-ink-faint">
            {t("loadtest.cannotDo")}
          </li>
          {CANNOT.map((key) => (
            <li key={key} className="flex items-start gap-1.5 text-[11px] leading-snug text-ink-faint">
              <X size={11} strokeWidth={2.6} aria-hidden className="mt-[3px] shrink-0" />
              {t(key)}
            </li>
          ))}
        </ul>
      </div>

      <p className="text-[10px] leading-relaxed text-ink-faint">{t("loadtest.scopeNote")}</p>
    </Section>
  );
}
