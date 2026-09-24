import { Clock, Copy, CornerUpLeft, LoaderCircle, SearchX, Trash } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { cn } from "@/lib/cn";
import { formatBytes, formatMs } from "@/lib/format";
import { methodTextClass, statusTextClass } from "@/lib/http";
import { useI18n } from "@/lib/i18n";
import type { TranslateFn } from "@/lib/i18n";
import * as ipc from "@/lib/ipc";
import { requestSpecSchema } from "@/lib/schemas";
import { zhCN } from "@/lib/i18n/zh-CN";
import type { HistoryEntry, RequestSpec } from "@/lib/types";
import { ContextMenu, type MenuItem } from "@/features/ui/ContextMenu";
import { EmptyState } from "@/features/ui/EmptyState";
import { useHistoryStore } from "@/store/history";
import { useRequestStore } from "@/store/request";
import { useUiStore } from "@/store/ui";
import { useWorkspaceStore } from "@/store/workspace";

/**
 * 历史记录列表（现在住在**历史主视图的左侧**那一列）。
 *
 * 三条实现选择：
 *
 * 1. **左键只选中**，详情由右侧主视图显示。回填编辑器移到右键菜单第一项 ——
 *    那个动作会把中间区从「历史」切走、换掉请求草稿，不该是单击的副作用。
 *    第一版是「左键回填」，那在侧栏里说得通（因为没有别处能显示内容）；
 *    搬进主视图之后就成了「点一下，右边还是空的」。
 * 2. **列表是「流」不是「页」**：时间倒序往下滚，滚到底自动续一页。
 *    也保留一个显式的「加载更多」按钮 —— 自动加载依赖滚动事件，
 *    而内容比视口短的时候不会触发滚动，那时按钮是唯一的出路。
 * 3. **分组标题用相对时间**（今天 / 昨天 / 更早）而不是逐条显示完整时间戳：
 *    一列毫秒时间戳没人在读，而「今天 / 昨天」是扫一眼就能用的定位信息。
 */
export function HistoryList() {
  const { t } = useI18n();

  const status = useHistoryStore((s) => s.status);
  const error = useHistoryStore((s) => s.error);
  const items = useHistoryStore((s) => s.items);
  const total = useHistoryStore((s) => s.total);
  const keyword = useHistoryStore((s) => s.keyword);
  const loadingMore = useHistoryStore((s) => s.loadingMore);
  const load = useHistoryStore((s) => s.load);
  /** 当前选中的那一条 —— 详情在主视图右侧渲染，所以选中态在 store 里 */
  const selectedId = useHistoryStore((s) => s.selectedId);
  const select = useHistoryStore((s) => s.select);

  const [menu, setMenu] = useState<{ x: number; y: number; entry: HistoryEntry } | null>(
    null,
  );

  // 首次挂载拉第一页。
  //
  // 依赖工作区 id 而不是「挂载一次」：这两个板块由 Ctrl+2 直接进入时，
  // 应用启动序列（`workspace.load()`）可能还没回来，那一刻没有工作区 id，
  // `load()` 会直接返回 —— 只跑一次的话就会永远卡在转圈上。
  // 搜索词的变化不在这里管，`setKeyword` 自己会重新 load。
  const workspaceId = useWorkspaceStore((s) => s.workspace?.id ?? null);
  useEffect(() => {
    if (workspaceId !== null) void load();
  }, [workspaceId, load]);

  const groups = useMemo(() => groupByDay(items), [items]);
  const hasMore = items.length < total;

  // ---------------------------------------------------------------
  // 自动续页
  // ---------------------------------------------------------------

  const sentinelRef = useRef<HTMLDivElement>(null);
  const loadMore = useHistoryStore((s) => s.loadMore);

  useEffect(() => {
    const el = sentinelRef.current;
    if (!el) return;
    // `rootMargin` 提前 160px 触发：等真的滚到底再发请求，用户会看到一段空白
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) void loadMore();
      },
      { rootMargin: "160px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [loadMore]);

  // ---------------------------------------------------------------
  // 回填 / 删除
  // ---------------------------------------------------------------

  /**
   * 把这一条快照灌进请求编辑器。
   *
   * ⚠️ 只在**右键菜单**里调用，不做单击的副作用。
   *
   * 历史存的是快照，回填出来的草稿没有归属（`sourceId = null`）；
   * 而且这个动作必须离开「历史」板块才看得见结果 —— 也就是说，
   * 单击它等于「用户想看一眼响应，结果被切走了」。所以一并切到「集合」，
   * 让结果出现在用户眼前，而不是在背后悄悄改掉一份草稿。
   */
  const fill = useCallback((entry: HistoryEntry) => {
    const spec = specOf(entry);
    if (!spec) return;
    // 快照和树上任何一行都不对应，别让树高亮着上一个请求 ——
    // 否则会出现「左边选中 A，右边编辑的是另一种内容」
    useUiStore.getState().setActiveRequestId(null);
    useUiStore.getState().setSection("collections");
    useRequestStore.getState().openSpec(spec, entry.requestName);
  }, []);

  const menuItems = useMemo((): MenuItem[] => {
    if (!menu) return [];
    const { entry } = menu;
    return [
      {
        key: "fill",
        label: t("history.fill"),
        icon: CornerUpLeft,
        disabled: specOf(entry) === null,
        onSelect: () => fill(entry),
      },
      {
        key: "copy-url",
        label: t("history.copyUrl"),
        icon: Copy,
        onSelect: () => {
          const url = urlOf(entry);
          if (url !== "") void navigator.clipboard?.writeText(url);
        },
      },
      {
        key: "delete",
        label: t("history.moveToTrash"),
        icon: Trash,
        danger: true,
        separatorBefore: true,
        onSelect: () => void useHistoryStore.getState().remove(entry.id),
      },
    ];
  }, [menu, t, fill]);

  // ---------------------------------------------------------------
  // 三态
  // ---------------------------------------------------------------

  if (status === "idle" || (status === "loading" && items.length === 0)) {
    return <Loading />;
  }

  if (status === "error" && items.length === 0) {
    return (
      <EmptyState
        icon={Clock}
        title={t("history.loadFailed")}
        // 文案来自后端错误码，不在这里写死（见 lib/ipc.ts 的契约）
        description={error?.localized}
        action={
          <button
            type="button"
            onClick={() => void load()}
            className="h-7 rounded-sm border border-line bg-surface px-2.5 text-xs text-ink hover:bg-surface-hover"
          >
            {t("common.retry")}
          </button>
        }
        className="mt-8"
      />
    );
  }

  if (items.length === 0) {
    return keyword === "" ? (
      <EmptyState
        icon={Clock}
        title={t("common.empty")}
        description={t("history.empty")}
        className="mt-8"
      />
    ) : (
      <EmptyState
        icon={SearchX}
        title={t("common.noMatch")}
        description={t("history.noMatchHint", { keyword })}
        className="mt-8"
      />
    );
  }

  return (
    <>
      {/* 计数行。带搜索时用「匹配」而不是「共」—— 后者会让人以为搜索没生效 */}
      <div className="flex items-baseline gap-2 px-1.5 pb-1 pt-0.5">
        <span className="text-[10px] text-ink-faint">
          {keyword === ""
            ? t("history.total", { count: total })
            : t("history.filteredTotal", { count: total })}
        </span>
        {/* 右键菜单没有任何视觉痕迹，得明说一句 —— 否则这个功能等于不存在 */}
        <span className="ml-auto shrink-0 text-[10px] text-ink-faint/70">
          {t("history.rightClickHint")}
        </span>
      </div>

      {groups.map((group) => (
        <div key={group.key}>
          <p className="px-1.5 pb-0.5 pt-2 text-[10px] font-medium tracking-wide text-ink-faint">
            {group.labelKey === null ? group.rawLabel : t(group.labelKey)}
          </p>
          <ul className="flex flex-col gap-px">
            {group.entries.map((entry) => (
              <li key={entry.id}>
                <HistoryRow
                  entry={entry}
                  selected={entry.id === selectedId}
                  onSelect={() => select(entry.id)}
                  onMenu={(e) => {
                    e.preventDefault();
                    e.stopPropagation();
                    setMenu({ x: e.clientX, y: e.clientY, entry });
                  }}
                />
              </li>
            ))}
          </ul>
        </div>
      ))}

      {/* 续页。这个 div 永远渲染 —— 观察器需要它一直在 DOM 里 */}
      <div ref={sentinelRef} className="flex justify-center py-2">
        {hasMore ? (
          loadingMore ? (
            <span className="flex items-center gap-1.5 text-[10px] text-ink-faint">
              <LoaderCircle size={11} strokeWidth={2.2} className="animate-spin" />
              {t("common.loading")}
            </span>
          ) : (
            <button
              type="button"
              onClick={() => void loadMore()}
              className="h-6 rounded-sm border border-line px-2 text-[10px] text-ink-muted hover:bg-surface-hover hover:text-ink"
            >
              {t("history.loadMore")}
            </button>
          )
        ) : (
          <span className="text-[10px] text-ink-faint">{t("history.noMore")}</span>
        )}
      </div>

      {menu ? (
        <ContextMenu
          items={menuItems}
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
        />
      ) : null}
    </>
  );
}

/** 加载态。列表区不能用 EmptyState —— 那看起来像「加载完了，但是空的」 */
function Loading() {
  const { t } = useI18n();
  return (
    <div className="mt-8 flex items-center justify-center gap-2 px-6 text-xs text-ink-faint">
      <LoaderCircle size={13} strokeWidth={2.2} className="animate-spin" />
      <span>{t("common.loading")}</span>
    </div>
  );
}

function HistoryRow({
  entry,
  selected,
  onSelect,
  onMenu,
}: {
  entry: HistoryEntry;
  selected: boolean;
  onSelect: () => void;
  onMenu: (e: React.MouseEvent) => void;
}) {
  const { t } = useI18n();
  const method = methodOf(entry);
  const url = urlOf(entry);
  const point = timePoint(entry.executedAt);

  // 悬停给全：地址 + 出错原因（如果有）+「右键能做什么」。
  // 不再说「点击回填」—— 那已经不是单击的行为了
  const hint = [url, statusTitleOf(entry), t("history.rightClickHint")]
    .filter((line): line is string => line !== undefined && line !== "")
    .join("\n");

  return (
    <button
      type="button"
      onClick={onSelect}
      onContextMenu={onMenu}
      title={hint}
      aria-current={selected ? "true" : undefined}
      className={cn(
        "flex w-full flex-col gap-0.5 rounded-sm px-1.5 py-1 text-left transition-colors duration-75",
        selected ? "bg-brand/12" : "hover:bg-surface-hover",
      )}
    >
      <span className="flex w-full items-center gap-1.5">
        <span
          className={cn(
            "w-[40px] shrink-0 font-mono text-[10px] font-semibold tracking-tight",
            methodTextClass(method),
          )}
        >
          {method}
        </span>
        <span className="min-w-0 flex-1 truncate text-xs text-ink">
          {entry.requestName || url || t("common.unsaved")}
        </span>
        <span className={cn("shrink-0 font-mono text-[10px]", statusClassOf(entry))}>
          {statusLabelOf(entry, t)}
        </span>
      </span>

      <span className="flex w-full items-center gap-1.5 pl-[46px] text-[10px] text-ink-faint">
        <span className="min-w-0 flex-1 truncate">{url || t("common.empty")}</span>
        <span className="tabular shrink-0">
          {entry.durationMs === null ? "" : `${formatMs(entry.durationMs)} · `}
          {entry.respSize === null ? "" : `${formatBytes(entry.respSize)} · `}
          {point}
        </span>
      </span>
    </button>
  );
}

// ---------------------------------------------------------------
// 取值 —— 历史行的所有信息都从快照里来
// ---------------------------------------------------------------

/** 快照 → 可信的 `RequestSpec`；快照坏了（或没有地址）返回 null */
export function specOf(entry: HistoryEntry): RequestSpec | null {
  const parsed = requestSpecSchema.safeParse(entry.snapshot);
  if (!parsed.success) return null;
  return parsed.data.url.trim() === "" ? null : parsed.data;
}

export function methodOf(entry: HistoryEntry): string {
  const raw = entry.snapshot["method"];
  return typeof raw === "string" && raw !== "" ? raw : "GET";
}

export function urlOf(entry: HistoryEntry): string {
  const raw = entry.snapshot["url"];
  return typeof raw === "string" ? raw : "";
}

/**
 * 状态那一栏显示什么。
 *
 * 行内**永远只给一小段**（三位状态码，或「失败」两个字）：
 * 后端文案的长短不可控 —— `E_CONN_REFUSED` 是一整句
 * 「连接被拒绝：端口没有服务在监听，或被防火墙拦住了」，
 * 塞进 10px 的状态位会把整行撑开、时间列全部错位。
 * 完整原因放 `statusTitleOf()`，挂在行的 `title` 上。
 */
export function statusLabelOf(entry: HistoryEntry, t: TranslateFn): string {
  if (entry.status !== null) return String(entry.status);
  return t("history.failed");
}

/**
 * 悬停时给出的完整出错原因。
 *
 * 带占位符的模板（如「域名解析失败：{host}」）在这里补不上参数：
 * 历史行只存了错误码，**没存参数**。露出一个 `{host}` 比不给原因更糟，
 * 所以退回错误码本身 —— 排查时够用了。
 */
function statusTitleOf(entry: HistoryEntry): string | undefined {
  if (entry.error === null) return undefined;
  if (!ipc.isErrorCode(entry.error)) return entry.error;

  const template = zhCN[ipc.ERROR_KEYS[entry.error]];
  return template.includes("{") ? entry.error : template;
}

export function statusClassOf(entry: HistoryEntry): string {
  if (entry.status !== null) return statusTextClass(entry.status);
  return "text-err";
}

// ---------------------------------------------------------------
// 时间
// ---------------------------------------------------------------

interface DayGroup {
  key: string;
  /** 今天/昨天用 key；更早的日期直接显示 `MM-DD`，所以 `labelKey` 为 null */
  labelKey: "history.today" | "history.yesterday" | "history.earlier" | null;
  rawLabel: string;
  entries: HistoryEntry[];
}

/**
 * 按「天」分组。
 *
 * 后端已经按执行时间倒序返回，所以这里**只做相邻归并**，不排序 ——
 * 再排一次会把「跨页续上来的那条」重新洗一遍，用户正看着的行会跳位置。
 */
function groupByDay(items: HistoryEntry[]): DayGroup[] {
  const today = startOfDay(new Date());
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);

  const out: DayGroup[] = [];
  for (const entry of items) {
    const at = new Date(entry.executedAt);
    const day = startOfDay(at);

    let key: string;
    let labelKey: DayGroup["labelKey"];
    let rawLabel = "";

    if (day.getTime() === today.getTime()) {
      key = "today";
      labelKey = "history.today";
    } else if (day.getTime() === yesterday.getTime()) {
      key = "yesterday";
      labelKey = "history.yesterday";
    } else {
      key = `${day.getFullYear()}-${day.getMonth() + 1}-${day.getDate()}`;
      // 跨年的显示年份，否则只显示月日 —— 「03-14」在今年是有意义的，在去年不是
      labelKey = null;
      rawLabel =
        day.getFullYear() === today.getFullYear()
          ? `${pad(day.getMonth() + 1)}-${pad(day.getDate())}`
          : `${day.getFullYear()}-${pad(day.getMonth() + 1)}-${pad(day.getDate())}`;
    }

    const last = out[out.length - 1];
    if (last && last.key === key) last.entries.push(entry);
    else out.push({ key, labelKey, rawLabel, entries: [entry] });
  }
  return out;
}

function startOfDay(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate());
}

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

/** 行内的时间：同一天给 `HH:mm`，更早给 `MM-DD HH:mm` */
function timePoint(iso: string): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return "";
  const clock = `${pad(at.getHours())}:${pad(at.getMinutes())}`;
  const today = startOfDay(new Date());
  return startOfDay(at).getTime() === today.getTime()
    ? clock
    : `${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${clock}`;
}
