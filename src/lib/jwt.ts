/* ============================================================
   从一段粘贴进来的 token 里读过期时间
   ------------------------------------------------------------
   为什么值得为它单开一个文件：手贴 token 是真实且高频的用法
   （真库 2026-09-17 的只读统计：16 处写死的 token，全部已过期 —— 最短的
   过期 204 天，最长的 1625 天）。它们发出去 100% 是 401，而界面上看不出
   任何异常：头名对、值形状也对（就是个 JWT），只是早就过期了。

   手贴唯一不如自动刷新的地方，就是「不知道什么时候过期」。
   而 JWT 的 payload 是 base64url 明文，本地就能读 exp ——
   不需要密钥、不需要联网、不需要验签。

   ⚠️ **我们只读，不验。**
   这个数字是给用户看的提示，不是任何安全判断的依据：签名可能是假的、
   算法可能是 none、exp 可能被伪造。谁都不该拿它决定「要不要发这个请求」。
   要不要发始终由用户说了算 —— 本工具从不替用户拦请求。

   认不出就返回 null，**绝不猜**。不给假的倒计时，也不假装「大概没过期」：
   一个错的倒计时比没有倒计时更糟，因为它会让人相信一个不存在的事实。
   ============================================================ */

/**
 * 恰好三段、每段都是 base64url 字符。
 *
 * 第三段允许为空 —— `alg: none` 的 JWT 就是 `header.payload.` 这个形状。
 * 签名段不参与解码，只用来确认「这确实是个 JWT 而不是恰好带点的普通值」。
 */
const JWT_SHAPE = /^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*$/;

export interface JwtExpiry {
  /** `exp` 声明的 Unix **秒**（JWT 规范用的是秒，不是毫秒） */
  expSeconds: number;
  /** 同一时刻的 ISO8601，直接给 `Date` 用 */
  expiresAt: string;
}

/**
 * 从一段头值里读 JWT 的过期时间。读不出来返回 `null`。
 *
 * 入参是**整段头值**而不是裸 token：用户粘进去的往往是
 * `Bearer eyJ…`（网站页面上写的就是这样），前缀得由这里剥。
 * 做法是按空白切开逐个试，而不是硬编码剥 `Bearer ` ——
 * 真库里 `X-Access-Token` 的 15 处都是**裸 token 不带前缀**，
 * 而 `Authorization` 的写法带前缀。两种都在同一个输入框里出现。
 */
export function readJwtExpiry(value: string): JwtExpiry | null {
  for (const part of value.trim().split(/\s+/)) {
    if (!JWT_SHAPE.test(part)) continue;

    const payload = decodeBase64Url(part.split(".")[1] ?? "");
    if (payload === null) continue;

    const exp = readExpSeconds(payload);
    if (exp === null) continue;

    return { expSeconds: exp, expiresAt: new Date(exp * 1000).toISOString() };
  }
  return null;
}

/** `exp` 必须是 JSON 数字。字符串形式的 `"1735689600"` 不算 —— 不猜 */
function readExpSeconds(payload: string): number | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(payload);
  } catch {
    return null;
  }

  if (typeof parsed !== "object" || parsed === null) return null;
  const exp = (parsed as Record<string, unknown>).exp;
  if (typeof exp !== "number" || !Number.isFinite(exp) || exp <= 0) return null;

  return Math.trunc(exp);
}

/**
 * base64url → UTF-8 字符串。解不出来返回 `null`。
 *
 * `atob` 只认标准 base64，而且要 padding —— 两个都得先补齐。
 * 解出的字节再走一次 `TextDecoder`：payload 里出现非 ASCII（中文名字、
 * 多字节 claim）时，直接当 latin-1 用会得到乱码，随后 `JSON.parse` 失败，
 * 表现为「明明是个 JWT 却不显示倒计时」。
 */
function decodeBase64Url(input: string): string | null {
  if (input === "") return null;

  const normalized = input.replace(/-/g, "+").replace(/_/g, "/");
  const padding = (4 - (normalized.length % 4)) % 4;
  const padded = normalized + "=".repeat(padding);

  try {
    const binary = atob(padded);
    const bytes = Uint8Array.from(binary, (ch) => ch.charCodeAt(0));
    return new TextDecoder().decode(bytes);
  } catch {
    return null;
  }
}
