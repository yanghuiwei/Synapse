import type { HttpVersion, LoadtestConfig, LoadtestRun, LoadtestSummary } from "@/lib/types";

/**
 * 归档里那两列 JSON（`loadtest_run.config` / `loadtest_run.summary`）的**唯一**读取处。
 *
 * 形状是后端 `service::loadtest::StoredRunConfig` 定死的契约：
 * `{ target: { method, url }, config: { ...LoadtestConfig } }`。
 * 侧栏的历史列表和应用内的报告都要读它 —— 两边各写一份的话，契约一变
 * 就会出现「列表能显示并发、报告显示不出来」这种半截症状（已经发生过一次：
 * 列表读了 `config`，报告只读了 `target`）。
 *
 * 读取一律是**防御式**的：老记录、库被手改过、跑了一半的行，都返回「缺」，
 * 而不是在这里造一个默认值。造默认值会让「这轮还没跑完」看起来像
 * 「这轮打了 10 并发」—— 而那个数字是要被引用、被拿去对比的。
 */

/** 只接受真对象。数组也是 object，但这两列里不该出现数组 */
function objectOf(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/**
 * 把 `undefined` 的键真去掉。
 *
 * 必须做这一步：`{ concurrency: undefined }` 的 `Object.keys()` 长度是 1，
 * 调用方就没法用「一个键都没认出来」来判断这是不是我们存的那份快照了。
 */
function compact<T>(source: Partial<T>): Partial<T> {
  const out: Partial<T> = {};
  for (const [key, value] of Object.entries(source)) {
    if (value !== undefined) out[key as keyof T] = value as T[keyof T];
  }
  return out;
}

/**
 * 这一轮压的是哪个地址（`METHOD url`）。
 *
 * 读不到就退回 `requestName`，再退回 `#id` —— 老记录或库被手改过时，
 * 一个「缺失」比一个空字符串好定位。
 */
export function runTargetOf(run: LoadtestRun): string {
  const target = objectOf(run.config["target"]);
  const method = target?.["method"];
  const url = target?.["url"];
  if (typeof method === "string" && typeof url === "string") return `${method} ${url}`;
  return run.requestName !== "" ? run.requestName : `#${run.id}`;
}

const HTTP_VERSIONS = ["auto", "http1", "http2"] as const;

function httpVersionOf(value: unknown): HttpVersion | undefined {
  return typeof value === "string" && (HTTP_VERSIONS as readonly string[]).includes(value)
    ? (value as HttpVersion)
    : undefined;
}

/**
 * 当时那一轮的压测参数（八项，与导出的 HTML 报告里那一节同源）。
 *
 * 缺的键**不出现**在返回值里 —— 调用方自己决定是显示 `—` 还是整块不画。
 * 全缺（`{}`）意味着这不是我们存的那份形状。
 */
export function runConfigOf(run: LoadtestRun): Partial<LoadtestConfig> {
  const c = objectOf(run.config["config"]);
  if (c === null) return {};

  const num = (key: string): number | undefined => {
    const v = c[key];
    return typeof v === "number" && Number.isFinite(v) ? v : undefined;
  };
  const bool = (key: string): boolean | undefined => {
    const v = c[key];
    return typeof v === "boolean" ? v : undefined;
  };

  return compact<LoadtestConfig>({
    concurrency: num("concurrency"),
    total: num("total"),
    timeoutMs: num("timeoutMs"),
    followRedirects: bool("followRedirects"),
    keepAlive: bool("keepAlive"),
    thinkTimeMs: num("thinkTimeMs"),
    httpVersion: httpVersionOf(c["httpVersion"]),
    warmupRequests: num("warmupRequests"),
  });
}

/**
 * 这一轮的概要。`null` = 还没跑完（库里是 `create_run` 写的 `{}` 占位），
 * 或形状认不出来。
 *
 * 判据用「要渲染的那几个字段是否都在」而不是「对象非空」：概要整体很宽，
 * 而这里只承诺「返回值里那几个数一定能显示」。
 * 校验只做这一个地方 —— 调用方拿到 `LoadtestSummary | null` 之后不必再判形状。
 */
export function runSummaryOf(run: LoadtestRun): LoadtestSummary | null {
  const s = objectOf(run.summary);
  if (s === null) return null;

  if (typeof s["total"] !== "number") return null;
  if (typeof s["succeeded"] !== "number" || typeof s["failed"] !== "number") return null;
  if (typeof s["successRate"] !== "number" || typeof s["elapsedMs"] !== "number") return null;

  const latency = objectOf(s["latency"]);
  if (latency === null || typeof latency["p95Ms"] !== "number") return null;

  const throughput = objectOf(s["throughput"]);
  if (throughput === null || typeof throughput["avgRps"] !== "number") return null;

  return run.summary as unknown as LoadtestSummary;
}
