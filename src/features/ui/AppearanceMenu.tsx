import { Check, Monitor, Moon, Sun } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { ACCENTS, ACCENT_ORDER, type Accent } from "@/lib/theme";
import { useTheme } from "@/lib/useTheme";

/**
 * 外观设置：一个入口（配色色块），面板里两个分区（明暗 / 配色）。
 *
 * ## 为什么合并，而原注释说过「别合成一个下拉」
 *
 * 那条注释针对的是**「合成一个选项列表」** —— 把 3×5=15 种组合铺成 15 行，
 * 用户要在一堆「深色 × 藕粉」里挑，选完还是只改了一个维度。
 * 这里合并的是**入口**，不是**维度**：面板里明暗与配色仍是两组独立控件，
 * 各自的状态、快捷键与选中标记都没变，少掉的只是顶栏上那两个并排的按钮。
 *
 * 收益：顶栏右侧从 ~110px 压到 ~32px，且**外观设置本身变得更完整** ——
 * 原来「主题」与「配色」分居两处、互相看不见对方的存在，
 * 现在打开一个面板就能看到「明暗 × 配色」的完整形状。
 *
 * ## 为什么面板要自算坐标
 *
 * 挂载点是 `TopBar`，而它的祖先链上有 `overflow-hidden` 的滚动容器 ——
 * `absolute` 会被裁掉，所以走 `fixed` + `getBoundingClientRect`。
 * 与 `AccentPicker` 同一个理由，那份注释里「下次再有人想用 absolute，先查祖先有没有
 * overflow」的提醒依然成立，**这里第三次踩同一个位置**。
 *
 * ## `null` 表示还没量过 —— 此时不渲染面板
 *
 * 写成「先渲染再量」会让它在正确的位置出现之前先在 (0,0) 闪一帧。
 */
export function AppearanceMenu() {
  const { t } = useI18n();
  const { mode, accent, setMode, setAccent } = useTheme();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  // 每次打开都重量：窗口可能被用户拖过 / 改变过大小，顶栏的 x 会变
  useEffect(() => {
    if (!open) return;
    const el = rootRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    // 264 而不是 240：三等分后每格约 83px，「跟随系统」(图标14 + gap6 + 文字48 + 内边距8
    // = 76px) 才放得下。240 时每格 75px，**只差 1px 就截断** ——
    // 这种「刚好卡边」最脆，字体回退一变（中英文标点宽度不同）就溢出。
    const width = 264;
    // 右对齐到按钮右缘；贴近屏幕右缘时改为左对齐，否则面板会被裁掉一半
    const left = Math.min(r.right - width, Math.max(8, window.innerWidth - width - 8));
    setPos({ top: r.bottom + 6, left });
  }, [open]);

  // 点外面 / Esc 关掉
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node;
      if (rootRef.current?.contains(target)) return;
      if (panelRef.current?.contains(target)) return;
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const current = ACCENTS[accent];

  return (
    <>
      <button
        ref={rootRef}
        type="button"
        aria-label={t("appearance.label")}
        // 提示把**两个维度**都报出来：这个按钮代表的不只是配色
        title={`${t(current.labelKey)} · ${t(MODE_LABEL_KEYS[mode])}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "inline-flex h-7 w-7 items-center justify-center rounded-md border transition-colors duration-100",
          open
            ? "border-line-strong bg-surface-hover"
            : "border-line bg-canvas-subtle hover:bg-surface-hover",
        )}
      >
        {/* 当前配色的双拼色块：上半浅、下半深。
            写成上下两半是**刻意的**：单色小方块只能表达明暗之一，
            而配色要和明暗正交 —— 一个格子同时给两面才对得了话 */}
        <span
          aria-hidden
          className="h-3.5 w-3.5 overflow-hidden rounded-[3px] ring-1 ring-inset ring-line-strong"
          style={{
            background: `linear-gradient(to bottom, ${current.swatch} 0 50%, ${darkSwatch(accent)} 50% 100%)`,
          }}
        />
      </button>

      {open && pos && (
        <div
          ref={panelRef}
          role="dialog"
          aria-label={t("appearance.label")}
          style={{ top: pos.top, left: pos.left }}
          className="fixed z-[var(--z-dropdown)] w-[264px] origin-top-right rounded-lg border border-line bg-surface p-1.5 shadow-e3 animate-pop-in"
        >
          {/* ---- 明暗 ---- */}
          <SectionLabel text={t("appearance.theme")} />
          <div
            role="radiogroup"
            aria-label={t("appearance.theme")}
            className="flex items-center gap-0.5 rounded-md border border-line bg-canvas-subtle p-0.5"
          >
            {MODES.map((m) => (
              <ModeButton
                key={m}
                mode={m}
                active={mode === m}
                label={t(MODE_LABEL_KEYS[m])}
                onSelect={() => setMode(m)}
              />
            ))}
          </div>

          {/* 分区之间的分隔线。用 `border-line` 而不是更重的：
              两个分区是靠**标题文字**区分的，这条线只是加固，
              画重了会让人以为「下面这块比上面那块重要」 */}
          <div className="mx-1.5 my-1.5 h-px bg-line" aria-hidden />

          {/* ---- 配色 ---- */}
          <SectionLabel text={t("appearance.accent")} />
          <ul role="radiogroup" aria-label={t("appearance.accent")} className="space-y-0.5">
            {ACCENT_ORDER.map((a) => {
              const def = ACCENTS[a];
              const active = a === accent;
              return (
                <li key={a}>
                  <button
                    type="button"
                    role="radio"
                    aria-checked={active}
                    onClick={() => setAccent(a)}
                    className={cn(
                      "flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left transition-colors duration-100",
                      active ? "bg-surface-hover" : "hover:bg-surface-hover",
                    )}
                  >
                    {/* 上下双色：这一个格子就回答了「浅色什么样 + 深色什么样」 */}
                    <span
                      aria-hidden
                      className="h-6 w-6 shrink-0 overflow-hidden rounded-[5px] ring-1 ring-inset ring-line-strong"
                      style={{
                        background: `linear-gradient(to bottom, ${def.swatch} 0 50%, ${darkSwatch(a)} 50% 100%)`,
                      }}
                    />
                    <span
                      className={cn(
                        "flex-1 text-[13px]",
                        active ? "text-ink" : "text-ink-muted",
                      )}
                    >
                      {t(def.labelKey)}
                    </span>
                    {active && <Check size={14} strokeWidth={2.5} className="text-brand" />}
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </>
  );
}

const MODES = ["light", "dark", "system"] as const;

/**
 * 模式 → 文案键。
 *
 * 单独一张表而不是 `import type { ThemeMode }` + 索引推导：
 * 那样写能过，但加一个模式时忘了加文案就得到 `undefined`，界面上是空白。
 */
const MODE_LABEL_KEYS = {
  light: "theme.light",
  dark: "theme.dark",
  system: "theme.system",
} as const;

const MODE_ICONS = {
  light: Sun,
  dark: Moon,
  system: Monitor,
} as const;

/** 分区标题。10px + `tracking-wide` 是与 `CommandPalette` 那套分组标题同一档的视觉重量 */
function SectionLabel({ text }: { text: string }) {
  return (
    <p className="px-2 pb-1 pt-0.5 text-[10px] font-medium tracking-wide text-ink-faint">
      {text}
    </p>
  );
}

/**
 * 明暗的一格。
 *
 * 分段控件（三个图标并排）而不是下拉：主题是高频操作，少一次点击差别很大
 * —— 与原 `ThemeToggle` 的判断一致，只是从顶栏搬进了面板。
 */
function ModeButton({
  mode,
  active,
  label,
  onSelect,
}: {
  mode: (typeof MODES)[number];
  active: boolean;
  label: string;
  onSelect: () => void;
}) {
  const Icon = MODE_ICONS[mode];
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      aria-label={label}
      title={label}
      onClick={onSelect}
      className={cn(
        // `min-w-0` 缺了会出事：选中那格多一段文字，不选中那格只有图标，
        // `flex-1` 给的是相同 basis，内容宽度不同就按内容分配 ——
        // 结果「跟随系统」把文字顶出分段控件右缘（实测溢出 30px+）。
        // 补上 `min-w-0` 后三格真正等分，文字不够就 `truncate`。
        "inline-flex h-7 min-w-0 flex-1 items-center justify-center gap-1.5 rounded-sm px-1",
        "transition-colors duration-100",
        active
          ? "bg-surface text-ink shadow-[var(--shadow-1)]"
          : "text-ink-faint hover:bg-surface-hover hover:text-ink-muted",
      )}
    >
      <Icon size={14} strokeWidth={2} className="shrink-0" />
      {/* 文字只在选中时出现：不这么做的话「当前是哪个」要靠记图标，
          认错图标是常事。三格等宽，所以选中那格变长不会引起整排宽度变化。 */}
      {active && <span className="truncate text-[12px] leading-none">{label}</span>}
    </button>
  );
}

/**
 * 每个 accent 的深色品牌色。
 *
 * ⚠️ 与 `theme.css` 里的 `.dark[data-accent="x"] { --brand }` 是**两份**数据，
 * 这是有意的取舍：色板要在浮层里画几个 24px 的小方块，
 * 为此让 CSS 变成唯一来源需要 5 个额外的 data 属性 + 一堆工具类，
 * 得不偿失。代价是**改深色 brand 时必须同步改这里**（浅色侧有
 * `ACCENTS[].swatch` 顶着，改错了色板上能看出来；深色侧看不出来，
 * 所以这里单独注释提醒）。
 */
function darkSwatch(a: Accent): string {
  switch (a) {
    case "indigo":
      return "#7ba8e8";
    case "azure":
      return "#6bb3de";
    case "violet":
      return "#a385f0";
    case "clay":
      return "#e08a63";
    case "teal":
      return "#5cbea4";
  }
}
