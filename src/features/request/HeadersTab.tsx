import { ChevronDown, ChevronRight, KeyRound, TriangleAlert } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { cn } from "@/lib/cn";
import { formatSeconds } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import { readJwtExpiry } from "@/lib/jwt";
import { parsePlaceholders } from "@/lib/placeholder";
import type { Folder, KeyValueItem, TokenSource, TokenStatus } from "@/lib/types";
import { useRequestStore } from "@/store/request";
import { useTokenStore } from "@/store/token";
import { useWorkspaceStore } from "@/store/workspace";
import { Button } from "@/features/ui/Button";
import {
  draftFromSource,
  emptyTokenSourceDraft,
  TokenSourceForm,
  type TokenSourceDraft,
} from "@/features/token/TokenSourceForm";
import { TokenStatusBadge } from "@/features/token/TokenStatusBadge";
import { useResolvedTokenSource } from "@/features/token/useTokenState";
import { HeaderInsertPanel } from "./HeaderInsertPanel";
import { KeyValueTable } from "./KeyValueTable";

/**
 * 请求头页签 —— 007 之后它同时是**凭据页**。
 *
 * ## 为什么「授权」页消失了
 *
 * 原先有两处真相（`request.auth` 一套 JSON、`request.headers` 一个数组），
 * 两者最终都能变成同一个 `Authorization` 头。为了不让它们打架，后端写了一条
 * 覆盖规则（显式授权赢），前端还要解释「继承自哪个分类」。这三样复杂度
 * **全部来自「授权不是头」这个前提** —— 而它本来就是头。
 *
 * 前提一撤，覆盖规则、继承链、类型下拉三样一起消失。页面上只剩两件事：
 * 一张普通请求头表（凭据就是其中一行），和一个折叠的 Token 来源块
 * （那一行的值从哪来）。
 *
 * ## 为什么这两件事必须同屏
 *
 * 真库里那条 401（id=14「填写率明细」）的因果链是：请求配了「API Key（全空）」，
 * 这**中止了**继承链，于是分类上那条配得好好的 `Bearer {{token.access}}`
 * 拿不到；同时 spec 里没有 `token` 命名空间，配好的 Token 源一次都不会被调用。
 * 而界面上「Token 刷新」那块显示得一字不差 —— 它和「授权 = API Key（空）」
 * 之间没有任何视觉关联。表格里能看到 `{{token.access}}`、折叠区里能看到它的值
 * 怎么来，两件事在同一次滚动里，这个失败就没法再藏。
 */
export interface HeadersTabProps {
  className?: string;
}

export function HeadersTab({ className }: HeadersTabProps) {
  const { t } = useI18n();
  const draft = useRequestStore((s) => s.draft);
  const setRows = useRequestStore((s) => s.setRows);

  const folders = useWorkspaceStore((s) => s.folders);

  /**
   * 系统分类「token刷新」。Token 来源固定配在它下面 —— 后端在启动时就把它
   * 建好了（`folder::ensure_system_folder`），所以这里找不到只可能是
   * 「工作区还没加载完」这一个瞬时状态。
   */
  const systemFolder = useMemo(() => folders.find((f) => f.isSystem) ?? null, [folders]);

  /** 配置改完之后要重新解析一次「生效的 Token 来源」 */
  const [configVersion, setConfigVersion] = useState(0);
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

  /**
   * 有 Token 源时 `{{token.access}}` 的字面形态。
   *
   * 从 i18n 取而不是写死：`auth.tokenPlaceholder` 这个 key 本来就是为它准备的
   * （输入框的 placeholder 也用它）。两处各写一份，改语法时会漏一处。
   */
  const tokenRef = resolvedSource === null ? null : t("auth.tokenPlaceholder");

  const status = resolvedSource ? (statuses[resolvedSource.id] ?? null) : null;

  // ---------------------------------------------------------------
  // 一次性说明：原「授权」页去哪了
  // ---------------------------------------------------------------

  const [noticeDismissed, setNoticeDismissed] = useState(readNoticeSeen);

  const dismissNotice = () => {
    setNoticeDismissed(true);
    try {
      localStorage.setItem(NOTICE_KEY, "1");
    } catch {
      // 存不下就算了：下次还会显示一遍，比「为了它报错」划算
    }
  };

  // ---------------------------------------------------------------
  // Token 来源块：真的用到 token 时才默认展开
  // ---------------------------------------------------------------

  const headerWantsToken = useMemo(
    () =>
      draft.headers.some((row) =>
        parsePlaceholders(row.value).some((hit) => hit.namespace === "token"),
      ),
    [draft.headers],
  );
  /** `null` = 还没被用户手动开合过，跟着 `headerWantsToken` 走 */
  const [tokenOpenOverride, setTokenOpenOverride] = useState<boolean | null>(null);
  const tokenExpanded = tokenOpenOverride ?? headerWantsToken;

  // ---------------------------------------------------------------
  // 凭据行：值里能解出 JWT 的那些
  // ---------------------------------------------------------------

  /**
   * 只解一次 exp，解析结果与「现在几点」分开。
   *
   * 合在一起写的话，每秒的重渲染会连带重跑 base64 解码与 `JSON.parse` ——
   * 而这两个依赖的只有值本身。
   */
  const credentialRows = useMemo(
    () =>
      draft.headers
        .map((row, index) => ({ row, index, expiry: readJwtExpiry(row.value) }))
        .filter((entry) => entry.expiry !== null && entry.row.key.trim() !== ""),
    [draft.headers],
  );

  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (credentialRows.length === 0) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [credentialRows.length]);

  // ---------------------------------------------------------------
  // 表格操作
  // ---------------------------------------------------------------

  /**
   * 插入一行。
   *
   * 已在同名行（不分大小写）时**改那一行的值**，不追加第二行 ——
   * 「快捷插入不会产生第二条同名头」是验收项：两条同名的 `Authorization`
   * 在界面上看起来一样，而只有第二条会被服务端认（或两条一起被拒），
   * 排查时看不出哪条在起作用。想要「手贴 + 自动」并存两条的用户可以手动加。
   *
   * 覆盖时保留 `desc` 与 `enabled`：备注是用户写的，不该被一次插入抹掉。
   */
  const insertRow = (row: KeyValueItem) => {
    const list = draft.headers.slice();
    const at = list.findIndex(
      (existing) => existing.key.trim().toLowerCase() === row.key.trim().toLowerCase(),
    );

    if (at >= 0) {
      const current = list[at];
      if (current) list[at] = { ...current, value: row.value, enabled: true };
    } else {
      list.push(row);
    }
    setRows("headers", list);
  };

  const [pasteNotice, setPasteNotice] = useState(false);
  useEffect(() => {
    if (!pasteNotice) return;
    const timer = window.setTimeout(() => setPasteNotice(false), 4000);
    return () => window.clearTimeout(timer);
  }, [pasteNotice]);

  /**
   * 粘贴时的 `Bearer` 前缀归一化。
   *
   * 从网站上复制 token 时页面上写的就是 `Bearer xxx`，而这一行里已经写着
   * `Bearer ` —— 直接粘下去是 `Bearer Bearer xxx`。这个值会**原样发出去**，
   * 服务端必然回 401，而界面上看不出哪里不对（两个词都是白的）。
   *
   * 只在「两边都带前缀」时动手，其余一律交给浏览器默认行为 ——
   * 猜错一次（比如把用户真想保留的前缀吃掉）比不做这件事更糟。
   */
  const onPasteValue = useCallback(
    (_index: number, pasted: string, row: KeyValueItem): string | null => {
      const PASTE_PREFIX = /^\s*Bearer\s+/i;
      const HEAD_PREFIX = /^Bearer\s+/i;

      if (!PASTE_PREFIX.test(pasted)) return null;
      if (!HEAD_PREFIX.test(row.value)) return null;

      setPasteNotice(true);
      return `Bearer ${pasted.replace(PASTE_PREFIX, "")}`;
    },
    [],
  );

  /**
   * 把这一行从「手贴」换成「自动刷新」。
   *
   * 保留原来的 `Bearer ` 前缀形态（`X-Access-Token` 那 15 处是裸 token，
   * 给它补前缀会把每一处都改成 401）。没有 Token 源时不改值 ——
   * 而是把 Token 来源块打开：改请求内容是用户的决定，
   * 我能做的是把「去哪儿配」摆到他眼前。
   */
  const switchToTokenSource = (index: number, row: KeyValueItem) => {
    if (tokenRef === null) {
      setTokenOpenOverride(true);
      return;
    }
    const list = draft.headers.slice();
    const prefix = /^Bearer\s+/i.test(row.value) ? "Bearer " : "";
    list[index] = { ...row, value: `${prefix}${tokenRef}` };
    setRows("headers", list);
  };

  return (
    <div className={cn("flex min-h-0 flex-col", className)}>
      {/* 快捷插入条 */}
      <div className="shrink-0 border-b border-line px-2.5 py-1.5">
        <HeaderInsertPanel
          rows={draft.headers}
          tokenRef={tokenRef}
          onInsert={insertRow}
        />
      </div>

      {pasteNotice ? (
        <p className="shrink-0 border-b border-line bg-warn/10 px-2.5 py-1 text-[11px] text-warn">
          {t("headers.pasteDeduped")}
        </p>
      ) : null}

      {noticeDismissed ? null : (
        <div className="flex shrink-0 items-start gap-2 border-b border-line bg-brand/8 px-2.5 py-1.5">
          <p className="min-w-0 flex-1 text-[11px] leading-relaxed text-ink-muted">
            {t("headers.notice.authMerged")}
          </p>
          <button
            type="button"
            onClick={dismissNotice}
            className="shrink-0 rounded-sm px-1.5 py-0.5 text-[11px] text-brand transition-colors duration-100 hover:bg-brand/12"
          >
            {t("headers.notice.dismiss")}
          </button>
        </div>
      )}

      {/* 凭据行的 token 倒计时 —— 手贴的 token 唯一缺的那件事 */}
      {credentialRows.length === 0 ? null : (
        <div className="shrink-0 space-y-1 border-b border-line px-2.5 py-1.5">
          {credentialRows.map(({ row, index, expiry }) => (
            <CredentialRowStatus
              key={`${row.key}-${index}`}
              name={row.key}
              expSeconds={expiry?.expSeconds ?? 0}
              now={now}
              hasTokenSource={tokenRef !== null}
              onSwitch={() => switchToTokenSource(index, row)}
            />
          ))}
        </div>
      )}

      {/* 头表本身 */}
      <div className="min-h-0 flex-1 overflow-auto">
        <KeyValueTable
          rows={draft.headers}
          onChange={(rows) => setRows("headers", rows)}
          showDesc
          onPasteValue={onPasteValue}
          keyPlaceholder={t("kv.key")}
          valuePlaceholder={t("kv.value")}
        />
      </div>

      {/* Token 来源（折叠）*/}
      <div className="shrink-0 border-t border-line">
        <button
          type="button"
          onClick={() => setTokenOpenOverride(!tokenExpanded)}
          className={cn(
            "flex w-full items-center gap-1.5 px-2.5 py-1.5 text-left",
            "text-[11px] text-ink-muted transition-colors duration-100 hover:bg-surface",
          )}
        >
          {tokenExpanded ? (
            <ChevronDown size={12} strokeWidth={2.4} aria-hidden className="shrink-0" />
          ) : (
            <ChevronRight size={12} strokeWidth={2.4} aria-hidden className="shrink-0" />
          )}
          {t("headers.tokenSource.title")}
        </button>

        {tokenExpanded ? (
          <div className="px-2.5 pb-2">
            <TokenSourcePanel
              folder={systemFolder}
              source={resolvedSource}
              status={status}
              refreshing={resolvedSource !== null && refreshingId === resolvedSource.id}
              onRefresh={() => resolvedSource && void refresh(resolvedSource.id)}
              // 新建 / 修改之后「生效的来源」很可能变了，重新解析一次
              onSaved={() => setConfigVersion((v) => v + 1)}
            />
          </div>
        ) : null}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------
// 手贴 token 的倒计时
// ---------------------------------------------------------------

/**
 * 一条「值能解出 JWT」的头行的状态。
 *
 * ⚠️ 这个倒计时**只读不验**：`exp` 来自 payload 的明文，签名没校验过。
 * 它是给用户看的提示，不参与任何判断 —— 要不要发这个请求始终由用户说了算。
 * 解不出来（不是 JWT、没有 exp）时整块不出现，不给假倒计时。
 */
function CredentialRowStatus({
  name,
  expSeconds,
  now,
  hasTokenSource,
  onSwitch,
}: {
  name: string;
  /** JWT 的 `exp`（Unix 秒） */
  expSeconds: number;
  /** 每秒推进的当前时刻 */
  now: number;
  hasTokenSource: boolean;
  onSwitch: () => void;
}) {
  const { t } = useI18n();
  const remaining = Math.round(expSeconds - now / 1000);
  const expired = remaining <= 0;

  return (
    <div className="flex min-w-0 items-center gap-2">
      <KeyRound size={11} strokeWidth={2.2} aria-hidden className="shrink-0 text-ink-faint" />
      <code className="min-w-0 shrink-0 truncate font-mono text-[11px] text-ink-muted">
        {name}
      </code>
      <span
        className={cn(
          "flex shrink-0 items-center gap-1 text-[11px]",
          expired ? "text-warn" : "text-ink-faint",
        )}
      >
        {expired ? (
          <TriangleAlert size={11} strokeWidth={2.2} aria-hidden className="shrink-0" />
        ) : null}
        {expired
          ? t("headers.credentials.expired", { time: formatSeconds(Math.abs(remaining)) })
          : t("headers.credentials.expiresIn", { time: formatSeconds(remaining) })}
      </span>

      <span className="flex-1" />

      <Button size="sm" variant="ghost" onClick={onSwitch}>
        {t(
          hasTokenSource
            ? "headers.credentials.useTokenSource"
            : "headers.credentials.configureTokenSource",
        )}
      </Button>
    </div>
  );
}

// ---------------------------------------------------------------
// Token 来源（从原 AuthTab 整体搬来）
// ---------------------------------------------------------------

/**
 * Token 来源。
 *
 * ## 为什么是页内折叠，而不是对话框
 *
 * 这一块原先点「配置」会弹一个对话框，里面还要先选分类再选请求 —— 而
 * 分类固定是系统分类「token刷新」、获取与刷新又是同一个接口，
 * 那三个选择项里有两个是假的。现在直接在这一块里展开表单，
 * 页面上下文（我正在看哪条请求）全程不丢，也少一层遮罩。
 *
 * ## 为什么保存按钮的可用性在这里算，而不是在表单里
 *
 * 表单只管「这几个字段长什么样」，而「能不能保存」取决于**跨字段**的约束
 * （名字非空 + 选了接口 + access 路径合法）。把它放进表单里，表单就得暴露一个
 * `onValidityChange` 回调给调用方 —— 一个组件两个出口，还不如让持有提交动作的
 * 那一层自己判断。
 */
function TokenSourcePanel({
  folder,
  source,
  status,
  refreshing,
  onRefresh,
  onSaved,
}: {
  /** 系统分类「token刷新」。`null` = 工作区还没加载完 */
  folder: Folder | null;
  source: TokenSource | null;
  status: TokenStatus | null;
  refreshing: boolean;
  onRefresh: () => void;
  onSaved: () => void;
}) {
  const { t } = useI18n();
  const create = useTokenStore((s) => s.create);
  const save = useTokenStore((s) => s.save);

  /**
   * `null` = 没在编辑。
   *
   * 草稿**只在用户点了「配置」时算一次**（而不是挂个依赖 `source` 的 effect）：
   * 那样的 effect 会把用户已经敲进去的内容在父组件重渲染时冲掉 ——
   * 而这里每次自动保存都会让父组件重渲染。
   */
  const [draft, setDraft] = useState<TokenSourceDraft | null>(null);
  const [saving, setSaving] = useState(false);

  const requests = useWorkspaceStore((s) => s.requests);
  /** 能当 Token 接口的候选：系统分类「token刷新」下的那些请求 */
  const candidates = useMemo(
    () => (folder ? requests.filter((r) => r.folderId === folder.id) : []),
    [requests, folder],
  );

  if (!folder) return null;

  const beginCreate = () =>
    setDraft({
      ...emptyTokenSourceDraft(folder.id),
      // 预填一个名字：这个分类下就一份配置，让用户少想一件事。
      // 名字不能留空（后端 `clean_name` 会拒），也不能是中文文案常量 ——
      // 所以走 i18n，换语言时它跟着换
      name: t("token.defaultName"),
    });

  const canSave =
    draft !== null &&
    draft.name.trim() !== "" &&
    draft.fetchRequestId !== 0 &&
    pathOk(draft.accessPath, true) &&
    pathOk(draft.expiresInPath, false) &&
    !saving;

  const submit = async () => {
    if (!draft) return;
    setSaving(true);
    const done = source
      ? await save({ ...source, ...draft, name: draft.name.trim() })
      : await create({ ...draft, name: draft.name.trim() });
    setSaving(false);
    if (!done) return;
    setDraft(null);
    onSaved();
  };

  const editable = draft !== null;

  return (
    <div className="space-y-1.5 rounded-md border border-line p-2">
      <div className="flex items-center gap-1.5">
        <KeyRound size={11} strokeWidth={2.2} aria-hidden className="shrink-0 text-ink-faint" />
        <span className="text-[11px] font-medium text-ink-muted">{t("token.refreshTitle")}</span>

        <span className="flex-1" />

        {/* 编辑时保存/取消跟着标题走：表单本身已经压到三行，再单开一行按钮
            就是白白多占一行 */}
        {editable ? (
          <>
            <Button size="sm" variant="secondary" onClick={() => setDraft(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              size="sm"
              variant="primary"
              disabled={!canSave}
              onClick={() => void submit()}
            >
              {saving ? t("common.loading") : t("common.save")}
            </Button>
          </>
        ) : (
          <Button
            size="sm"
            variant="ghost"
            icon={<KeyRound size={12} strokeWidth={2} />}
            onClick={() => (source ? setDraft(draftFromSource(source)) : beginCreate())}
          >
            {source ? t("token.manage") : t("token.configure")}
          </Button>
        )}
      </div>

      {editable ? (
        <TokenSourceForm value={draft} onChange={setDraft} requests={candidates} />
      ) : source === null ? (
        <>
          <p className="text-[11px] leading-relaxed text-ink-faint">{t("token.noneHint")}</p>
          <p className="text-[11px] leading-relaxed text-ink-faint">{t("token.globalHint")}</p>
        </>
      ) : (
        <>
          <div className="flex min-w-0 items-center gap-2">
            <span className="min-w-0 flex-1 truncate text-xs text-ink">
              {t("token.effectiveName", { name: source.name })}
            </span>
            <TokenStatusBadge status={status} refreshing={refreshing} onRefresh={onRefresh} />
          </div>
          {/* 配置不在系统分类下说明这是一份老的「按分类配的」来源 ——
              那它只对那个分类的子树生效，别让用户以为它对所有请求都算数 */}
          <p className="text-[11px] leading-relaxed text-ink-faint">
            {source.folderId === folder.id
              ? t("token.globalHint")
              : t("token.scopedHint")}
          </p>
        </>
      )}
    </div>
  );
}

/** 保存按钮的判据，与 `TokenSourceForm` 里那两个 `pathError` 是同一条规则 */
function pathOk(path: string, required: boolean): boolean {
  const trimmed = path.trim();
  if (trimmed === "") return !required;
  return trimmed.startsWith("$");
}

/** 「原『授权』页已并入这里」只提示一次 */
const NOTICE_KEY = "reqly.headers.authMergedNotice";

function readNoticeSeen(): boolean {
  try {
    return localStorage.getItem(NOTICE_KEY) === "1";
  } catch {
    // 读不了存储（隐私模式之类）就别骚扰用户 —— 当作已经看过
    return true;
  }
}
