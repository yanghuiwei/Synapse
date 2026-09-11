import { Monitor, Moon, Sun } from "lucide-react";
import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { useTheme } from "@/lib/useTheme";
import type { ThemeMode } from "@/lib/theme";

const OPTIONS: ReadonlyArray<{ mode: ThemeMode; icon: typeof Sun }> = [
  { mode: "light", icon: Sun },
  { mode: "dark", icon: Moon },
  { mode: "system", icon: Monitor },
];

const LABEL_KEYS = {
  light: "theme.light",
  dark: "theme.dark",
  system: "theme.system",
} as const;

/**
 * 三态主题切换（浅色 / 深色 / 跟随系统）。
 * 做成分段控件而不是下拉菜单：主题是高频操作，少一次点击差别很大。
 */
export function ThemeToggle() {
  const { mode, setMode } = useTheme();
  const { t } = useI18n();

  return (
    <div
      role="group"
      aria-label={t("theme.toggle")}
      className="flex items-center gap-0.5 rounded-md border border-line bg-canvas-subtle p-0.5"
    >
      {OPTIONS.map(({ mode: m, icon: Icon }) => {
        const active = mode === m;
        const label = t(LABEL_KEYS[m]);
        return (
          <button
            key={m}
            type="button"
            aria-pressed={active}
            aria-label={label}
            title={label}
            onClick={() => setMode(m)}
            className={cn(
              "inline-flex h-6 w-7 items-center justify-center rounded-sm transition-colors duration-100",
              active
                ? "bg-surface text-ink shadow-[var(--shadow-1)]"
                : "text-ink-faint hover:text-ink-muted",
            )}
          >
            <Icon size={14} strokeWidth={2} />
          </button>
        );
      })}
    </div>
  );
}
