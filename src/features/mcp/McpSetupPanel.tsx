import { Check, Copy, LoaderCircle, TriangleAlert } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import { Button } from "@/features/ui/Button";
import { Dialog } from "@/features/ui/Dialog";
import { EmptyState } from "@/features/ui/EmptyState";
import { useI18n } from "@/lib/i18n";
import * as ipc from "@/lib/ipc";
import type { McpSetup } from "@/lib/types";

/**
 * 「连接 AI 客户端」面板（plan.md P7-7）。
 *
 * 只做一件事：把后端拼好的两段配置原样展示出来，让用户复制走。
 * 片段**在前端拼过一次**是刻意的 —— 路径转义（反斜杠、空格）是格式相关的知识，
 * 后端 `mcp::config_gen` 已经有一份带测试的实现，前端再拼就是第二份，迟早分叉。
 */

type LoadState =
  | { kind: "loading" }
  | { kind: "ready"; setup: McpSetup }
  | { kind: "failed"; message: string };

/**
 * 一段可复制的配置片段。
 *
 * 复制按钮的「已复制」只留 1.6 秒：反馈一直挂着会让人以为还能再点一次，
 * 而那时点下去没有任何变化。
 */
function Snippet({ label, text }: { label: string; text: string }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const copy = () => {
    // `?.` 之后不能直接 `.then` —— clipboard 不可用时整个表达式是 undefined，
    // 接着调 `.then` 会抛 TypeError，而这个按钮本来不该有失败态。
    const pending = navigator.clipboard?.writeText(text);
    if (!pending) return;
    void pending.then(
      () => setCopied(true),
      () => setCopied(false),
    );
  };

  return (
    <section className="space-y-1.5">
      <header className="flex items-center gap-2">
        <h3 className="flex-1 text-xs font-semibold tracking-tight text-ink">{label}</h3>
        <Button
          size="sm"
          variant="secondary"
          icon={
            copied ? (
              <Check size={13} strokeWidth={2.2} />
            ) : (
              <Copy size={13} strokeWidth={2} />
            )
          }
          onClick={copy}
        >
          {copied ? t("mcp.copied") : t("mcp.copy")}
        </Button>
      </header>
      <pre className="max-h-56 overflow-auto rounded-sm border border-line bg-canvas-subtle p-2.5 font-mono text-[11px] leading-relaxed text-ink">
        {text}
      </pre>
    </section>
  );
}

function PathRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline gap-2 text-[11px]">
      <span className="w-24 shrink-0 text-ink-faint">{label}</span>
      <span className="min-w-0 flex-1 break-all font-mono text-ink-muted">{value}</span>
    </div>
  );
}

export function McpSetupPanel({ onClose }: { onClose: () => void }) {
  const { t } = useI18n();
  const [state, setState] = useState<LoadState>({ kind: "loading" });

  const load = useCallback(() => {
    setState({ kind: "loading" });
    ipc.mcp
      .setup()
      .then((setup) => setState({ kind: "ready", setup }))
      .catch((error: unknown) => {
        // 契约：后端只给错误码，文案由 `IpcError.localized` 在前端查表得出
        const message =
          error instanceof ipc.IpcError ? error.localized : String(error);
        setState({ kind: "failed", message });
      });
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  return (
    <Dialog
      title={t("mcp.title")}
      description={t("mcp.subtitle")}
      onClose={onClose}
      width={620}
      footer={
        <Button size="sm" variant="secondary" data-autofocus onClick={onClose}>
          {t("common.close")}
        </Button>
      }
    >
      {state.kind === "loading" ? (
        <div className="flex items-center justify-center gap-2 py-10 text-xs text-ink-faint">
          <LoaderCircle size={13} strokeWidth={2.2} className="animate-spin" />
          <span>{t("mcp.loading")}</span>
        </div>
      ) : state.kind === "failed" ? (
        <EmptyState
          icon={TriangleAlert}
          title={t("mcp.loadFailed")}
          description={state.message}
          action={
            <Button size="sm" variant="secondary" onClick={load}>
              {t("common.retry")}
            </Button>
          }
          className="py-6"
        />
      ) : (
        <div className="space-y-4">
          {/* 配置片段语法正确但 exe 不存在，是这一块唯一会让人查半天的坑 */}
          {state.setup.commandExists ? null : (
            <div className="flex items-start gap-2 rounded-sm border border-warn bg-canvas-subtle p-2.5">
              <TriangleAlert
                size={13}
                strokeWidth={2}
                aria-hidden
                className="mt-0.5 shrink-0 text-warn"
              />
              <p className="text-[11px] leading-relaxed text-ink-muted">
                {t("mcp.missingBin")}
              </p>
            </div>
          )}

          <Snippet label={t("mcp.jsonLabel")} text={state.setup.jsonSnippet} />
          <Snippet label={t("mcp.yamlLabel")} text={state.setup.yamlSnippet} />

          <section className="space-y-1.5">
            <h3 className="text-xs font-semibold tracking-tight text-ink">
              {t("mcp.pathsTitle")}
            </h3>
            <PathRow label={t("mcp.commandPath")} value={state.setup.commandPath} />
            <PathRow label={t("mcp.dbPath")} value={state.setup.dbPath} />
          </section>

          <section className="space-y-1.5">
            <h3 className="text-xs font-semibold tracking-tight text-ink">
              {t("mcp.transportTitle")}
            </h3>
            <ul className="space-y-1 text-[11px] leading-relaxed text-ink-muted">
              <li>{t("mcp.stdio")}</li>
              <li>{t("mcp.http")}</li>
              <li>{t("mcp.readonly")}</li>
            </ul>
          </section>

          <section className="space-y-1.5">
            <h3 className="text-xs font-semibold tracking-tight text-ink">
              {t("mcp.toolsTitle")}
            </h3>
            <p className="text-[11px] leading-relaxed text-ink-muted">{t("mcp.tools")}</p>
          </section>
        </div>
      )}
    </Dialog>
  );
}
