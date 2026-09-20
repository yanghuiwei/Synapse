import { Clock, Gauge, Layers, Settings, Trash } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import { useUiStore, type SidebarSection } from "@/store/ui";

interface RailItem {
  section: SidebarSection;
  icon: LucideIcon;
  labelKey: MessageKey;
}

/** 主功能，靠上排列 */
const PRIMARY: readonly RailItem[] = [
  { section: "collections", icon: Layers, labelKey: "nav.collections" },
  { section: "history", icon: Clock, labelKey: "nav.history" },
  { section: "loadtest", icon: Gauge, labelKey: "nav.loadtest" },
];

/** 次要功能，贴底排列 */
const SECONDARY: readonly RailItem[] = [
  { section: "trash", icon: Trash, labelKey: "nav.trash" },
  { section: "settings", icon: Settings, labelKey: "nav.settings" },
];

function RailButton({ item }: { item: RailItem }) {
  const { t } = useI18n();
  const section = useUiStore((s) => s.section);
  const setSection = useUiStore((s) => s.setSection);
  const active = section === item.section;
  const label = t(item.labelKey);
  const Icon = item.icon;

  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-current={active ? "page" : undefined}
      onClick={() => setSection(item.section)}
      className={cn(
        "relative flex h-10 w-10 items-center justify-center rounded-md transition-colors duration-100",
        active
          ? "bg-surface text-brand"
          : "text-ink-faint hover:bg-surface hover:text-ink-muted",
      )}
    >
      {/* 选中态左侧的强调条 */}
      <span
        aria-hidden
        className={cn(
          "absolute -left-2 h-5 w-[3px] rounded-r-full bg-brand transition-opacity duration-150",
          active ? "opacity-100" : "opacity-0",
        )}
      />
      <Icon size={18} strokeWidth={1.9} />
    </button>
  );
}

/**
 * 最左侧的图标竖条（VS Code 那种 activity bar）。
 * 固定宽度、不可拖拽 —— 可拖拽的是它右边的 Sidebar。
 */
export function ActivityRail() {
  const { t } = useI18n();
  return (
    <nav
      aria-label={t("nav.primary")}
      className="flex w-14 shrink-0 flex-col items-center gap-1 border-r border-line bg-canvas-subtle py-2"
    >
      {PRIMARY.map((item) => (
        <RailButton key={item.section} item={item} />
      ))}

      <div className="flex-1" />

      {SECONDARY.map((item) => (
        <RailButton key={item.section} item={item} />
      ))}
    </nav>
  );
}
