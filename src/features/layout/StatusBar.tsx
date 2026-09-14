import { CircleCheck, LoaderCircle, TriangleAlert, WifiOff } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { cn } from "@/lib/cn";
import { APP_VERSION } from "@/lib/constants";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";

export type AppStatus = "ready" | "sending" | "error" | "offline";

interface StatusMeta {
  icon: LucideIcon;
  labelKey: MessageKey;
  className: string;
  spin?: boolean;
}

const STATUS_META: Record<AppStatus, StatusMeta> = {
  ready: {
    icon: CircleCheck,
    labelKey: "status.ready",
    className: "text-ok",
  },
  sending: {
    icon: LoaderCircle,
    labelKey: "status.sending",
    className: "text-brand",
    spin: true,
  },
  error: {
    icon: TriangleAlert,
    labelKey: "status.error",
    className: "text-err",
  },
  offline: {
    icon: WifiOff,
    labelKey: "status.offline",
    className: "text-warn",
  },
};

export interface StatusBarProps {
  status?: AppStatus;
  /** 当前环境名；未选择时传 null */
  environmentName?: string | null;
  /** 待刷新的 Token 数量 */
  pendingTokens?: number;
  /** 已判定有效的 Token 数量（没有待刷新时显示这个，别拿 0 冒充） */
  validTokens?: number;
  /**
   * 后端不可达（例如在浏览器里直接打开 dev server）。
   * 为真时**覆盖** `status` 的显示 —— 这一条比「就绪」重要得多。
   */
  offline?: boolean;
  /** 实际版本号，来自后端 `get_app_info`；拿不到时回退到构建期常量 */
  appVersion?: string;
  className?: string;
}

/**
 * 底部状态栏。固定 26px，数字统一 tabular-nums，
 * 免得计时/计数跳动时整条栏都在抖。
 */
export function StatusBar({
  status = "ready",
  environmentName = null,
  pendingTokens = 0,
  validTokens = 0,
  offline = false,
  appVersion,
  className,
}: StatusBarProps) {
  const { t } = useI18n();
  // offline 优先：后端连不上时显示「就绪」是误导
  const meta = STATUS_META[offline ? "offline" : status];
  const StatusIcon = meta.icon;
  const hasPending = pendingTokens > 0;

  return (
    <footer
      className={cn(
        "flex h-[26px] shrink-0 items-center gap-3 border-t border-line bg-canvas-subtle px-2.5",
        "text-[11px] text-ink-faint",
        className,
      )}
    >
      <span className={cn("flex items-center gap-1.5", meta.className)}>
        <StatusIcon
          size={12}
          strokeWidth={2.2}
          className={cn(meta.spin && "animate-spin")}
        />
        <span className="tabular">{t(meta.labelKey)}</span>
      </span>

      <span className="h-3 w-px bg-line" aria-hidden />

      <span className="flex items-center gap-1">
        {t("status.env")}
        <span className="text-ink-muted">{environmentName ?? t("env.none")}</span>
      </span>

      <span className="flex items-center gap-1">
        {t("status.tokens")}
        <span className={cn("tabular", hasPending ? "text-warn" : "text-ink-muted")}>
          {hasPending
            ? t("status.tokenExpired", { count: pendingTokens })
            : t("status.tokenValid", { count: validTokens })}
        </span>
      </span>

      <div className="flex-1" />

      <span className="tabular">
        {t("status.version", { version: appVersion ?? APP_VERSION })}
      </span>
    </footer>
  );
}
