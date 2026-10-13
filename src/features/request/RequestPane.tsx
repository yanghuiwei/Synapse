import { Ban, ChevronDown, Save, Send } from "lucide-react";
import { useMemo, useState } from "react";

import { cn } from "@/lib/cn";
import { HTTP_METHODS, methodTextClass } from "@/lib/http";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import type { KeyValueItem, RequestSettings } from "@/lib/types";
import { useRequestStore, type RequestDraft } from "@/store/request";
import { useUiStore } from "@/store/ui";
import { useWorkspaceStore } from "@/store/workspace";
import { Button } from "@/features/ui/Button";
import { TabStrip, type TabItem } from "@/features/ui/TabStrip";
import { LoadTestPanel } from "@/features/loadtest/LoadTestPanel";
import { parsePlaceholders } from "@/lib/placeholder";
import { TokenGate } from "./TokenGate";
import { BodyEditor } from "./BodyEditor";
import { HeadersTab } from "./HeadersTab";
import { KeyValueTable } from "./KeyValueTable";
import { SaveDraftDialog } from "./SaveDraftDialog";
import { UrlBar } from "./UrlBar";

type RequestTab = "params" | "headers" | "body" | "settings" | "loadtest";

const TAB_LABEL: Record<RequestTab, MessageKey> = {
  params: "request.tab.params",
  headers: "request.tab.headers",
  body: "request.tab.body",
  settings: "request.tab.settings",
  loadtest: "loadtest.tab",
};

/**
 * 页签顺序。
 *
 * 「请求体」排在最前：粘一份 JSON 就发是这个工具最高频的用法，而它默认要落在
 * 第一个页签上（`DEFAULT_TAB`），否则每次打开请求都得先点一下才能开始打字。
 *
 * 「压测」排在最后：它**不是**在编辑请求 —— 它是拿当前草稿去打一轮。
 * 插在 body/headers 中间会让人以为它也是请求定义的一部分。
 *
 * ⚠️ 「压测」页**必须留在请求区**。压测压的是**当前编辑器里的草稿**
 * （`store/loadtest.ts` 的 `start()` 直接读 `request.draft`），所以它的入口
 * 天然属于请求：「改完地址点开始，打的就是这个地址」。曾经把它做成独立板块的
 * 「新压测」页，等于把配置和它真正作用的那个对象分到两个地方 ——
 * 你在压测页改不了要打的请求，在请求页看不到正在跑什么。
 *
 * ⚠️ 这里**没有「授权」页**了（007）：授权就是请求头里的一行，
 * 独立的页签只会让「我配的 token 到底生效没有」这个问题多一层要翻的地方。
 */
const TAB_ORDER: readonly RequestTab[] = [
  "body",
  "params",
  "headers",
  "settings",
  "loadtest",
];

/** 打开一份内容时先展示哪个页签 —— 见 `TAB_ORDER` 上的理由 */
const DEFAULT_TAB: RequestTab = "body";

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

  // 「打开的是哪一份内容」由 store 的显式动作决定，这里**不**再看选中项 ——
  // 历史快照能灌进编辑器却没有任何 id 可挂（见 store/request 里的注释）
  const requests = useWorkspaceStore((s) => s.requests);

  const draft = useRequestStore((s) => s.draft);
  const sourceId = useRequestStore((s) => s.sourceId);
  const sourceName = useRequestStore((s) => s.sourceName);

  // 已保存的请求可能在树上被改过名，显示时优先取工作区里的最新名字；
  // 查不到（刚被删掉）就退回打开时的快照名
  const openedName = useMemo(
    () =>
      sourceId === null
        ? sourceName
        : (requests.find((r) => r.id === sourceId)?.name ?? sourceName),
    [requests, sourceId, sourceName],
  );
  const runId = useRequestStore((s) => s.runId);
  const cancelling = useRequestStore((s) => s.cancelling);
  const patch = useRequestStore((s) => s.patch);
  const setRows = useRequestStore((s) => s.setRows);
  // 请求体类型不走 `patch` —— 它要先把当前内容写回分槽快照，见 `setBodyType`
  const setBodyType = useRequestStore((s) => s.setBodyType);
  const setBodyRows = useRequestStore((s) => s.setBodyRows);
  // multipart 与 urlencoded 各自一张表，也一样不能走 `patch` —— 见 `setFormFields`
  const setFormFields = useRequestStore((s) => s.setFormFields);
  const send = useRequestStore((s) => s.send);
  const cancel = useRequestStore((s) => s.cancel);

  const [tab, setTab] = useState<RequestTab>(DEFAULT_TAB);

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
  const saved = sourceId !== null;
  const headerLabel = openedName ?? t("common.unsaved");

  // 这条草稿到底用不用得上 Token。在这里算而不丢给 `TokenGate`：
  // 「整块要不要出现」是排版决定（它同时决定了地址栏里还剩多少宽度给 URL）
  const wantsToken = useMemo(() => draftWantsToken(draft), [draft]);

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

        {/* Token 状态条就摆在发送键旁边 —— 在这之前它只在「授权」页里出现过，
            而发送前没人会去翻那儿，于是「这次带的是不是新 token」只能靠发完
            看 401 来发现。没写 `{{token.*}}` 时它整块不出现，不制造噪音 */}
        <TokenGate requestId={draft.requestId} wantsToken={wantsToken} />

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
        <span className="flex shrink-0 items-center gap-1.5 pl-2.5 pr-1">
          <span
            aria-hidden
            className={cn("h-1.5 w-1.5 rounded-full", saved ? "bg-ok" : "bg-warn")}
          />
          <span className="max-w-[140px] truncate text-[11px] text-ink-muted">
            {headerLabel}
          </span>
          {/* 未保存的草稿没有归属分类，而 Token 来源按分类链继承 ——
              这里必须给出「存到哪儿去」的入口，否则请求头页那句提示是条死路 */}
          {saved ? null : (
            <button
              type="button"
              onClick={() => useUiStore.getState().openSaveDraft()}
              title={t("request.saveAsHint")}
              className={cn(
                "flex shrink-0 items-center gap-1 rounded-sm px-1.5 py-0.5",
                "text-[11px] text-brand transition-colors duration-100",
                "hover:bg-brand/12",
              )}
            >
              <Save size={11} strokeWidth={2.2} aria-hidden className="shrink-0" />
              {t("request.saveAs")}
            </button>
          )}
        </span>

        <TabStrip items={tabs} value={tab} onChange={setTab} className="min-w-0 flex-1" />
      </div>

      {/* 内容区 */}
      {tab === "loadtest" ? (
        // 压测面板自带滚动与内边距（它的内容比一个表格复杂得多），
        // 所以不像其它页签那样再套一层 overflow 容器。
        // 报告**就地渲染**（跑完不用跳板块）—— 见 `LoadTestPanel` 的注释
        <LoadTestPanel className="min-h-0 flex-1" />
      ) : tab === "body" ? (
        <BodyEditor
          className="min-h-0 flex-1"
          bodyType={draft.bodyType}
          body={draft.body}
          bodyRows={draft.bodyRows}
          formFields={draft.formFields}
          onBodyTypeChange={setBodyType}
          onBodyChange={(body) => patch({ body })}
          onBodyRowsChange={setBodyRows}
          onFormFieldsChange={setFormFields}
        />
      ) : tab === "settings" ? (
        <div className="min-h-0 flex-1 overflow-auto">
          <SettingsForm />
        </div>
      ) : tab === "params" ? (
        <div className="min-h-0 flex-1 overflow-auto">
          <KeyValueTable
            rows={draft.params}
            onChange={(rows) => setRows("params", rows)}
            keyPlaceholder={t("kv.key")}
            valuePlaceholder={t("kv.value")}
          />
        </div>
      ) : (
        // 请求头页自带滚动、内边距与折叠区（007 起它同时是凭据页），
        // 所以不像 params 那样再套一层容器
        <HeadersTab className="min-h-0 flex-1" />
      )}

      {/* 它是个 portal；挂在这里是为了跟着当前请求走，
          而且切页签不会把它卸掉 */}
      <SaveDraftDialog />
    </section>
  );
}

/**
 * 这份草稿里有没有写 `{{token.*}}` —— 决定发送区要不要出现 Token 状态条。
 *
 * ## 这是一份**镜像**，不是权威
 *
 * 权威在 Rust 的 `template::resolver::scan_spec`：真正「这次要不要 token」
 * 由它决定（`http::pipeline::token_context` 用它判断该不该去取 Token）。
 * 这里只服务于「要不要显示那个徽标」，所以两边不一致的代价被限制在
 * **显示层**：漏判 = 少一个入口（用户还能去「请求头」页看那个折叠的
 * Token 来源块），
 * 误判 = 多一个入口（点开发现没有来源，正好说明问题）。
 *
 * ⚠️ 但**字段清单必须跟着 `scan_spec` 一起改**。它扫的是 url / params /
 * headers / body —— 这里同样一条不落。少扫一处（比如以为
 * 「授权里不会写 token」，那已经是 007 之前的事了）就会在一整类请求上
 * 静默地不显示这个入口。
 *
 * 宁可多扫：未勾选的行、文件路径都一起看。**多显示一个入口**只是多一次点击，
 * **漏显示**才是那个「上次改了 token 配置，发出去还是旧 token」的场景。
 */
function draftWantsToken(draft: RequestDraft): boolean {
  const texts: string[] = [
    draft.url,
    draft.body,
    // 凭据头（`Authorization: {{token.access}}`）天然就在 headers 里，
    // 不必再单列一项 —— 这正是 007 的收益之一
    ...draft.params.flatMap((row) => [row.key, row.value]),
    ...draft.headers.flatMap((row) => [row.key, row.value]),
    ...draft.bodyRows.flatMap((row) => [row.key, row.value]),
    ...draft.formFields.flatMap((row) => [row.key, row.value, row.file ?? ""]),
  ];

  return texts.some((text) =>
    parsePlaceholders(text).some((hit) => hit.namespace === "token"),
  );
}
