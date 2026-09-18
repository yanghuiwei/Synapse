//! 占位符里的动态函数（`{{$uuid}}` `{{$timestamp}}` ……）。
//!
//! ## 为什么「时钟」和「熵」都是显式参数
//!
//! 这些函数的**全部作用**就是产出一个值。若内部直接调 `Local::now()` /
//! `rand::random()`，单测就只剩「结果是个数字」这类毫无信息量的断言。
//! 抽成参数之后，`{{$datetime '%Y-%m-%d'}}` 能断言到具体的日期，
//! `$uuid` 能断言格式与唯一性 —— 而它们在真实请求里依然是随机的。
//!
//! ## 这里不是密码学安全的
//!
//! `$uuid` / `$randomInt` 是**造测试数据**用的，不是用来生成密钥或令牌的。
//! 所以用 splitmix64 而不是引一个 CSPRNG：后者会把依赖树撑大一块，
//! 却解决不了一个「请求里想要个随机订单号」的问题。

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::format::{Item, StrftimeItems};
use chrono::{DateTime, Local};

/// `$randomInt` 的上界（含），与 design.md §4.2 的「0–10000」一致
pub const RANDOM_INT_MAX: u64 = 10_000;

/// 熵源。见文件头对「为什么显式注入」的说明。
pub struct Entropy {
    state: u64,
}

impl Entropy {
    /// 线上用法：拿系统时间当种子。
    pub fn from_system() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        Self { state: nanos }
    }

    /// 单测用法：固定种子 → 固定序列。
    pub fn seeded(seed: u64) -> Self {
        Self { state: seed }
    }

    /// splitmix64。短、快、无依赖，对「造数据」这个用途绰绰有余。
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `0..=max` 的整数。
    ///
    /// 用「乘 2^64 取高位」而不是 `% (max + 1)`：后者在 max+1 不是 2 的幂时有
    /// 可测的偏斜，而 `$randomInt` 的 10001 恰好不是 2 的幂 —— 偏斜会全落在
    /// 最上面那一段数上，也就是「上限附近的值出现得偏少」。
    pub fn int_inclusive(&mut self, max: u64) -> u64 {
        if max == 0 {
            return 0;
        }
        let span = u128::from(max) + 1;
        ((u128::from(self.next_u64()) * span) >> 64) as u64
    }

    /// RFC 4122 v4 形状的 UUID 字符串。
    ///
    /// 版本位与变体位都按规范写死 —— 有些服务端会校验这两处。
    pub fn uuid_v4(&mut self) -> String {
        let hi = self.next_u64().to_be_bytes();
        let lo = self.next_u64().to_be_bytes();

        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&hi);
        bytes[8..].copy_from_slice(&lo);
        bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
        bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 1（RFC 4122）

        let hex = |slice: &[u8]| -> String {
            use std::fmt::Write as _;
            slice.iter().fold(String::new(), |mut acc, b| {
                let _ = write!(acc, "{b:02x}");
                acc
            })
        };

        format!(
            "{}-{}-{}-{}-{}",
            hex(&bytes[0..4]),
            hex(&bytes[4..6]),
            hex(&bytes[6..8]),
            hex(&bytes[8..10]),
            hex(&bytes[10..16])
        )
    }
}

/// 求值一个动态函数表达式（`{{` `}}` **之内**的内容，调用方已 trim）。
///
/// 返回 `None` 表示「这不是一个能识别的动态函数表达式」——
/// 调用方据此把占位符**原样保留**并记一条警告，而不是替换成空串。
pub fn evaluate(expr: &str, now: &DateTime<Local>, entropy: &mut Entropy) -> Option<String> {
    let (name, args) = split_name(expr);
    match name {
        "$uuid" => no_args(args).then(|| entropy.uuid_v4()),
        "$timestamp" => no_args(args).then(|| now.timestamp().to_string()),
        "$timestampMs" => no_args(args).then(|| now.timestamp_millis().to_string()),
        "$randomInt" => no_args(args).then(|| entropy.int_inclusive(RANDOM_INT_MAX).to_string()),
        "$datetime" => datetime(args, now),
        "$basic" => basic(args),
        _ => None,
    }
}

/// 表达式是不是「函数的样子」（`$` 开头）。
///
/// 解析器用它区分「命名空间不认识」（`{{foo.bar}}`）与「函数写错了」
/// （`{{$datetim}}` / `{{$datetime}}`）—— 两种提示语指向的修法完全不同。
pub fn looks_like_function(expr: &str) -> bool {
    expr.starts_with('$')
}

/// 把 `$datetime '%Y-%m-%d'` 拆成 `("$datetime", "'%Y-%m-%d'")`。
fn split_name(expr: &str) -> (&str, &str) {
    let end = expr
        .char_indices()
        .find(|(i, c)| *i > 0 && !(c.is_ascii_alphanumeric() || *c == '$'))
        .map(|(i, _)| i)
        .unwrap_or(expr.len());
    let (name, rest) = expr.split_at(end);
    (name, rest.trim())
}

fn no_args(args: &str) -> bool {
    args.is_empty()
}

/// `$datetime '<fmt>'`。格式串是 **chrono 的 strftime 语法**（`%Y-%m-%d`）。
///
/// 特意挡一下「不含 `%` 的格式串」：design.md §4.2 的例子里写的是
/// `'YYYY-MM-DD'`（moment 风格），但底层是 chrono —— 照字面量输出会得到一个
/// **看起来对、实际是死字符串**的日期。这种静默错误比直接报错难查得多，
/// 所以这种格式串一律判为不合法，走「原样保留 + 警告」。
fn datetime(args: &str, now: &DateTime<Local>) -> Option<String> {
    let fmt = unquote(args)?;
    if !fmt.contains('%') {
        return None;
    }
    // 先过一遍 `StrftimeItems` 再真正格式化：chrono 遇到非法说明符时
    // 是在 `Display` 里失败的，直接 `format()` 可能 panic，而不是返回错误。
    if StrftimeItems::new(fmt).any(|item| matches!(item, Item::Error)) {
        return None;
    }
    Some(now.format(fmt).to_string())
}

/// 去掉一层成对的单/双引号。
///
/// **不接受**没有引号的参数：格式串里常带空格（`%Y-%m-%d %H:%M`），
/// 不加引号无法确定边界。
fn unquote(args: &str) -> Option<&str> {
    let bytes = args.as_bytes();
    let first = *bytes.first()?;
    let last = *bytes.last()?;
    if first == last && (first == b'\'' || first == b'"') && bytes.len() >= 2 {
        return Some(&args[1..args.len() - 1]);
    }
    None
}

/// `$basic "user" "pass"` → `Basic dXNlcjpwYXNz`。
///
/// 存在的理由只有一个：Basic 的全部麻烦就是那个 base64。把这一个知识点收进
/// 函数名之后，「授权」就不必再是一种独立的**类型**了 ——
/// `Authorization: {{$basic "u" "p"}}` 与 `Authorization: Bearer {{token.access}}`
/// 是同一张表里的两行，没有类型，也就没有「类型和头不一致」这种可能。
///
/// ## 只吃字面量，不吃命名空间
///
/// `{{$basic env.API_USER env.API_PASS}}` **不被支持**：那要求函数参数能做
/// 命名空间引用，而现在 `evaluate` 拿不到 `TemplateContext`（它只收 now /
/// entropy）。这是设计稿 §7「待定点」里明确留的降级方案 ——
/// 功能等效，代价是密码只能写在请求里。
///
/// ⚠️ 关键在**失败方式**：不支持的写法返回 `None`，于是调用方**原样保留
/// 占位符并记一条警告**（见 `evaluate` 的返回约定）。绝不会因为「猜了个边界」
/// 而拼出一段看起来像凭据、实际是垃圾的字符串发到服务端。
fn basic(args: &str) -> Option<String> {
    let (user, pass) = split_two_quoted(args)?;
    Some(format!(
        "Basic {}",
        base64(&[user.as_bytes(), b":", pass.as_bytes()].concat())
    ))
}

/// 拆出两个成对的引号参数：`"u" "p"` → `("u", "p")`。
///
/// 刻意只认「两个成对引号 + 中间可有空白 + 后面没别的」这一种形状，
/// 其余一律 `None`。理由同 [`unquote`]：参数里可以有空格，没有引号就定不了边界，
/// 而猜错边界的输出会**原样发到服务端**。
///
/// 两个引号必须是同一种；**不支持转义** —— 没有转义规则时，
/// 密码里含引号会落进 `None` 分支（原样保留 + 警告），而不是被悄悄截断。
fn split_two_quoted(args: &str) -> Option<(String, String)> {
    let rest = args.trim_start();
    let quote = *rest.as_bytes().first()?;
    if quote != b'\'' && quote != b'"' {
        return None;
    }

    let after_open = &rest[1..];
    let close = after_open.find(quote as char)?;
    let user = &after_open[..close];

    let rest = after_open[close + 1..].trim_start();
    if *rest.as_bytes().first()? != quote {
        return None;
    }
    let after_open = &rest[1..];
    let close = after_open.find(quote as char)?;
    let pass = &after_open[..close];

    if !after_open[close + 1..].trim().is_empty() {
        return None;
    }
    Some((user.to_string(), pass.to_string()))
}

/// 标准 base64：`A–Za–z0–9+/` 字母表，带 `=` 填充。
///
/// 自己写而不是引 crate：为这十几行加一个依赖的代价是 `Cargo.lock` 变动
/// 外加一次联网拉包，而这台机器上 cargo 走网络会被一个来路不明的代理卡死
/// （见 `refs/rust.md`）。输入也只有 `user:pass` 这种短串，不值得为性能选库。
///
/// ⚠️ 这里**不许**被当成加解密用 —— base64 只是编码，Basic 凭据在网络上
/// 本来就要靠 TLS 保护。
fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map(u32::from);
        let b2 = chunk.get(2).copied().map(u32::from);
        let n = (b0 << 16) | (b1.unwrap_or(0) << 8) | b2.unwrap_or(0);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if b1.is_some() {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if b2.is_some() {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// 固定时刻：2026-09-11 16:46:17 +08:00
    fn clock() -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 9, 11, 16, 46, 17)
            .single()
            .expect("构造固定时刻")
    }

    fn entropy() -> Entropy {
        Entropy::seeded(0x5EED)
    }

    #[test]
    fn timestamps_come_from_the_injected_clock() {
        let now = clock();
        let mut e = entropy();
        assert_eq!(
            evaluate("$timestamp", &now, &mut e).unwrap(),
            now.timestamp().to_string()
        );
        assert_eq!(
            evaluate("$timestampMs", &now, &mut e).unwrap(),
            now.timestamp_millis().to_string()
        );
    }

    /// 注入时钟的意义就在这里：`$datetime` 的断言能落到具体日期上
    #[test]
    fn datetime_uses_strftime_format() {
        let now = clock();
        let mut e = entropy();
        assert_eq!(
            evaluate("$datetime '%Y-%m-%d'", &now, &mut e).unwrap(),
            "2026-09-11"
        );
        assert_eq!(
            evaluate(r#"$datetime "%H:%M:%S""#, &now, &mut e).unwrap(),
            "16:46:17"
        );
        // 格式串里带空格 —— 这也是必须加引号的原因
        assert_eq!(
            evaluate("$datetime '%Y-%m-%d %H:%M'", &now, &mut e).unwrap(),
            "2026-09-11 16:46"
        );
    }

    #[test]
    fn datetime_rejects_moment_style_and_malformed_formats() {
        let now = clock();
        let mut e = entropy();
        // moment 风格（design.md 里那个例子）—— 静默输出字面量比报错更难查
        assert!(evaluate("$datetime 'YYYY-MM-DD'", &now, &mut e).is_none());
        // 缺参数
        assert!(evaluate("$datetime", &now, &mut e).is_none());
        // 没加引号
        assert!(evaluate("$datetime %Y", &now, &mut e).is_none());
        // 空格式串
        assert!(evaluate("$datetime ''", &now, &mut e).is_none());
    }

    /// `$basic` 存在的唯一理由就是那个 base64。
    /// 期望值都与 Python `base64.b64encode(...)` 对齐过。
    #[test]
    fn basic_encodes_user_and_password_with_a_colon_between() {
        let now = clock();
        let mut e = entropy();

        assert_eq!(
            evaluate(r#"$basic "user" "pass""#, &now, &mut e).unwrap(),
            "Basic dXNlcjpwYXNz"
        );
        assert_eq!(
            evaluate(r#"$basic "apiUser" "apiPass""#, &now, &mut e).unwrap(),
            "Basic YXBpVXNlcjphcGlQYXNz"
        );
        // 单引号也认
        assert_eq!(
            evaluate("$basic 'user' 'pass'", &now, &mut e).unwrap(),
            "Basic dXNlcjpwYXNz"
        );
        // 空密码必须留下那个冒号，`=` 填充一个都不能省
        assert_eq!(
            evaluate(r#"$basic "u" """#, &now, &mut e).unwrap(),
            "Basic dTo="
        );
        // 用户名里带冒号是合法的：Basic 只按**第一个**冒号切分
        assert_eq!(
            evaluate(r#"$basic "a:b" "c""#, &now, &mut e).unwrap(),
            "Basic YTpiOmM="
        );
    }

    /// 不支持的形状必须返回 `None`（调用方原样保留 + 警告）。
    /// 绝不能猜一个边界、然后拼出一段**看起来像凭据**的垃圾发到服务端 ——
    /// 那是「配置全对、就是没用上」这类 401 的经典来源。
    #[test]
    fn basic_rejects_anything_that_is_not_two_literal_quotes() {
        let now = clock();
        let mut e = entropy();

        for expr in [
            // 命名空间引用：设计稿 §7 明确留作降级方案，当前「不支持」
            "$basic env.API_USER env.API_PASS",
            // 只给了一半
            r#"$basic "u""#,
            // 尾巴上还有东西
            r#"$basic "u" "p" extra"#,
            // 没加引号 —— 参数里能有空格，不加引号定不了边界
            "$basic u p",
            // 两个引号不是同一种
            r#"$basic "u" 'p'"#,
            // 没有参数
            "$basic",
        ] {
            assert!(
                evaluate(expr, &now, &mut e).is_none(),
                "`{expr}` 不该被当成合法的 $basic"
            );
        }
    }

    /// base64 的边界：长度 1 / 2 / 3 的尾块分别出 2 / 3 / 4 个字符。
    /// 填充位算错会得到一个**长度也对、看着也像**的错串，最难查。
    #[test]
    fn base64_pads_each_remainder_correctly() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // 字母表里最大的两个下标（`+` = 62、`/` = 63）要真的取到：
        // 写成 URL-safe 变体（`-_`）时，普通 Basic 头会被服务端判成非法
        assert_eq!(base64(&[0xfb, 0xf0]), "+/A=");
        assert_eq!(base64(&[0xff, 0xff, 0xff]), "////");
        assert_eq!(base64(&[0x00, 0x00, 0x00]), "AAAA");
    }

    #[test]
    fn uuid_is_v4_shaped_and_unique() {
        let now = clock();
        let mut e = entropy();
        let a = evaluate("$uuid", &now, &mut e).unwrap();
        let b = evaluate("$uuid", &now, &mut e).unwrap();

        assert_ne!(a, b, "两次 $uuid 不该相同");
        assert_eq!(a.len(), 36);
        let parts: Vec<&str> = a.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12]
        );
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        // 版本位 = 4，变体位 ∈ {8,9,a,b}
        assert_eq!(a.as_bytes()[14], b'4');
        assert!(matches!(a.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
    }

    #[test]
    fn random_int_stays_within_inclusive_bounds() {
        let now = clock();
        let mut e = entropy();

        for _ in 0..500 {
            let v: u64 = evaluate("$randomInt", &now, &mut e)
                .unwrap()
                .parse()
                .unwrap();
            assert!(v <= RANDOM_INT_MAX, "{v} 超出了 0..={RANDOM_INT_MAX}");
        }
    }

    /// 上界是**含**的（design.md 写的是 0–10000）。
    ///
    /// 在 10001 上抽样撞到端点几乎不可能（1/10001），所以要验「两端都取得到」
    /// 得换一个 span 只有 2 的输入 —— 直接测 `int_inclusive` 而不是绕表达式。
    #[test]
    fn int_inclusive_covers_both_ends() {
        let mut e = Entropy::seeded(99);
        let mut seen_low = false;
        let mut seen_high = false;

        for _ in 0..200 {
            match e.int_inclusive(1) {
                0 => seen_low = true,
                1 => seen_high = true,
                other => panic!("0..=1 之外的值: {other}"),
            }
        }

        assert!(seen_low && seen_high, "两个端点都该取到");
        assert_eq!(e.int_inclusive(0), 0, "span 为 1 时只能是 0");
    }

    #[test]
    fn same_seed_produces_same_sequence() {
        let now = clock();
        let mut a = Entropy::seeded(7);
        let mut b = Entropy::seeded(7);
        assert_eq!(
            evaluate("$uuid", &now, &mut a),
            evaluate("$uuid", &now, &mut b)
        );
    }

    /// 认不出来的表达式必须返回 None（调用方据此原样保留），
    /// 而不是「当字面量返回」或 panic
    #[test]
    fn unknown_or_malformed_functions_return_none() {
        let now = clock();
        let mut e = entropy();
        for expr in [
            "$datetim",
            "$uuid extra",
            "$",
            "$timestamp Ms",
            "",
            "env.baseUrl",
            "$datetime %Y-%m-%d",
            "$datetime 'YYYY/MM/DD'",
        ] {
            assert!(
                evaluate(expr, &now, &mut e).is_none(),
                "`{expr}` 不该被当成合法函数"
            );
        }
    }

    #[test]
    fn looks_like_function_only_matches_dollar_prefix() {
        assert!(looks_like_function("$uuid"));
        assert!(looks_like_function("$"));
        assert!(!looks_like_function("env.baseUrl"));
        assert!(!looks_like_function("a$b"));
    }
}
