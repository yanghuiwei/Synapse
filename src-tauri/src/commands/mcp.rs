//! MCP 客户端的「连接」信息（plan.md P7-7）。
//!
//! 面板上要展示两段能直接粘进配置文件的片段，以及它们指向的两个路径。
//! 为什么不把片段在前端拼：路径的转义规则（反斜杠、空格）是**格式相关**的知识，
//! 已经沉淀在 `reqly_core::mcp::config_gen` 里并有测试钉着（见那里的模块注释）——
//! 前端再拼一遍就是第二份实现，两份迟早会分叉。
//!
//! 两个路径都如实回给界面，是为了让用户能自己核对。这一块最常见的故障
//! 就是把 `command` 写成了桌面端的 `reqly.exe`，界面上看不到真实路径时无从发现。

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::AppState;
use reqly_core::mcp::config_gen::{json_snippet, yaml_snippet, McpPaths};

/// MCP 可执行文件名。
///
/// 期望它与桌面端 `reqly.exe` **同目录**，所以从 `current_exe()` 的父目录推得。
///
/// ⚠️ 「同目录」这条**开发态成立、打包后不成立**：
/// `cargo build` 会把同一个 package 的**所有** bin 都放进 `target/<profile>/`（所以 dev 下能推对），
/// 但 Tauri 打包只收主 bin，兄弟 bin target 不会自动进安装目录。
/// 要让它进安装包，得在 `tauri.conf.json` 的 `bundle.externalBin` 里登记，
/// 且文件名必须带 `-$TARGET_TRIPLE` 后缀 —— 也就是构建前得先把它搬到 `src-tauri/binaries/` 并改名。
/// （<https://v2.tauri.app/develop/sidecar/>）归 P8 打包那一步做，见 plan.md §15.6。
const MCP_BIN_NAME: &str = "reqly-mcp.exe";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSetup {
    /// WorkBuddy / Claude Code / Cursor 用的 JSON 片段
    pub json_snippet: String,
    /// Hermes Agent 用的 YAML 片段
    pub yaml_snippet: String,
    /// 回给界面是为了展示，不是为了让用户去点 —— 它不一定存在（见下）
    pub command_path: String,
    pub db_path: String,
    /// MCP 可执行文件在那个路径上是否真的存在。
    ///
    /// 开发态跑 `cargo run`（只编桌面端 bin）时它是 **false** —— 此时配置片段
    /// 语法没错但连不上。界面必须能说清是「还没编译」而不是让用户对着一段
    /// 看似正确的配置反复重连。
    pub command_exists: bool,
}

#[tauri::command]
pub fn get_mcp_setup(state: State<'_, AppState>) -> AppResult<McpSetup> {
    let command = mcp_command_path()?;
    let paths = McpPaths::new(&command, &state.db_path);

    Ok(McpSetup {
        json_snippet: json_snippet(&paths),
        yaml_snippet: yaml_snippet(&paths),
        command_path: command.to_string_lossy().into_owned(),
        db_path: state.db_path.to_string_lossy().into_owned(),
        command_exists: command.is_file(),
    })
}

fn mcp_command_path() -> AppResult<PathBuf> {
    let exe = std::env::current_exe()
        .map_err(|e| AppError::Internal(format!("cannot resolve current exe: {e}")))?;
    let dir = exe.parent().ok_or_else(|| {
        AppError::Internal(format!("current exe has no parent dir: {}", exe.display()))
    })?;
    Ok(dir.join(MCP_BIN_NAME))
}
