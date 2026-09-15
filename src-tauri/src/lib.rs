//! Reqly 桌面端入口（Tauri 层）。
//!
//! 这一层只剩三件事：把核心层接上 Tauri、注册命令、起窗口。
//! 业务实现全部在 `reqly-core` 里 —— 桌面端、MCP Server、CLI 三个入口共用它。
//!
//! MCP Server 已经不在这个 crate 里了（P8 拆成 `reqly-mcp` package）：
//! 它只读写同一个 SQLite，却因为链到这里而把整个 tauri 拖进了二进制。
//! 桌面端留下的 MCP 相关代码只有 `commands/mcp.rs` —— 那个面板要展示的
//! 配置片段由 `reqly_core::mcp::config_gen` 生成。
//!
//! 启动顺序：解析数据目录 → 建连接池 → 跑迁移 → 保证有默认工作区 → 注册状态。
//! 任一步失败都直接终止启动 —— 存储层起不来时应用没有任何可用功能，
//! 与其带着半死不活的状态运行，不如把错误明确抛出来。

pub mod commands;
mod external_changes;

/// 命令层的 `use crate::AppState;` 靠这条成立。
pub use reqly_core::AppState;
/// 核心层在桌面 crate 里的转发面。
///
/// `commands/*` 原本写的是 `crate::storage::repo` / `crate::error::AppError` 这类路径。
/// 这些模块搬进 `reqly-core` 之后，用一组 re-export 把它们重新挂回 `crate::` 下，
/// 于是几千行命令层代码一行都不用动 —— 拆 crate 的风险主要就在这种「大面积改引用」上，
/// 能用转发消掉就不去逐个改。
///
/// **这是为迁移服务的**：新写的模块请直接 `use reqly_core::...`，不要再加深这层别名。
pub use reqly_core::{error, http, importer, loadtest, storage, template};

use std::path::PathBuf;

use tauri::Manager;

use reqly_core::error::{AppError, AppResult};

/// 数据库文件名。放在 tauri 的 app_data_dir 下
/// （Windows 上是 `%APPDATA%\<bundle identifier>\`）。
///
/// 它只属于桌面端 —— MCP 与 CLI 的库路径由 `--db` 直接给，不去猜数据目录。
const DB_FILE_NAME: &str = "reqly.db";

fn resolve_db_path(app: &tauri::AppHandle) -> AppResult<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Internal(format!("cannot resolve app data dir: {e}")))?;
    Ok(dir.join(DB_FILE_NAME))
}

fn init_storage(app: &tauri::AppHandle) -> AppResult<AppState> {
    AppState::new(resolve_db_path(app)?)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // setup 的错误类型是 Box<dyn Error>，转成 String 以保留原始描述
            let state = init_storage(app.handle()).map_err(|e| e.to_string())?;

            // 先 clone 池再交出去：后台轮询任务只需要池，而 `manage` 会把 state 移走。
            // 池是 r2d2 的 `Pool`，clone 一份是 Arc 级别的操作，不是深拷贝。
            let pool = state.pool.clone();
            app.manage(state);

            // MCP / CLI 改了同一个库时把界面叫醒（design.md §8.1 承诺的文件监听）。
            // 放在 `setup` 里是因为它需要 `AppHandle` 才能往后发事件。
            external_changes::spawn(app.handle().clone(), pool);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // 设置
            commands::settings::get_setting,
            commands::settings::set_setting,
            commands::settings::get_all_settings,
            commands::settings::delete_setting,
            commands::settings::get_app_info,
            // 工作区 / 集合树
            commands::workspace::list_workspaces,
            commands::workspace::get_active_workspace,
            commands::folder::list_folders,
            commands::request::list_requests,
            // 集合的增删改（P3）
            commands::collection::create_folder,
            commands::collection::rename_folder,
            commands::collection::set_folder_token_source,
            commands::collection::set_folder_auth,
            commands::collection::move_folder,
            commands::collection::delete_folder,
            commands::collection::restore_folder,
            commands::collection::purge_folder,
            commands::collection::create_request,
            commands::collection::save_request,
            commands::collection::rename_request,
            commands::collection::duplicate_request,
            commands::collection::move_request,
            commands::collection::delete_request,
            commands::collection::restore_request,
            commands::collection::purge_request,
            commands::collection::list_trash,
            commands::collection::purge_trash,
            // 历史记录（P3）
            commands::history::list_history,
            commands::history::get_history,
            commands::history::record_history,
            commands::history::delete_history,
            commands::history::restore_history,
            commands::history::purge_history,
            commands::history::clear_history,
            // 请求执行
            commands::request::send_request,
            commands::request::cancel_request,
            // 导入器（P5）
            commands::import_::preview_import,
            commands::import_::commit_import,
            // 环境与变量（P4）
            commands::env::list_environments,
            commands::env::get_active_environment,
            commands::env::create_environment,
            commands::env::rename_environment,
            commands::env::set_active_environment,
            commands::env::delete_environment,
            commands::env::restore_environment,
            commands::env::purge_environment,
            commands::env::list_env_vars,
            commands::env::replace_env_vars,
            // Token 源（P4）
            commands::token::list_token_sources,
            commands::token::create_token_source,
            commands::token::save_token_source,
            commands::token::delete_token_source,
            commands::token::resolve_token_source,
            commands::token::get_token_status,
            commands::token::refresh_token,
            // 压测（P6）
            commands::loadtest::start_loadtest,
            commands::loadtest::cancel_loadtest,
            commands::loadtest::list_loadtest_runs,
            commands::loadtest::get_loadtest_run,
            commands::loadtest::get_loadtest_samples,
            commands::loadtest::get_loadtest_chart,
            commands::loadtest::delete_loadtest_run,
            commands::loadtest::export_loadtest_report,
            // MCP（P7）
            commands::mcp::get_mcp_setup,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
