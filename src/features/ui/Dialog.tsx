import { X } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { Button, IconButton } from "@/features/ui/Button";

export interface DialogProps {
  title: string;
  /** 标题下面的一行说明。有它就不用在正文里再解释一遍 */
  description?: ReactNode;
  onClose: () => void;
  children: ReactNode;
  /** 底部按钮区 */
  footer?: ReactNode;
  /** 关掉「点遮罩关闭」—— 有未保存输入时用 */
  dismissOnOverlay?: boolean;
  width?: number;
}

/**
 * 模态对话框。
 *
 * 三个细节：
 *
 * 1. **遮罩用 `bg-scrim` 令牌，不是 `bg-ink/40`**。深色主题里 `--ink` 是浅色，
 *    拿它当遮罩会得到一层白雾。遮罩的语义是「压暗」，与文字色无关。
 * 2. **焦点管理是必须的**：打开时把焦点移进面板，关闭时还给原来的元素。
 *    少了这一步，键盘用户的焦点会留在背后的列表上，
 *    于是「在对话框里按回车」实际触发的是背后那一行。
 * 3. **首尾各一个「焦点哨兵」**：Tab 到尽头时把焦点拉回另一端，
 *    实现焦点圈定。比手写 keydown 遍历可聚焦元素简单，也不会漏掉动态元素。
 */
export function Dialog({
  title,
  description,
  onClose,
  children,
  footer,
  dismissOnOverlay = true,
  width = 420,
}: DialogProps) {
  const { t } = useI18n();
  const panelRef = useRef<HTMLDivElement>(null);
  const restoreRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    restoreRef.current = document.activeElement as HTMLElement | null;

    // 优先聚焦第一个输入框，没有就聚焦面板本身
    const input = panelRef.current?.querySelector<HTMLElement>(
      "input, textarea, select, [data-autofocus]",
    );
    (input ?? panelRef.current)?.focus();

    return () => {
      // 焦点还给触发它的那个按钮，键盘流才不会断
      restoreRef.current?.focus?.();
    };
  }, []);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation(); // 别让 Esc 冒泡去「取消请求」
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [onClose]);

  /** 焦点哨兵：无可见内容，聚焦到它就说明 Tab 到头了 */
  const sentinel = (side: "start" | "end") => (
    <div
      tabIndex={0}
      aria-hidden
      onFocus={() => {
        const focusables = panelRef.current?.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
        );
        if (!focusables || focusables.length === 0) return;
        const target =
          side === "start" ? focusables[focusables.length - 1] : focusables[0];
        target?.focus();
      }}
    />
  );

  return createPortal(
    <div
      style={{ zIndex: "var(--z-dialog)" }}
      className="fixed inset-0 flex items-center justify-center bg-scrim p-6"
      onPointerDown={
        dismissOnOverlay
          ? (e) => {
              if (e.target === e.currentTarget) onClose();
            }
          : undefined
      }
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        style={{ width }}
        className={cn(
          "max-h-full overflow-y-auto rounded-lg border border-line bg-surface",
          "shadow-e2 outline-none",
        )}
      >
        {sentinel("start")}

        <header className="flex items-start gap-2 px-4 pb-2 pt-3.5">
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-[13px] font-semibold tracking-tight text-ink">
              {title}
            </h2>
            {description ? (
              <p className="mt-1 text-xs leading-relaxed text-ink-muted">{description}</p>
            ) : null}
          </div>
          <IconButton size="sm" label={t("common.close")} onClick={onClose}>
            <X size={14} strokeWidth={2} />
          </IconButton>
        </header>

        <div className="px-4 pb-3">{children}</div>

        {footer ? (
          <footer className="flex items-center justify-end gap-2 border-t border-line px-4 py-2.5">
            {footer}
          </footer>
        ) : null}

        {sentinel("end")}
      </div>
    </div>,
    document.body,
  );
}

export interface ConfirmDialogProps {
  title: string;
  description?: ReactNode;
  /** 确认按钮上的字。默认「确定」 */
  confirmLabel?: string;
  /** 危险操作的确认按钮用红色 */
  danger?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}

/**
 * 二次确认。
 *
 * 只有「不可撤销」的操作才值得弹它 —— 能进回收站的操作不该拦，
 * 拦多了用户就会变成闭眼点「确定」，等到真正危险的那次也照样点过去。
 */
export function ConfirmDialog({
  title,
  description,
  confirmLabel,
  danger = false,
  onConfirm,
  onClose,
}: ConfirmDialogProps) {
  const { t } = useI18n();
  return (
    <Dialog
      title={title}
      description={description}
      onClose={onClose}
      width={380}
      footer={
        <>
          <Button size="sm" variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant={danger ? "danger" : "primary"}
            data-autofocus
            onClick={() => {
              onConfirm();
              onClose();
            }}
          >
            {confirmLabel ?? t("common.confirm")}
          </Button>
        </>
      }
    >
      {/* 正文留空：确认框的全部信息都在标题与描述里 */}
      <div />
    </Dialog>
  );
}

export interface PromptDialogProps {
  title: string;
  description?: ReactNode;
  defaultValue?: string;
  placeholder?: string;
  confirmLabel?: string;
  /**
   * 提交前校验。返回 `null` 表示通过，返回字符串则是给用户看的错误文案。
   * 校验放在这里而不是调用方：对话框要**在输入时就**禁掉确认按钮，
   * 等到提交才报错等于让用户白填一次。
   */
  validate?: (value: string) => string | null;
  onSubmit: (value: string) => void;
  onClose: () => void;
}

/**
 * 单行输入的对话框（新建分类 / 新建接口）。
 *
 * 回车即提交 —— 这个对话框只干一件事，不该逼用户去够按钮。
 * 但**不**用 `<form>`：表单的默认提交行为会把整页刷新掉，
 * 而那正是「在 Tauri 里套着 WebView」最容易踩的一脚。
 */
export function PromptDialog({
  title,
  description,
  defaultValue = "",
  placeholder,
  confirmLabel,
  validate,
  onSubmit,
  onClose,
}: PromptDialogProps) {
  const { t } = useI18n();
  const [value, setValue] = useState(defaultValue);

  const trimmed = value.trim();
  const error = validate ? validate(trimmed) : null;
  const canSubmit = trimmed !== "" && error === null;
  // 空的输入框不该一打开就飘红字。「还没填」由按钮置灰表达就够了 ——
  // 一上来就报错，用户会以为自己已经做错了什么
  const shownError = trimmed === "" ? null : error;

  const submit = () => {
    if (!canSubmit) return;
    onSubmit(trimmed);
    onClose();
  };

  return (
    <Dialog
      title={title}
      description={description}
      onClose={onClose}
      width={380}
      footer={
        <>
          <Button size="sm" variant="secondary" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button size="sm" variant="primary" disabled={!canSubmit} onClick={submit}>
            {confirmLabel ?? t("common.confirm")}
          </Button>
        </>
      }
    >
      <div className="space-y-1.5">
        <input
          type="text"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              submit();
            }
          }}
          placeholder={placeholder}
          spellCheck={false}
          aria-label={title}
          aria-invalid={shownError !== null}
          className={cn(
            "h-8 w-full rounded-sm border bg-surface px-2.5",
            "text-xs text-ink placeholder:text-ink-faint",
            "transition-colors duration-100",
            shownError ? "border-err" : "border-line hover:border-line-strong focus:border-brand",
          )}
        />
        {/* 校验错误就地显示：这里说明白比让按钮「莫名其妙点不动」友好 */}
        {shownError ? <p className="text-[11px] text-err">{shownError}</p> : null}
      </div>
    </Dialog>
  );
}
