import { CornerDownLeft, Search } from "lucide-react";
import {
  useEffect,
  useMemo,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

import { cn } from "@/lib/cn";
import { formatShortcut } from "@/lib/constants";
import { methodTextClass } from "@/lib/http";
import { useI18n, type TranslateFn } from "@/lib/i18n";
import type { ApiRequest, Folder } from "@/lib/types";
import { useRequestStore } from "@/store/request";
import { useUiStore } from "@/store/ui";
import { useWorkspaceStore } from "@/store/workspace";
import { fuzzyScore, mergePositions } from "@/features/command/fuzzy";
import { PALETTE_COMMANDS, type PaletteGroup } from "@/features/command/registry";

/**
 * 输入关键字后最多展示多少条。
 *
 * 全量候选是「13 条命令 + 全部请求」，一次铺满会拖慢滚动、也看不清。
 * 超过就靠继续输入收窄 —— 模糊排序的意义正在于此。
 */
const RESULT_LIMIT = 50;

/**
 * **空查询**时接口区先摆几条。
 *
 * 空手按 Ctrl+K 多半是想切板块或新建，把所有接口铺满一屏反而把命令挤没了。
 */
const IDLE_REQUEST_LIMIT = 8;

interface PaletteItem {
  key: string;
  label: string;
  /** 右侧灰字：命令是快捷键，接口是所在分类（或命中的地址） */
  hint?: string;
  icon: ReactNode;
  /** 命中位置（下标对应 `label`），用于高亮 */
  positions: number[];
  /** 展平后的下标。键盘导航与 `aria-activedescendant` 都用它 */
  index: number;
  run: () => void;
}

interface PaletteGroupView {
  key: string;
  /** 分组标题。空串 = 不画标题 */
  label: string;
  items: PaletteItem[];
}

/**
 * 命令面板。
 *
 * 「关闭即卸载」是刻意的：查询串与选中位置都是**一次性**状态，
 * 关掉就该消失。挂载时初始化、卸载时丢掉，比在打开时手写 reset 更难漏字段
 * （漏一个就会出现「上次搜的词还在」）。
 */
export function CommandPalette() {
  const open = useUiStore((s) => s.paletteOpen);
  if (!open) return null;
  return <PalettePanel />;
}

function PalettePanel() {
  const { t } = useI18n();
  const close = useUiStore((s) => s.closePalette);
  const requests = useWorkspaceStore((s) => s.requests);
  const folders = useWorkspaceStore((s) => s.folders);

  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);

  const groups = useMemo(
    () => buildGroups({ query, t, folders, requests }),
    [query, t, folders, requests],
  );

  /**
   * 键盘导航走**展平后**的下标，而不是「第几组第几项」。
   *
   * 分组只影响渲染，移动逻辑必须是一条连续的线 —— 否则「在最后一组底部按下 ↓」
   * 要写一堆跨组判断。
   */
  const total = useMemo(
    () => groups.reduce((n, g) => n + g.items.length, 0),
    [groups],
  );

  // 候选变了就把选中项拉回第一行：停在原来的下标上会指到另一条命令，
  // 用户接着按回车就执行错了
  useEffect(() => {
    setCursor(0);
  }, [query]);

  const active = total === 0 ? -1 : Math.min(cursor, total - 1);

  // 键盘移到屏幕外的项时把它滚进来。用 `nearest` —— `center` 会让列表
  // 每按一次方向键就整体跳一下
  useEffect(() => {
    if (active < 0) return;
    document
      .getElementById(itemDomId(active))
      ?.scrollIntoView({ block: "nearest" });
  }, [active, total]);

  /**
   * Esc 必须**在捕获阶段**吃掉。
   *
   * `App.tsx` 的全局 keydown 把 Esc 绑成了「取消当前请求」，
   * 放任它冒到那儿的话，关面板会顺手把正在跑的请求掐掉。
   * `Dialog` 用的是同一招。
   */
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      close();
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [close]);

  const runItem = (item: PaletteItem) => {
    // 先关再执行：面板要执行的动作有的会弹对话框（导入 / 新建），
    // 面板留在上面会挡住它们
    close();
    item.run();
  };

  const pick = (index: number) => {
    const item = findItem(groups, index);
    if (item) runItem(item);
  };

  const onKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key === "ArrowDown" || (e.key === "Tab" && !e.shiftKey)) {
      e.preventDefault();
      setCursor((c) => Math.min(c + 1, total - 1));
      return;
    }
    if (e.key === "ArrowUp" || (e.key === "Tab" && e.shiftKey)) {
      e.preventDefault();
      setCursor((c) => Math.max(c - 1, 0));
      return;
    }
    if (e.key === "PageDown") {
      e.preventDefault();
      setCursor((c) => Math.min(c + 10, total - 1));
      return;
    }
    if (e.key === "PageUp") {
      e.preventDefault();
      setCursor((c) => Math.max(c - 10, 0));
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      pick(active);
    }
  };

  return createPortal(
    <div
      style={{ zIndex: "var(--z-dialog)" }}
      className="fixed inset-0 flex items-start justify-center bg-scrim p-6 pt-[12vh] animate-fade-in"
      onPointerDown={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={t("palette.placeholder")}
        onKeyDown={onKeyDown}
        className="flex max-h-[70vh] w-[560px] max-w-full origin-top flex-col overflow-hidden rounded-lg border border-line bg-surface shadow-e3 animate-pop-in"
      >
        <div className="flex items-center gap-2 border-b border-line px-3">
          <Search size={14} strokeWidth={2} className="shrink-0 text-ink-faint" />
          <input
            autoFocus
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("palette.placeholder")}
            spellCheck={false}
            role="combobox"
            aria-expanded
            aria-controls="command-palette-list"
            aria-activedescendant={active >= 0 ? itemDomId(active) : undefined}
            className="h-11 flex-1 bg-transparent text-[13px] text-ink outline-none placeholder:text-ink-faint"
          />
          <kbd className="shrink-0 rounded-sm border border-line px-1.5 py-0.5 text-[10px] text-ink-faint">
            Esc
          </kbd>
        </div>

        <div
          id="command-palette-list"
          role="listbox"
          className="min-h-0 flex-1 overflow-y-auto py-1"
        >
          {total === 0 ? (
            <p className="px-3 py-8 text-center text-xs text-ink-faint">
              {t("palette.empty")}
            </p>
          ) : (
            groups.map((group) => (
              <div key={group.key}>
                {group.label ? (
                  <p className="px-3 pb-1 pt-2 text-[10px] font-medium tracking-wide text-ink-faint">
                    {group.label}
                  </p>
                ) : null}
                {group.items.map((item) => (
                  <ItemRow
                    key={item.key}
                    item={item}
                    active={item.index === active}
                    onHover={setCursor}
                    onPick={() => runItem(item)}
                  />
                ))}
              </div>
            ))
          )}
        </div>

        <p className="flex shrink-0 items-center gap-1 border-t border-line px-3 py-1.5 text-[11px] text-ink-faint">
          <CornerDownLeft size={11} strokeWidth={2} />
          {t("palette.footerHint")}
        </p>
      </div>
    </div>,
    document.body,
  );
}

/** 列表项的 DOM id —— `aria-activedescendant` 与滚动定位都靠它 */
function itemDomId(index: number): string {
  return `command-palette-item-${index}`;
}

/** 按下标取项（展平后）。下标越界返回 undefined —— 调用方自己判 */
function findItem(
  groups: readonly PaletteGroupView[],
  index: number,
): PaletteItem | undefined {
  for (const group of groups) {
    if (index < group.items.length) return group.items[index];
    index -= group.items.length;
  }
  return undefined;
}

function ItemRow({
  item,
  active,
  onHover,
  onPick,
}: {
  item: PaletteItem;
  active: boolean;
  onHover: (index: number) => void;
  onPick: () => void;
}) {
  return (
    <div
      id={itemDomId(item.index)}
      role="option"
      aria-selected={active}
      onMouseEnter={() => onHover(item.index)}
      // 用 pointerdown 而不是 click：click 要等鼠标抬起，
      // 而「按下就执行」更跟手
      onPointerDown={(e) => {
        e.preventDefault();
        onPick();
      }}
      className={cn(
        "flex cursor-pointer items-center gap-2 px-3 py-1.5",
        active ? "bg-surface-hover" : "bg-transparent",
      )}
    >
      <span className="shrink-0 text-ink-faint">{item.icon}</span>
      <span className="min-w-0 flex-1 truncate text-xs text-ink">
        <Highlight text={item.label} positions={item.positions} />
      </span>
      {item.hint ? (
        <span className="max-w-[45%] shrink-0 truncate text-[11px] text-ink-faint">
          {item.hint}
        </span>
      ) : null}
    </div>
  );
}

/** 把命中的字符染成品牌色。未命中时原样输出（不包任何标签） */
function Highlight({
  text,
  positions,
}: {
  text: string;
  positions: readonly number[];
}) {
  if (positions.length === 0) return <>{text}</>;

  const out: ReactNode[] = [];
  let at = 0;
  for (const [start, end] of mergePositions(positions)) {
    if (start > at) out.push(text.slice(at, start));
    out.push(
      <span key={start} className="font-medium text-brand">
        {text.slice(start, end + 1)}
      </span>,
    );
    at = end + 1;
  }
  if (at < text.length) out.push(text.slice(at));
  return <>{out}</>;
}

/** HTTP 方法徽标。颜色走 `--m-*` 令牌（`lib/http.ts` 的映射） */
function MethodBadge({ method }: { method: string }) {
  return (
    <span
      className={cn(
        "inline-block w-12 shrink-0 text-[10px] font-semibold tracking-tight",
        methodTextClass(method),
      )}
    >
      {method.toUpperCase()}
    </span>
  );
}

// ---------------------------------------------------------------
// 候选构造
// ---------------------------------------------------------------

function buildGroups(args: {
  query: string;
  t: TranslateFn;
  folders: Folder[];
  requests: ApiRequest[];
}): PaletteGroupView[] {
  const { query, t, folders, requests } = args;

  // ---- 命令 ----
  const commands: Array<{ item: Omit<PaletteItem, "index">; score: number; order: number; group: PaletteGroup }> =
    [];
  PALETTE_COMMANDS.forEach((cmd, order) => {
    // 当前状态下没意义的命令直接不出现（见 `PaletteCommand.enabled`）
    if (cmd.enabled && !cmd.enabled()) return;

    const label = t(cmd.titleKey);
    const haystack = cmd.keywords?.length
      ? `${label} ${cmd.keywords.join(" ")}`
      : label;
    const hit = fuzzyScore(query, haystack);
    if (!hit) return;

    // 高亮只认 `label` 上的命中：命中的是别名时 label 里一个字都没亮起来，
    // 看起来像「匹配错了」。别名只参与「能不能搜到」，不参与高亮。
    const labelHit = fuzzyScore(query, label);

    commands.push({
      item: {
        key: cmd.id,
        label,
        hint: cmd.shortcut ? formatShortcut(cmd.shortcut) : undefined,
        icon: <cmd.icon size={14} strokeWidth={1.9} />,
        positions: labelHit?.positions ?? [],
        run: cmd.run,
      },
      score: hit.score,
      order,
      group: cmd.group,
    });
  });

  // ---- 接口 ----
  const folderById = new Map(folders.map((f) => [f.id, f]));
  const pathCache = new Map<number, string>();

  const requests_: Array<{
    item: Omit<PaletteItem, "index">;
    score: number;
    order: number;
  }> = [];

  requests.forEach((request, order) => {
    const path =
      request.folderId === null
        ? t("palette.unfiled")
        : folderPathOf(request.folderId, folderById, pathCache);

    // 名字优先、其次地址、最后分类路径。三者只取第一个命中的 ——
    // 名字命中了就足够说明「用户找的就是它」，再比 URL 上的分数只会让排序难解释
    const nameHit = fuzzyScore(query, request.name);
    const urlHit = nameHit ? null : fuzzyScore(query, request.url);
    const pathHit = !nameHit && !urlHit ? fuzzyScore(query, path) : null;
    if (query !== "" && !nameHit && !urlHit && !pathHit) return;

    // 权重递减：地址 / 路径上的命中是「次优证据」，不该盖过名字直接命中的那条
    const score = nameHit
      ? nameHit.score
      : urlHit
        ? urlHit.score * 0.7
        : pathHit
          ? pathHit.score * 0.5
          : 0;

    requests_.push({
      item: {
        key: `request:${request.id}`,
        label: request.name,
        // 名字没命中说明是地址 / 分类命中的，把地址显在右侧 ——
        // 否则「它为什么会被搜出来」看不出来
        hint: nameHit ? path : request.url || path,
        icon: <MethodBadge method={request.method} />,
        positions: nameHit?.positions ?? [],
        run: () => openRequestInEditor(request),
      },
      score,
      order,
    });
  });

  const groups: PaletteGroupView[] = [];

  // ---- 输入了关键字：命令与接口统一按分数排，谁最匹配谁在最上面 ----
  //
  // 刻意**不分组**：分组会让「接口名里正好含关键字」永远排在所有命令后面，
  // 而那恰恰是用户最想找的一条
  if (query !== "") {
    const all = [...commands, ...requests_]
      .sort((a, b) => b.score - a.score || a.order - b.order)
      .slice(0, RESULT_LIMIT);
    if (all.length > 0) {
      groups.push({
        key: "results",
        label: "",
        items: all.map((x) => ({ ...x.item, index: 0 })),
      });
    }
  } else {
    // ---- 空查询：按「前往 / 命令 / 应用 / 接口」分组摆出来 ----
    const byGroup: Record<PaletteGroup, PaletteItem[]> = {
      go: [],
      action: [],
      app: [],
    };
    for (const c of commands) {
      byGroup[c.group].push({ ...c.item, index: 0 });
    }

    const shown = requests_.slice(0, IDLE_REQUEST_LIMIT);
    const restCount = requests_.length - shown.length;
    const requestItems: PaletteItem[] = shown.map((r) => ({
      ...r.item,
      index: 0,
    }));
    if (restCount > 0) {
      requestItems.push({
        key: "more",
        label: t("palette.hintMore", { count: restCount }),
        icon: null,
        positions: [],
        index: 0,
        // 这一条只是提示，不是可执行的命令：点了关掉面板反而莫名其妙，
        // 所以什么都不做（用户会自然地去打字）
        run: () => {},
      });
    }

    const sections: Array<[string, string, PaletteItem[]]> = [
      ["go", t("palette.groupGo"), byGroup.go],
      ["action", t("palette.groupAction"), byGroup.action],
      ["app", t("palette.groupApp"), byGroup.app],
      ["request", t("palette.groupRequest"), requestItems],
    ];
    for (const [key, label, items] of sections) {
      if (items.length > 0) groups.push({ key, label, items });
    }
  }

  // 展平后的下标统一在这里赋值 —— 让「渲染顺序」与「键盘顺序」
  // 只有一个来源，不然两处各算一遍迟早对不上
  let next = 0;
  for (const group of groups) {
    for (const item of group.items) {
      item.index = next;
      next += 1;
    }
  }

  return groups;
}

/**
 * 分类的完整路径（`父 / 子`）。
 *
 * 带缓存是因为一次按键要给每条请求各算一遍路径，
 * 而同一条路径会被同一分类下的所有请求重复用到。
 */
function folderPathOf(
  folderId: number,
  byId: Map<number, Folder>,
  cache: Map<number, string>,
): string {
  const cached = cache.get(folderId);
  if (cached !== undefined) return cached;

  const parts: string[] = [];
  let cur = byId.get(folderId);
  // `guard` 防环：导入来的脏数据可能让 parentId 成环
  //（与后端用 `UNION` 而非 `UNION ALL`、`folderSubtree` 用 `Set` 同理）
  for (let guard = 0; cur && guard <= byId.size; guard += 1) {
    parts.unshift(cur.name);
    cur = cur.parentId === null ? undefined : byId.get(cur.parentId);
  }

  const path = parts.join(" / ");
  cache.set(folderId, path);
  return path;
}

/**
 * 在编辑器里打开某个请求。
 *
 * 与集合树里的 `openInEditor` 是同一组动作（树上的高亮 + 编辑器里的内容），
 * 少了后者就会出现「树亮着 A、右边显示 B」。
 *
 * 顺带切回集合板块并展开侧栏：面板是从任意板块呼出来的，
 * 不切的话用户会看到编辑器换了内容、而左边还停在回收站。
 */
function openRequestInEditor(request: ApiRequest): void {
  const ui = useUiStore.getState();
  ui.setSection("collections");
  if (ui.sidebarCollapsed) ui.setSidebarCollapsed(false);
  ui.setActiveRequestId(request.id);
  useRequestStore.getState().openRequest(request);
}
