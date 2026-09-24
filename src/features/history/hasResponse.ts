import type { HistoryEntry } from "@/lib/types";

/**
 * 这条历史有没有响应可看。
 *
 * 单独一个模块而不是留在某个组件里：它有两个调用方（列表决定点开是否有东西、
 * 详情面板决定显示响应还是「这次没有响应」），而它们的判断**必须一致** ——
 * 两边不一致就会出现「能点开、点开是空的」。
 */
export function hasResponse(entry: HistoryEntry): boolean {
  return entry.status !== null || entry.respBody !== null;
}
