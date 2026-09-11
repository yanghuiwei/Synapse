import { ChevronDown, Globe, Import, PanelLeft, PanelLeftClose } from "lucide-react";
import { useI18n } from "@/lib/i18n";
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

      {/* 环境选择器：P0 只是外观，P1 接 environments store */}
      <Button
        size="sm"
        variant="ghost"
        icon={<Globe size={13} strokeWidth={2} />}
        trailingIcon={<ChevronDown size={12} strokeWidth={2.4} />}
      >
        {t("env.none")}
      </Button>

      {/* 授权 / Token 状态：P1 接 token store */}
      <Button size="sm" variant="ghost" className="hidden md:inline-flex">
        {t("status.tokens")}
        <span className="tabular text-ink-faint">0</span>
      </Button>

      <div className="mx-0.5 h-5 w-px bg-line" aria-hidden />

      <Button
        size="sm"
        variant="secondary"
        icon={<Import size={13} strokeWidth={2} />}
      >
        {t("sidebar.import")}
      </Button>

      <ThemeToggle />
    </header>
  );
}
