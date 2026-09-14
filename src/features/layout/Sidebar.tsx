import {
  Clock,
  FolderPlus,
  Gauge,
  Globe,
  Import,
  Layers,
  LoaderCircle,
  Plug,
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
import { useHistoryStore } from "@/store/history";
import { useUiStore, type SidebarSection } from "@/store/ui";
import { useWorkspaceStore } from "@/store/workspace";
import { Button, IconButton } from "@/features/ui/Button";
import { ConfirmDialog } from "@/features/ui/Dialog";
import { EmptyState } from "@/features/ui/EmptyState";
import { CollectionTree } from "@/features/collections/CollectionTree";
import { EnvManager } from "@/features/env/EnvManager";
import { ConnectedImportDialog } from "@/features/import/ImportDialog";
import { HistoryList } from "@/features/history/HistoryList";
import { RunList } from "@/features/loadtest/RunList";
import { McpSetupPanel } from "@/features/mcp/McpSetupPanel";
import { TrashBin } from "@/features/trash/TrashBin";

/**
 * 顶部的一枚操作按钮。
 *
 * `id` 而不是内联回调：这张表是**模块级常量**，而回调要用到 store
 * 与组件状态（确认框）。写在表里就得把 store 也搬到模块作用域，
 * 于是「点按钮发生什么」会被拆到两个地方。现在是「表描述长相，
 * `runAction` 描述行为」。
 */
type ActionId = "newRequest" | "newFolder" | "import" | "clearHistory" | "newEnv" | "mcp";

interface SidebarAction {
  id: ActionId;
  labelKey: MessageKey;
  icon: LucideIcon;
  danger?: boolean;
  primary?: boolean;
  /** 还没实现的入口：画出来但点不动，比凭空消失更容易让人知道「以后会有」 */
  disabled?: boolean;
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
      { id: "newRequest", labelKey: "sidebar.newRequest", icon: Plus, primary: true },
      { id: "newFolder", labelKey: "sidebar.newFolder", icon: FolderPlus },
      // cURL / Postman / ApiPost / Reqly 四种来源（P5）
      { id: "import", labelKey: "sidebar.import", icon: Import },
    ],
    searchable: true,
    searchPlaceholderKey: "sidebar.filterPlaceholder",
  },
  history: {
    titleKey: "nav.history",
    emptyIcon: Clock,
    emptyTitleKey: "common.empty",
    emptyDescKey: "sidebar.emptyHistory",
    actions: [{ id: "clearHistory", labelKey: "history.clearAll", icon: Trash }],
    searchable: true,
    searchPlaceholderKey: "history.searchPlaceholder",
  },
  environments: {
    titleKey: "nav.environments",
    emptyIcon: Globe,
    emptyTitleKey: "common.empty",
    actions: [{ id: "newEnv", labelKey: "env.new", icon: Plus, primary: true }],
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
    // 「清空回收站」交给 `TrashBin` 自己画：它贴着那张总览，
    // 还能把「3 个分类 / 12 个接口 / 40 条历史」写进确认文案
    actions: [],
    searchable: false,
  },
  settings: {
    titleKey: "nav.settings",
    emptyIcon: Settings,
    emptyTitleKey: "common.empty",
    // 真正的设置页要到 P8 才排上；这一栏现在只放「连接 AI 客户端」。
    // 它够格单独占个位置：配置片段要粘到**别的软件**里才生效，
    // 用户在这儿复制完就切窗口走了，不会回来找第二处入口。
    actions: [{ id: "mcp", labelKey: "mcp.entry", icon: Plug, primary: true }],
    searchable: false,
  },
};

/**
 * 搜索框。
 *
 * 受控与否由 `value` 是否传入决定：集合树与历史各有自己的筛选状态
 * （一个在 `ui` store、一个在 `history` store），而环境/压测那两个板块
 * 还没有筛选逻辑 —— 那时它退回非受控，至少输入框本身是能用的。
 */
function SearchField({
  placeholder,
  className,
  value,
  onChange,
}: {
  placeholder: string;
  className?: string;
  value?: string;
  onChange?: (value: string) => void;
}) {
  const [local, setLocal] = useState("");
  const controlled = value !== undefined;
  const current = controlled ? value : local;

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
        // Ctrl+K 靠它找到这个输入框（见 App.tsx 的全局快捷键）
        data-sidebar-search
        value={current}
        onChange={(e) => {
          setLocal(e.target.value);
          onChange?.(e.target.value);
        }}
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

  const openCreateDialog = useUiStore((s) => s.openCreateDialog);
  const openEnvDialog = useUiStore((s) => s.openEnvDialog);
  const openImport = useUiStore((s) => s.openImport);
  const filter = useUiStore((s) => s.filter);
  const setFilter = useUiStore((s) => s.setFilter);
  const historyKeyword = useHistoryStore((s) => s.keyword);
  const setHistoryKeyword = useHistoryStore((s) => s.setKeyword);

  const [clearHistoryOpen, setClearHistoryOpen] = useState(false);
  // 「新建环境」「导入」走 ui store，是因为对话框的宿主在别的子树里。
  // 这一个不同：按钮和面板都在本组件内，local state 就够了 —— 多搬一个
  // 全局状态出去，只会让「谁控制这个框」变得需要跨文件才能读出来。
  const [mcpOpen, setMcpOpen] = useState(false);

  const runAction = (id: ActionId) => {
    switch (id) {
      case "newRequest":
        // 建在哪儿由对话框决定（根级），之后想挪就拖
        openCreateDialog("request", null);
        break;
      case "newFolder":
        openCreateDialog("folder", null);
        break;
      case "clearHistory":
        setClearHistoryOpen(true);
        break;
      case "newEnv":
        // 走 ui store 而不是 local state：对话框的宿主在 `EnvManager`，
        // 而这个按钮在 `Sidebar` 的标题区 —— 两棵子树隔着一层
        openEnvDialog({ kind: "new" });
        break;
      case "import":
        // 与「新建环境」同一个理由：按钮在标题区，对话框挂在下面
        openImport();
        break;
      case "mcp":
        setMcpOpen(true);
        break;
    }
  };

  /** 搜索框接哪个 store —— 每个板块记自己那份筛选，切回来还是原样 */
  const search =
    section === "collections"
      ? { value: filter, onChange: setFilter }
      : section === "history"
        ? { value: historyKeyword, onChange: setHistoryKeyword }
        : { value: undefined, onChange: undefined };

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
          const title = a.disabled ? `${label}（${t("common.comingSoon")}）` : label;
          return a.primary ? (
            <Button
              key={a.id}
              size="sm"
              variant="primary"
              icon={<Icon size={13} strokeWidth={2.2} />}
              disabled={a.disabled}
              title={title}
              onClick={() => runAction(a.id)}
            >
              {label}
            </Button>
          ) : (
            <IconButton
              key={a.id}
              size="sm"
              label={title}
              variant={a.danger ? "danger" : "ghost"}
              disabled={a.disabled}
              onClick={() => runAction(a.id)}
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
            value={search.value}
            onChange={search.onChange}
          />
        </div>
      ) : null}

      {/* 列表 / 空态 */}
      <div className="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2">
        {section === "collections" ? (
          <CollectionsBody />
        ) : section === "history" ? (
          <HistoryList />
        ) : section === "environments" ? (
          <EnvManager />
        ) : section === "loadtest" ? (
          <RunList />
        ) : section === "trash" ? (
          <TrashBin />
        ) : (
          <EmptyState
            icon={meta.emptyIcon}
            title={t(meta.emptyTitleKey)}
            description={meta.emptyDescKey ? t(meta.emptyDescKey) : undefined}
            className="mt-8"
          />
        )}
      </div>

      {/* 清空历史是「能撤」的（记录进回收站），所以不套红色确认按钮 ——
          红色留给真正不可撤销的那几个操作 */}
      {clearHistoryOpen ? (
        <ConfirmDialog
          title={t("history.clearConfirmTitle")}
          description={t("history.clearConfirm")}
          confirmLabel={t("history.clearAll")}
          onConfirm={() => void useHistoryStore.getState().clearAll()}
          onClose={() => setClearHistoryOpen(false)}
        />
      ) : null}

      {/* 挂在侧栏根节点上（它是个 portal），所以切板块不会把用户正在填的
          导入内容卸掉 —— 填到一半去别的板块看一眼是常事 */}
      <ConnectedImportDialog />

      {/* 设置栏现在唯一的实体就是这个面板。开着它去别的板块核对路径是常事，
          所以同样挂在根节点，不随板块切换卸载 */}
      {mcpOpen ? <McpSetupPanel onClose={() => setMcpOpen(false)} /> : null}
    </aside>
  );
}
