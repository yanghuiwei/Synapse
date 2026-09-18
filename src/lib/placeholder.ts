/* ============================================================
   占位符的**前端镜像** —— 只服务于「输入时高亮」这一个用途
   ------------------------------------------------------------
   ⚠️ 解析的**权威**在 Rust（`src-tauri/src/template/resolver.rs`）。
     这里做的事只有一件：让用户在敲字的过程中就能看出
     「这个 `{{...}}` 到时候会不会被解析」。

   为什么值得再写一遍规则（而不是等发送完看结果）：
   一个写错的占位符**不会**让请求失败 —— 它会原样跟着请求发出去。
   于是用户看到的是接口返回 404 / 401，而不是「你变量名拼错了」。
   在输入框里当场变色，是这个功能里性价比最高的一环。

   规则一旦漂移，表现是「高亮说没问题、实际没解析出来」——
   所以权威解析结果（`unresolvedPlaceholders`）在发送后**照样**会摆出来，
   前端镜像只负责提前提示，不负责最终裁定。
   ============================================================ */

/** 与 Rust `Namespace::as_str()` 一一对应 */
export type PlaceholderNamespace =
  | "func"
  | "env"
  | "token"
  | "resp"
  | "prev"
  | "folder"
  | "unknown";

/** `token.*` 认的三个键名，与 Rust `TokenValues::get` 一致 */
const TOKEN_KEYS = new Set(["access", "refresh", "expiresAt"]);

/** 支持的动态函数。**必须**与 `template::functions::evaluate` 的分支对齐 */
const FUNCTION_NAMES = new Set([
  "$uuid",
  "$timestamp",
  "$timestampMs",
  "$randomInt",
  "$datetime",
  // Basic 凭据：`{{$basic "u" "p"}}` → 发送时展开成 `Basic base64(u:p)`。
  // 它存在于这个清单里只是为了让输入框**当场**把它染成「会被解析」；
  // 真正的编码在 Rust（`template::functions::basic`），前端不做 base64
  "$basic",
]);

export interface ParsedPlaceholder {
  /** 原文，含花括号 */
  raw: string;
  /** 花括号里去掉首尾空白后的内容 */
  inner: string;
  namespace: PlaceholderNamespace;
  /** 命名空间之后的部分 */
  key: string;
}

export type PlaceholderKind =
  /** 前端判断它会被解析出来 */
  | "resolved"
  /** 写错了：命名空间不认识、函数名不对、token 键名不对 */
  | "unknown"
  /** 本阶段明确不支持（`prev.*` / `folder.*`）—— 不是用户写错了 */
  | "unsupported";

/** 占位符字面量。中间不允许再出现花括号，免得一层贪婪匹配把两个连成一个 */
const PLACEHOLDER_RE = /\{\{([^{}]*)\}\}/g;

/**
 * 扫出文本里的所有占位符。
 *
 * ⚠️ 与 Rust 的 `scan` 有一处**刻意的差异**：Rust 遇到 `{{` 找不到收尾的
 * `}}` 时会 `break`（后面的都不再看），而这里用正则天然就是「跳过它继续」。
 * 差异只影响「正在输入的半截文本」的高亮，不影响真正发送的结果。
 */
export function parsePlaceholders(text: string): ParsedPlaceholder[] {
  const out: ParsedPlaceholder[] = [];

  for (const match of text.matchAll(PLACEHOLDER_RE)) {
    const inner = (match[1] ?? "").trim();
    const { namespace, key } = classify(inner);
    out.push({ raw: match[0], inner, namespace, key });
  }
  return out;
}

/** 按前缀分派命名空间 —— 与 Rust `classify` 同一套规则 */
export function classify(inner: string): {
  namespace: PlaceholderNamespace;
  key: string;
} {
  // `$` 开头一律当函数：Rust 用的就是这一条判据
  if (inner.startsWith("$")) return { namespace: "func", key: inner };

  const dot = inner.indexOf(".");
  if (dot > 0) {
    const head = inner.slice(0, dot);
    const rest = inner.slice(dot + 1);
    if (
      head === "env" ||
      head === "token" ||
      head === "resp" ||
      head === "prev" ||
      head === "folder"
    ) {
      return { namespace: head, key: rest };
    }
  }
  return { namespace: "unknown", key: inner };
}

/**
 * 判断一处占位符会被怎么处理。
 *
 * 只看「写法是否合法」，**不看值是否存在** —— 值存不存在取决于运行时
 * （当前激活的环境、这条请求的分类链上有没有 Token 源），
 * 前端在输入框里拿不到，硬猜只会给出误导性的提示。
 * 那种「写法没错但取不到值」的情况由发送后的 `unresolvedPlaceholders` 报出来。
 */
export function classifyPlaceholder(parsed: ParsedPlaceholder): PlaceholderKind {
  switch (parsed.namespace) {
    case "env":
      return parsed.key === "" ? "unknown" : "resolved";

    case "token":
      return TOKEN_KEYS.has(parsed.key) ? "resolved" : "unknown";

    case "resp":
      return parsed.key === "" ? "unknown" : "resolved";

    case "func": {
      if (!FUNCTION_NAMES.has(parsed.key.split(/\s/)[0] ?? "")) return "unknown";
      return "resolved";
    }

    case "prev":
    case "folder":
      return "unsupported";

    default:
      return "unknown";
  }
}

/** 一段文本里有没有写坏的占位符（用来决定要不要显示那行黄色提示） */
export function hasBrokenPlaceholder(text: string): boolean {
  return parsePlaceholders(text).some((p) => classifyPlaceholder(p) !== "resolved");
}
