import { Minus, Square, X } from "lucide-react";
import { useEffect, useState } from "react";

import { cn } from "@/lib/cn";
import { isTauriRuntime } from "@/lib/ipc";
import { useI18n } from "@/lib/i18n";

/**
 * 窗口控制按钮（最小化 / 最大化 / 关闭）。
 *
 * ## 为什么必须自己画
 *
 * `tauri.conf.json` 里 `"decorations": false` 去掉了系统标题栏，
 * 换来的是省掉约 28px 高度 + 顶栏能整条拖动。代价就是这三个按钮得自己画。
 *
 * ## 为什么不直接把图标按钮写进 TopBar
 *
 * 三个原因，逐条都是实际会遇到的：
 * 1. **它们不属于「应用功能」**。TopBar 里其他控件都进命令面板、能用键盘走到；
 *    窗口按钮属于**操作系统**，混进同一排会让 Tab 顺序变得奇怪。
 * 2. **浏览器里必须消失**。开发时 `pnpm dev` 是在浏览器里跑的（没有窗口可控制），
 *    画出来就是三个点了没反应的假按钮 —— 与本轮刚修掉的 Token 计数同一类错误。
 * 3. **最大化态需要实时跟踪**。用户可以在标题栏外用 Win+↑ 最大化，
 *    这时「还原」图标必须跟着变。订阅 `onResized` 才跟得上。
 *
 * ## 尺寸刻意与顶栏其他控件对齐
 *
 * 窗口按钮惯例是「无边框、直角、hover 变色」，与顶栏的圆角小按钮风格不同 ——
 * 这是**故意的**：它们是系统控件的替身，做得越像系统控件越好用。
 * 高度对齐顶栏的 44px（`h-11`），宽度用 Windows 惯例的 46px。
 */

/** 宽度。Windows 11 原生标题栏按钮约 46px 宽，这个数值直接对齐系统观感 */
const BTN_W = 46;

type WindowAction = "minimize" | "toggleMaximize" | "close";

export function WindowControls({ className }: { className?: string }) {
  const { t } = useI18n();
  const [maximized, setMaximized] = useState(false);

  /**
   * 浏览器里整个组件不渲染。
   *
   * 钩子无条件调用、条件只放在 return 之后 —— 否则「开发时有没有窗口」
   * 会变成一个影响钩子顺序的条件渲染，React 会在切模式时报错。
   */
  const available = isTauriRuntime();

  useEffect(() => {
    if (!available) return;
    // `getCurrentWindow` 在浏览器里 import 本身没问题（只是没有 IPC），
    // 但不必在非 Tauri 环境下白跑一次 import
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const win = getCurrentWindow();
        setMaximized(await win.isMaximized());
        // Win+↑ / 双击标题栏 / 拖到屏幕上沿 —— 都会走窗口 resize，
        // 只听 resize 才不会漏掉「用键盘最大化了但图标没变」
        unlisten = await win.onResized(() => {
          if (!disposed) {
            void win
              .isMaximized()
              .then(setMaximized)
              // 同上：`is-maximized` 权限被拒时不能让它变成静默失败，
              // 否则图标会一直停在「最大化」，方向恰好是反的
              .catch((e: unknown) => console.error("[reqly] 读最大化状态失败：", e));
          }
        });
      } catch (e) {
        // 连 `import` 或 `getCurrentWindow` 失败都会落到这里。
        // 记一条日志，别让「图标永远不翻转」变成无迹可寻的怪现象。
        console.error("[reqly] 窗口状态监听初始化失败：", e);
      }
    })();

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [available]);

  if (!available) return null;

  const run = async (action: WindowAction) => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      if (action === "minimize") await win.minimize();
      else if (action === "close") await win.close();
      else await win.toggleMaximize();
    } catch (e) {
      // ⚠️ 这个 catch 不是「防御性编程」，是**必需的**。
      // 权限被 ACL 拒掉时 `invoke` 会 reject，而调用点写的是 `void run(...)`
      // —— 不接住的话异常变成 unhandledrejection，在界面上的表现是
      // **「按钮点了完全没反应，也不报错」**，与本轮 P0 修掉的
      // Token 计数假按钮是同一类错误（看起来能点，实际什么都不会发生）。
      //
      // 真实踩过：`capabilities/default.json` 里只有 `core:default`，
      // 而它间接包含的 `core:window:default` **全是只读状态**，
      // 最小化 / 最大化 / 关闭 / 拖拽四个权限一个都没有。
      // 详见 `src-tauri/src/lib.rs` 里那段 capability 说明。
      console.error(`[reqly] 窗口操作「${action}」失败：`, e);
    }
  };

  return (
    <div
      // ⚠️ **不要 `ml-auto`**（2026-10-13 真机踩到）：
      // 顶栏中间那块是 `flex-1`，会吃掉全部剩余空间，于是 `ml-auto` 分不到任何
      // 空间 —— 按钮组停在离右缘 45px 的地方，**渲染了却点不到**，
      // 外表与「按钮没渲染」一模一样。贴右缘的职责交给 TopBar 里
      // 「右侧整组」那一层（它是 header 最后一个 flex 项）。
      //
      // 「与右侧控件之间留一道视觉缝」改由外层 `gap-3` 承担 ——
      // 它们是「窗口」不是「工具」，紧贴「导入」会让人误以为同一类东西。
      className={cn("flex shrink-0 items-stretch", className)}
    >
      <WindowButton
        label={t("window.minimize")}
        onClick={() => void run("minimize")}
        icon={<Minus size={15} strokeWidth={1.6} />}
      />
      <WindowButton
        // 图标随最大化态翻转。不这么做的话，用户 Win+↑ 之后
        // 按钮还画着「最大化」，按一下反而把窗口还原了 —— 方向是反的
        label={t(maximized ? "window.restore" : "window.maximize")}
        onClick={() => void run("toggleMaximize")}
        icon={
          maximized ? (
            // 「还原」不是一个正方形：Windows 用两个错位的方框表示「回到原来的大小」
            <RestoreGlyph />
          ) : (
            <Square size={12} strokeWidth={1.6} />
          )
        }
      />
      <WindowButton
        label={t("window.close")}
        onClick={() => void run("close")}
        icon={<X size={15} strokeWidth={1.6} />}
        danger
      />
    </div>
  );
}

function WindowButton({
  label,
  icon,
  onClick,
  danger,
}: {
  label: string;
  icon: React.ReactNode;
  onClick: () => void;
  /** 关闭按钮 hover 走红底 —— 这是 Windows 的肌肉记忆，换了颜色会让人犹豫 */
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      style={{ width: BTN_W }}
      className={cn(
        "flex h-11 items-center justify-center text-ink-muted transition-colors duration-75",
        danger
          ? "hover:bg-[var(--err)] hover:text-white"
          : "hover:bg-surface-hover hover:text-ink",
      )}
    >
      {icon}
    </button>
  );
}

/**
 * 「还原」图标：两个错位的方框。
 *
 * 不用 lucide 的现成图标 —— 那个画的是「上下重叠」，
 * 而 Windows 表达的是「后面那个更小、前面那个压在左上」，
 * 方向相反。宁可自己画 8px 的 SVG，也不引一个语义不对的图标。
 */
function RestoreGlyph() {
  return (
    <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden>
      {/* 后方那个（被还原到的那个窗口） */}
      <rect x="1.5" y="3.5" width="7" height="7" stroke="currentColor" strokeWidth="1.2" />
      {/* 前方盖住左上角的窗口 */}
      <path d="M4 1.5h5a1 1 0 0 1 1 1v5" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
    </svg>
  );
}
