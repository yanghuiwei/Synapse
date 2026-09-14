import { ChevronDown, Globe, KeyRound, ShieldQuestion } from "lucide-react";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import * as ipc from "@/lib/ipc";
import type { AuthConfig, Folder, TokenSource, TokenStatus } from "@/lib/types";
import { useRequestStore } from "@/store/request";
import { useTokenStore } from "@/store/token";
import { useWorkspaceStore } from "@/store/workspace";
import { Button } from "@/features/ui/Button";
import { TokenSourceDialog } from "@/features/token/TokenSourceDialog";
import { TokenStatusBadge } from "@/features/token/TokenStatusBadge";
import { PlaceholderInput } from "./PlaceholderInput";

type AuthKind = AuthConfig["type"];

const KIND_LABELS: Record<AuthKind, MessageKey> = {
  inherit: "auth.inherit",
  none: "auth.none",
  bearer: "auth.bearer",
  basic: "auth.basic",
  apiKey: "auth.apiKey",
};

/** 切换类型时给新变体的初值。字符串一律给空串 —— 猜出来的默认值比空着更难改 */
function emptyAuth(kind: AuthKind): AuthConfig {
  switch (kind) {
    case "bearer":
      return { type: "bearer", token: "" };
    case "basic":
      return { type: "basic", username: "", password: "" };
    case "apiKey":
      return { type: "apiKey", key: "", value: "", location: "header" };
    case "none":
      return { type: "none" };
    default:
      return { type: "inherit" };
  }
}

/**
 * 授权标签页。
 *
 * ## 三件事挤在一个标签页里，是因为它们回答同一个问题
 *
 * 「这次请求带着什么凭据出去」的答案由三处拼起来：
 * 1. 本请求自己的授权配置（页签上半部分）
 * 2. 分类链上继承来的配置（配 `inherit` 时才有意义）
 * 3. 分类链上的 Token 来源（`{{token.access}}` 的值从哪来）
 *
 * 分成三个页签的话，用户得来回切才能拼出完整答案 —— 而这三者恰恰是
 * 最容易互相错配的地方（配了 Bearer，但 Token 来源挂在另一个分类下）。
 *
 * ## 改动**立刻生效**，不需要保存
 *
 * 授权写在草稿里，发送时随 `RequestSpec` 一起过去。自动保存的静默期
 * 只影响「落库」，不影响「这次发出去用什么」—— 所以没配好时直接点发送就能验证。
 */
export function AuthTab() {
  const { t } = useI18n();
  const draft = useRequestStore((s) => s.draft);
  const patch = useRequestStore((s) => s.patch);
  const auth = draft.auth;

  const folders = useWorkspaceStore((s) => s.folders);
  const setFolderAuth = useWorkspaceStore((s) => s.setFolderAuth);

  /** 配置对话框改完之后要重新解析一次「生效的 Token 来源」 */
  const [configVersion, setConfigVersion] = useState(0);
  const [dialog, setDialog] = useState<{ folderId: number; source: TokenSource | null } | null>(
    null,
  );

  const inherited = useMemo(
    () => findInheritedAuth(folders, draft.folderId),
    [folders, draft.folderId],
  );

  const { source: resolvedSource } = useResolvedTokenSource(draft.folderId, configVersion);

  const statuses = useTokenStore((s) => s.statuses);
  const refreshingId = useTokenStore((s) => s.refreshingId);
  const refresh = useTokenStore((s) => s.refresh);

  // Token 状态是「进这个页签看一眼」才需要的，放在这里懒加载 ——
  // 启动时就全量拉会给首屏加一趟无谓的 IPC
  useEffect(() => {
    const store = useTokenStore.getState();
    if (store.status === "idle") void store.load();
  }, []);

  const setAuth = useCallback((next: AuthConfig) => patch({ auth: next }), [patch]);

  const status = resolvedSource ? (statuses[resolvedSource.id] ?? null) : null;

  return (
    <div className="space-y-3 p-3">
      <Row label={t("request.tab.auth")}>
        <KindSelect value={auth.type} onChange={(kind) => setAuth(emptyAuth(kind))} />
      </Row>

      {auth.type === "inherit" ? (
        <>
          <InheritHint inherited={inherited} />
          {draft.folderId !== null ? (
            <FolderAuthBlock
              folderId={draft.folderId}
              folders={folders}
              onSave={(next) => void setFolderAuth(draft.folderId as number, next)}
            />
          ) : null}
        </>
      ) : (
        <AuthFields value={auth} onChange={setAuth} />
      )}

      <TokenSourceBlock
        folderId={draft.folderId}
        folders={folders}
        source={resolvedSource}
        status={status}
        refreshing={resolvedSource !== null && refreshingId === resolvedSource.id}
        onRefresh={() => resolvedSource && void refresh(resolvedSource.id)}
        onConfigure={() =>
          setDialog({ folderId: draft.folderId as number, source: resolvedSource })
        }
      />

      {dialog ? (
        <TokenSourceDialog
          folderId={dialog.folderId}
          source={dialog.source}
          onClose={() => {
            setDialog(null);
            // 重新解析：新建/修改之后「生效的来源」很可能变了
            setConfigVersion((v) => v + 1);
          }}
        />
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------
// 字段
// ---------------------------------------------------------------

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-2">
      <span className="w-[92px] shrink-0 text-xs text-ink-muted">{label}</span>
      {children}
    </div>
  );
}

/** 带 `{{占位符}}` 高亮的单行输入（授权值里放 `{{token.access}}` 是最常见的用法） */
function HighlightedField({
  value,
  onChange,
  label,
  placeholder,
}: {
  value: string;
  onChange: (value: string) => void;
  label: string;
  placeholder?: string;
}) {
  return (
    <PlaceholderInput
      className={BOX}
      value={value}
      onChange={onChange}
      label={label}
      placeholder={placeholder}
    />
  );
}

function PlainField({
  value,
  onChange,
  label,
  placeholder,
}: {
  value: string;
  onChange: (value: string) => void;
  label: string;
  placeholder?: string;
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
        "h-8 flex-1 rounded-sm border border-line bg-surface px-2",
        "font-mono text-xs text-ink placeholder:font-sans placeholder:text-ink-faint",
        "outline-none transition-colors duration-100",
        "hover:border-line-strong focus:border-brand",
      )}
    />
  );
}

function KindSelect({
  value,
  onChange,
}: {
  value: AuthKind;
  onChange: (kind: AuthKind) => void;
}) {
  const { t } = useI18n();
  return (
    <div className="relative flex-1">
      <select
        aria-label={t("request.tab.auth")}
        value={value}
        onChange={(e) => onChange(e.target.value as AuthKind)}
        className={cn(
          "h-8 w-full appearance-none rounded-sm border border-line bg-surface",
          "pl-2 pr-7 text-xs text-ink outline-none transition-colors duration-100",
          "hover:border-line-strong focus:border-brand",
        )}
      >
        {(Object.keys(KIND_LABELS) as AuthKind[]).map((kind) => (
          <option key={kind} value={kind}>
            {t(KIND_LABELS[kind])}
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
 * 一份具体授权（非 `inherit`）的字段。
 *
 * 抽出来是为了让「请求自己的授权」与「分类上的授权」共用同一套字段 ——
 * 两处各写一遍的话，将来加一种授权类型必然只加一处。
 */
function AuthFields({
  value,
  onChange,
}: {
  value: Exclude<AuthConfig, { type: "inherit" }>;
  onChange: (next: AuthConfig) => void;
}) {
  const { t } = useI18n();

  switch (value.type) {
    case "none":
      return null;

    case "bearer":
      return (
        <Row label={t("auth.bearer")}>
          <HighlightedField
            value={value.token}
            onChange={(token) => onChange({ type: "bearer", token })}
            label={t("auth.bearer")}
            placeholder={t("auth.bearerPlaceholder")}
          />
        </Row>
      );

    case "basic":
      return (
        <>
          <Row label={t("auth.username")}>
            <HighlightedField
              value={value.username}
              onChange={(username) => onChange({ ...value, username })}
              label={t("auth.username")}
            />
          </Row>
          <Row label={t("auth.password")}>
            <HighlightedField
              value={value.password}
              onChange={(password) => onChange({ ...value, password })}
              label={t("auth.password")}
            />
          </Row>
        </>
      );

    case "apiKey":
      return (
        <>
          <Row label={t("auth.keyName")}>
            <PlainField
              value={value.key}
              onChange={(key) => onChange({ ...value, key })}
              label={t("auth.keyName")}
              placeholder="X-Api-Key"
            />
          </Row>
          <Row label={t("auth.keyValue")}>
            <HighlightedField
              value={value.value}
              onChange={(v) => onChange({ ...value, value: v })}
              label={t("auth.keyValue")}
            />
          </Row>
          <Row label={t("auth.addTo")}>
            <div className="flex flex-1 gap-1">
              {(["header", "query"] as const).map((location) => (
                <button
                  key={location}
                  type="button"
                  aria-pressed={value.location === location}
                  onClick={() => onChange({ ...value, location })}
                  className={cn(
                    "h-7 rounded-sm border px-2.5 text-xs transition-colors duration-100",
                    value.location === location
                      ? "border-brand bg-brand/12 text-brand"
                      : "border-line text-ink-muted hover:border-line-strong hover:text-ink",
                  )}
                >
                  {t(location === "header" ? "auth.addToHeader" : "auth.addToQuery")}
                </button>
              ))}
            </div>
          </Row>
        </>
      );
  }
}

// ---------------------------------------------------------------
// 继承
// ---------------------------------------------------------------

/** `inherit` 时告诉用户「到底继承到了什么」 */
function InheritHint({ inherited }: { inherited: Folder | null }) {
  const { t } = useI18n();

  if (!inherited) {
    return (
      <p className="flex items-start gap-1.5 text-[11px] leading-relaxed text-ink-faint">
        <ShieldQuestion size={12} strokeWidth={2.2} aria-hidden className="mt-0.5 shrink-0" />
        {t("auth.inheritNone")}
      </p>
    );
  }

  return (
    <p className="flex items-start gap-1.5 text-[11px] leading-relaxed text-ink-muted">
      <Globe size={12} strokeWidth={2.2} aria-hidden className="mt-0.5 shrink-0 text-ink-faint" />
      <span>
        {t("auth.inheritFrom", { name: inherited.name })}
        {" · "}
        {t(KIND_LABELS[inherited.auth.type])}
      </span>
    </p>
  );
}

/**
 * 沿分类链向上找第一个「不是 inherit」的配置。
 *
 * ⚠️ 与后端 `http::pipeline::inherited_auth` 是**同一套规则的两份实现**。
 * 前端这份只用于**显示**（告诉用户继承到了谁），发送时的裁定始终在后端 ——
 * 所以即使哪天漂移了，后果也只是一句提示不准，不会让请求带错凭据。
 */
function findInheritedAuth(folders: readonly Folder[], folderId: number | null): Folder | null {
  const byId = new Map(folders.map((f) => [f.id, f]));
  const seen = new Set<number>();
  let cursor = folderId;

  while (cursor !== null) {
    // 环保护：库被外部改坏时不能让界面死循环
    if (seen.has(cursor)) return null;
    seen.add(cursor);

    const folder = byId.get(cursor);
    if (!folder) return null;
    if (folder.auth.type !== "inherit") return folder;
    cursor = folder.parentId;
  }
  return null;
}

/**
 * 在当前分类上直接改授权。
 *
 * 这是**继承链的数据源**：把它设成 Bearer，下面所有配 `inherit` 的请求
 * 就都用它。放在这里而不是只让用户去别的板块找，省掉的是一次
 * 「我刚才看的是哪个分类来着」。
 */
function FolderAuthBlock({
  folderId,
  folders,
  onSave,
}: {
  folderId: number;
  folders: readonly Folder[];
  onSave: (auth: AuthConfig) => void;
}) {
  const { t } = useI18n();
  const folder = folders.find((f) => f.id === folderId);
  const current = folder?.auth ?? { type: "inherit" as const };

  return (
    <div className="space-y-2 rounded-md border border-line p-2.5">
      <p className="text-[11px] text-ink-muted">
        <span className="text-ink">{folder?.name ?? ""}</span>
        {" · "}
        {t("token.folderHint")}
      </p>

      <Row label={t("request.tab.auth")}>
        <KindSelect value={current.type} onChange={(kind) => onSave(emptyAuth(kind))} />
      </Row>

      {current.type !== "inherit" ? (
        <AuthFields
          value={current}
          onChange={(next) => onSave(next)}
        />
      ) : null}

      <p className="text-[11px] leading-relaxed text-ink-faint">{t("auth.typeHint")}</p>
    </div>
  );
}

// ---------------------------------------------------------------
// Token 来源
// ---------------------------------------------------------------

/**
 * 分类链上生效的 Token 来源。
 *
 * 前端**自己不做继承推导**（不像授权那样在本地走一遍链）：判定要用到
 * `is_token_source` 标记与「最近的优先」，而那条 SQL（`first_in_chain`）
 * 已经在后端了。再写一遍就多一处会漂移的规则。
 */
function useResolvedTokenSource(folderId: number | null, version: number) {
  const [source, setSource] = useState<TokenSource | null>(null);

  useEffect(() => {
    if (folderId === null) {
      setSource(null);
      return;
    }

    let cancelled = false;
    void ipc.tokens
      .resolve(folderId)
      .then((next) => {
        if (!cancelled) setSource(next);
      })
      .catch((e) => {
        if (cancelled) return;
        console.warn("[reqly] 解析 Token 来源失败：", ipc.toIpcError(e).debug);
        setSource(null);
      });

    return () => {
      cancelled = true;
    };
  }, [folderId, version]);

  return { source };
}

function TokenSourceBlock({
  folderId,
  folders,
  source,
  status,
  refreshing,
  onRefresh,
  onConfigure,
}: {
  folderId: number | null;
  folders: readonly Folder[];
  source: TokenSource | null;
  status: TokenStatus | null;
  refreshing: boolean;
  onRefresh: () => void;
  onConfigure: () => void;
}) {
  const { t } = useI18n();
  const owner = source ? folders.find((f) => f.id === source.folderId) : undefined;

  return (
    <div className="space-y-1.5 rounded-md border border-line p-2.5">
      <div className="flex items-center gap-1.5">
        <KeyRound size={11} strokeWidth={2.2} aria-hidden className="shrink-0 text-ink-faint" />
        <span className="text-[11px] font-medium text-ink-muted">{t("auth.tokenSource")}</span>

        <span className="flex-1" />

        {folderId !== null ? (
          <Button
            size="sm"
            variant="ghost"
            icon={<KeyRound size={12} strokeWidth={2} />}
            onClick={onConfigure}
          >
            {source ? t("token.manage") : t("token.configure")}
          </Button>
        ) : null}
      </div>

      {folderId === null ? (
        <p className="text-[11px] leading-relaxed text-ink-faint">{t("token.unsavedFolder")}</p>
      ) : source === null ? (
        <p className="text-[11px] leading-relaxed text-ink-faint">{t("token.noneHint")}</p>
      ) : (
        <div className="flex min-w-0 items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-xs text-ink">
            {source.folderId === folderId
              ? t("token.effectiveSelf", { name: source.name })
              : t("token.effectiveInherited", {
                  name: source.name,
                  folder: owner?.name ?? String(source.folderId),
                })}
          </span>
          <TokenStatusBadge status={status} refreshing={refreshing} onRefresh={onRefresh} />
        </div>
      )}
    </div>
  );
}

/** 高亮输入框的外框（与 `UrlBar` 同一套视觉） */
const BOX =
  "flex-1 rounded-sm border border-line bg-surface transition-colors duration-100 hover:border-line-strong focus-within:border-brand";
