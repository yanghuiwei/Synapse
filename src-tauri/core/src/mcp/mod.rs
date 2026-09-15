//! `mcp` —— 只装配置片段的生成器。
//!
//! ⚠️ 这里**不是** MCP Server 本身。`server` / `transport` / `tools` 住在
//! `src-tauri/mcp/` 那个独立 package 里（它们要 rmcp 与 axum），
//! 而 [`config_gen`] 是纯字符串拼接 —— 桌面端的「连接 AI 客户端」面板要它，
//! CLI 的 `reqly mcp config` 也要它，两份实现迟早分叉，所以它归核心。

pub mod config_gen;
