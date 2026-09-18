import {
  CircleCheck,
  CircleHelp,
  LoaderCircle,
  RefreshCw,
  Timer,
  TriangleAlert,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ReactNode, RefObject } from "react";
import { createPortal } from "react-dom";

import { cn } from "@/lib/cn";
import { formatClock, formatCountdown, formatSeconds } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import type { TokenSource, TokenStatus } from "@/lib/types";
import { useTokenStore } from "@/store/token";
import { useWorkspaceStore } from "@/store/workspace";
import { IconButton } from "@/features/ui/Button";
import { useResolvedTokenSource, useTokenCountdown } from "@/features/token/useTokenState";

/**
 * 发送区 Token 状态条的六态。
 *
 * `expiring` 与 `expired` 分开是**必须**的：后端的 `expired` 已经把
 * `skew_seconds`（提前量）算进去了，所以「还剩 20 秒、提前量 30 秒」时
 * 它就已经是 `expired = true`。那时 token 明明还能用、只是**下一次发送**
 * 会触发刷新 —— 混成一句话，用户会以为这次请求注定失败。
 *
 * `noSource` 也单列，而且它是这一整块**最该被看见**的状态：
 * 请求里写了 `{{token.*}}` 却没有可用来源，发送时那串占位符会原样带出去。
 */
type GateState =
  | "noSource"
  | "unknown"
  | "neverFetched"
  | "noExpiry"
  | "valid"
  | "expiring"
  | "expired";

const STATE_META: Record<GateState, { icon: LucideIcon; className: string }> = {
  // 「还没配」「还没取过」用警告色而不是错误色：它们不是故障，
  // 而是两个动作明确的状态（去配 / 去刷）
  noSource: { icon: TriangleAlert, className: "text-warn" },
  unknown: { icon: CircleHelp, className: "text-ink-faint" },
  neverFetched: { icon: TriangleAlert, className: "text-warn" },
  noExpiry: { icon: CircleCheck, className: "text-ok" },
  valid: { icon: Timer, className: "text-ok" },
  expiring: { icon: Timer, className: "text-warn" },
  expired: { icon: TriangleAlert, className: "text-warn" },
};

/** 详情浮层的宽度，同时也是它贴边时的夹紧依据 */
const PANEL_WIDTH = 320;
const MARGIN = 8;

export interface TokenGateProps {
  /** 当前草稿挂在哪个分类下 —— 决定沿哪条链找 Token 来源 */
  folderId: number | null;
  /**
   * 这条请求里有没有写 `{{token.*}}`。
   *
   * 由调用方扫草稿得出（见 `RequestPane` 的 `draftWantsToken`）。
   * 放在外面算，是因为「这一整块要不要出现」是排版决定，
   * 而这个组件该关心的只有「出现之后长什么样」。
   */
  wantsToken: boolean;
  className?: string;
}

/**
 * 发送按钮旁边的 Token 状态条。
 *
 * ## 它解决什么
 *
 * 在这之前，Token 的状态只在「请求头」页那个折叠的 Token 来源块里看得见
 * —— 而发送前没人会去翻那里。
 * 于是「这次发出去带的是新 token 还是两小时前那个」只能靠发完看 401 来发现。
 *
 * 这块东西把四个问题摆到发送键旁边：
 *
 * 1. **这次用不用得上 Token** —— 没写 `{{token.*}}` 就整块不出现（不制造噪音）
 * 2. **还有多久该刷新** —— 秒级倒计时，基准值来自后端、推进在本地
 *    （见 `useTokenCountdown` 里对「为什么敢自己走秒」的说明）
 * 3. **刷新会打到哪个接口** —— 展开就能看到那个接口的方法与地址。
 *    这是「token 接口改了、这边没改」唯一能一眼对上的地方
 * 4. **能不能提前刷** —— 不必等它过期：按钮一直在。点了走的是**同一个单飞锁**
 *    （后端保证不会因此多打一次网络请求）
 *
 * ## 为什么详情是浮层，而不是页内展开
 *
 * 展开面板要插在地址栏与页签栏之间，一开一关整块编辑区都会跳一下。
 * 「扫一眼倒计时对不对」是高频动作，跳一下的代价比多一层浮层大。
 *
 * 浮层走 `createPortal` + `fixed`（与 `ContextMenu` 同一套做法）：
 * 留在原地会被 `overflow` 裁掉，而地址栏这一层正好在各种裁切容器的交界上。
 */
export function TokenGate({ folderId, wantsToken, className }: TokenGateProps) {
  const { t } = useI18n();

  const { source } = useResolvedTokenSource(folderId);
  const statuses = useTokenStore((s) => s.statuses);
  const refreshingId = useTokenStore((s) => s.refreshingId);
  const refresh = useTokenStore((s) => s.refresh);

  const [open, setOpen] = useState(false);
  const anchorRef = useRef<HTMLDivElement>(null);

  // Token 状态是「站在发送键旁边看一眼」才需要的，所以在这里懒加载 ——
  // 一进应用就全量拉会给首屏加一趟无谓的 IPC（与请求头页那个 Token 来源块同一个判断）
  useEffect(() => {
    const store = useTokenStore.getState();
    if (store.status === "idle") void store.load();
  }, []);

  const status = source ? (statuses[source.id] ?? null) : null;
  const countdown = useTokenCountdown(status);
  const refreshing = source !== null && refreshingId === source.id;

  if (!wantsToken) return null;

  const state = resolveState(source, status);
  const meta = STATE_META[state];
  // 刷新中优先显示转圈：那一刻「还剩多少秒」是旧信息，转圈才是当下的状态
  const Icon = refreshing ? LoaderCircle : meta.icon;

  return (
    <div ref={anchorRef} className={cn("relative flex shrink-0 items-center", className)}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        aria-haspopup="dialog"
        title={t("token.gate.open")}
        className={cn(
          "flex h-7 items-center gap-1 rounded-sm border border-transparent px-1.5",
          "text-[11px] transition-colors duration-100",
          "hover:border-line hover:bg-surface-hover",
          meta.className,
          // 「提前量窗口内 / 已过期」时补一个边框：这两种状态要能把人拽过来，
          // 否则它和「有效」的差别只剩一个颜色，扫一眼很容易略过
          refreshNeeded(state) && "border-line bg-warn/8",
        )}
      >
        <Icon
          size={11}
          strokeWidth={2.2}
          aria-hidden
          className={cn("shrink-0", refreshing && "animate-spin")}
        />
        <span className="tabular whitespace-nowrap font-medium">
          {badgeLabel(t, state, countdown)}
        </span>
      </button>

      {/* 没有来源时不给刷新按钮：刷一个不存在的东西只会让人更困惑，
          徽标本身已经在喊「未配置来源」了 */}
      {source === null ? null : (
        <IconButton
          size="sm"
          label={refreshing ? t("token.refreshing") : t("token.refresh")}
          disabled={refreshing}
          onClick={() => void refresh(source.id)}
        >
          <RefreshCw size={11} strokeWidth={2.2} />
        </IconButton>
      )}

      {open ? (
        <TokenGatePanel
          source={source}
          status={status}
          countdown={countdown}
          refreshing={refreshing}
          anchorRef={anchorRef}
          onRefresh={() => source && void refresh(source.id)}
          onClose={() => setOpen(false)}
        />
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------
// 徽标
// ---------------------------------------------------------------

function resolveState(source: TokenSource | null, status: TokenStatus | null): GateState {
  if (source === null) return "noSource";
  if (status === null) return "unknown";
  if (!status.hasCache) return "neverFetched";
  if (status.expiresAt === null) return "noExpiry";
  if (!status.expired) return "valid";
  // `expired` 是后端含提前量的判定。倒计时还没走到 0，说明是**提前量**
  // 把它算成过期的 —— 那时候该说的是「即将刷新」，不是「已过期」
  return (status.remainingSeconds ?? 0) > 0 ? "expiring" : "expired";
}

/** 这两种状态是「该动手了」：给徽标补一个边框 */
function refreshNeeded(state: GateState): boolean {
  return state === "expiring" || state === "expired";
}

function badgeLabel(
  t: (key: MessageKey, params?: Record<string, string | number>) => string,
  state: GateState,
  countdown: number | null,
): string {
  switch (state) {
    case "noSource":
      return t("token.gate.noSource");
    case "unknown":
      return t("token.gate.unknown");
    case "neverFetched":
      return t("token.gate.neverFetched");
    case "noExpiry":
      return t("token.gate.noExpiry");
    case "expiring":
      return t("token.gate.expiring", { time: formatCountdown(countdown ?? 0) });
    case "expired":
      return t("token.gate.expired", { time: formatCountdown(countdown ?? 0) });
    default:
      // 「有效」不写字，只给数字：宽度是硬约束，而旁边那个绿表已经说明了一切
      return countdown === null ? "—" : formatCountdown(countdown);
  }
}

// ---------------------------------------------------------------
// 详情浮层
// ---------------------------------------------------------------

interface TokenGatePanelProps {
  /** `null` = 请求引用了 Token，但分类链上没有来源（要给出「怎么办」） */
  source: TokenSource | null;
  status: TokenStatus | null;
  countdown: number | null;
  refreshing: boolean;
  anchorRef: RefObject<HTMLDivElement | null>;
  onRefresh: () => void;
  onClose: () => void;
}

function TokenGatePanel({
  source,
  status,
  countdown,
  refreshing,
  anchorRef,
  onRefresh,
  onClose,
}: TokenGatePanelProps) {
  const { t } = useI18n();
  const requests = useWorkspaceStore((s) => s.requests);
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  // 「刷新会打到哪个接口」。后端的选法是 `refresh_request_id` 优先
  // （见 `http::token::fetch`）—— 界面必须跟着它，
  // 否则指的接口不是实际会用的那个，用户会去改一个无关的请求
  const endpointId = source ? (source.refreshRequestId ?? source.fetchRequestId) : null;
  const endpoint = requests.find((r) => r.id === endpointId) ?? null;

  /**
   * 「获取」和「刷新」确实是两条接口时，把**另一条**也点出来。
   *
   * ⚠️ 要说的不是刷新那条（上面 `endpoint` 行显示的已经是它了，再说一遍等于
   * 同一句话说两遍），而是**获取**那条 —— 用户看到「Token 接口」时默认会以为
   * 初次获取也走它，而这条配置下并不是。
   *
   * 目前界面上配不出这种配置（表单只暴露一个「Token 接口」），
   * 但 MCP / 直接写库能造出来 —— 那时界面不能装作它们是一条。
   */
  const splitEndpoints =
    source && source.refreshRequestId !== null && source.refreshRequestId !== source.fetchRequestId
      ? {
          fetch: requests.find((r) => r.id === source.fetchRequestId)?.name ?? null,
          refresh: requests.find((r) => r.id === source.refreshRequestId)?.name ?? null,
        }
      : null;

  useLayoutEffect(() => {
    const rect = anchorRef.current?.getBoundingClientRect();
    if (!rect) return;
    setPos({
      // 右对齐到徽标右边缘，再夹进视口 —— 靠右时右对齐才不会跑出窗口
      left: Math.max(
        MARGIN,
        Math.min(rect.right - PANEL_WIDTH, window.innerWidth - PANEL_WIDTH - MARGIN),
      ),
      top: rect.bottom + 4,
    });
  }, [anchorRef]);

  // 关闭时机与 `ContextMenu` 一致：外部按下 / Esc / 滚动 / 尺寸变化。
  // 用 pointerdown 而不是 click —— 等到 click 才关会先触发底层元素的 click
  useEffect(() => {
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node;
      if (ref.current?.contains(target)) return;
      if (anchorRef.current?.contains(target)) return; // 留给「再点一次收起」
      onClose();
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation(); // 别冒泡成「取消请求」
        onClose();
      }
    };
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown, true);
    // capture 阶段监听 scroll：内层滚动容器的滚动不冒泡到 window
    window.addEventListener("scroll", onClose, true);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("scroll", onClose, true);
      window.removeEventListener("resize", onClose);
    };
  }, [anchorRef, onClose]);

  return createPortal(
    <div
      ref={ref}
      role="dialog"
      aria-label={t("token.gate.title")}
      style={{
        left: pos?.left ?? -9999,
        top: pos?.top ?? -9999,
        width: PANEL_WIDTH,
        // z 层级走 theme.css 的令牌，不写裸数字（「谁盖住谁」得能在一个地方看全）
        zIndex: "var(--z-dropdown)",
        // 量出位置之前先藏起来，避免看到面板在别处闪一下
        visibility: pos ? "visible" : "hidden",
      }}
      className="fixed rounded-md border border-line bg-surface p-2.5 shadow-e2"
    >
      <div className="flex items-center gap-1.5">
        <span className="text-[11px] font-medium text-ink-muted">{t("token.gate.title")}</span>
        <span className="flex-1" />
        {source === null ? null : (
          <button
            type="button"
            disabled={refreshing}
            onClick={onRefresh}
            className={cn(
              "flex h-6 items-center gap-1 rounded-sm border border-line px-1.5",
              "text-[11px] text-ink transition-colors duration-100",
              "hover:border-line-strong hover:bg-surface-hover",
              "disabled:pointer-events-none disabled:opacity-45",
            )}
          >
            {refreshing ? (
              <LoaderCircle size={11} strokeWidth={2.2} aria-hidden className="animate-spin" />
            ) : (
              <RefreshCw size={11} strokeWidth={2.2} aria-hidden />
            )}
            {refreshing ? t("token.refreshing") : t("token.refresh")}
          </button>
        )}
      </div>

      {source === null ? (
        <>
          <p className="mt-2 text-[11px] leading-relaxed text-warn">
            {t("token.gate.noSourceHint")}
          </p>
          <p className="mt-1 text-[11px] leading-relaxed text-ink-faint">
            {t("token.gate.manageHint")}
          </p>
        </>
      ) : (
        <>
          <dl className="mt-2 space-y-1 text-[11px]">
            <Field label={t("token.gate.source")}>
              <span className="truncate text-ink">{source.name}</span>
            </Field>

            <Field label={t("token.gate.endpoint")}>
              {endpoint ? (
                <>
                  <span className="shrink-0 font-mono font-semibold text-ink-muted">
                    {endpoint.method}
                  </span>
                  {/* 折行而不是截断：这一行存在的唯一理由就是「一眼对上是哪个接口」，
                      而截断恰好把它要传达的那一段切掉 —— 同一个域名下挂了两个取
                      token 的接口时，`…/ou…` 两边长得一模一样。浮层里纵向空间不紧张 */}
                  <span className="min-w-0 break-all font-mono text-ink" title={endpoint.url}>
                    {endpoint.url}
                  </span>
                </>
              ) : (
                <span className="truncate text-warn">{t("token.gate.endpointUnknown")}</span>
              )}
            </Field>

            {splitEndpoints === null ? null : (
              <Field label="">
                <span className="truncate text-ink-faint">
                  {t("token.gate.splitEndpoints", {
                    fetch: splitEndpoints.fetch ?? t("token.gate.endpointUnknown"),
                    refresh: splitEndpoints.refresh ?? t("token.gate.endpointUnknown"),
                  })}
                </span>
              </Field>
            )}

            <Field label={t("token.gate.fetchedAt")}>
              <span className="tabular text-ink">
                {status?.updatedAt ? formatClock(status.updatedAt) : "—"}
              </span>
            </Field>

            <Field label={t("token.gate.expiresAt")}>
              <span className="tabular text-ink">
                {status?.expiresAt ? formatClock(status.expiresAt) : t("token.noExpiry")}
              </span>
              {countdown === null ? null : (
                <span className="tabular text-ink-faint">
                  {countdown >= 0
                    ? `（${t("token.expiresIn", { time: formatSeconds(countdown) })}）`
                    : `（${t("token.expiredAgo", { time: formatSeconds(countdown) })}）`}
                </span>
              )}
            </Field>

            <Field label={t("token.gate.autoRefresh")}>
              <span className={cn(status?.autoRefresh ? "text-ink" : "text-warn")}>
                {status?.autoRefresh ? t("token.gate.on") : t("token.gate.off")}
              </span>
              <span className="tabular text-ink-faint">
                · {t("token.gate.skew")} {source.skewSeconds}s
              </span>
            </Field>
          </dl>

          <p className="mt-2 border-t border-line pt-1.5 text-[11px] leading-relaxed text-ink-faint">
            {status?.autoRefresh === false
              ? t("token.gate.autoRefreshOff")
              : t("token.gate.endpointHint")}
          </p>
        </>
      )}
    </div>,
    document.body,
  );
}

/**
 * 浮层里的一行「标签 → 值」。
 *
 * `label` 给空串时值通栏，并缩进到与其它行的值对齐 —— 那种行是**续行说明**
 * （比如「刷新走另一条接口」），它属于上一行的值，不该再占一个标签位。
 *
 * 标签列宽 72px 是按最长的那条标签（「自动重新获取」6 个汉字 ≈ 66px）定的。
 * 定窄了标签会折成两行，而定宽了是从**地址列**里扣 —— 地址是这里最需要宽度的值。
 */
function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 gap-2">
      {label === "" ? null : <dt className="w-[72px] shrink-0 text-ink-faint">{label}</dt>}
      <dd className={cn("flex min-w-0 flex-1 items-center gap-1", label === "" && "pl-[80px]")}>
        {children}
      </dd>
    </div>
  );
}
