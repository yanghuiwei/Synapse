//! 模板引擎：把请求里字符串字段上的 `{{...}}` 展开成真实值。
//!
//! 三个子模块各管一件事：
//! * [`functions`] —— `{{$uuid}}` `{{$timestamp}}` 这类动态函数（纯函数，时钟与熵注入）
//! * [`jsonpath`] —— `{{resp.$.data.token}}` 里 `$` 之后那截路径（明确子集）
//! * [`resolver`] —— 扫描、分派、原样保留未命中，以及 `resolve_spec` 的字段编排
//!
//! **这里不碰数据库、不碰网络**：Token 的值由 `http::token` 在解析之前备好。
//! 这条边界让解析器的全部行为都能在同步单测里钉死。

pub mod functions;
pub mod jsonpath;
pub mod resolver;

pub use resolver::{
    resolve, resolve_spec, scan, scan_spec, Namespace, ResolvedPlaceholder, ResolvedSpec,
    ResolvedText, TemplateContext, TokenValues, UnresolvedPlaceholder, UnresolvedReason,
};
