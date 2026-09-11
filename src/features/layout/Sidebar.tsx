import {
  Clock,
  FolderPlus,
  Gauge,
  Globe,
  Import,
  Layers,
  LoaderCircle,
  Plus,
  Search,
  Settings,
  Trash,
  TriangleAlert,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useState } from "react";
import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n";
import { useUiStore, type SidebarSection } from "@/store/ui";
import { useWorkspaceStore } from "@/store/workspace";
import { Button, IconButton } from "@/features/ui/Button";
import { EmptyState } from "@/features/ui/EmptyState";
import { CollectionTree } from "@/features/collections/CollectionTree";

/** 顶部的一枚操作按钮 */
interface SidebarAction {
  labelKey: MessageKey;
  icon: LucideIcon;
  danger?: boolean;
  primary?: boolean;
}

interface SectionMeta {
  titleKey: MessageKey;
  /** 空态用的图标 */
  emptyIcon: LucideIcon;
  emptyTitleKey: MessageKey;
  emptyDescKey?: MessageKey;
  actions: readonly SidebarAction[];
  searchable: boolean;
  searchPlaceholderKey?: MessageKey;
}

const SECTION_META: Record<SidebarSection, SectionMeta> = {
  collections: {
    titleKey: "nav.collections",
    emptyIcon: Layers,
    emptyTitleKey: "common.empty",
    emptyDescKey: "sidebar.emptyCollections",
    actions: [
      { labelKey: "sidebar.newRequest", icon: Plus, primary: true },
      { labelKey: "sidebar.newFolder", icon: FolderPlus },
      { labelKey: "sidebar.import", icon: Import },
    ],
    searchable: true,
    searchPlaceholderKey: "sidebar.filterPlaceholder",
  },
  history: {
    titleKey: "nav.history",
    emptyIcon: Clock,
    emptyTitleKey: "common.empty",
    emptyDescKey: "sidebar.emptyHistory",
    actions: [{ labelKey: "history.clearAll", icon: Trash }],
    searchable: true,
    searchPlaceholderKey: "history.searchPlaceholder",
  },
  environments: {
    titleKey: "nav.environments",
    emptyIcon: Globe,
    emptyTitleKey: "common.empty",
    actions: [{ labelKey: "env.new", icon: Plus, primary: true }],
    searchable: true,
    searchPlaceholderKey: "common.search",
  },
  loadtest: {
    titleKey: "nav.loadtest",
    emptyIcon: Gauge,
    emptyTitleKey: "common.empty",
    emptyDescKey: "loadtest.noRuns",
    actions: [],
    searchable: false,
  },
  trash: {
    titleKey: "nav.trash",
    emptyIcon: Trash,
    emptyTitleKey: "common.empty",
    emptyDescKey: "history.emptyTrash",
    actions: [{ labelKey: "history.deleteForever", icon: Trash, danger: true }],
    searchable: false,
  },
  settings: {
    titleKey: "nav.settings",
    emptyIcon: Settings,
    emptyTitleKey: "common.empty",
    actions: [],
    searchable: false,
  },
};

function SearchField({
  placeholder,
  className,
}: {
  placeholder: string;
  className?: string;
}) {
  const [value, setValue] = useState("");
  return (
    <div className={cn("relative", className)}>
      <Search
        size={14}
        strokeWidth={2}
        aria-hidden
        className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-ink-faint"
      />
      <input
        type="search"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        placeholder={placeholder}
        className={cn(
          "h-7 w-full rounded-sm border border-line bg-surface pl-7 pr-2",
          "text-xs text-ink placeholder:text-ink-faint",
          "transition-colors duration-100 hover:border-line-strong focus:border-brand",
        )}
      />
    </div>
  );
}

/**
 * 「集合」板块的内容。
 *
 * 三态都要有交代：加载中（首次启动要建池 + 跑迁移，不是瞬时的）、
 * 失败（带重试 —— 库被占用是可以重试成功的）、就绪。
 */
function CollectionsBody() {
  const { t } = useI18n();
  const status = useWorkspaceStore((s) => s.status);
  const error = useWorkspaceStore((s) => s.error);
  const load = useWorkspaceStore((s) => s.load);

  if (status === "idle" || status === "loading") {
    return (
      <div className="mt-8 flex items-center justify-center gap-2 px-6 text-xs text-ink-faint">
        <LoaderCircle size={13} strokeWidth={2.2} className="animate-spin" />
        <span>{t("common.loading")}</span>
      </div>
    );
  }

  if (status === "error") {
    return (
      <EmptyState
        icon={TriangleAlert}
        title={t("collections.loadFailed")}
        // 错误文案来自后端错误码，不在这里写死（见 lib/ipc.ts 的契约）
        description={error?.localized}
        action={
          <Button size="sm" variant="secondary" onClick={() => void load()}>
            {t("common.retry")}
          </Button>
        }
        className="mt-8"
      />
    );
  }

  return <CollectionTree />;
}

/**
 * 左栏内容区。宽度由外层 SplitPane 的 Panel 决定，这里不做宽度管理。
 */
export function Sidebar() {
  const { t } = useI18n();
  const section = useUiStore((s) => s.section);
  const meta = SECTION_META[section];

  return (
    <aside className="flex h-full min-h-0 flex-col border-r border-line bg-canvas-subtle">
      {/* 标题行 */}
      <header className="flex h-10 shrink-0 items-center gap-1 px-2.5">
        <h2 className="flex-1 truncate text-[13px] font-semibold tracking-tight text-ink">
          {t(meta.titleKey)}
        </h2>
        {meta.actions.map((a) => {
          const Icon = a.icon;
          const label = t(a.labelKey);
          return a.primary ? (
            <Button
              key={a.labelKey}
              size="sm"
              variant="primary"
              icon={<Icon size={13} strokeWidth={2.2} />}
            >
              {label}
            </Button>
          ) : (
            <IconButton
              key={a.labelKey}
              size="sm"
              label={label}
              variant={a.danger ? "danger" : "ghost"}
            >
              <Icon size={14} strokeWidth={2} />
            </IconButton>
          );
        })}
      </header>

      {/* 搜索 */}
      {meta.searchable ? (
        <div className="shrink-0 px-2.5 pb-2">
          <SearchField
            placeholder={t(meta.searchPlaceholderKey ?? "common.search")}
          />
        </div>
      ) : null}

      {/* 列表 / 空态 */}
      <div className="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2">
        {section === "collections" ? (
          <CollectionsBody />
        ) : (
          <EmptyState
            icon={meta.emptyIcon}
            title={t(meta.emptyTitleKey)}
            description={meta.emptyDescKey ? t(meta.emptyDescKey) : undefined}
            className="mt-8"
          />
        )}
      </div>
    </aside>
  );
}
