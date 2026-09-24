/* ============================================================
   把一段粘进来的 token 拆开看
   ------------------------------------------------------------
   与 `lib/jwt.ts` 的分工：那个**只回答「什么时候过期」**，
   因为它要给请求头那一行做发送前提示；这里回答「这段明文里到底写了什么」，
   服务于用户主动想把 token 看个明白的场景（「工具 → Token 解析」）。

   为什么值得单独一块：

   真库里压着一堆手贴的死 token（见 `jwt.ts` 顶部的只读统计）。出了问题
   第一反应就是拿那个值出来看 —— 但 base64 串在界面上是一团乱码，
   只能去找在线解码网站。把一串 token 粘进第三方网站是有代价的：
   那是个凭据，粘过去就等于发出去了。本地能解，就不该往外送。

   ⚠️ **只解码，不验签。**
   这里读出来的一切都只是「这段文本自己写着的内容」——
   签名可能是假的、`alg` 可能是 `none`、`exp` 可能是伪造的。
   谁都不该拿它做安全判断（要不要发、能不能信），
   和 `jwt.ts` 是同一条立场。认不出来就返回 `null`，**绝不猜**。

   这一层是**纯函数**，不依赖 React 也不依赖时钟：时间类的相对量
   （「已过期 3 小时」）由 UI 层用 `Date.now()` 现算，这样同一份输入
   在任何时刻都能得到同一份解析结果，测试里也不用注入时钟。
   ============================================================ */

/**
 * JWT 的形状：恰好三段、每段都是 base64url，第三段（签名）允许为空。
 *
 * 与 `jwt.ts` 的 `JWT_SHAPE` 同一条正则的用意 —— 认形状比认前缀可靠：
 * 用户粘的可能是 `Bearer eyJ…`，也可能是裸值，两种都得认。
 * 但这里的 payload 段要求非空：读过期时间时 payload 空了直接跳过就行，
 * 而作为「请给我看这个 token」的入口，空 payload 的候选不值得单独报错。
 */
const JWT_SHAPE = /^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*$/;

/** 版本号段（`v1.xxx` 这类）也长得像三段点分，但每一段都是纯数字/版本号，不像 base64 */
const MIN_SEGMENT_LEN = 4;

export interface TokenClaims {
  /** 原样解出的 JSON 解析结果；解不出 JSON 时为 `null` */
  json: Record<string, unknown> | null;
  /** base64url 解出的原文（JSON 非法时也要原样给出来，不能吞掉） */
  text: string;
}

export interface ParsedToken {
  /** 用户输入的原文（可能带 `Bearer ` 前缀或包在引号里） */
  raw: string;
  /** 真正被解码的那一段（去掉了前缀） */
  token: string;
  header: TokenClaims;
  payload: TokenClaims;
  /**
   * 签名段。**不解码** —— 它是二进制，base64url 解出来多半是乱字节，
   * 展示它的长度与截断即可。`""` 表示 `header.payload.` 这种 `alg: none` 的形状。
   */
  signature: string;
}

/**
 * 从一段**头值**里挑出 token 并解析。挑不出来返回 `null`。
 *
 * 入参是整段头值而不是裸 token：用户粘进来的往往是 `Bearer eyJ…`
 * 或 `X-Access-Token: eyJ…` 这样的一整行。做法与 `readJwtExpiry` 一致 ——
 * 按空白切开逐个试，而不是硬编码剥 `Bearer `（真库里两种写法都有），
 * 额外再剥一次两端的引号与逗号，因为从 JSON / cookie 里复制出来的值常带这些。
 */
export function parseToken(value: string): ParsedToken | null {
  for (const word of value.trim().split(/\s+/)) {
    const candidate = word.replace(/^["'[{]+/, "").replace(/["'\],;]+$/, "");
    const parsed = parseOne(candidate);
    if (parsed !== null) return parsed;
  }
  return null;
}

/**
 * 只认「三段都像 base64url 且 base64url 解出来是合法 JSON」的候选。
 *
 * 两道校验缺一不可：`1.2.3` 这种版本号过得了形状那一关，
 * 而解出非 JSON 之后就不该再当成 token 报出去 —— 宁可说「这是普通文本」，
 * 也不要给出一份看起来像模像样、其实全错的解析。
 */
function parseOne(candidate: string): ParsedToken | null {
  if (!JWT_SHAPE.test(candidate)) return null;

  const [headerPart = "", payloadPart = "", signature = ""] = candidate.split(".");
  if (headerPart.length < MIN_SEGMENT_LEN || payloadPart.length < MIN_SEGMENT_LEN) {
    return null;
  }

  const headerJson = decodeSegment(headerPart);
  if (headerJson === null) return null;

  // payload 必须能解出 JSON —— 这条是整个「认得出」判据的落点
  const payloadJson = decodeSegment(payloadPart);
  if (payloadJson === null) return null;

  return {
    raw: candidate,
    token: candidate,
    header: { json: headerJson, text: stringify(headerJson) },
    payload: { json: payloadJson, text: stringify(payloadJson) },
    signature,
  };
}

/**
 * base64url 段 → JSON 对象。任何一步不成立都返回 `null`。
 *
 * 与 `jwt.ts` 的 `decodeBase64Url` 是同一套手法（补 padding、换字符表、
 * 再走一次 `TextDecoder`）—— payload 里有中文 claim 时，直接当 latin-1
 * 用会得到乱码，随后 `JSON.parse` 失败，表现为「明明是个 token 却解析不出来」。
 * 之所以没把那个函数导出来复用：它服务于「只要 exp」这一条窄路径，
 * 而这里要的是「解不出来时也要能给出原始文本」，返回形状和容错口径都不一样。
 */
function decodeSegment(input: string): Record<string, unknown> | null {
  const text = decodeBase64Url(input);
  if (text === null || text === "") return null;

  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return null;
  }

  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return null;
  return parsed as Record<string, unknown>;
}

/** base64url → UTF-8 字符串。解不出来返回 `null` */
function decodeBase64Url(input: string): string | null {
  if (input === "") return null;

  const normalized = input.replace(/-/g, "+").replace(/_/g, "/");
  const padding = (4 - (normalized.length % 4)) % 4;

  try {
    const binary = atob(normalized + "=".repeat(padding));
    const bytes = Uint8Array.from(binary, (ch) => ch.charCodeAt(0));
    // `fatal: false`：个别 token 的签名段是二进制，解不干净时不该整条失败
    return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
  } catch {
    return null;
  }
}

function stringify(value: Record<string, unknown>): string {
  return JSON.stringify(value, null, 2);
}

/* ============================================================
   claim 的展示口径
   ============================================================ */

/**
 * JWT 注册 claim 的说明。
 *
 * 只收**标准里定义了含义**的（RFC 7519 §4）加上在 OAuth / OIDC 里
 * 含义明确的几个。不认识的 claim 一律**原样显示键名**，不给它编一个说法 ——
 * 编错的说法比没有说法更糟：用户会照着错的意思去查问题。
 */
const CLAIM_LABELS: Record<string, string> = {
  iss: "签发者",
  sub: "主体",
  aud: "受众",
  exp: "过期时间",
  nbf: "生效时间",
  iat: "签发时间",
  jti: "Token 编号",
  scope: "授权范围",
  scp: "授权范围",
  client_id: "客户端 ID",
  azp: "授权方",
  sid: "会话 ID",
  typ: "用途",
  name: "名称",
  preferred_username: "用户名",
  email: "邮箱",
  role: "角色",
  roles: "角色",
};

/**
 * 哪些 claim 是**秒级**时间戳。
 *
 * `exp` / `nbf` / `iat` 是标准里写死的秒。`auth_time` 也是秒（OIDC）。
 * 与毫秒值的分辨方式：JWT 规范里这三者都是 NumericDate，
 * 而秒级时间戳在近几十年内恒为 10 位 —— 超过 13 位的不可能是秒，
 * 那种值照原样显示数字，不做「大概是什么时间」的推断。
 */
const TIME_CLAIMS = new Set(["exp", "nbf", "iat", "auth_time"]);

/** 秒级时间戳的合理上限（约公元 2286 年）。超过它说明这不是秒级时间戳 */
const MAX_SECONDS = 1e10;

export interface ClaimRow {
  key: string;
  /** 中文说明；不是已知 claim 时为 `null`（界面显示键名即可） */
  label: string | null;
  /** 值的展示文本 */
  value: string;
  /** 时间类 claim 时才有：它对应的时刻（ISO8601），交给界面渲染成本地时间 */
  at: string | null;
  /** 值是不是对象 / 数组（界面渲染成等宽的多行块更易读） */
  block: boolean;
}

/**
 * 把 payload 摊平成一行行「claim / 值」。
 *
 * 值一律转成字符串在这里做完，界面只负责排版 —— 否则「对象怎么显示」
 * 这件事会同时散在好几个渲染分支里。
 */
export function describeClaims(
  payload: Record<string, unknown>,
): ClaimRow[] {
  return Object.entries(payload).map(([key, value]) => {
    const at = readInstant(key, value);
    return {
      key,
      label: CLAIM_LABELS[key] ?? null,
      value: formatClaimValue(value),
      at,
      block: typeof value === "object" && value !== null,
    };
  });
}

/**
 * 时间类 claim → ISO8601 时刻；说不准就返回 `null`。
 *
 * 只认「10 位上下的整数秒」，且**必须是这个 claim 名称**——
 * 不认识的键里出现 10 位数字时，那更可能是订单号或者随机 id，
 * 把它渲染成日期是典型的过度解读。
 */
function readInstant(key: string, value: unknown): string | null {
  if (!TIME_CLAIMS.has(key)) return null;
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  if (value <= 0 || value >= MAX_SECONDS) return null;

  const at = new Date(value * 1000);
  return Number.isNaN(at.getTime()) ? null : at.toISOString();
}

/**
 * claim 值的展示文本。
 *
 * 字符串**不带引号**：`"admin"` 和 `admin` 在这里是同一件事，
 * 多一层引号只是让人读得慢。对象与数组走 `JSON.stringify`（紧凑，不缩进）——
 * 它们由界面用等宽块单独排版，这里缩进反而会多出一堆首行空白。
 */
function formatClaimValue(value: unknown): string {
  if (value === null) return "null";
  if (typeof value === "string") return value;
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}
