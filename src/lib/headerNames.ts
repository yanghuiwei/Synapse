/* ============================================================
   头名用量统计 —— 给「插入凭据头」的下拉排序用
   ------------------------------------------------------------
   为什么要有它：第一版设计里的快捷插入是硬编码的 RFC 三件套
   （`Authorization` / `X-Api-Key`）。查过真库之后这个设计是错的 ——
   真正在用的鉴权头是 `X-Access-Token`（15 处，值是**裸 token** 不带前缀），
   三个 RFC 按钮一条都覆盖不到。

   所以候选头名改为「从你自己的库里长出来」：读一次工作区，数一遍
   各请求的 `headers` 里出现过哪些头名、各多少次，按次数降序。

   ⚠️ **纯显示用途，不参与任何语义判断。**
   它只决定下拉里谁排前面，不决定「哪个头算凭据」——那个判断在 Rust 的
   `redact.rs`（打码用）与本目录的 `headerNames` 无关。数错了的代价
   仅限于「下拉排序不理想」，绝不会让请求发出不一样的东西。

   顺带一提：这里**不需要**新表。头名本来就分散在各请求的 headers JSON 里，
   工作区加载完就在内存里，数一遍是 O(总行数)。
   ============================================================ */

import type { ApiRequest } from "./types";

export interface HeaderNameUsage {
  /** 显示用的拼写 */
  name: string;
  count: number;
}

/**
 * 内置候选：库里一个都用过时的兜底。
 *
 * 新装的工作区里 headers 是空的，只给一个「自定义」会让面板看起来是坏的。
 * 这几个是通用度最高的写法，按 RFC 排在前面 —— 用过之后它们会被
 * 真实用量挤到后面去。
 */
export const BUILTIN_HEADER_NAMES: readonly string[] = [
  "Authorization",
  "X-Access-Token",
  "X-Api-Key",
  "X-Auth-Token",
];

/**
 * 数一遍头名用量，按次数降序。
 *
 * 三条规则：
 *
 * 1. **只数启用的行**（与设计文档 §1.1 的口径一致）。停用行是用户
 *    试过又放弃的写法，让它占着下拉的首位不合适。
 * 2. **大小写不敏感合并**。真库里 `authorization` 与 `Authorization`
 *    各出现过一次 —— 当成两个候选是纯噪音，HTTP 头名本来就不区分大小写。
 *    合并后显示**出现次数最多的那个拼写**（同分时用先出现的），
 *    因为它最接近用户自己的习惯。
 * 3. 同分时按名字字典序。不是为了好看，是为了**稳定**：排序随渲染抖动的话，
 *    下拉里的位置每次打开都不一样，肌肉记忆就白建了。
 */
export function headerNameUsage(requests: readonly ApiRequest[]): HeaderNameUsage[] {
  /** 小写名 → (实际拼写 → 次数) */
  const byLower = new Map<string, Map<string, number>>();

  for (const request of requests) {
    for (const row of request.headers) {
      if (!row.enabled) continue;
      const name = row.key.trim();
      if (name === "") continue;

      const lower = name.toLowerCase();
      const spellings = byLower.get(lower) ?? new Map<string, number>();
      spellings.set(name, (spellings.get(name) ?? 0) + 1);
      byLower.set(lower, spellings);
    }
  }

  const usage: HeaderNameUsage[] = [];
  for (const spellings of byLower.values()) {
    let best = "";
    let bestCount = 0;
    let total = 0;
    for (const [name, count] of spellings) {
      total += count;
      if (count > bestCount) {
        best = name;
        bestCount = count;
      }
    }
    usage.push({ name: best, count: total });
  }

  return usage.sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
}

/**
 * 下拉里的完整候选列表 = 用过的（按次数降序）+ 还没用过的内置项。
 *
 * 内置项补在**后面**而不是混进去排序：它们是「猜的」，而用过的头名是
 * 「你实际在用的」。让猜的排在真实用量前面，等于把这个功能的意义反过来写。
 */
export function headerNameChoices(requests: readonly ApiRequest[]): HeaderNameUsage[] {
  const used = headerNameUsage(requests);
  const taken = new Set(used.map((u) => u.name.toLowerCase()));

  const builtins = BUILTIN_HEADER_NAMES.filter((n) => !taken.has(n.toLowerCase())).map(
    (name) => ({ name, count: 0 }),
  );

  return [...used, ...builtins];
}
