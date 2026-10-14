import { Check, ChevronDown, Search, TriangleAlert } from "lucide-react";
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";

import { cn } from "@/lib/cn";
import { Badge } from "@/features/ui/Badge";
import { methodTextClass } from "@/lib/http";
import { useI18n } from "@/lib/i18n";
import type { ApiRequest } from "@/lib/types";
import { readScopeWords } from "@/features/token/scope";

/**
 * 面板宽度下限。
 *
 * 定在 420 而不是「与触发器同宽」：触发器可能只有 200px 宽（表单里跟名称挤一行），
 * 那个宽度下地址只能看到域名 —— 而域名恰好是两条候选里最像的一段
 * （`api.` 与 `apidemo.` 只差四个字母，一个是生产一个是测试）。
 */
const PANEL_MIN_WIDTH = 420;
/** 上限。再宽只是把空白铺开，扫起来反而更慢 */
const PANEL_MAX_WIDTH = 640;
/**
 * 列表高度上限。超出就滚 —— 候选铺满整个请求区就没法一边看表单一边选了。
 *
 * 定在 400 而不是 260：**每一项现在有三行**（方法+名字 / 地址 / scope），
 * 行高约 60px。260 只装得下四行，滚动条一竖起来，「比一比哪条的 scope 覆盖了
 * 目标服务」这件事反而要来回滚 —— 而比较正是这个面板存在的理由。
 */
const LIST_MAX_HEIGHT = 400;
const MARGIN = 8;

export interface RequestPickerProps {
  /** 当前选中的请求 id。`0` = 没选 */
  value: number;
  onChange: (id: number) => void;
  /**
   * 候选。**由调用方决定给哪些** —— 这里是系统分类「token刷新」里的请求，
   * 表单自己不去全局捞：全工作区 145 条铺开没人找得到要的那条。
   */
  requests: readonly ApiRequest[];
  /**
   * 当前值指向的请求。
   *
   * 三种情况在这一处分清（**混起来会撒谎**）：
   * * `current` 在 `requests` 里 —— 正常选中
   * * `current` 不在 —— 老配置指向分类外的请求。后端只认 `fetch_request_id`、
   *   根本不看分类，所以它**照样在正常工作**，界面不能把它说成已删除
   *   （真库里 146 条 Token 源有 32 条是这种，全是播种脚本造的）
   * * `current === null` —— 那个请求真的没了
   */
  current: ApiRequest | null;
  /** 原生 tooltip。用来承载「说明」，省掉控件下面那一行小字 */
  title?: string;
  className?: string;
}

/**
 * 「选一个取 Token 的接口」的下拉。
 *
 * ## 为什么不用原生 `<select>`
 *
 * 原生 select 的每一项只能是**一行纯文本**，而选这个接口要同时看三件事：
 *
 * 1. **是哪个接口** —— 名字（`生产拿token` / `测试环境拿token (2)`）
 * 2. **打到哪个环境** —— 地址里的 host（`api.example.com` 与
 *    `apidemo.example.com` 只差几个字母，一个是生产一个是测试）
 * 3. **拿回来的 Token 能调什么服务** —— 请求体里的 `scope`
 *
 * 这三件事恰好是「选错」的全部成因，而现象都是发送后 401、界面上一切正常。
 *
 * ## 为什么是 portal + fixed
 *
 * 面板住在「请求头」页最下面的折叠块里：往下展开会被窗口底裁掉，往右展开会被
 * 表单容器裁掉。`TokenGate` / `ContextMenu` 用的是同一套做法。
 */
export function RequestPicker({
  value,
  onChange,
  requests,
  current,
  title,
  className,
}: RequestPickerProps) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const anchorRef = useRef<HTMLDivElement>(null);

  /**
   * 候选 + 各自的 scope 词。
   *
   * 抽 scope 要做一次切分与解码，整表算一遍不是零成本；而 `requests` 的引用
   * 只在工作区重新加载时变 —— 挂在这上面就够了。
   */
  const items = useMemo(
    () => requests.map((request) => ({ request, words: readScopeWords(request) })),
    [requests],
  );

  const inCandidates = items.find((it) => it.request.id === value) ?? null;
  const currentWords = useMemo(() => (current ? readScopeWords(current) : []), [current]);
  const selected = inCandidates ?? (current ? { request: current, words: currentWords } : null);

  /** 选着一个老配置：请求还在，只是不在「token 刷新」分类里 */
  const outsideFolder = current !== null && inCandidates === null;
  /** 选着一个不存在的 id */
  const gone = value !== 0 && current === null;

  return (
    <div ref={anchorRef} className={cn("relative", className)}>
      {/*
        `min-h-8` 而不是 `h-8`：没选时它和旁边那个输入框一样高（32px），
        选完才长到两行。高度跟着信息量走，而不是一上来就占两行。
      */}
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="listbox"
        aria-expanded={open}
        title={title}
        className={cn(
          "flex min-h-8 w-full items-start gap-2 rounded-sm border bg-surface px-2 py-1 text-left",
          "outline-none transition-colors duration-100",
          "border-line hover:border-line-strong",
          open && "border-brand",
        )}
      >
        <span className="min-w-0 flex-1">
          {selected ? (
            <>
              <span className="flex min-w-0 items-center gap-1.5">
                <MethodLabel method={selected.request.method} />
                <span className="min-w-0 flex-1 truncate text-xs text-ink">
                  {selected.request.name}
                </span>
                {outsideFolder ? (
                  // lucide 图标只吃 SVG 属性，`title` 得挂在外面的元素上
                  <span className="shrink-0" title={t("token.pickerOutOfFolder")}>
                    <TriangleAlert size={11} strokeWidth={2.2} aria-hidden className="text-warn" />
                  </span>
                ) : null}
                <ScopeChips words={selected.words} max={2} />
              </span>
              {/* 地址用 truncate 而不是折行：触发器折成三行会把下面
                  「从响应里取什么」那一组整体推下去。完整地址在面板里是通栏的 */}
              <span
                className="mt-0.5 block truncate font-mono text-[10px] text-ink-faint"
                title={selected.request.url}
              >
                {selected.request.url}
              </span>
            </>
          ) : gone ? (
            <span className="flex items-start gap-1.5 py-0.5 text-[11px] text-warn">
              <TriangleAlert size={12} strokeWidth={2.2} aria-hidden className="mt-px shrink-0" />
              {t("token.fetchRequestGone")}
            </span>
          ) : (
            <span className="block py-0.5 text-xs text-ink-faint">{t("token.pickRequest")}</span>
          )}
        </span>

        <ChevronDown
          size={13}
          strokeWidth={2.4}
          aria-hidden
          className={cn(
            "mt-1 shrink-0 text-ink-faint transition-transform duration-100",
            open && "rotate-180",
          )}
        />
      </button>

      {open ? (
        <PickerPanel
          items={items}
          value={value}
          gone={gone}
          pinned={outsideFolder ? { request: current, words: currentWords } : null}
          anchorRef={anchorRef}
          onPick={(id) => {
            onChange(id);
            setOpen(false);
          }}
          onClose={() => setOpen(false)}
        />
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------
// 触发器与列表里共用的两小块
// ---------------------------------------------------------------

function MethodLabel({ method }: { method: string }) {
  return (
    <span
      className={cn(
        "shrink-0 font-mono text-[10px] font-semibold tracking-tight",
        methodTextClass(method),
      )}
    >
      {method.toUpperCase()}
    </span>
  );
}

/**
 * `scope` 词。
 *
 * 空的时候**必须**说话（「无 scope」）而不是不显示 —— 这恰好是最该被看见的一种：
 * 那条取 Token 请求谁的服务都调不通，而它在列表里和配好的那些长得一模一样。
 *
 * `max` / `wrap` 是「一行里挤得下多少」的两种口径：
 * * 触发器：两行高、旁边还挤着名称 ⇒ 传 `max`，放不下的折成 `+3`
 * * 面板：scope **独占一行** ⇒ 两个都不传，有几个铺几个，一行不够就换行
 *
 * 面板里不折叠是这个控件的重点：被折掉的那几个恰好是「这个 Token 能不能调
 * 那个服务」的唯一判据。
 */
function ScopeChips({
  words,
  max,
  wrap = false,
}: {
  words: string[];
  max?: number;
  wrap?: boolean;
}) {
  const { t } = useI18n();

  if (words.length === 0) {
    return (
      <span
        className={cn("text-[10px] text-ink-faint", !wrap && "shrink-0")}
        title={t("token.pickerScopeHint")}
      >
        {t("token.pickerScopeNone")}
      </span>
    );
  }

  const shown = max === undefined ? words : words.slice(0, max);
  const rest = words.length - shown.length;

  return (
    <span className={cn("flex items-center gap-1", wrap ? "flex-wrap" : "shrink-0")}>
      {shown.map((word) => (
        <span
          key={word}
          title={t("token.pickerScopeHint")}
          className="rounded-xs border border-brand/25 bg-brand-subtle px-1 font-mono text-[10px] text-brand-on"
        >
          {word}
        </span>
      ))}
      {rest > 0 ? <span className="text-[10px] text-ink-faint">+{rest}</span> : null}
    </span>
  );
}

// ---------------------------------------------------------------
// 弹出面板
// ---------------------------------------------------------------

/**
 * 列表的一行。
 *
 * `request === null` = 当前那个 id 指向的请求已经没了（选不动的一行）。
 * `pinned` = 它是「当前在用、但不在候选里」的老配置，钉在候选上方。
 */
interface Row {
  key: string;
  request: ApiRequest | null;
  words: string[];
  pinned?: boolean;
}

interface PickerPanelProps {
  items: readonly { request: ApiRequest; words: string[] }[];
  value: number;
  gone: boolean;
  pinned: { request: ApiRequest; words: string[] } | null;
  anchorRef: RefObject<HTMLDivElement | null>;
  onPick: (id: number) => void;
  onClose: () => void;
}

function PickerPanel({ items, value, gone, pinned, anchorRef, onPick, onClose }: PickerPanelProps) {
  const { t } = useI18n();
  const ref = useRef<HTMLDivElement>(null);
  /** 候选列表那一格。键盘跟随滚动只动它（见下面那个 effect） */
  const listRef = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const [pos, setPos] = useState<PanelPos | null>(null);

  /**
   * 过滤。
   *
   * 名字、地址、方法、scope 四样都参与 —— 输入 `masterdata2` 时想找的其实是
   * 「scope 覆盖它的那条」，而那时它的名字里可能一个字都不含。
   */
  const rows = useMemo<Row[]>(() => {
    const q = query.trim().toLowerCase();
    const matched = items.filter(({ request, words }) => {
      if (q === "") return true;
      return (
        request.name.toLowerCase().includes(q) ||
        request.url.toLowerCase().includes(q) ||
        request.method.toLowerCase().includes(q) ||
        words.some((word) => word.includes(q))
      );
    });

    const list: Row[] = matched.map(({ request, words }) => ({
      key: `r${request.id}`,
      request,
      words,
    }));

    // 钉住的那一行只在没搜索时出现：它是「你现在选的是这么个东西」的状态说明。
    // 一旦开始打字，用户是在找替代品，把一行位置固定不变的项压在顶上只会挡路
    const head: Row[] = [];
    if (q === "") {
      if (gone) head.push({ key: "gone", request: null, words: [] });
      else if (pinned) {
        head.push({ key: "pinned", request: pinned.request, words: pinned.words, pinned: true });
      }
    }

    return [...head, ...list];
  }, [items, query, gone, pinned]);

  // 候选变了就把光标拉回第一行。停在原下标上会指到另一条，
  // 用户接着按回车就选中了错的那条（与命令面板同一条判断）
  useEffect(() => {
    setCursor(0);
  }, [query]);

  const active = rows.length === 0 ? -1 : Math.min(cursor, rows.length - 1);

  /**
   * 光标移到列表可视区外的项时把它滚进来。
   *
   * ⚠️ **只滚列表容器，不用 `scrollIntoView`**。后者的语义是「把元素滚进
   * 所有可滚祖先的视口」，会连带滚动外层页面 —— 而页面一滚，这个面板
   * 就没了（见下面那条 scroll 监听的说明）。手工算 `scrollTop` 的代价是
   * 六行代码，换来的是「键盘选到底部时面板不会自己消失」。
   */
  useEffect(() => {
    if (active < 0) return;
    const list = listRef.current;
    const el = document.getElementById(optionDomId(active));
    if (!list || !el) return;

    const listRect = list.getBoundingClientRect();
    const elRect = el.getBoundingClientRect();
    // `nearest` 口径：只补差值，已经在视口里就不动它
    if (elRect.top < listRect.top) list.scrollTop -= listRect.top - elRect.top;
    else if (elRect.bottom > listRect.bottom) list.scrollTop += elRect.bottom - listRect.bottom;
  }, [active, rows.length]);

  /**
   * 定位。
   *
   * **向上弹**是必须的，不是保险：这块表单住在「请求头」页最底部，
   * 它下方只剩窗口边框 —— 向下展开的面板会有一半在屏幕外。
   */
  useLayoutEffect(() => {
    const rect = anchorRef.current?.getBoundingClientRect();
    if (rect) setPos(anchorPos(rect));
  }, [anchorRef]);

  /** 面板高度上限：贴着触发器的那一侧还剩多少 */
  const maxHeight = useMemo(() => {
    const rect = anchorRef.current?.getBoundingClientRect();
    if (!rect || !pos) return LIST_MAX_HEIGHT;
    const space = pos.up ? rect.top - MARGIN * 2 : window.innerHeight - rect.bottom - MARGIN * 2;
    return Math.max(160, Math.min(LIST_MAX_HEIGHT + 60, space));
  }, [anchorRef, pos]);

  // 关闭时机与 `ContextMenu` / `TokenGate` 一致：外部按下 / Esc / 滚动 / 尺寸变化。
  // 用 pointerdown 而不是 click —— 等 click 才关会先触发底层元素的 click
  useEffect(() => {
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node;
      if (ref.current?.contains(target)) return;
      if (anchorRef.current?.contains(target)) return; // 留给「再点一次收起」
      onClose();
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation(); // 别冒泡成 App 那条「取消请求」
        onClose();
      }
    };

    /**
     * 滚动。
     *
     * ## 面板内部的滚动**不算**「外面动了」
     *
     * `capture` 阶段的 window 监听能收到**任何**滚动 —— 包括这个面板自己那个
     * 列表的。第一版就是一棵树上吊死的写法：`window.addEventListener("scroll",
     * onClose, true)` 一句，于是「用滚轮滚一下候选列表」和「光标走到需要滚动
     * 才看得见的最后一项」都会当场把面板关掉，而用户只是想看看下面还有什么。
     *
     * ## 外部滚动时**跟着走**，而不是关掉
     *
     * 面板是 `fixed` 的，不跟随就会与触发器错位。但「错位」比「突然消失」好得多，
     * 而两者都不是唯一选项 —— 锚点还看得见就重算位置（视觉上跟着触发器走），
     * 只有它滚出视口才关：那时候面板已经没有任何参照物了。
     *
     * 顺带解决第三种情况：候选少到不需要滚动时，滚轮的滚动会**溢出到页面**
     * （面板吃不下这次滚动），旧写法同样会把自己关掉。
     */
    const onScroll = (e: Event) => {
      const target = e.target;
      if (target instanceof Node && ref.current?.contains(target)) return;

      const rect = anchorRef.current?.getBoundingClientRect();
      if (rect && rect.bottom > 0 && rect.top < window.innerHeight) {
        setPos(anchorPos(rect));
        return;
      }
      onClose();
    };

    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onClose);
    };
  }, [anchorRef, onClose]);

  const showQuery = items.length > 6;

  /**
   * 方向键与回车挂在 **window** 上，而不是面板元素上。
   *
   * 面板要能用键盘，前提是事件能送到它手里。而焦点落不落得到面板，取决于
   * 打开它的那一下 —— 鼠标点击由浏览器决定把焦点留给触发器，而那个时机
   * 在 input 序列里、晚于 React 的 effect。**实测**：候选少时不渲染搜索框，
   * 面板自己那次 `focus()` 会被随后的默认聚焦盖掉，于是 ↑↓ 全无反应，
   * 而面板看起来一切正常（`activeElement` 是那个 `<button>`）。
   *
   * 把键位绑在焦点上，就等于把键盘可用性押在那场竞态上。Esc 本来就是 window
   * 监听，键位跟着它走，口径也统一。
   */
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setCursor((c) => Math.min(c + 1, rows.length - 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setCursor((c) => Math.max(c - 1, 0));
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const row = rows[active];
        if (row?.request) onPick(row.request.id);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [rows, active, onPick]);

  /**
   * 焦点仍然尽力交给面板 —— 这一条是为了无障碍，**不是**键盘能不能用的前提。
   *
   * `aria-activedescendant` 要求持焦的是输入框或列表本身，读屏软件靠它念出
   * 当前选中的是哪一项。拿不到也不影响操作（上面那套 window 监听照常工作）。
   */
  useEffect(() => {
    if (!showQuery) ref.current?.focus();
  }, [showQuery]);

  return createPortal(
    <div
      ref={ref}
      role="listbox"
      aria-label={t("token.fetchRequest")}
      // 可聚焦是 `aria-activedescendant` 模式的要求：持焦的必须是输入框或列表本身
      tabIndex={-1}
      style={{
        left: pos?.left ?? -9999,
        top: pos?.top ?? -9999,
        width: pos?.width,
        maxHeight,
        zIndex: "var(--z-dropdown)",
        // 量出位置之前先藏起来，否则会看到面板在别处闪一下
        visibility: pos ? "visible" : "hidden",
        // 上翻靠这个 inline transform 实现，所以入场动画**只能**用 fade-in：
        // CSS 动画的 transform 与行内 transform 会互相覆盖（行内优先级更高，
        // 但 transform 属于可动画属性，动画期间由关键帧接管）——
        // 两者同时生效时面板会先按关键帧的 scale 弹一下再跳到 translateY(-100%)，
        // 表现为「弹出来又瞬移到上方」。只给淡入就没有这个冲突。
        transform: pos?.up ? "translateY(-100%)" : undefined,
      }}
      className="fixed flex flex-col overflow-hidden rounded-md border border-line bg-surface shadow-e3 outline-none animate-fade-in"
    >
      {/* 候选只有几条时搜索框纯属占地方 —— 一眼能看完的东西不需要再过滤一次 */}
      {showQuery ? (
        <div className="flex shrink-0 items-center gap-2 border-b border-line px-2.5">
          <Search size={12} strokeWidth={2.2} aria-hidden className="shrink-0 text-ink-faint" />
          <input
            autoFocus
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("token.pickerSearch")}
            spellCheck={false}
            autoComplete="off"
            aria-activedescendant={active >= 0 ? optionDomId(active) : undefined}
            className="h-8 min-w-0 flex-1 bg-transparent text-xs text-ink outline-none placeholder:font-sans placeholder:text-ink-faint"
          />
        </div>
      ) : null}

      {/* `overscroll-contain`：滚到两端时别把滚动链交给页面 —— 页面一动
          就会走上面那条 scroll 监听，面板跟着锚点重算位置（虽然不关了，
          但滚轮滚列表却把整页滚走仍然很怪） */}
      <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto overscroll-contain py-1">
        {rows.length === 0 ? (
          <p className="px-3 py-6 text-center text-[11px] text-ink-faint">
            {t("token.pickerEmpty")}
          </p>
        ) : (
          rows.map((row, index) => (
            <Option
              key={row.key}
              id={optionDomId(index)}
              row={row}
              active={index === active}
              current={row.request !== null && row.request.id === value}
              onHover={() => setCursor(index)}
              onPick={() => {
                if (row.request) onPick(row.request.id);
              }}
            />
          ))
        )}
      </div>

      <p className="shrink-0 border-t border-line px-2.5 py-1.5 text-[10px] leading-relaxed text-ink-faint">
        {t("token.pickerScopeHint")}
      </p>
    </div>,
    document.body,
  );
}

function optionDomId(index: number): string {
  return `request-picker-option-${index}`;
}

/** 面板相对视口的位置，以及它该朝上还是朝下弹 */
interface PanelPos {
  left: number;
  top: number;
  width: number;
  up: boolean;
}

/**
 * 由锚点算面板位置。
 *
 * 抽成模块级函数是因为它有**两个调用点**：挂载时（`useLayoutEffect`）
 * 与外部滚动时（跟着锚点走）。两处各写一份的话，「滚动之后面板不再贴着
 * 触发器」这种偏差只能靠肉眼发现。
 */
function anchorPos(rect: DOMRect): PanelPos {
  const width = Math.min(
    PANEL_MAX_WIDTH,
    Math.max(PANEL_MIN_WIDTH, rect.width),
    window.innerWidth - MARGIN * 2,
  );

  // 右对齐兜底：靠右的触发器向左展开会把面板推出窗口
  const left = Math.max(MARGIN, Math.min(rect.left, window.innerWidth - MARGIN - width));

  // 下方装不下一整个列表、而上方的余地更大 —— 就向上弹
  const spaceBelow = window.innerHeight - rect.bottom - MARGIN;
  const up = spaceBelow < LIST_MAX_HEIGHT && rect.top > spaceBelow;

  return { left, top: up ? rect.top - 4 : rect.bottom + 4, width, up };
}

function Option({
  id,
  row,
  active,
  current,
  onHover,
  onPick,
}: {
  id: string;
  row: Row;
  active: boolean;
  current: boolean;
  onHover: () => void;
  onPick: () => void;
}) {
  const { t } = useI18n();

  // 指向一个不存在的请求：这一行**选不动**。点它只是把那个 id 再写一遍，
  // 等于什么都没做，而看起来像「修好了」
  if (row.request === null) {
    return (
      <div
        id={id}
        role="option"
        aria-selected={false}
        className="flex cursor-default items-center gap-1.5 border-b border-line px-2.5 py-1.5 text-[11px] text-warn"
      >
        <TriangleAlert size={12} strokeWidth={2.2} aria-hidden className="shrink-0" />
        {t("token.fetchRequestGone")}
      </div>
    );
  }

  const { request, words } = row;

  return (
    <div
      id={id}
      role="option"
      aria-selected={active}
      onMouseEnter={onHover}
      // pointerdown 而不是 click：「按下就选中」更跟手，且不会先触发别的
      onPointerDown={(e) => {
        e.preventDefault();
        onPick();
      }}
      className={cn(
        "flex cursor-pointer flex-col gap-0.5 border-l-2 px-2.5 py-1.5",
        active ? "border-brand bg-surface-hover" : "border-transparent",
        // 钉住的那行与候选之间画一条线：它不属于候选，只是「你现在用的是它」
        row.pinned && "border-b border-b-line",
      )}
    >
      <span className="flex min-w-0 items-center gap-1.5">
        <MethodLabel method={request.method} />
        <span className="min-w-0 flex-1 truncate text-xs text-ink">{request.name}</span>
        {row.pinned ? (
          <Badge tone="warn">{t("token.pickerOutOfFolder")}</Badge>
        ) : null}
        {current && !row.pinned ? (
          <Check size={12} strokeWidth={2.6} aria-hidden className="shrink-0 text-brand" />
        ) : null}
      </span>
      {/* 面板里通栏显示完整地址：同一域名下挂两条取 token 的接口时，
          被截掉的那一段恰好就是分辨它们的那一段 */}
      <span className="min-w-0 truncate font-mono text-[10px] text-ink-faint" title={request.url}>
        {request.url}
      </span>
      {/* scope 独占一行、有几个铺几个。
          和名字挤同一行时它只能占右边一小块，放不下就折成 `+3` ——
          而被折掉的那几个恰好就是「这个 Token 能不能调那个服务」的答案。 */}
      <ScopeChips words={words} wrap />
    </div>
  );
}
