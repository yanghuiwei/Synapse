//! 凭据打码。
//!
//! 名单只留这一份：MCP/CLI 的对外 DTO 打码（`service::ops`）与 curl 导出
//! （`http::curl`）用的是**同一套判定**。各写一份的话，某一处补了 `x-api-key`、
//! 另一处没补，表现是「同一个 token 在 AI 那边打了码、在 curl 里是明文」——
//! 而没有人会去比对这两处，等发现的时候凭据已经贴出去了。

/// 打码后的替身。与 `design.md` §8.3 写的字符一致。
pub const MASK: &str = "••••";

/// 凭据头（按小写字面量比较）。
///
/// `design.md` §8.3 只点名了 `set-cookie` 与 `authorization`，但**照字面做只防了一半**：
/// 真实接口里 `x-api-key` / `x-auth-token` 这类自定义头与 `Authorization` 完全等价，
/// 漏掉它们，凭据照样进 AI 的上下文。
const SECRET_HEADERS: [&str; 6] = [
    "set-cookie",
    "cookie",
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
];

/// 再加一条后缀规则，省得穷举每个厂商的命名（`x-auth-token`、`x-client-secret`…）。
///
/// 方向刻意偏向「宁可多打」：多打一个非敏感头，代价是少看见一个值；
/// 漏掉一个凭据头，代价是它进了 AI 的上下文或用户的剪贴板。两者不对称。
///
/// ## 为什么**不带**前导连字符
///
/// 原来的名单是 `["-key", "-token", "-secret", "-password"]`，四个都带连字符。
/// 于是驼峰命名**一个都命中不了** —— 而本机真库里真实用过的凭据头里，
/// 有 4 个是这么漏掉的：
///
/// | 头名 | 次数 | 旧规则 |
/// |---|---|---|
/// | `x-wlk-Authorization` | 4 | ❌ 漏（`-authorization` 不在名单里） |
/// | `x-http-uniquetoken` | 2 | ❌ 漏（是 `uniquetoken`，没有连字符） |
/// | `securityKey` | 1 | ❌ 漏（`Key` 前面没有连字符） |
/// | `refreshToken` | 1 | ❌ 漏（`Token` 前面没有连字符） |
///
/// 而本文件的模块注释自己写着「漏掉一个凭据头，代价是它进了 AI 的上下文
/// 或用户的剪贴板」。所以判据改成**只看词尾、不分大小写**。
///
/// `authorization` 也进了名单：`proxy-authorization` 那条已经在显式名单里，
/// 但 `x-wlk-Authorization`、`x-forwarded-authorization` 这类厂商写法只有
/// 后缀规则能覆盖 —— 为某一个厂商写一条字面量，下一个厂商又得再加一条。
const SECRET_HEADER_SUFFIXES: [&str; 5] = ["token", "key", "secret", "password", "authorization"];

/// 这个名字对应的值算不算凭据。
pub fn is_secret_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SECRET_HEADERS.contains(&lower.as_str())
        || SECRET_HEADER_SUFFIXES
            .iter()
            .any(|suffix| lower.ends_with(suffix))
}

/// 按名字决定要不要把值换成打码串。非凭据头原样返回。
pub fn mask_value(name: &str, value: &str) -> String {
    if is_secret_header(name) {
        MASK.to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_headers_are_masked() {
        for name in [
            "Authorization",
            "proxy-authorization",
            "Set-Cookie",
            "Cookie",
            "X-Api-Key",
            "Api-Key",
        ] {
            assert_eq!(mask_value(name, "secret"), MASK, "{name} 必须打码");
        }
    }

    /// 后缀规则是「宁可多打」的，这里把它的两边都钉住：
    /// 厂商自定义的凭据头必须命中，普通头不能被误伤
    #[test]
    fn suffix_rule_catches_vendor_names_without_touching_plain_headers() {
        for name in [
            "X-Auth-Token",
            "X-Client-Secret",
            "X-User-Password",
            "X-Tenant-Key",
        ] {
            assert_eq!(mask_value(name, "v"), MASK, "{name} 必须打码");
        }
        for name in ["Content-Type", "Accept", "User-Agent", "X-Request-Id"] {
            assert_eq!(mask_value(name, "v"), "v", "{name} 不该被打码");
        }
    }

    /// 把前导连字符去掉之后，本机真库里**真实用过的**凭据头一个都不能漏。
    ///
    /// 四条 `❌` 的那几个就是这条规则的来处 —— 它们在旧规则下是明文进
    /// curl 导出与 MCP DTO 的（2026-09-17 实测）。
    #[test]
    fn credential_headers_actually_used_in_the_local_db_are_masked() {
        for name in [
            "x-wlk-Authorization",         // 旧规则漏：`-authorization` 不在名单里
            "x-http-uniquetoken",          // 旧规则漏：没有连字符
            "securityKey",                 // 旧规则漏：驼峰
            "refreshToken",                // 旧规则漏：驼峰
            "X-Access-Token",              // 旧规则命中（`-token`）
            "x-acs-dingtalk-access-token", // 旧规则命中
        ] {
            assert_eq!(mask_value(name, "v"), MASK, "{name} 必须打码");
        }
    }

    /// ⚠️ 已知、且**刻意接受**的误伤：只看词尾之后，`monkey` / `donkey` 这种
    /// 「碰巧以 key 结尾」的普通头也会被打码。
    ///
    /// 把它写下来是为了让后人知道这是**取舍的结果**，不是忘了收窄规则。
    /// 方向是不对称的：多打一个非敏感头，代价是少看见一个值；
    /// 漏掉一个凭据头，代价是它进了 AI 的上下文或别人的剪贴板。
    /// 真要收窄，得回到「词边界」判据（连字符 / 大小写切换），
    /// 而那会重新漏掉 `x-http-uniquetoken` —— 整件事的起因就是它。
    #[test]
    fn the_loose_suffix_rule_deliberately_over_masks_key_ending_words() {
        assert_eq!(mask_value("X-Monkey", "v"), MASK);
        assert_eq!(mask_value("X-Turkey", "v"), MASK);
        // 但真的不该命中的仍然不命中
        for name in ["Content-Length", "Cache-Control", "X-Trace-Id", "Origin"] {
            assert_eq!(mask_value(name, "v"), "v", "{name} 不该被打码");
        }
    }
}
