import { useEffect, useState } from "react";

import * as ipc from "@/lib/ipc";
import type { TokenSource, TokenStatus } from "@/lib/types";
import { useTokenStore } from "@/store/token";

/**
 * 这条请求实际会用到哪个 Token 来源。
 *
 * 前端**自己不做推导**：判定规则是
 * 「先沿分类链找最近的，再兜底到系统分类那个」，而这条规则连同它的 SQL
 * （`first_in_chain` / `system_source`）都已经在后端了。再写一遍就多一处
 * 会漂移的东西 —— 而漂移的后果是「界面说生效的是 A，发送时用的是 B」。
 *
 * ⚠️ `folderId` 为 `null`（请求还没归档）时**同样要解析**：Token 来源固定配在
 * 系统分类下，那恰恰是最常见的情形。早先这里直接置 `null`，于是所有没归档的
 * 请求一律显示「还没有配置」。
 *
 * ## 为什么抽成独立文件
 *
 * 两个地方要问同一个问题：「请求头」页那块折叠的 Token 来源要显示
 * 「生效的是哪一个」，发送区要在发送**前**就把倒计时和「立即刷新」摆出来。
 * 复制一份的下场是两处对 `version` 的处理迟早不一致，
 * 表现为「改完 Token 来源，发送区还是旧的来源」。
 *
 * @param version 配置变更后递增，用来触发重新解析
 */
export function useResolvedTokenSource(
  folderId: number | null,
  version = 0,
): { source: TokenSource | null } {
  const [source, setSource] = useState<TokenSource | null>(null);

  useEffect(() => {
    let cancelled = false;
    void ipc.tokens
      .resolve(folderId)
      .then((next) => {
        if (!cancelled) setSource(next);
      })
      .catch((e) => {
        if (cancelled) return;
        console.warn("[reqly] 解析 Token 来源失败：", ipc.toIpcError(e).debug);
        setSource(null);
      });

    return () => {
      cancelled = true;
    };
  }, [folderId, version]);

  return { source };
}

/**
 * 把后端给的「还剩多少秒」变成一个**会自己走**的倒计时。
 *
 * ## 为什么前端敢自己走秒
 *
 * 「算不算过期」的权威在后端 —— `skew_seconds` 的判定只有一份
 * （`http::token::is_expired`），前端再算一遍就会在边界上分叉。
 * 但「距离那个时间点还有多久」只是个减法：每秒重新问一次后端，
 * 是拿一趟 IPC 换一个纯算术结果，换来的还有「数字卡住一下再跳两秒」的抖动。
 *
 * 所以基准值来自后端、推进在本地，而**判定不参与**：
 * 「有效 / 已过期」这两种说法始终来自 `status.expired`，
 * 这个 hook 只提供数字。倒计时走到负数是正常的（就是已经过期了多久），
 * 不会反过来改变徽标的状态。
 *
 * ## 为什么锚点是 `updatedAt`
 *
 * 状态每次重新查回来（或刷新完）`updatedAt` 会跟着变，拿它当依赖就能让
 * 倒计时自动重新对齐 —— 不需要调用方记得在刷新成功后手动调一个 `reset()`。
 * 那种「要记得调」的约定，漏一处就是「刚刷完的 token 显示的还是旧倒计时」。
 */
export function useTokenCountdown(status: TokenStatus | null): number | null {
  const base = status?.remainingSeconds ?? null;
  const anchor = status?.updatedAt ?? null;
  const [tick, setTick] = useState(0);

  useEffect(() => {
    if (base === null) return;

    const startedAt = Date.now();
    setTick(0); // 基准刚换过，先把偏移清零，别让上一轮的秒数残留一帧

    const timer = window.setInterval(() => {
      setTick(Math.trunc((Date.now() - startedAt) / 1000));
    }, 1000);

    return () => window.clearInterval(timer);
  }, [base, anchor]);

  return base === null ? null : base - tick;
}

/**
 * 还有哪些来源与它**共用同一份 token**（名字列表，不含自己）。
 *
 * ## 为什么需要展示出来
 *
 * 「配置相同的来源共用一份 token 与一个倒计时」是在**数据库那一层**成立的
 * （后端 `repo::token::cache_owner`：同一个共享指纹 ⇒ 同一行缓存）。
 * 用户配完第二条来源，看到的倒计时和第一条一模一样 —— 不解释的话，
 * 第一反应是「我这条配置没生效」，然后是「那我去改点什么让它生效」，
 * 而真正该有的反应是「这两条配的是同一个取 token 接口，本来就该共用」。
 *
 * ## 判定依据
 *
 * `cacheOwnerId` 是后端给的代表行 id，不是前端算什么。前端只要按它分组 ——
 * 任何「自己推导共享关系」的写法都是在复制后端的规则，迟早分叉。
 *
 * 拿不到状态（还没查过 / 后端没给这个字段）时返回空列表：宁可不说，
 * 也不要报一个自己都没把握的共用关系。
 *
 * @param sourceId 当前生效/正在查看的来源
 * @param cacheOwnerId 它的代表行 id（`null` / 负数 = 不知道）
 */
export function useSharedTokenSources(
  sourceId: number | null,
  cacheOwnerId: number | null,
): string[] {
  // 两个选择器都落在 store 自己的对象引用上（不是每次新建的对象），
  // 否则 zustand 会当成「变了」而无限重渲染
  const sources = useTokenStore((s) => s.sources);
  const statuses = useTokenStore((s) => s.statuses);

  if (sourceId === null || cacheOwnerId === null || cacheOwnerId < 0) return [];

  return sources
    .filter((s) => s.id !== sourceId && statuses[s.id]?.cacheOwnerId === cacheOwnerId)
    .map((s) => s.name);
}
