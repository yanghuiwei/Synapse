import { Import, PanelLeft, PanelLeftClose, Search } from "lucide-react";
import { cn } from "@/lib/cn";
import { formatShortcut } from "@/lib/constants";
import { useI18n } from "@/lib/i18n";
import { tokenCounts, useTokenStore } from "@/store/token";
import { useUiStore } from "@/store/ui";
import { Button, IconButton } from "@/features/ui/Button";
import { AppearanceMenu } from "@/features/ui/AppearanceMenu";
import { WindowControls } from "@/features/layout/WindowControls";

/** 应用标识：一个方块 + 字标。刻意做得克制，别跟工具栏抢注意力 */
function BrandMark() {
  return (
    <div className="flex items-center gap-2 pl-1">
      <div
        aria-hidden
        className="flex h-6 w-6 items-center justify-center rounded-md bg-brand text-brand-fg"
      >
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none">
          <circle cx="5.5" cy="12" r="2.6" fill="currentColor" opacity="0.55" />
          <circle cx="18.5" cy="12" r="2.6" fill="currentColor" opacity="0.55" />
          <line x1="8.1" y1="12" x2="15.9" y2="12" stroke="currentColor" strokeWidth="1.6" opacity="0.55" />
          <circle cx="12" cy="12" r="2" fill="currentColor" />
        </svg>
      </div>
      <span className="select-none text-[13px] font-semibold tracking-tight text-ink">
        Synapse
      </span>
    </div>
  );
}

/**
 * 全局搜索入口。
 *
 * ## 为什么是「长得像输入框的按钮」而不是真的 input
 *
 * 它自己不接收输入 —— 真正的输入框是 `CommandPalette` 里那个（面板打开就 `autoFocus`）。
 * 这里做成 input 会有两个问题：
 * 1. 焦点停在顶栏这个框里，用户敲的字无处可去（面板还没打开）；
 * 2. 屏幕阅读器会把它当成一个真的、当前可编辑的文本框。
 *
 * 所以画成按钮，但**视觉上必须像输入框** —— 用户一眼就知道「这里能搜」。
 * 真正让它好用的是右端那枚 `Ctrl K`：把一个纯隐藏的快捷键变成看得见的入口。
 * 快捷键文案走 `formatShortcut("commandPalette")`，不手写 `"Ctrl+K"` ——
 * 改 `SHORTCUTS` 时这里会跟着变。
 *
 * ## 宽度交给父容器，这里只封顶
 *
 * `w-full max-w-[26rem]`：父容器是 `flex-1`，所以窄窗口下它自动缩宽
 * （靠文字的 `truncate` 让出空间），宽窗口下停在 26rem 居中。
 * 写成固定 320px 的话，800px 默认窗口下左右两侧会被它挤到贴边，
 * 而窗口拉宽时那条空白又原样留着。
 */
function GlobalSearch() {
  const { t } = useI18n();
  const openPalette = useUiStore((s) => s.openPalette);

  return (
    <button
      type="button"
      onClick={openPalette}
      title={t("palette.placeholder")}
      className={cn(
        "group flex h-7 min-w-0 items-center gap-2 rounded-sm border border-line bg-surface px-2.5",
        "transition-colors duration-100 hover:border-line-strong hover:bg-surface-hover",
        "w-full max-w-[26rem]",
      )}
    >
      <Search
        size={13}
        strokeWidth={2}
        aria-hidden
        className="shrink-0 text-ink-faint group-hover:text-ink-muted"
      />
      <span className="min-w-0 flex-1 truncate text-left text-xs text-ink-faint">
        {t("palette.placeholder")}
      </span>
      <kbd
        aria-hidden
        className="shrink-0 rounded-sm border border-line px-1 py-px font-mono text-[10px] leading-[14px] text-ink-faint"
      >
        {formatShortcut("commandPalette")}
      </kbd>
    </button>
  );
}

/**
 * 顶部工具栏。
 * 布局用 flex + 三段的写法（左右自适应、中间吃掉剩余空间），
 * 这样右侧按钮增多/变少时中间那块不会跳。
 */
export function TopBar() {
  const { t } = useI18n();
  const collapsed = useUiStore((s) => s.sidebarCollapsed);
  const toggleSidebar = useUiStore((s) => s.toggleSidebar);
  // 顶栏这个「导入」与侧栏标题区那个是**同一个动作的两个入口**：
  // 侧栏那个只在「集合」板块出现，而导入是全应用级的（往工作区里加东西），
  // 不该逼用户先切回集合板块才能点。对话框本体只有一份（`ConnectedImportDialog`）。
  const openImport = useUiStore((s) => s.openImport);
  const pendingTokens = useTokenStore((s) => tokenCounts(s).pending);
  const validTokens = useTokenStore((s) => tokenCounts(s).valid);

  return (
    // `data-tauri-drag-region`：**裸值**（不给 "deep"）。
    //
    // 查了 tauri-2.11.5 的 `src/window/scripts/drag.js`，它有两种语义：
    //   裸值 / "true" → 只有**直接点在带这个属性的元素上**才拖
    //   "deep"        → 点它子树里任何地方都拖
    // 这里要裸值 —— 一旦用 "deep"，搜索框、导入、外观面板就全被拖拽吃掉，
    // 按不动了。（脚本里那套 `isClickableElement` 会自动给
    // BUTTON / INPUT / 带 tabindex / role 匹配的子元素让位，
    // 所以不用手写 `data-tauri-drag-region="false"`，但**别用 deep**。）
    //
    // 双击最大化是脚本内置的（`e.detail === 2` → `internal_toggle_maximize`），
    // 同样不用自己实现。
    <header
      data-tauri-drag-region
      className="flex h-11 shrink-0 items-center gap-3 border-b border-line bg-canvas pl-2.5"
    >
      <BrandMark />

      <IconButton
        size="sm"
        label={t(collapsed ? "sidebar.expand" : "sidebar.collapse")}
        onClick={toggleSidebar}
        className="ml-1"
      >
        {collapsed ? (
          <PanelLeft size={15} strokeWidth={2} />
        ) : (
          <PanelLeftClose size={15} strokeWidth={2} />
        )}
      </IconButton>

      {/* 中间这块用 `flex-1` + 内部居中，而不是「占位符 + 固定宽度按钮」：
          占位符会把搜索框推到右侧控件左边，窗口一宽它就贴着走了。
          搜索框吃满剩余空间再 `justify-center`，窄了自动缩宽、宽了居中，两头都稳。

          ⚠️ `flex-1` 会吃掉**所有**剩余空间，所以它**右边**的元素若想「贴右缘」，
          不能再靠自己的 `ml-auto`（前面已有 flex-1，auto margin 分不到空间）。
          `WindowControls` 的 `ml-auto` 就是这样害得按钮停在离右缘 45px 的地方 ——
          按钮明明渲染了、却点不到（外表与「没渲染」一模一样，排查先量坐标）。
          现在改成 header 右侧统一由「右侧组」这一层的 flex 布局贴边，
          `WindowControls` 内部的 `ml-auto` 已去掉。

          窄于 `md`（768px）整块让位。按实际控件宽度算过：
          左右两组固定控件约占 440px，768px 时中间还剩 ~310px —— 塞得下一个
          缩窄的搜索框，但那时它的 placeholder 已经要截断成「搜索接口或…」。
          与其给一个半截的框，不如整个不出现。
          快捷键 Ctrl+K 那个入口本来就一直在（见 `GlobalSearch` 右端那枚 kbd），
          所以窄屏隐藏不会让人找不到 —— 这也是为什么那枚 kbd 不能省。 */}
      <div className="hidden min-w-0 flex-1 items-center justify-center px-2 md:flex">
        <GlobalSearch />
      </div>

      {/* Token 计数：和状态栏同一份数字（`tokenCounts`），
          有待刷新时显示待刷新数并转警示色，否则显示有效数

          ⚠️ 这里刻意**不套 `Button`**（原先套了）。
          套上之后它有 hover 底色、有手型光标，看起来能点 —— 但它**没有 onClick**，
          而且**至今也没有能落地的目的地**：
            · 它数的是「工作区里 144 个 Token 来源里几个过期了」
            · 而「工具 › Token 解析」是把一段 JWT 字符串拆开看明文
          两件事没有关系，接过去等于造第二个「点得动但落错地方」的入口。
          Token 源真正的处置界面是**单条请求**级的（`TokenGate` / `TokenSourcePanel`），
          工作区级的总览还不存在 —— 等它做出来，这里再换成按钮不迟。

          现在它是纯文本，只保留 `title`（悬停能看到完整句子）。
          副产物：`hidden md:inline-flex` 那个响应式隐藏一起去了 ——
          窄窗口下它一消失，右侧整排控件就会往左跳一下。 */}
      {/* 右侧整组（Token 计数 / 分隔线 / 导入 / 外观 / 窗口按钮）贴住窗口右缘。

          ⚠️ **为什么必须包这一层**（2026-10-13 真机踩到）：
          中间那块是 `flex-1`，它会吃掉**全部**剩余空间，导致它**之后**的元素
          即使带 `ml-auto` 也分不到空间（auto margin 只能分配「该元素之后的剩余」）。
          于是窗口按钮组停在离右缘 45px 的地方 —— **按钮渲染了、坐标也对，
          但用户点不到**，外表与「按钮没渲染」一模一样。
          排查时先量按钮坐标、再下结论，别只看「有没有画出来」。

          这一层不带 `flex-1`、也不带 `ml-auto`：它作为 header 最后一个 flex 项，
          中间的 `flex-1` 吃掉剩余后它自然紧贴右缘。窗口按钮组内部也去掉了
          自己的 `ml-auto`（那正是把按钮推离右缘的元凶）。 */}
      <div className="flex shrink-0 items-center gap-3">
        <span
          title={
            pendingTokens > 0
              ? t("status.tokenExpired", { count: pendingTokens })
              : t("status.tokenValid", { count: validTokens })
          }
          className="hidden shrink-0 select-none text-xs md:block"
        >
          {t("status.tokens")}
          <span
            className={cn("tabular", pendingTokens > 0 ? "text-warn" : "text-ink-faint")}
          >
            {" "}
            {pendingTokens > 0 ? pendingTokens : validTokens}
          </span>
        </span>

        <div className="mx-0.5 h-5 w-px bg-line" aria-hidden />

        <Button
          size="sm"
          variant="secondary"
          icon={<Import size={13} strokeWidth={2} />}
          onClick={openImport}
        >
          {t("sidebar.import")}
        </Button>

        {/* 外观（明暗 + 配色）收成**一个**入口。
            详见 AppearanceMenu 的注释。 */}
        <AppearanceMenu />

        {/* 窗口按钮贴住窗口右缘（Windows 惯例）。它自己不再带 `ml-auto`，
            由上方这一层的「最后一项」定位把它送到右缘。 */}
        <WindowControls />
      </div>
    </header>
  );
}
