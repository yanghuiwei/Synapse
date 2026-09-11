//! Reqly 后端入口。
//!
//! 启动顺序：解析数据目录 → 建连接池 → 跑迁移 → 保证有默认工作区 → 注册状态。
//! 任一步失败都直接终止启动 —— 存储层起不来时应用没有任何可用功能，
//! 与其带着半死不活的状态运行，不如把错误明确抛出来。

pub mod commands;
pub mod error;
pub mod http;
pub mod storage;

use std::path::PathBuf;

use tauri::Manager;

use error::{AppError, AppResult};
use http::{ClientCache, InflightRegistry};
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
            // 请求执行
            commands::request::send_request,
            commands::request::cancel_request,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
