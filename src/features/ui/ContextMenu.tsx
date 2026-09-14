import type { LucideIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { cn } from "@/lib/cn";

export interface MenuItem {
  key: string;
  label: string;
  icon?: LucideIcon;
  /** 危险操作（删除 / 清空）—— 用错误色，不靠位置暗示 */
  danger?: boolean;
  disabled?: boolean;
  /** 在这一项**上方**画一条分隔线 */
  separatorBefore?: boolean;
  onSelect: () => void;
}

export interface ContextMenuProps {
  items: MenuItem[];
  /** 打开位置：视口坐标（`clientX` / `clientY`） */
  x: number;
  y: number;
  onClose: () => void;
}

/** 与视口边缘保持的最小间距 */
const MARGIN = 6;

/**
 * 右键菜单。
 *
 * 四个实现要点：
 *
 * 1. **`createPortal` 到 `body`**。集合树住在 `overflow-y-auto` 的侧栏里，
 *    菜单要是留在原地，贴底部的行一右键就会被裁掉一半。
 * 2. **`clientX/clientY` 定位 + 越界翻转**。菜单是 `fixed` 的，
 *    所以直接用视口坐标，不需要减去容器偏移。靠右/靠下时往回收，
 *    否则菜单会跑到窗口外（用户只能看到一半）。
 * 3. **先量再显示**。用 `useLayoutEffect` 量出真实尺寸后才设位置并显示，
 *    避免「先画在鼠标位置、再跳到修正位置」的闪动。
 * 4. **键盘可达**。打开即聚焦容器，上下键移动、回车触发、Esc 关闭 ——
 *    P3 要求「拖拽必须有键盘替代方案」，菜单就是那个入口。
 */
export function ContextMenu({ items, x, y, onClose }: ContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const [active, setActive] = useState(0);

  /** 可跳过的项（禁用项不参与上下键移动） */
  const enabledIndexes = items
    .map((item, index) => (item.disabled ? -1 : index))
    .filter((index) => index >= 0);

  // 量尺寸 → 夹紧到视口内 → 显示
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    setPos({
      left: Math.max(MARGIN, Math.min(x, window.innerWidth - width - MARGIN)),
      top: Math.max(MARGIN, Math.min(y, window.innerHeight - height - MARGIN)),
    });
  }, [x, y]);

  // 打开时聚焦，键盘才接得上
  useEffect(() => {
    ref.current?.focus();
  }, []);

  // 关闭时机：外部按下 / Esc / 滚动 / 窗口尺寸变化。
  // 用 pointerdown 而不是 click：在别处按下鼠标时菜单就该消失，
  // 等到 click 才关会先触发底层元素的 click，等于误操作了一次。
  useEffect(() => {
    const onPointerDown = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation(); // 别让 Esc 冒泡到「取消请求」
        onClose();
      }
    };
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown, true);
    // capture 阶段监听 scroll：内层滚动容器的滚动不会冒泡到 window
    window.addEventListener("scroll", onClose, true);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("scroll", onClose, true);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  const firstEnabled = enabledIndexes[0] ?? 0;
  useEffect(() => {
    setActive(firstEnabled);
  }, [firstEnabled]);

  const move = (delta: number) => {
    if (enabledIndexes.length === 0) return;
    const at = enabledIndexes.indexOf(active);
    const next = at < 0 ? 0 : (at + delta + enabledIndexes.length) % enabledIndexes.length;
    setActive(enabledIndexes[next] ?? firstEnabled);
  };

  return createPortal(
    <div
      ref={ref}
      role="menu"
      tabIndex={-1}
      aria-orientation="vertical"
      onContextMenu={(e) => e.preventDefault()}
      onKeyDown={(e) => {
        switch (e.key) {
          case "ArrowDown":
            e.preventDefault();
            move(1);
            break;
          case "ArrowUp":
            e.preventDefault();
            move(-1);
            break;
          case "Home":
            e.preventDefault();
            setActive(firstEnabled);
            break;
          case "End":
            e.preventDefault();
            setActive(enabledIndexes[enabledIndexes.length - 1] ?? firstEnabled);
            break;
          case "Enter":
          case " ":
            e.preventDefault();
            items[active]?.onSelect();
            onClose();
            break;
          default:
            break;
        }
      }}
      style={{
        left: pos?.left ?? x,
        top: pos?.top ?? y,
        // z 层级走 theme.css 的令牌（--z-dropdown / --z-dialog / --z-toast），
        // 不写裸数字 —— 「谁盖住谁」得在一个地方能看全
        zIndex: "var(--z-dropdown)",
        // 量尺寸之前先藏起来，否则会看到菜单在鼠标位置闪一下再跳位
        visibility: pos ? "visible" : "hidden",
      }}
      className={cn(
        "fixed min-w-[176px] rounded-md border border-line bg-surface p-1",
        "shadow-e2 outline-none",
      )}
    >
      {items.map((item, index) => {
        const Icon = item.icon;
        const isActive = index === active;
        return (
          <div key={item.key}>
            {item.separatorBefore ? (
              <div role="separator" className="my-1 h-px bg-line" />
            ) : null}
            <button
              type="button"
              role="menuitem"
              disabled={item.disabled}
              // 用 pointerenter 而不是 onMouseEnter：前者在触摸/笔输入下也准
              onPointerEnter={() => !item.disabled && setActive(index)}
              onClick={() => {
                item.onSelect();
                onClose();
              }}
              className={cn(
                "flex h-7 w-full items-center gap-2 rounded-sm px-2 text-left text-xs",
                "transition-colors duration-75",
                item.disabled && "opacity-45",
                !item.disabled && isActive
                  ? item.danger
                    ? "bg-err/12 text-err"
                    : "bg-surface-hover text-ink"
                  : item.danger
                    ? "text-err"
                    : "text-ink-muted",
              )}
            >
              {Icon ? <Icon size={13} strokeWidth={2} aria-hidden className="shrink-0" /> : (
                <span aria-hidden className="w-[13px] shrink-0" />
              )}
              <span className="flex-1 truncate">{item.label}</span>
            </button>
          </div>
        );
      })}
    </div>,
    document.body,
  );
}
