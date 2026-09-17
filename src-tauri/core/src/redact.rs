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
const SECRET_HEADER_SUFFIXES: [&str; 4] = ["-key", "-token", "-secret", "-password"];

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
}
