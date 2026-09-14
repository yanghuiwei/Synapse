//! Reqly 后端入口。
//!
//! 启动顺序：解析数据目录 → 建连接池 → 跑迁移 → 保证有默认工作区 → 注册状态。
//! 任一步失败都直接终止启动 —— 存储层起不来时应用没有任何可用功能，
//! 与其带着半死不活的状态运行，不如把错误明确抛出来。

pub mod commands;
pub mod error;
pub mod http;
pub mod importer;
pub mod loadtest;
pub mod storage;
pub mod template;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tauri::Manager;

use error::{AppError, AppResult};
use http::{ClientCache, InflightRegistry, TokenRefresher};
use loadtest::LoadtestRegistry;
use storage::pool::{self, SqlitePool};
use storage::{migrate, repo};

/// 数据库文件名。放在 tauri 的 app_data_dir 下
/// （Windows 上是 `%APPDATA%\<bundle identifier>\`）。
const DB_FILE_NAME: &str = "reqly.db";

/// 全局状态。命令通过 `State<'_, AppState>` 取用。
pub struct AppState {
    pub pool: SqlitePool,
    /// 保留绝对路径：设置页要显示、出问题时用户要能自己去打开
    pub db_path: PathBuf,
    /// 按 `RequestSettings` 复用的 HTTP 客户端。**必须全局只有一份** ——
    /// 它的价值在于内部的连接池（keep-alive），每次命令都新建就白搭了。
    pub clients: ClientCache,
    /// 进行中的请求登记表，`cancel_request` 靠它找回要打断的那次执行。
    pub inflight: InflightRegistry,
    /// Token 源的「航班锁」（design.md §4.3）。也**必须全局只有一份** ——
    /// 每个命令各建一个的话，「并发 5 个请求只刷一次」立刻退化成刷 5 次。
    pub tokens: TokenRefresher,
    /// 最近一次成功响应的 JSON，`{{resp.$.path}}` 的来源。
    ///
    /// 进程级、最近一次 —— design.md §4.2 写的是「最近一次响应（会话内）」，
    /// 没有按请求/按文件夹细分。细分要回答「同一个请求自己的上一次算不算」，
    /// 那是 P5 的事；先把语义钉成最简单的一种。
    ///
    /// 存 `Arc<Value>` 而不是 `Value`：读取时克隆一个 `Arc` 就能把锁放下，
    /// 不必抱着 `MutexGuard` 穿过 await（`std` 的 guard 不是 `Send`）。
    pub last_response: Mutex<Option<Arc<serde_json::Value>>>,
    /// 「当前正在跑的那一轮压测」的占坑与取消（P6）。
    ///
    /// 与 `inflight` 分开而不是合表：那一张是按 `runId` 索引、面向「一次请求」的，
    /// 而压测同时只允许一轮、且守卫要能搬进 `tokio::spawn` 出来的引擎任务
    /// （见 `loadtest::registry` 的模块说明）。
    pub loadtests: LoadtestRegistry,
}

fn resolve_db_path(app: &tauri::AppHandle) -> AppResult<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Internal(format!("cannot resolve app data dir: {e}")))?;
    Ok(dir.join(DB_FILE_NAME))
}

fn init_storage(app: &tauri::AppHandle) -> AppResult<AppState> {
    let db_path = resolve_db_path(app)?;
    let pool = pool::create_pool(&db_path)?;

    {
        let mut conn = pool.get()?;

        let applied = migrate::run(&mut conn)?;
        if applied.is_empty() {
            println!(
                "[reqly] schema up to date (v{})",
                migrate::current_version(&conn)?
            );
        } else {
            println!("[reqly] applied migrations: {applied:?}");
        }

        let ws = repo::workspace::ensure_default(&conn)?;
        println!(
            "[reqly] db ready: {} (workspace #{})",
            db_path.display(),
            ws.id
        );
    }

    Ok(AppState {
        pool,
        db_path,
        clients: ClientCache::new(),
        inflight: InflightRegistry::new(),
        tokens: TokenRefresher::new(),
        last_response: Mutex::new(None),
        loadtests: LoadtestRegistry::new(),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // setup 的错误类型是 Box<dyn Error>，转成 String 以保留原始描述
            let state = init_storage(app.handle()).map_err(|e| e.to_string())?;
            app.manage(state);
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
