//! 请求执行层。
//!
//! 为什么请求必须走 Rust（design.md §2.1）：
//! * 绕开浏览器/WebView 的 **CORS** —— 否则任何没配 `Access-Control-Allow-Origin` 的接口都发不出去
//! * 绕开 **HTTP/1.1 单域名 6 并发**上限 —— 压测根本没法做
//! * 能改 TLS 校验、能设代理、能拿到精确到微秒的计时
//!
//! 分层：[`client`] 负责「按配置拿到一个可复用的 Client」，[`execute`] 负责
//! 「一次请求的组装 → 发送 → 计时 → 读响应体」。压测（P6）会复用同一个 [`client::ClientCache`]，
//! 这样连接池是共享的，不会出现「压测自己成了压测瓶颈」。

pub mod cancel;
pub mod client;
pub mod curl;
pub mod execute;
pub mod pipeline;
pub mod token;

pub use cancel::{InflightGuard, InflightRegistry};
pub use client::ClientCache;
pub use curl::to_curl;
pub use execute::{
    execute, execute_with_blobs, execute_with_cancel, BodyEncoding, BodyFile, HttpHeader,
    HttpResponsePayload, RequestSpec,
};
pub use pipeline::{Pipeline, Prepared};
pub use token::{remaining_seconds, TokenDeps, TokenRefresher, TokenUsage};
