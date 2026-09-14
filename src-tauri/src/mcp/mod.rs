//! MCP Server（design.md §8）—— 把整个工作台暴露成 AI 客户端的工具集。
//!
//! 关键设计：MCP 进程**直接读写同一个 SQLite 文件**（WAL 支持多进程并发读），
//! 不依赖桌面 App 是否在运行。
//!
//! 模块划分：
//! - [`config_gen`]：生成客户端的配置片段（纯字符串逻辑，与运行时无关）
//! - [`tools`]：13 个 tool 的定义、注解与实现
//! - [`server`]：把 tool 装配成 rmcp 的 `ServerHandler`
//! - [`transport`]：stdio 与回环 HTTP 两条通道的启动

pub mod config_gen;
pub mod server;
pub mod tools;
pub mod transport;
