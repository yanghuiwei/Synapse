//! MCP Server 的库面 —— 只有 `bin/reqly-mcp` 一个消费者。
//!
//! ## 为什么 lib 与 bin 都在这个 package 里
//!
//! 因为 `server.rs` / `tools.rs` / `transport.rs` 的**唯一**使用者就是那个 bin。
//! 拆成两个 package 只会多一个 `Cargo.toml`，换不到任何隔离。
//! （对比 `reqly-core`：它被三个入口共用，那才值得单独成包。）
//!
//! ## 与核心层的边界
//!
//! 13 个操作的实现体在 `reqly_core::service::ops`；这里只做协议侧的三件事：
//! 注册 tool 与注解、生成 JSON Schema、把 `AppResult` 翻成 JSON-RPC 错误。

pub mod server;
pub mod tools;
pub mod transport;
