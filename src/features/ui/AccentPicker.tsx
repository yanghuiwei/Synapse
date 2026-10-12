import { useEffect, useRef, useState } from "react";
import { Check, Palette } from "lucide-react";
import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { ACCENTS, ACCENT_ORDER, type Accent } from "@/lib/theme";
import { useTheme } from "@/lib/useTheme";

/**
 * 配色选择器。
 *
 * 为什么是「色板 + 悬停预览」而不是把5 种配色铺成一行按钮：
 * 每个按钮只能显示一种颜色，而配色要和**明暗正交**——
 * 想让用户判断「藕粉配深色好不好看」，得在同一个小方块上同时看到明暗两面。
 * 所以色板画成上下两半（上浅下深），悬停时大图放大呈现。
 *
 * 浮层为什么用 `fixed` + 自算坐标而不是 `absolute`：
 * 它挂在 TopBar 里，而 TopBar 的祖先链上有 `overflow-hidden` 的滚动容器 ——
 * `absolute` 会被裁掉。这是本项目第一个需要「脱离文档流定位」的浮层，
 * 所以顺带记一笔：**下次再有人想用 absolute，先查祖先有没有 overflow。**
 */
export function AccentPicker() {
  const { accent, setAccent } = useTheme();
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  /**
   * 面板定位。`null` 表示还没量过 —— 此时**不渲染面板**。
   * 写成「先渲染再量」会让它在正确的位置出现之前先在 (0,0) 闪一帧。
   */
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  // 量按钮的位置，把面板挂到它正下方并右对齐。
  // 每次打开都重量：窗口可能被用户拖过，顶栏的 x 会变。
  useEffect(() => {
    if (!open) return;
    const el = rootRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const width = 260;
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
        aria-label={t("accent.picker.label")}
        title={t(current.labelKey)}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "inline-flex h-7 items-center gap-1.5 rounded-md border px-2 transition-colors duration-100",
          open
            ? "border-line-strong bg-surface-hover"
            : "border-line bg-canvas-subtle hover:bg-surface-hover",
        )}
      >
        {/* 当前配色的双拼色块：上半浅、下半深。
            与面板里的大方块同一个画法，只是尺寸更小。
            写成上下两半是**刻意的**：单色小方块只能表达明暗之一，
            而配色要和明暗正交 —— 一个格子同时给两面才对得了话 */}
        <span
          aria-hidden
          className="h-3.5 w-3.5 overflow-hidden rounded-[3px] ring-1 ring-inset ring-line-strong"
          style={{
            background: `linear-gradient(to bottom, ${current.swatch} 0 50%, ${darkSwatch(accent)} 50% 100%)`,
          }}
        />
        <Palette size={13} strokeWidth={2} className="text-ink-faint" />
      </button>

      {open && pos && (
        <div
          ref={panelRef}
          role="listbox"
          aria-label={t("accent.picker.label")}
          style={{ top: pos.top, left: pos.left }}
          className="fixed z-[var(--z-dropdown)] w-[260px] rounded-lg border border-line bg-surface p-1.5 shadow-e2"
        >
          {ACCENT_ORDER.map((a) => {
            const def = ACCENTS[a];
            const active = a === accent;
            return (
              <button
                key={a}
                type="button"
                role="option"
                aria-selected={active}
                onClick={() => {
                  setAccent(a);
                  setOpen(false);
                }}
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
                <span className={cn("flex-1 text-[13px]", active ? "text-ink" : "text-ink-muted")}>
                  {t(def.labelKey)}
                </span>
                {active && <Check size={14} strokeWidth={2.5} className="text-brand" />}
              </button>
            );
          })}
        </div>
      )}
    </>
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