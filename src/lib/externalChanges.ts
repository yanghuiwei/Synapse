/* ============================================================
   外部进程改库 → 刷新界面
   ------------------------------------------------------------
   design.md §8.1 承诺的「桌面 App 打开时通过文件监听刷新 UI」。
   后端盯 `PRAGMA data_version`（见 `src-tauri/src/external_changes.rs`），
   变了就发一个空载荷事件；这里负责「收到之后重拉哪些东西」。

   两条规矩，破哪一条都是静默出错：

   1. **只碰列表类状态，绝不重拉正在编辑的那条请求。**
      用户可能正对着输入框打字，而草稿存在前端 store 里 ——
      把库里那份覆盖回来会**静默吃掉他刚敲的字**，且没有任何提示。
      外部改动确实可能改了同一条请求，但那也是「保存时的冲突」，
      该由保存路径去处理（P8 之后的冲突提示），不是这里顺手覆盖。
   2. **去抖。** 后端只保证「一秒最多一次」，而一次外部改动（比如导入
      一段 curl 建了 10 条）会在几毫秒里连发好几个事件。合并掉它们，
      否则一次导入会让界面连着重拉好几轮。
   ============================================================ */

import * as ipc from "./ipc";
import { useEnvStore } from "@/store/env";
import { useHistoryStore } from "@/store/history";
import { useTokenStore } from "@/store/token";
import { useWorkspaceStore } from "@/store/workspace";

/**
 * 合并窗口。比后端 1s 的轮询间隔略长 —— 让同一批改动只触发一次重拉，
 * 同时又不至于让「AI 新建了一个请求」在界面上迟到超过两秒。
 */
const DEBOUNCE_MS = 1200;

/**
 * 开始监听外部改动。返回停止函数（React effect 里直接 return 它）。
 *
 * 不在 Tauri 里（浏览器直接开 dev server）时 `onDbChanged` 是 no-op，
 * 这里也就退化成「监听一个永远不会来的事件」—— 不需要额外的分支。
 */
export function watchExternalChanges(): () => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let stopped = false;

  const refresh = (): void => {
    timer = null;
    if (stopped) return;

    // 集合树：外部的新建 / 改名 / 移动 / 删除都落在这里
    void useWorkspaceStore.getState().refresh();

    // 环境与 Token：状态栏那两个数字（当前环境、有效 / 待刷新 Token 数）来自它们
    void useEnvStore.getState().load();
    void useTokenStore.getState().load();

    // 历史区没打开过就不用管 —— 等它挂载时自然会 load 一次。
    // 与 `store/trash.ts` 的 `syncOthers` 是同一条规矩。
    const history = useHistoryStore.getState();
    if (history.status !== "idle") void history.load();
  };

  const unlisten = ipc.app.onDbChanged(() => {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(refresh, DEBOUNCE_MS);
  });

  return () => {
    stopped = true;
    if (timer !== null) clearTimeout(timer);
    // 退订是异步的（要等 IPC 建好监听）；这里不 await ——
    // 调用点在 effect 的清理函数里，只能是同步的。
    void unlisten.then((off) => off());
  };
}
