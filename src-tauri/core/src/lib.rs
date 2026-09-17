//! Reqly 的核心层 —— 与 UI 框架无关的那一半。
//!
//! ## 为什么要有这个 crate
//!
//! 桌面端、MCP Server、CLI 三个入口做的是同一批事：读同一个 SQLite、跑同一套
//! 模板解析、发同一个 HTTP 请求。它们唯一的区别是**谁来触发**、**结果怎么呈现**。
//!
//! 在拆出这一层之前，`reqly-mcp.exe` 为了用上这些能力，一路链到桌面 crate，
//! 于是把整个 Tauri 拖进了二进制里 —— **21.5 MB**，而它其实只读写一个 SQLite 文件。
//! CLI 会是同一个下场，只会更重（多一个参数解析库）。
//!
//! ## 边界在哪
//!
//! 这个 crate **不依赖 tauri、rmcp、axum**（`Cargo.toml` 里没有它们，是硬约束）。
//! 三层各自往上加自己那点东西：
//!
//! | 入口 | 额外依赖 | 职责 |
//! |---|---|---|
//! | 桌面 `reqly` | tauri | `#[tauri::command]` 包装、窗口与事件 |
//! | `reqly-mcp` | rmcp + axum | JSON-RPC 通道、tool 的 schema 与注解 |
//! | `reqly-cli` | clap | 子命令、参数、退出码 |
//!
//! ## 分层
//!
//! * `error` —— 稳定错误码（对外契约：**只给码，不给文案**）
//! * `state` —— [`AppState`]：连接池 + 客户端缓存 + 在途请求 + Token 航班锁
//! * `storage` —— 连接池 / 迁移 / 模型 / 仓储（异步边界只在 `with_conn`）
//! * `http` —— 纯执行器（给 spec 就发，不碰数据库）
//! * `template` —— `{{...}}` 解析（函数 / JSONPath / 变量解析）
//! * `loadtest` —— 压测引擎、分位统计、报表
//! * `importer` —— curl / Postman / reqly 三类导入器的解析
//! * `service` —— **不吃 `tauri::State` 的编排层**（[`service::ops`] 是三个入口
//!   共用的 13 个操作），以及原先埋在 `commands/*` 里、只在 `&Connection` 上
//!   成立的几个函数
//! * `mcp` —— 只放 [`mcp::config_gen`]（给 WorkBuddy / Hermes 生成配置片段，
//!   桌面面板和 CLI 都要用同一份，所以它属于核心而不是某个入口）
//! * `redact` —— 凭据打码的唯一名单（`service::ops` 与 `http::curl` 共用）

pub mod error;
pub mod http;
pub mod importer;
pub mod loadtest;
pub mod mcp;
pub mod redact;
pub mod service;
pub mod state;
pub mod storage;
pub mod template;

pub use state::AppState;
