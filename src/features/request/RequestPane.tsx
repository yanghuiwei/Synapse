import { Ban, Braces, ChevronDown, ListTree, Lock, Send, Settings2, Terminal } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { cn } from "@/lib/cn";
import { HTTP_METHODS, methodTextClass } from "@/lib/http";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import type { KeyValueItem, RequestSettings } from "@/lib/types";
import { useRequestStore } from "@/store/request";
import { useUiStore } from "@/store/ui";
import { useWorkspaceStore } from "@/store/workspace";
import { Button } from "@/features/ui/Button";
import { EmptyState } from "@/features/ui/EmptyState";
import { TabStrip, type TabItem } from "@/features/ui/TabStrip";
import { BodyEditor } from "./BodyEditor";
import { KeyValueTable } from "./KeyValueTable";
import { UrlBar } from "./UrlBar";

type RequestTab = "params" | "headers" | "body" | "auth" | "scripts" | "settings";

const TAB_LABEL: Record<RequestTab, MessageKey> = {
  params: "request.tab.params",
  headers: "request.tab.headers",
  body: "request.tab.body",
  auth: "request.tab.auth",
  scripts: "request.tab.scripts",
  settings: "request.tab.settings",
};

const TAB_ICON: Record<RequestTab, typeof ListTree> = {
  params: ListTree,
  headers: ListTree,
  body: Braces,
  auth: Lock,
  scripts: Terminal,
  settings: Settings2,
};

const TAB_ORDER: readonly RequestTab[] = [
  "params",
  "headers",
  "body",
  "auth",
  "scripts",
  "settings",
];

/** 标签角标只数「真的会发出去」的行：勾选着的、并且有键名 */
function activeCount(rows: readonly KeyValueItem[]): number {
  return rows.filter((row) => row.enabled && row.key !== "").length;
}

/** 方法下拉。用原生 <select> 是刻意的：键盘可达性零成本 */
function MethodSelect({
  value,
  onChange,
}: {
  value: string;
  onChange: (next: string) => void;
}) {
  const { t } = useI18n();
  return (
    <div className="relative shrink-0">
      <select
        aria-label={t("request.method")}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className={cn(
          "h-8 w-[104px] appearance-none rounded-md border border-line bg-surface",
          "pl-2.5 pr-7 font-mono text-xs font-semibold tracking-tight",
          "transition-colors duration-100 hover:border-line-strong focus:border-brand",
          methodTextClass(value),
        )}
      >
        {HTTP_METHODS.map((m) => (
          <option key={m} value={m} className="font-mono text-ink">
            {m}
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

/** 逐请求的发送设置。放在标签栏里，而不是弹窗 —— 调超时的时候能直接看到请求内容 */
function SettingsForm() {
  const { t } = useI18n();
  const settings = useRequestStore((s) => s.draft.settings);
  const patch = useRequestStore((s) => s.patch);

  const setSettings = (next: Partial<RequestSettings>) =>
    patch({ settings: { ...settings, ...next } });

  const box = "h-3.5 w-3.5 cursor-pointer accent-[var(--brand)]";
  const field = cn(
    "h-7 rounded-sm border border-line bg-surface px-2 font-mono text-xs text-ink",
    "outline-none transition-colors duration-100 hover:border-line-strong focus:border-brand",
  );

  return (
    <div className="space-y-3 p-3 text-xs">
      <label className="flex items-center gap-2">
        <input
          type="checkbox"
          className={box}
          checked={settings.followRedirects}
          onChange={(e) => setSettings({ followRedirects: e.target.checked })}
        />
        <span className="text-ink">{t("request.followRedirects")}</span>
      </label>

      <label className="flex items-center gap-2">
        <input
          type="checkbox"
          className={box}
          checked={settings.verifyTls}
          onChange={(e) => setSettings({ verifyTls: e.target.checked })}
        />
        <span className="text-ink">{t("request.verifyTls")}</span>
      </label>

      <label className="flex items-center gap-2">
        <span className="w-[92px] shrink-0 text-ink-muted">{t("request.timeout")}</span>
        <input
          type="number"
          min={1}
          step={500}
          value={settings.timeoutMs}
          onChange={(e) =>
            // 0 / 空 → 1ms。后端会把 0 归一成 1ms（否则 reqwest 解读成"永不超时"），
            // 这里跟着归一，免得界面上显示 0 而实际是 1
            setSettings({ timeoutMs: Math.max(1, Math.trunc(Number(e.target.value)) || 1) })
          }
          className={cn(field, "w-28")}
        />
      </label>

      <label className="flex items-center gap-2">
        <span className="w-[92px] shrink-0 text-ink-muted">{t("request.proxy")}</span>
        <input
          type="text"
          spellCheck={false}
          autoComplete="off"
          // 空串 = 直连。后端对空串的处理和"没填"完全一样（trim 后判空）
          value={settings.proxy ?? ""}
          placeholder={t("request.proxyNone")}
          onChange={(e) => setSettings({ proxy: e.target.value })}
          className={cn(
            field,
            "min-w-0 flex-1 placeholder:font-sans placeholder:text-ink-faint",
          )}
        />
      </label>
    </div>
  );
}

/**
 * 请求面板（工作区上半部分）。
 *
 * 编辑状态来自 `store/request`，这里只负责把它接到控件上。
 * 组件本身**不持有**草稿 —— 否则快捷键（Ctrl+Enter）和按钮会各自看到一份状态。
 *
 * 关于「没选中请求也能编辑」：这是刻意的。粘个地址就发是这个工具最高频的用法，
 * 「先保存一个请求才能发」会把这个流程毁掉。所以这里没有"请先选择请求"的空态，
 * 只在标签栏左边用一个小圆点标出「当前这份草稿有没有对应的已保存请求」。
 */
export function RequestPane() {
  const { t } = useI18n();

  const activeRequestId = useUiStore((s) => s.activeRequestId);
  const requests = useWorkspaceStore((s) => s.requests);

  const active = useMemo(
    () => requests.find((r) => r.id === activeRequestId) ?? null,
    [requests, activeRequestId],
  );

  const draft = useRequestStore((s) => s.draft);
  const sourceId = useRequestStore((s) => s.sourceId);
  const runId = useRequestStore((s) => s.runId);
  const cancelling = useRequestStore((s) => s.cancelling);
  const patch = useRequestStore((s) => s.patch);
  const setRows = useRequestStore((s) => s.setRows);
  const loadFrom = useRequestStore((s) => s.loadFrom);
  const send = useRequestStore((s) => s.send);
  const cancel = useRequestStore((s) => s.cancel);

  const [tab, setTab] = useState<RequestTab>("params");

  /**
   * 把选中的请求灌进草稿。
   *
   * 只在**选中项 id 变了**的时候灌 —— 不能盯着 `active` 这个对象：
   * 工作区每次重新加载都会产生一批新对象，那时草稿会被"重置"，
   * 用户正在敲的字就凭空消失了。这是这个面板最容易写错的一处。
   */
  const loadedId = useRef<number | null | undefined>(undefined);
  useEffect(() => {
    if (loadedId.current === activeRequestId) return;
    loadedId.current = activeRequestId;
    loadFrom(active);
  }, [activeRequestId, active, loadFrom]);

  const tabs = useMemo<readonly TabItem<RequestTab>[]>(
    () =>
      TAB_ORDER.map((key) => ({
        key,
        labelKey: TAB_LABEL[key],
        count: key === "params" ? activeCount(draft.params) : key === "headers" ? activeCount(draft.headers) : undefined,
      })),
    [draft.params, draft.headers],
  );

  const running = runId !== null;
  const canSend = draft.url.trim() !== "";

  /**
   * 标签栏左边那枚指示点 / 名字。
   *
   * 用 `sourceId` 而不是「有没有选中」来判断"已保存"：选中的请求可能刚被删掉，
   * 而草稿还留着 —— 那时它就已经是个未保存的请求了，不该显示成保存过。
   */
  const saved = active !== null && active.id === sourceId;
  const headerLabel = saved ? active.name : t("common.unsaved");

  const BodyIcon = TAB_ICON[tab];

  return (
    <section className="flex h-full min-h-0 flex-col bg-canvas">
      {/* URL 栏 */}
      <div className="flex h-11 shrink-0 items-center gap-2 px-2.5">
        <MethodSelect value={draft.method} onChange={(method) => patch({ method })} />

        <UrlBar
          value={draft.url}
          onChange={(url) => patch({ url })}
          onSubmit={() => {
            if (canSend && !running) void send();
          }}
          placeholder={t("request.urlPlaceholder")}
        />

        {running ? (
          <Button
            variant="danger"
            icon={<Ban size={13} strokeWidth={2.2} />}
            className="shrink-0"
            disabled={cancelling}
            onClick={() => void cancel()}
            title={t("request.cancelShortcut")}
          >
            {t("request.cancel")}
          </Button>
        ) : (
          <Button
            variant="primary"
            icon={<Send size={13} strokeWidth={2.2} />}
            className="shrink-0"
            disabled={!canSend}
            onClick={() => void send()}
            title={t("request.sendShortcut")}
          >
            {t("request.send")}
          </Button>
        )}
      </div>

      {/* 标签栏 */}
      <div className="flex shrink-0 items-center gap-1 border-b border-line pr-2">
        <span className="flex items-center gap-1.5 pl-2.5 pr-1">
          <span
            aria-hidden
            className={cn("h-1.5 w-1.5 rounded-full", saved ? "bg-ok" : "bg-warn")}
          />
          <span className="max-w-[140px] truncate text-[11px] text-ink-muted">
            {headerLabel}
          </span>
        </span>

        <TabStrip items={tabs} value={tab} onChange={setTab} className="min-w-0 flex-1" />
      </div>

      {/* 内容区 */}
      {tab === "body" ? (
        <BodyEditor
          className="min-h-0 flex-1"
          bodyType={draft.bodyType}
          body={draft.body}
          onBodyTypeChange={(bodyType) => patch({ bodyType })}
          onBodyChange={(body) => patch({ body })}
        />
      ) : tab === "settings" ? (
        <div className="min-h-0 flex-1 overflow-auto">
          <SettingsForm />
        </div>
      ) : tab === "params" || tab === "headers" ? (
        <div className="min-h-0 flex-1 overflow-auto">
          <KeyValueTable
            rows={tab === "params" ? draft.params : draft.headers}
            onChange={(rows) => setRows(tab === "params" ? "params" : "headers", rows)}
            showDesc={tab === "headers"}
            keyPlaceholder={t("kv.key")}
            valuePlaceholder={t("kv.value")}
          />
        </div>
      ) : (
        // auth / scripts 分别是 P4 / P5 的事，先明确占位而不是留白
        <EmptyState
          icon={BodyIcon}
          title={t(TAB_LABEL[tab])}
          description={t("common.comingSoon")}
          className="min-h-0 flex-1"
        />
      )}
    </section>
  );
}
