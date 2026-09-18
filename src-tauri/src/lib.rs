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
use reqly_core::state::migrate_legacy_db;

/// 数据库文件名。放在 tauri 的 app_data_dir 下
/// （Windows 上是 `%APPDATA%\<bundle identifier>\`）。
///
/// 它只属于桌面端 —— MCP 与 CLI 的库路径由 `--db` 直接给，不去猜数据目录。
const DB_FILE_NAME: &str = "reqly.db";

/// 上一个 `identifier`。
///
/// Tauri 拿 identifier 当 app data 目录名，所以它一变，老用户的库就留在了旧目录里、
/// 界面上「什么都没有」。启动时要把它搬过来 —— 见
/// [`reqly_core::state::migrate_legacy_db`]。
///
/// 这个常量只在「从 `com.reqly.app` 迁移到 `com.reqly.desktop`」这一件事上有意义，
/// 搬完之后它就永远匹配不到东西（`migrate_legacy_db` 是幂等的）。留着是因为
/// 总会有用户从老版本升上来。
const LEGACY_APP_DIR: &str = "com.reqly.app";

/// 旧 identifier 目录下的库路径。
///
/// 单独拆出来是为了**能测**：`resolve_db_path` 要 `AppHandle` 才能跑，而这里面全是
/// 纯路径算术 —— 真正容易写错的就是「往上一级、再拼旧 identifier」这一步。
fn legacy_db_path(app_data_dir: &std::path::Path) -> Option<PathBuf> {
    app_data_dir
        .parent()
        .map(|parent| parent.join(LEGACY_APP_DIR).join(DB_FILE_NAME))
}

fn resolve_db_path(app: &tauri::AppHandle) -> AppResult<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Internal(format!("cannot resolve app data dir: {e}")))?;
    let db_path = dir.join(DB_FILE_NAME);

    // `app_data_dir()` 的形状就是 `<父目录>/<identifier>`，所以旧目录 = 同一个父目录
    // 加上旧 identifier。三个平台都是这个形状（Windows `%APPDATA%`、
    // macOS `~/Library/Application Support`、Linux `~/.local/share`），不用按平台分支。
    if let Some(legacy) = legacy_db_path(&dir) {
        // 失败就往上抛：带着一个「看起来是空的」库继续跑，比直接起不来更糟 ——
        // 用户一旦在新库里建了东西，旧数据到底要不要并过来就没有干净答案了。
        if migrate_legacy_db(&legacy, &db_path)? {
            eprintln!(
                "[reqly] migrated database from legacy identifier: {} -> {} (旧目录保留未删)",
                legacy.display(),
                db_path.display()
            );
        }
    }

    Ok(db_path)
}

fn init_storage(app: &tauri::AppHandle) -> AppResult<AppState> {
    AppState::new(resolve_db_path(app)?)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
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
            commands::history::latest_history_curl,
            // 请求执行
            commands::request::send_request,
            commands::request::cancel_request,
            // 「查看 curl」：不发只解析，用于还没跑过的那条请求
            commands::request::preview_curl,
            // 文件对话框与响应另存（上传文件 / 导出流落盘）
            commands::file::pick_file,
            commands::file::save_response_body,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_db_lives_next_to_the_current_identifier_directory() {
        // `app_data_dir()` 给的是 `<roaming>/<identifier>`，旧库在**隔壁**那个目录里 ——
        // 不是里面，也不是上两级。写错的话迁移会静默找不到源文件、直接返回 Ok(false)。
        let dir = std::path::Path::new(r"C:\Users\example\AppData\Roaming\com.reqly.desktop");

        let legacy = legacy_db_path(dir).expect("app data dir always has a parent");

        assert_eq!(
            legacy,
            std::path::PathBuf::from(r"C:\Users\example\AppData\Roaming\com.reqly.app\reqly.db")
        );
    }

    #[test]
    fn a_rootless_app_data_dir_yields_nothing_instead_of_panicking() {
        // `parent()` 对裸相对路径也可能是 `None`，这里不能 unwrap
        assert!(legacy_db_path(std::path::Path::new("")).is_none());
    }
}
