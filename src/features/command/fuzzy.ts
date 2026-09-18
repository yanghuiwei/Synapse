/* ============================================================
   子序列模糊匹配 —— 命令面板的排序核心
   ------------------------------------------------------------
   只做一件事：给「查询串 → 候选串」打分，分数高者排前面，并给出
   命中字符的位置用于高亮。**不做分词**：中文没有词边界，
   按字符逐位匹配反而更符合直觉（`工时` 能命中 `查询工时统计`）。

   为什么不用库（fuse.js / uFuzzy）：候选集是「几十条命令 + 几百条请求」，
   每次按键全量重算的成本远低于一个依赖的体积与心智负担。
   ============================================================ */

export interface FuzzyHit {
  /**
   * 分值，越大越靠前。
   *
   * 只在**同一批候选之间**做排序，绝对值没有意义 ——
   * 别拿它跟别的查询比，也别写 `score > 50` 这类断言。
   */
  score: number;
  /** 命中字符在目标串里的下标（按 UTF-16 code unit，与 `charAt` 一致），用于高亮 */
  positions: number[];
}

/**
 * 视为「词首」的分隔符。
 *
 * 命中词首额外加分：搜 `gw` 时 `get-weather` 应该排在 `longwidth` 前面 ——
 * 前者是缩写习惯，后者只是碰巧撞上。
 */
const SEPARATORS = " -_/.:?&=\\";

function isWordStart(target: string, index: number): boolean {
  if (index === 0) return true;
  return SEPARATORS.includes(target.charAt(index - 1));
}

/**
 * 匹配一次。
 *
 * @returns 命中则返回分值与前缀位置；**不命中返回 `null`**（而不是 0 分）——
 *   调用方要据此把候选整个丢掉，用 0 分表示「没命中」会跟
 *   「命中了但分数很低」混在一起。
 */
export function fuzzyScore(query: string, target: string): FuzzyHit | null {
  if (query === "") return { score: 0, positions: [] };
  if (target === "") return null;

  const q = query.toLowerCase();
  const t = target.toLowerCase();

  // 连续子串是最强的信号：搜 `user` 时 `getUserById` 应当排在
  // 「u…s…e…r 散落在整条串上」的那种命中前面。先给一笔大额加分，
  // 后面再按子序列的细粒度补分 —— 两笔都算，不互斥。
  const substring = t.indexOf(q);
  const positions: number[] = [];
  let score = substring >= 0 ? 60 - Math.min(substring, 20) : 0;

  let cursor = 0;
  let streak = 0;

  // 逐 **code unit** 迭代（`charAt`）而不是 `for…of`：后者按码点走，
  // 遇到代理对（emoji）时下标会与 `positions` 的口径错开。
  for (let qi = 0; qi < q.length; qi += 1) {
    const ch = q.charAt(qi);

    // 从上一个命中之后找第一个相等字符。贪心即可 ——
    // 我们要的是「能不能命中」和「排得合不合理」，不是最优对齐。
    let found = -1;
    for (let i = cursor; i < t.length; i += 1) {
      if (t.charAt(i) === ch) {
        found = i;
        break;
      }
    }
    if (found === -1) return null;

    if (found === cursor) {
      streak += 1;
      score += 8 + streak * 4; // 越长的连续段越值钱
    } else {
      streak = 0;
      score += 4;
    }
    if (isWordStart(t, found)) score += 10;

    positions.push(found);
    cursor = found + 1;
  }

  // 三条微调：前缀命中优先、目标越短越优先、首次命中越靠前越优先。
  // 没有它们的话，`git` 会同时高分命中 `git log` 和 `edit git config` 这类长串。
  const first = positions[0] ?? 0;
  if (first === 0) score += 6;
  score -= Math.floor(t.length / 8);
  score -= Math.floor(first / 4);

  return { score, positions };
}

/**
 * 把命中位置切成连续区间，供渲染方一次包一段 `<mark>` 用。
 *
 * 逐字符包标记会让「连着的几个字」看起来像被切开的两块
 * （每段各自有内边距），所以先合并再交出去。
 */
export function mergePositions(
  positions: readonly number[],
): Array<[start: number, end: number]> {
  const ranges: Array<[number, number]> = [];
  for (const p of positions) {
    const last = ranges[ranges.length - 1];
    if (last && p === last[1] + 1) last[1] = p;
    else ranges.push([p, p]);
  }
  return ranges;
}
