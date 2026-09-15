//! 业务编排层 —— 「不吃 `tauri::State`」的那些函数。
//!
//! ## 为什么单独成层
//!
//! `commands/*` 里每个函数都要 `tauri::State<'_, AppState>`，而 `tauri::State`
//! **没有公开构造函数** —— 非 Tauri 进程根本造不出来。所以 MCP 与 CLI 一行都复用不到，
//! 只能绕开命令层直连 `repo` + `Pipeline` 把编排重写一遍。
//!
//! P7 已经吃过这个亏：13 个 tool 的实现与 `commands/*` 各写了一份，症状是
//! 「桌面端点得对、AI 那边悄悄不对」—— 而那种 bug 不会有人去查。
//!
//! 把「编排」从「tauri 包装」里剥出来放进这里，三个入口就都调同一份。
//! 这一层的约定：**收 `&AppState` 或 `&mut Connection`，不碰任何 Tauri 类型**。
//!
//! ⚠️ 子模块是按**来源文件**分的（request / collection / loadtest / import），
//! 不是为了分层好看 —— 它们原先散在 `commands/*` 的中间，搬过来时保持原样，
//! 谁在哪一行出过问题可以直接对照。

pub mod collection;
pub mod import;
pub mod loadtest;
/// 13 个操作的实现体 —— MCP tool 与 CLI 子命令共用的那一份。
///
/// 它不是「按来源文件分」的（这一层里唯一的一个例外）：原先整块长在
/// `mcp/tools.rs` 里，P8 原样搬过来、只把 rmcp 的类型换成 `AppResult`。
pub mod ops;
pub mod request;
pub mod validate;
