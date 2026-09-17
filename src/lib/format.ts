/* ============================================================
   展示用的数值格式化
   ------------------------------------------------------------
   放在这里而不是塞进组件：响应头、状态栏、将来的历史列表与压测报表
   都要用同一套口径。三处各写一份 `toFixed(1) + " KB"`，
   迟早会出现「同一个大小在界面上有两个值」。
   ============================================================ */

/** 字节数 → 人类可读（B / KB / MB）。1024 进制，单位之间保留 1~2 位小数 */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
}

/** 毫秒 → 人类可读。超过 1 秒换成 `s`，否则一律 `ms` —— 调接口时 ms 的可比性更好 */
export function formatMs(ms: number): string {
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(2)} s`;
}

/**
 * 秒数 → 人类可读的时长（`45 秒` / `2 分 5 秒` / `3 小时 12 分`）。
 *
 * 输入可以是负数（Token 已经过期多久了）。取绝对值，符号由调用方决定怎么表达 ——
 * 「已过期 30 秒」和「30 秒后过期」是两句话，而格式化函数不该替调用方选。
 *
 * 用 `Math.trunc` 而不是 `Math.round`：向上取整会让「还有 59.6 秒」显示成
 * 「1 分」然后又跳回「59 秒」，看着像倒计时坏了。
 */
/**
 * 秒数 → 紧凑倒计时（`12:34` / `1:02:03`）。
 *
 * 与 `formatSeconds` 分开，是因为它服务的是**会每秒跳动**的位置
 * （发送区的 Token 徽标）。那里宽度是硬约束：`12 分 34 秒` 每跳一秒都要
 * 重新排版一次，徽标会左右抖 —— 定宽的 `12:34` 不会。
 *
 * 同样取绝对值：符号由调用方表达（「已过期 02:14」）。
 */
export function formatCountdown(seconds: number): string {
  const total = Math.trunc(Math.abs(seconds));
  const hours = Math.trunc(total / 3600);
  const minutes = Math.trunc((total % 3600) / 60);
  const rest = total % 60;

  const mm = String(minutes).padStart(2, "0");
  const ss = String(rest).padStart(2, "0");
  return hours > 0 ? `${hours}:${mm}:${ss}` : `${mm}:${ss}`;
}

/**
 * ISO8601 → 本地时刻（`10:36:01`；不是今天就带上月日）。
 *
 * 服务于「这一行是什么时候写下的」这类说明（Token 上次取回、过期时刻）。
 * **不带秒以下的精度**：那个位置上多三位小数只会让人读得慢。
 *
 * 跨天时补月日而不是另开一行显示日期 —— Token 有效期跨到明天很常见，
 * 只说 `10:36:01` 会被读成「今天上午」，而它可能已经是明天。
 */
export function formatClock(iso: string): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return iso;

  const pad = (n: number) => String(n).padStart(2, "0");
  const clock = `${pad(at.getHours())}:${pad(at.getMinutes())}:${pad(at.getSeconds())}`;

  const now = new Date();
  const sameDay =
    at.getFullYear() === now.getFullYear() &&
    at.getMonth() === now.getMonth() &&
    at.getDate() === now.getDate();

  return sameDay ? clock : `${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${clock}`;
}

export function formatSeconds(seconds: number): string {
  const total = Math.trunc(Math.abs(seconds));

  if (total < 60) return `${total} 秒`;

  const minutes = Math.trunc(total / 60);
  if (minutes < 60) {
    const rest = total % 60;
    return rest === 0 ? `${minutes} 分` : `${minutes} 分 ${rest} 秒`;
  }

  const hours = Math.trunc(minutes / 60);
  const restMinutes = minutes % 60;
  return restMinutes === 0 ? `${hours} 小时` : `${hours} 小时 ${restMinutes} 分`;
}
