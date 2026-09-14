import type { BodyType, KeyValueItem } from "@/lib/types";

/**
 * `application/x-www-form-urlencoded` 的键值行 ↔ 字符串互转。
 *
 * ## 为什么序列化放在前端
 *
 * 后端 `execute::build_body` 对 `Urlencoded` 的处理是**原样发出** `body` 字符串。
 * 让它去拼键值对意味着 `RequestSpec` 协议、压测、MCP 三条调用链都要跟着改；
 * 而前端只要在写库/发送前拼好，后端、压测、历史、导入全都自动一致。
 *
 * ## 占位符必须原样留着
 *
 * `{{token.access}}` 的替换发生在**后端** pipeline 里。这里如果无脑
 * `encodeURIComponent`，`{` 会变成 `%7B`，后端就再也找不到那个占位符了 ——
 * 表现是「Token 没注入，服务端回 401」，而界面上一切正常。
 * 所以先把 `{{…}}` 整段摘出来，编码其余部分，再原样放回。
 */

/** 一个占位符 `{{名字}}`。不含嵌套 —— 与后端的解析规则一致 */
const PLACEHOLDER_RE = /\{\{[^{}]*\}\}/g;

/** 编码时把 `{{…}}` 整段跳过（见文件头的说明） */
function encodePreservingPlaceholders(text: string): string {
  let out = "";
  let last = 0;
  for (const match of text.matchAll(PLACEHOLDER_RE)) {
    const at = match.index ?? 0;
    out += encodeURIComponent(text.slice(last, at)) + match[0];
    last = at + match[0].length;
  }
  return out + encodeURIComponent(text.slice(last));
}

/**
 * 解码，但**不炸**。
 *
 * 用户能在 `body` 里手写任意内容（老数据、curl 导入、MCP 写入），
 * 一个落单的 `%` 就会让 `decodeURIComponent` 抛异常。那种时候原样返回
 * 比抛出去好得多 —— 这是个「尽量把内容还原成表格」的尽力而为操作。
 */
function safeDecode(text: string): string {
  try {
    return decodeURIComponent(text.replace(/\+/g, " "));
  } catch {
    return text;
  }
}

/** 真正会参与拼接的行：勾选着的、且键名非空。键名为空等于没写 */
function usableRows(rows: readonly KeyValueItem[]): KeyValueItem[] {
  return rows.filter((row) => row.enabled && row.key !== "");
}

/**
 * 键值行 → `a=1&b=2`。
 *
 * 键与值都做百分号编码（空格编成 `%20` 而不是 `+`，两者服务端都认，
 * 但 `%20` 不与「`+` 表示空格」的老约定纠缠）。
 */
export function serializeUrlencoded(rows: readonly KeyValueItem[]): string {
  return usableRows(rows)
    .map(
      (row) =>
        `${encodePreservingPlaceholders(row.key)}=${encodePreservingPlaceholders(row.value)}`,
    )
    .join("&");
}

/**
 * `a=1&b=2` → 键值行。
 *
 * 只在**加载**时用（见 `store/request` 的 `rowsFrom`）：库里没有行、但
 * `body` 里有内容的老数据 / curl 导入 / MCP 写入，都靠它还原成表格。
 * 之后表格自己就是权威，不再反向解析 —— 否则「取消勾选一行」这种
 * 只存在于界面上的状态会在切走再切回来时消失。
 *
 * 没写 `=` 的段（`a&b`）按「键名 a、值为空」处理，与常见实现的宽容度一致。
 */
export function parseUrlencoded(body: string): KeyValueItem[] {
  const text = body.trim();
  if (text === "") return [];

  return text
    .split("&")
    .filter((pair) => pair !== "")
    .map((pair) => {
      const eq = pair.indexOf("=");
      if (eq === -1) {
        return { key: safeDecode(pair), value: "", enabled: true };
      }
      return {
        key: safeDecode(pair.slice(0, eq)),
        value: safeDecode(pair.slice(eq + 1)),
        enabled: true,
      };
    });
}

/**
 * 从库里读出来的键值行。
 *
 * 库里有行就用库里的（那份带着「未勾选」这类界面状态）；没有才去解析 `body`。
 */
export function rowsFrom(
  bodyType: BodyType,
  rows: readonly KeyValueItem[],
  body: string,
): KeyValueItem[] {
  const copy = rows.map((row) => ({ ...row }));
  if (bodyType !== "urlencoded") return copy;
  return copy.length > 0 ? copy : parseUrlencoded(body);
}
