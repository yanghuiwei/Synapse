import type { BodyType, KeyValueItem } from "@/lib/types";

/**
 * 键值型请求体（`urlencoded` 与 `multipart`）的键值行 ↔ 字符串互转。
 *
 * ## 为什么序列化放在前端
 *
 * 后端 `execute::build_body` 对 `Urlencoded` 的处理是**原样发出** `body` 字符串。
 * 让它去拼键值对意味着 `RequestSpec` 协议、压测、MCP 三条调用链都要跟着改；
 * 而前端只要在写库/发送前拼好，后端、压测、历史、导入全都自动一致。
 *
 * multipart 走的是另一条路（boundary 与文件内容只能由后端拼，见
 * `types.ts` 里 `formFields` 的说明），但这里仍给它拼一份 `k=v&k=@路径` 的
 * **文本镜像**：历史快照、MCP 写入、导入器都只认 `body` 这一列，
 * 没有镜像的话「从历史里打开一条 multipart 请求」会看到一张空表。
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
 *
 * 带 `file` 的行输出 `键=@路径` —— curl `-F name=@file` 的写法，
 * 后端 `form_fields_from_body` 认这个前缀。它在 urlencoded 上不会出现
 * （那种行不带 `file`），所以这一个函数服务两种类型是安全的。
 */
export function serializeUrlencoded(rows: readonly KeyValueItem[]): string {
  return usableRows(rows)
    .map((row) => {
      const key = encodePreservingPlaceholders(row.key);
      const file = row.file?.trim();
      if (file !== undefined && file !== "") {
        return `${key}=@${encodePreservingPlaceholders(file)}`;
      }
      return `${key}=${encodePreservingPlaceholders(row.value)}`;
    })
    .join("&");
}

/**
 * 键值对的分隔符：`&` 或换行。
 *
 * 连续的分隔符（`&&`、`&\n`、`\r\n`）并成一个，省掉一次过滤。
 *
 * 与 Rust 侧 `http::execute::form_fields_from_body` 是**同一个口径** ——
 * 那边服务「只写 `body` 的调用方」（MCP / CLI / 历史快照），只改这里
 * 会让同一个 body 在两条路径上长出不同的表格。
 */
const PAIR_SEPARATOR_RE = /[&\r\n]+/;

/**
 * `a=1&b=2` → 键值行。
 *
 * ## 为什么换行也算分隔符
 *
 * 只看 `&` 的话，「一行一个 `k=v`」的 body 会被整个压成**一个**字段：
 * 键是第一个 `=` 左边那截，值是剩下全部（含中间的换行）。这种形态很常见 ——
 * curl 的多行 `--data`、`.http` 文件、从接口文档里整段复制下来的表单内容。
 * 压成一行之后界面看着像「某一行里带换行」，实际发出去服务端只会收到一个
 * 字段，剩下几个（`client_secret`、`grant_type`）仿佛凭空消失。
 *
 * 未编码的换行本来就不该出现在值里（该编成 `%0A`，`serializeUrlencoded`
 * 就是这么写的），所以把它当分隔符不会误伤合法的键值对。
 *
 * 只在**加载**时用（见 `store/request` 的 `rowsFrom`）：库里没有行、但
 * `body` 里有内容的老数据 / curl 导入 / MCP 写入，都靠它还原成表格。
 * 之后表格自己就是权威，不再反向解析 —— 否则「取消勾选一行」这种
 * 只存在于界面上的状态会在切走再切回来时消失。
 *
 * 没写 `=` 的段（`a&b`）按「键名 a、值为空」处理，与常见实现的宽容度一致。
 *
 * `files` 只在 multipart 下开：把 `@路径` 认成文件字段。urlencoded 上**必须关掉** ——
 * `email=@qq.com` 这种值会被误判成文件行，而那一行在发送时会被当成路径去读盘。
 */
export function parseUrlencoded(
  body: string,
  opts: { files?: boolean } = {},
): KeyValueItem[] {
  const text = body.trim();
  if (text === "") return [];

  return text
    .split(PAIR_SEPARATOR_RE)
    .filter((pair) => pair !== "")
    .map((pair) => {
      const eq = pair.indexOf("=");
      if (eq === -1) {
        return { key: safeDecode(pair), value: "", enabled: true };
      }

      const key = safeDecode(pair.slice(0, eq));
      const raw = safeDecode(pair.slice(eq + 1));
      if (opts.files === true) {
        const path = raw.startsWith("@") ? raw.slice(1) : null;
        if (path !== null && path !== "") {
          return { key, value: "", enabled: true, file: path };
        }
      }
      return { key, value: raw, enabled: true };
    });
}

/** 值里有没有裸换行。单行控件敲不出来，出现即说明这一行是被解析坏的 */
const NEWLINE_RE = /[\r\n]/;

/**
 * 把「一个字段的值里裹着换行」拆成多个字段。
 *
 * 这类行是**旧解析器留下的伤**：换行还不算分隔符的时候，一段「一行一个 `k=v`」
 * 的 body 会被压成**一个**字段 —— 键取第一行 `=` 左边那截，值是其后的全部
 * （换行原样带着）。库里存下来的就是这个坏形状，所以光把 `parseUrlencoded`
 * 改对是不够的：已经落库的行得在**读出来的时候**顺一遍，否则那张表永远是
 * 「一行、里面挤着四行」，用户只能手抄一遍。
 *
 * 单行控件本来就敲不出换行（`<input>` 会 strip newlines），值里出现裸换行
 * 只可能是被解析坏的 —— 拿它当分隔符重拆不会误伤正常数据。
 *
 * 形状没问题的行**原样返回**（同一个对象引用）：它们带着「未勾选」这类只存在于
 * 界面上的状态，白白拷一遍没有好处。
 */
function splitRowsAtNewlines(rows: KeyValueItem[], files: boolean): KeyValueItem[] {
  if (!rows.some((row) => NEWLINE_RE.test(row.value))) return rows;

  const out: KeyValueItem[] = [];
  for (const row of rows) {
    if (!NEWLINE_RE.test(row.value)) {
      out.push(row);
      continue;
    }
    // 整行拼回 `键=值` 再走一遍解析 —— 换行就在那一层被当分隔符拆开了
    out.push(...parseUrlencoded(`${row.key}=${row.value}`, { files }));
  }
  return out;
}

/**
 * **urlencoded 的**键值行：库里有就用库里的，没有才去解析 `body`。
 *
 * 库里有行时优先用它，是因为那份带着「未勾选」这类只存在于界面上的状态；
 * 每次都从 `body` 重解析会把它们悄悄丢掉。
 *
 * 只服务 `urlencoded` 一种类型 —— multipart 有自己的列与自己的入口
 * （见 [`formFieldsFrom`]）。两者共用一列的话，切类型会把上一种表格的行带过去。
 */
export function rowsFrom(
  bodyType: BodyType,
  rows: readonly KeyValueItem[],
  body: string,
): KeyValueItem[] {
  const copy = rows.map((row) => ({ ...row }));
  if (copy.length > 0) return splitRowsAtNewlines(copy, false);
  if (bodyType === "urlencoded") return parseUrlencoded(body);
  return [];
}

/**
 * multipart 的字段行：库里有就用库里的，没有才从 `body` 的镜像串还原。
 *
 * 需要这条兜底的是「只写 `body` 的写入方」——MCP 的 `update_request`、curl 导入、
 * 手改过的旧数据。它们写下的 `k=v&k=@路径` 必须能重新长出表格来，
 * 否则从那些路径进来的 multipart 请求打开就是一张空表，而请求本身看着是好的。
 */
export function formFieldsFrom(
  rows: readonly KeyValueItem[],
  body: string,
): KeyValueItem[] {
  const copy = rows.map((row) => ({ ...row }));
  return copy.length > 0 ? splitRowsAtNewlines(copy, true) : parseUrlencoded(body, { files: true });
}
