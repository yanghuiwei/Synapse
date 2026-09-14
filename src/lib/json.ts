/* ============================================================
   JSON 的高亮分类与重排
   ------------------------------------------------------------
   请求体编辑器（P2）与响应查看器（P3）都要把一段 JSON 上色。
   两份实现各自留在组件里迟早会漂移，而症状是「同一段 JSON 在请求里
   和响应里颜色不一样」—— 这种不一致没人会去查，只会被当成「本来就这样」。

   ⚠️ tokenizer 只做**分类**，不做校验，并且必须**逐字符守恒**：
   请求体里可能是一段用户正在敲的残缺 JSON，匹配不上的字符要原样吐出来
   （归到 `ws`，不上色）。叠加式高亮（见 `JsonCodeArea`）就靠这一点对齐 ——
   高亮层少一个字符，光标与看到的字就整体错位一格，而且不会有任何报错。
   ============================================================ */

export type TokenKind = "key" | "str" | "num" | "bool" | "null" | "punct" | "ws";

export interface JsonToken {
  text: string;
  kind: TokenKind;
}

/** 语义色类。空串 = 不上色（空白，以及认不出来的字符） */
export const JSON_TOKEN_CLASS: Record<TokenKind, string> = {
  key: "text-syn-key",
  str: "text-syn-str",
  num: "text-syn-num",
  bool: "text-syn-bool",
  null: "text-syn-null",
  punct: "text-syn-punct",
  ws: "",
};

/**
 * 一次正则扫完整段 JSON。
 *
 * 为什么不引 highlight.js / prism：为了一个只高亮 JSON 的场景，
 * 那是几十 KB 的依赖 + 一套主题系统；而 `--syn-*` 设计令牌本来就在主题里备好了。
 */
const JSON_TOKEN_RE =
  /("(?:\\.|[^"\\])*")(\s*:)?|\b(true|false)\b|\bnull\b|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|([{}[\],:])/g;

export function tokenizeJson(text: string): JsonToken[] {
  const out: JsonToken[] = [];
  let cursor = 0;

  for (const match of text.matchAll(JSON_TOKEN_RE)) {
    const start = match.index ?? 0;
    const raw = match[0];
    if (start > cursor) {
      out.push({ text: text.slice(cursor, start), kind: "ws" });
    }

    const quoted = match[1];
    const colon = match[2];
    const literal = match[3];
    const number = match[4];
    const punct = match[5];

    if (quoted !== undefined) {
      // 引号后面跟着冒号 = 对象的键，否则是字符串值
      out.push({ text: quoted, kind: colon !== undefined ? "key" : "str" });
      if (colon !== undefined) out.push({ text: colon, kind: "punct" });
    } else if (literal !== undefined) {
      out.push({ text: raw, kind: "bool" });
    } else if (number !== undefined) {
      out.push({ text: raw, kind: "num" });
    } else if (raw === "null") {
      out.push({ text: raw, kind: "null" });
    } else if (punct !== undefined) {
      out.push({ text: raw, kind: "punct" });
    }

    cursor = start + raw.length;
  }

  if (cursor < text.length) {
    out.push({ text: text.slice(cursor), kind: "ws" });
  }
  return out;
}

// ---------------------------------------------------------------
// 体积上限
// ---------------------------------------------------------------

/**
 * 超过这个大小就不再「美化」。
 *
 * `JSON.parse` + `JSON.stringify` 在 2MB 上已经是几百毫秒的卡顿，
 * 而且美化后的字符串体积是原来的两三倍 —— 卡的是主线程，用户看到的是界面直接僵住。
 */
export const PRETTY_LIMIT = 2 * 1024 * 1024;

/** 再往上是连语法高亮都别做：每个 token 一个 `<span>`，几十万个节点会把浏览器拖死 */
export const HIGHLIGHT_LIMIT = 400 * 1024;

/**
 * 试着重排成缩进 JSON；不是合法 JSON 就返回 `null`（这不是错误）。
 *
 * 调用方对 `null` 的处理各不相同：响应区退回原文并藏起「美化 / 原文」开关，
 * 请求体编辑器则拿它当「这段还写不合法」的判据，提示用户而不是清空内容。
 */
export function tryPrettyJson(body: string): string | null {
  try {
    return JSON.stringify(JSON.parse(body), null, 2);
  } catch {
    return null;
  }
}
