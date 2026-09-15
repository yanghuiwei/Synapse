import { useCallback, useEffect, useState } from "react";
import { Panel, usePanelRef } from "react-resizable-panels";
import type { PanelSize } from "react-resizable-panels";
import { matchShortcut, SECTION_SHORTCUTS } from "@/lib/constants";
import { watchExternalChanges } from "@/lib/externalChanges";
import * as ipc from "@/lib/ipc";
import { bootstrapSettings } from "@/lib/sync";
import { useTheme } from "@/lib/useTheme";
import { useEnvStore } from "@/store/env";
import { useRequestStore } from "@/store/request";
import { tokenCounts, useTokenStore } from "@/store/token";
import { useUiStore } from "@/store/ui";
import { useWorkspaceStore } from "@/store/workspace";
import { ActivityRail } from "@/features/layout/ActivityRail";
import { Sidebar } from "@/features/layout/Sidebar";
import { SplitHandle, SplitPane } from "@/features/layout/SplitPane";
import { StatusBar } from "@/features/layout/StatusBar";
import { TopBar } from "@/features/layout/TopBar";
import { RequestPane } from "@/features/request/RequestPane";
import { ResponsePane } from "@/features/response/ResponsePane";

/**
 * 应用骨架。
 *
 * 布局结构：
 *   TopBar
 *   ├─ ActivityRail（固定 56px）
 *   └─ SplitPane#main（横向可拖）
 *      ├─ Panel#sidebar  → Sidebar
 *      └─ Panel#workspace
 *         └─ SplitPane#workspace（纵向可拖）
 *            ├─ Panel#request  → RequestPane
 *            └─ Panel#response → ResponsePane
 *   StatusBar
 */
export function App() {
  const { toggle: toggleTheme } = useTheme();

  const collapsed = useUiStore((s) => s.sidebarCollapsed);
  const setSidebarCollapsed = useUiStore((s) => s.setSidebarCollapsed);
  const workspaceStatus = useWorkspaceStore((s) => s.status);
  const workspaceId = useWorkspaceStore((s) => s.workspace?.id);
  const requestRunning = useRequestStore((s) => s.runId !== null);

  /**
   * 状态栏的两个数字。
   *
   * `pendingTokenCount` 是**派生**的（`sources × statuses` 的函数），不在 store 里存一份 ——
   * 存一份就得在每次刷新后同步更新，而这种同步迟早会漏。
   */
  const environmentName = useEnvStore((s) => s.active?.name ?? null);
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
   * 环境与 Token 的启动加载。
   *
   * ⚠️ 依赖必须是 `workspaceId`：首挂时 `workspace.load()` 还在路上，
   * 两个 store 的 `load()` 拿到的 workspace 是 null 会直接返回 ——
   * 那样状态栏会**永远**显示「不使用环境 / 0 个有效」，而且不报错。
   */
  useEffect(() => {
    if (workspaceId === undefined) return;
    void useEnvStore.getState().load();
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
      for (const { name, section } of SECTION_SHORTCUTS) {
        if (!matchShortcut(event, name)) continue;
        event.preventDefault();
        const ui = useUiStore.getState();
        ui.setSection(section);
        // 侧栏被折叠时切板块等于什么都没发生 —— 顺手展开，
        // 否则用户会以为快捷键坏了
        if (ui.sidebarCollapsed) ui.setSidebarCollapsed(false);
        return;
      }

      // Ctrl+N 新建请求。先切到「集合」：新建对话框挂在集合树里，
      // 在历史/回收站板块下弹不出来
      if (matchShortcut(event, "newRequest")) {
        event.preventDefault();
        const ui = useUiStore.getState();
        ui.setSection("collections");
        if (ui.sidebarCollapsed) ui.setSidebarCollapsed(false);
        ui.openCreateDialog("request", null);
        return;
      }

      // Ctrl+K 聚焦侧栏搜索框。当前板块没有搜索框时它是空操作
      if (matchShortcut(event, "focusSearch")) {
        event.preventDefault();
        document
          .querySelector<HTMLInputElement>("[data-sidebar-search]")
          ?.focus();
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

        <SplitPane
          id="main"
          orientation="horizontal"
          className="min-w-0 flex-1"
        >
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
            <SplitPane id="workspace" orientation="vertical">
              <Panel id="request" defaultSize="56%" minSize={140}>
                <RequestPane />
              </Panel>

              <SplitHandle orientation="vertical" />

              <Panel id="response" minSize={120}>
                <ResponsePane />
              </Panel>
            </SplitPane>
          </Panel>
        </SplitPane>
      </div>

      <StatusBar
        status={
          workspaceStatus === "error"
            ? "error"
            : requestRunning
              ? "sending"
              : "ready"
        }
        environmentName={environmentName}
        pendingTokens={pendingTokens}
        validTokens={validTokens}
        offline={offline}
        appVersion={appVersion}
      />
    </div>
  );
}

export default App;
