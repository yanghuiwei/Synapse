import type { ApiRequest, KeyValueItem } from "@/lib/types";

/**
 * 一条取 Token 请求声明的 `scope` 词表（小写）。
 *
 * ## 它为什么值得摆在界面上
 *
 * `scope` 决定「这把 Token 能调哪些服务」。后端播种 Token 源时的判据是
 * **目标接口 URL 第 2 段的服务名**（`/PSA/MasterData2/v1/…` → `masterdata2`）
 * 必须出现在取 Token 请求的 `scope` 里，否则拿回来的 Token 调那个服务就是 401
 * （见 `core/src/storage/seed.rs` 的 `pick` 与 `service_of`）。
 *
 * 在此之前这件事在界面上完全不可见：选接口时只能看到一个名字，
 * 名字里写没写环境、scope 覆不覆盖，全靠记。这里把它读出来只是为了**看得见**。
 *
 * ⚠️ **这里不参与任何发送判定**。真要用它来改行为，就得先把它挪到后端 ——
 * 两处判据并存迟早漂移，而漂移的表现是「界面说覆盖、实际不覆盖」，
 * 比不显示更坏。
 *
 * ## 三种存法都要试
 *
 * 库里压着三代写法，顺序与后端 `scope_words` 一致（结构化 → 原文）：
 * `formFields`（005 之后的 multipart 行）→ `bodyRows`（urlencoded 行）
 * → `body` 原文（老请求只有这一列，形态是 `k=v&k=v` 或一行一个 `k=v`）。
 */
export function readScopeWords(request: ApiRequest): string[] {
  const raw =
    rowValue(request.formFields, "scope") ??
    rowValue(request.bodyRows, "scope") ??
    rawValue(request.body);

  if (raw === null) return [];

  return decodePercent(raw)
    .split(/\s+/)
    .filter((word) => word !== "")
    .map((word) => word.toLowerCase());
}

/** 键值行里取某个键的值。键名两侧空白与大小写都不计较（后端两处口径不同，这里取宽的那个） */
function rowValue(rows: readonly KeyValueItem[], key: string): string | null {
  const hit = rows.find((row) => row.key.trim().toLowerCase() === key);
  return hit ? hit.value : null;
}

/**
 * 从 `k=v&k=v` 或按行分隔的 `k=v` 原文里取某个键的值。
 *
 * 只按第一个 `=` 切 —— 值里带 `=`（base64 填充、`scope=a=b`）时按最后一个切
 * 会把键名吃掉半截。与后端 `raw_scope` 的 `split_once('=')` 同一条规则。
 */
function rawValue(body: string): string | null {
  for (const pair of body.split(/[&\n]/)) {
    const eq = pair.indexOf("=");
    if (eq === -1) continue;
    if (pair.slice(0, eq).trim().toLowerCase() === "scope") {
      return pair.slice(eq + 1).trim();
    }
  }
  return null;
}

/**
 * `%XX` 与 `+` 的解码。
 *
 * `decodeURIComponent` 遇到落单的 `%` 会整段抛，而后端的取舍是
 * 「不合法转义原样留下」。这里同样不解到一半：宁可这个词匹配不上，
 * 也不要吃掉字符把它变成另一个 scope 词 —— 那才是真正会骗人的结果。
 */
function decodePercent(raw: string): string {
  try {
    return decodeURIComponent(raw.replace(/\+/g, " "));
  } catch {
    return raw;
  }
}
