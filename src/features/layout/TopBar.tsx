import { Import, PanelLeft, PanelLeftClose } from "lucide-react";
import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { tokenCounts, useTokenStore } from "@/store/token";
import { useUiStore } from "@/store/ui";
import { Button, IconButton } from "@/features/ui/Button";
import { ThemeToggle } from "@/features/ui/ThemeToggle";

/** 应用标识：一个方块 + 字标。刻意做得克制，别跟工具栏抢注意力 */
function BrandMark() {
  return (
    <div className="flex items-center gap-2 pl-1">
      <div
        aria-hidden
        className="flex h-6 w-6 items-center justify-center rounded-md bg-brand text-brand-fg"
      >
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none">
          <path
            d="M7 7h5.5a3.5 3.5 0 0 1 0 7H10l4 4"
            stroke="currentColor"
            strokeWidth="2.4"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
          <circle cx="7" cy="7" r="1.6" fill="currentColor" />
        </svg>
      </div>
      <span className="select-none text-[13px] font-semibold tracking-tight text-ink">
        Reqly
      </span>
    </div>
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

      <div className="flex-1" />

      {/* Token 计数：和状态栏同一份数字（`tokenCounts`），
          有待刷新时显示待刷新数并转警示色，否则显示有效数 */}
      <Button
        size="sm"
        variant="ghost"
        className="hidden md:inline-flex"
        title={
          pendingTokens > 0
            ? t("status.tokenExpired", { count: pendingTokens })
            : t("status.tokenValid", { count: validTokens })
        }
      >
        {t("status.tokens")}
        <span
          className={cn("tabular", pendingTokens > 0 ? "text-warn" : "text-ink-faint")}
        >
          {pendingTokens > 0 ? pendingTokens : validTokens}
        </span>
      </Button>

      <div className="mx-0.5 h-5 w-px bg-line" aria-hidden />

      <Button
        size="sm"
        variant="secondary"
        icon={<Import size={13} strokeWidth={2} />}
        onClick={openImport}
      >
        {t("sidebar.import")}
      </Button>

      <ThemeToggle />
    </header>
  );
}
