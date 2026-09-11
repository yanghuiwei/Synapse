/**
 * HTTP 方法与「方法色」的映射。
 * 颜色令牌定义在 theme.css（--m-get / --m-post / ...），
 * 这里只负责把方法名翻成对应的 Tailwind 类名 ——
 * 不要在组件里写死 hex。
 */

import type { RequestSettings } from "./types";

/**
 * 新请求的默认发送设置。
 *
 * ⚠️ 必须与 Rust 侧 `RequestSettings::default()`（`storage/models.rs`）保持一致。
 * 两处不一致**不会报错**，只会表现为「界面上新建的请求和后端默认值不同」。
 * 好在请求一旦保存，settings 就整份落到 DB 里，之后以库里的值为准 ——
 * 所以这处重复的影响范围只有「尚未保存过的新请求」。
 */
export const DEFAULT_REQUEST_SETTINGS: RequestSettings = {
  followRedirects: true,
  maxRedirects: 10,
  verifyTls: true,
  timeoutMs: 30_000,
  proxy: null,
  httpVersion: "auto",
};

export const HTTP_METHODS = [
  "GET",
  "POST",
  "PUT",
  "PATCH",
  "DELETE",
  "HEAD",
  "OPTIONS",
] as const;

export type HttpMethod = (typeof HTTP_METHODS)[number];

export function isHttpMethod(value: string): value is HttpMethod {
  return (HTTP_METHODS as readonly string[]).includes(value);
}

const METHOD_TEXT_CLASS: Record<HttpMethod, string> = {
  GET: "text-m-get",
  POST: "text-m-post",
  PUT: "text-m-put",
  PATCH: "text-m-patch",
  DELETE: "text-m-delete",
  HEAD: "text-m-other",
  OPTIONS: "text-m-other",
};

const METHOD_BG_CLASS: Record<HttpMethod, string> = {
  GET: "bg-m-get/12",
  POST: "bg-m-post/12",
  PUT: "bg-m-put/12",
  PATCH: "bg-m-patch/12",
  DELETE: "bg-m-delete/12",
  HEAD: "bg-m-other/12",
  OPTIONS: "bg-m-other/12",
};

/** 未知方法统一走 --m-other */
export function methodTextClass(method: string): string {
  return isHttpMethod(method) ? METHOD_TEXT_CLASS[method] : "text-m-other";
}

/** 方法徽章底色（低透明度，保证两种主题下文字都够清晰） */
export function methodBgClass(method: string): string {
  return isHttpMethod(method) ? METHOD_BG_CLASS[method] : "bg-m-other/12";
}

/**
 * 状态码 → 文字色。
 *
 * 按语义分档而不是按十位数机械映射：
 * * 2xx 成功 → 绿
 * * 3xx 重定向 → 青（是"要看一眼"的信息，但不是错误）
 * * 4xx / 5xx → 红
 * * 1xx 及拿不到的 → 中性灰
 *
 * 注意这里**不是**涨跌色 —— 那是股市的规矩，跟 HTTP 状态码无关。
 */
export function statusTextClass(status: number): string {
  if (status >= 200 && status < 300) return "text-ok";
  if (status >= 300 && status < 400) return "text-info";
  if (status >= 400) return "text-err";
  return "text-ink-muted";
}
