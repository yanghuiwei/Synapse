import { Import, PanelLeft, PanelLeftClose, Search } from "lucide-react";
import { cn } from "@/lib/cn";
import { formatShortcut } from "@/lib/constants";
import { useI18n } from "@/lib/i18n";
import { tokenCounts, useTokenStore } from "@/store/token";
import { useUiStore } from "@/store/ui";
import { Button, IconButton } from "@/features/ui/Button";
import { AppearanceMenu } from "@/features/ui/AppearanceMenu";

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
    <header className="flex h-11 shrink-0 items-center gap-3 border-b border-line bg-canvas px-2.5">
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

      {/* 顶栏**没有** Token 解析栏的开关：工具现在是「工具」板块的主视图，
          而左栏的活动栏本身就是那块的路由。在顶栏再开一个入口，
          会让人以为它是个跟板块无关的浮层 —— 而它并不是 */}
      {/* 外观（明暗 + 配色）收成**一个**入口。
          原先是两个并排的控件（`ThemeToggle` 三段 + `AccentPicker` 色块），
          顶栏右侧被占掉 ~110px；而这两个维度是低频设置，
          却和「导入」这种高频动作抢同一排位置。
          ⚠️ 合并的是**入口**，不是**维度** —— 面板里明暗与配色仍是两组独立控件，
          15 种组合各自可选。原来那条「别合成一个下拉」的判断依然成立，
          它反对的是把 15 种组合铺成 15 行，不是反对「一个按钮打开两个分区」。
          详见 `AppearanceMenu` 的注释。 */}
      <AppearanceMenu />
    </header>
  );
}
