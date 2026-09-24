import { useCallback, useEffect, useState } from "react";
import { Panel, usePanelRef } from "react-resizable-panels";
import type { PanelSize } from "react-resizable-panels";
import { matchShortcut, SECTION_SHORTCUTS } from "@/lib/constants";
import { watchExternalChanges } from "@/lib/externalChanges";
import * as ipc from "@/lib/ipc";
import { bootstrapSettings } from "@/lib/sync";
import { useTheme } from "@/lib/useTheme";
import { useRequestStore } from "@/store/request";
import { tokenCounts, useTokenStore } from "@/store/token";
import { useUiStore, type SidebarSection } from "@/store/ui";
import { useWorkspaceStore } from "@/store/workspace";
import { CommandPalette } from "@/features/command/CommandPalette";
import { goToSection, openCreate } from "@/features/command/registry";
import { ActivityRail } from "@/features/layout/ActivityRail";
import { Sidebar } from "@/features/layout/Sidebar";
import { SplitHandle, SplitPane } from "@/features/layout/SplitPane";
import { StatusBar } from "@/features/layout/StatusBar";
import { TopBar } from "@/features/layout/TopBar";
import { HistoryWorkspace } from "@/features/history/HistoryWorkspace";
import { LoadtestWorkspace } from "@/features/loadtest/LoadtestWorkspace";
import { RequestPane } from "@/features/request/RequestPane";
import { ResponsePane } from "@/features/response/ResponsePane";
import { ToolsWorkspace } from "@/features/tools/ToolsWorkspace";

/**
 * 哪些板块**不显示侧栏**。
 *
 * 目前是空的 —— 曾经用过一次（「压测」板块），但那个板块被改回
 * 「历史轮次列表放主视图」之后，它和别的板块没有区别了，不需要特殊对待。
 * 留着这个常量是为了让「侧栏可以按板块关掉」这件事有个明确的落点，
 * 而不是散在 JSX 里的条件判断。
 */
const SIDEBAR_LESS: readonly SidebarSection[] = [];

/**
 * 应用骨架。
 *
 * 布局结构：
 *   TopBar
 *   ├─ ActivityRail（固定 56px）
 *   └─ SplitPane#main（横向可拖）
 *      ├─ Panel#sidebar  → Sidebar
 *      └─ Panel#workspace → **跟着左侧板块切**：
 *         ├─ 「历史」 → HistoryWorkspace（左列表 / 右详情）
 *         ├─ 「压测」 → LoadtestWorkspace（左历史轮次 / 右配置与报告）
 *         ├─ 「工具」 → ToolsWorkspace
 *         └─ 其余     → SplitPane#workspace（请求 / 响应）
 *   StatusBar
 *
 * 中间那块是这一版唯一随板块切换的区域 —— 它曾经写死成请求/响应，
 * 于是点进「历史」「压测」「工具」什么也没变，各板块的内容只能挤在侧栏
 * 或某个请求页签里。回收站 / 设置的主视图是后续单独一轮的事。
 */
export function App() {
  const { toggle: toggleTheme } = useTheme();

  const collapsed = useUiStore((s) => s.sidebarCollapsed);
  const setSidebarCollapsed = useUiStore((s) => s.setSidebarCollapsed);
  const section = useUiStore((s) => s.section);
  const hasSidebar = !SIDEBAR_LESS.includes(section);
  const workspaceStatus = useWorkspaceStore((s) => s.status);
  const workspaceId = useWorkspaceStore((s) => s.workspace?.id);
  const requestRunning = useRequestStore((s) => s.runId !== null);

  /**
   * 状态栏的两个数字（Token 计数）。
   *
   * `pendingTokenCount` 是**派生**的（`sources × statuses` 的函数），不在 store 里存一份 ——
   * 存一份就得在每次刷新后同步更新，而这种同步迟早会漏。
   */
  const pendingTokens = useTokenStore((s) => tokenCounts(s).pending);
  const validTokens = useTokenStore((s) => tokenCounts(s).valid);

  /** 版本号以后端为准（`get_app_info`），拿不到就回退到构建期常量 */
  const [appVersion, setAppVersion] = useState<string | undefined>(undefined);

  /** 不在 Tauri 里说明后端进程根本没起来（比如浏览器直接开 dev server） */
  const offline = !ipc.isTauriRuntime();

  const sidebarRef = usePanelRef();

  /**
   * 启动序列。四件事互相独立，所以并发发起：
   * 1. 设置：读 SQLite → 纠偏主题 → 之后每次切主题写回库
   * 2. 应用信息：状态栏的版本号
   * 3. 集合树：工作区 / 文件夹 / 请求
   * 4. 外部改动监听：MCP / CLI 改了同一个库时把界面叫醒
   *
   * 全都**不阻塞首帧** —— 渲染完骨架再填充，比白屏等 IPC 体验好得多。
   */
  useEffect(() => {
    let stopThemeSync: (() => void) | undefined;
    let cancelled = false;

    void bootstrapSettings().then((cleanup) => {
      // StrictMode 下 effect 会跑两遍：第一遍的 promise 回来时已经 cleanup 过了，
      // 此时立刻退订，否则会残留一个订阅者
      if (cancelled) cleanup();
      else stopThemeSync = cleanup;
    });

    void ipc.app
      .info()
      .then((info) => setAppVersion(info.appVersion))
      .catch((e) => {
        console.warn("[reqly] 读取应用信息失败：", ipc.toIpcError(e).debug);
      });

    void useWorkspaceStore.getState().load();

    // 同步注册（不等 IPC），所以在 StrictMode 下第二遍挂载会立刻拿到一个
    // 同步的 stop 函数，不会像上面那个 promise 那样漏掉退订
    const stopExternalWatch = watchExternalChanges();

    return () => {
      cancelled = true;
      stopThemeSync?.();
      stopExternalWatch();
    };
  }, []);

  /**
   * Token 的启动加载。
   *
   * ⚠️ 依赖必须是 `workspaceId`：首挂时 `workspace.load()` 还在路上，
   * 拿到的 workspace 是 null 会直接返回 —— 那样状态栏会显示「0 个有效」且没报错。
   */
  useEffect(() => {
    if (workspaceId === undefined) return;
    void useTokenStore.getState().load();
  }, [workspaceId]);

  // 拖拽到最小宽度会触发库的自动折叠，这里把状态同步回 store，
  // 否则 TopBar 上的图标会跟实际状态对不上。
  const handleSidebarResize = useCallback(
    (size: PanelSize) => {
      setSidebarCollapsed(size.inPixels <= 1);
    },
    [setSidebarCollapsed],
  );

  // store → 面板：只在状态真的不一致时动手，避免来回抖动
  useEffect(() => {
    const panel = sidebarRef.current;
    if (!panel) return;
    if (collapsed && !panel.isCollapsed()) panel.collapse();
    else if (!collapsed && panel.isCollapsed()) panel.expand();
  }, [collapsed, sidebarRef]);

  // 全局快捷键
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (matchShortcut(event, "toggleTheme")) {
        event.preventDefault();
        toggleTheme();
        return;
      }

      // Ctrl+1 / 2 / 3：集合 / 历史 / 回收站
      //
      // 这三条与命令面板里的「前往 X」是**同一个动作**，所以不在这里重写一遍 ——
      // 走注册表里的 `goToSection`，行为（含折叠时自动展开）只有一份
      for (const { name, section } of SECTION_SHORTCUTS) {
        if (!matchShortcut(event, name)) continue;
        event.preventDefault();
        goToSection(section);
        return;
      }

      // Ctrl+N 新建请求。对话框挂在集合树里，所以先切到「集合」——
      // 这条也是注册表里的命令，同样不重写
      if (matchShortcut(event, "newRequest")) {
        event.preventDefault();
        openCreate("request");
        return;
      }

      // Ctrl+K 命令面板（跳转任意请求 / 执行任意命令）
      if (matchShortcut(event, "commandPalette")) {
        event.preventDefault();
        useUiStore.getState().togglePalette();
        return;
      }

      // Ctrl+Enter 发送。放全局而不是绑在某个输入框上：
      // 焦点在响应区、在集合树、在请求体里都应该能发出去。
      if (matchShortcut(event, "send")) {
        event.preventDefault();
        const request = useRequestStore.getState();
        // 地址为空时不发 —— 和后端那句"地址不合法"比起来，
        // 什么都不发生更接近用户的预期（他就是还没填完）
        if (request.draft.url.trim() !== "") void request.send();
        return;
      }

      // Esc 取消。刻意不要求修饰键：请求卡住时第一反应就是按 Esc。
      // 没在跑的时候它是空操作，所以不会干扰别处的 Esc 语义。
      if (event.key === "Escape") {
        void useRequestStore.getState().cancel();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [toggleTheme]);

  return (
    <div className="flex h-full flex-col overflow-hidden bg-canvas text-ink">
      <TopBar />

      <div className="flex min-h-0 flex-1">
        <ActivityRail />

        {/* 「压测」板块没有侧栏 —— 它该放的东西已经全在板块自己的页签里了。
            留着那条空栏白占 240px，还让人以为有内容没加载出来。
            整块让给主视图，历史列表因此能铺满（地址不用再截断） */}
        {hasSidebar ? (
          <SplitPane id="main" orientation="horizontal" className="min-w-0 flex-1">
            <Panel
              id="sidebar"
              defaultSize={260}
              minSize={180}
              maxSize={480}
              collapsible
              collapsedSize={0}
              onResize={handleSidebarResize}
              panelRef={sidebarRef}
            >
              <Sidebar />
            </Panel>

            <SplitHandle orientation="horizontal" />

            <Panel id="workspace" minSize="35%">
              <Workspace section={section} />
            </Panel>
          </SplitPane>
        ) : (
          // `min-w-0` 不能少：主视图里有长 URL 与图表，没有它这一块会被顶宽
          <div className="min-w-0 flex-1">
            <Workspace section={section} />
          </div>
        )}
      </div>

      <StatusBar
        status={
          workspaceStatus === "error"
            ? "error"
            : requestRunning
              ? "sending"
              : "ready"
        }
        pendingTokens={pendingTokens}
        validTokens={validTokens}
        offline={offline}
        appVersion={appVersion}
      />

      {/* 命令面板自己 portal 到 body，挂在哪一层都一样 ——
          放在最后只是为了「读 JSX 时先看到布局、再看到浮层」 */}
      <CommandPalette />
    </div>
  );
}

/**
 * 主视图，**跟着左侧板块切**。
 *
 * 已经各自有主视图的板块：历史、压测、工具。
 * 其余（集合 / 回收站 / 设置）暂时共用请求/响应 —— 那些板块的主视图是
 * 后续单独一轮的事，每个都要按自己的需要重排，不是照搬。
 */
function Workspace({ section }: { section: SidebarSection }) {
  if (section === "history") return <HistoryWorkspace />;
  if (section === "loadtest") return <LoadtestWorkspace />;
  if (section === "tools") return <ToolsWorkspace />;

  return (
    <SplitPane id="workspace" orientation="vertical">
      <Panel id="request" defaultSize="56%" minSize={140}>
        <RequestPane />
      </Panel>

      <SplitHandle orientation="vertical" />

      <Panel id="response" minSize={120}>
        <ResponsePane />
      </Panel>
    </SplitPane>
  );
}

export default App;
