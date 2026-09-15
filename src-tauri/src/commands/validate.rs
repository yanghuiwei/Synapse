//! 校验助手的转发入口。
//!
//! 实现已搬去 `reqly-core`（三个入口共用同一套名字规则）。保留这一层是为了
//! 让 `crate::commands::validate::...` 这个既有路径继续有效。

pub use reqly_core::service::validate::{clean_name, MAX_NAME_LEN};
