//! 应用设置命令。

use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::storage::models::Setting;
use crate::storage::{migrate, repo, with_conn};
use crate::AppState;

/// 已知的设置键。
///
/// 这里**不作为白名单校验**（否则每加一个键都要改 Rust 并重新编译），
/// 只用于文档化和拼写参考 —— 实际写入仍接受任意合法键名。
pub mod keys {
    /// 主题：light | dark | system
    pub const THEME: &str = "theme";
    /// 配色：indigo | azure | violet | clay | teal
    ///
    /// ⚠️ 与 `THEME` **正交**，是两个键而不是一个枚举。
    /// 后加的键：老库里没有这一行，读出来是 None，
    /// 前端据此判定「用户没选过配色」并保留 localStorage 里的值
    /// （见 `src/lib/sync.ts` 的 `bootstrapSettings`）。
    /// 白名单校验仍然只在前端（`toAccent`）—— 与 THEME 同一条理由：
    /// 不因为加一个键就改 Rust 再重新编译。
    pub const ACCENT: &str = "accent";
    /// 界面语言：zh-CN
    pub const LOCALE: &str = "locale";
    /// 回收站保留天数
    pub const TRASH_RETENTION_DAYS: &str = "trashRetentionDays";
}

// ⚠️ 曾经有过一个 `activeEnvironmentId` 键，P4 把它删了。
//
// 「当前环境」的唯一真源是 `environment.is_active` 这一列（见 `commands::env`
// 与 `repo::env::set_active`）。两处都表达同一件事的结果是：
// 界面在这一处改、发送路径读那一处，然后出现「切换了环境但请求还是老变量」
// 这种几乎无法从代码上看出因果的 bug。
//
// 保留这条注释是因为**老库里的那一行还在**（设 `setting` 是自由键值表，
// 删它要写一次迁移，而它的代价只是几十字节）。读到它时直接忽略即可 ——
// 前端也不再有这个键（见 `src/lib/sync.ts` 的 `SETTING_KEYS`）。

/// 键名合法性：非空、不超过 64 字节、只允许字母数字与 `._-`。
///
/// 挡的是「前端传了个空字符串或带空格的东西进来」这类低级错误，
/// 不试图做业务语义校验。
fn validate_key(key: &str) -> AppResult<()> {
    if key.is_empty() {
        return Err(AppError::InvalidInput {
            field: "key",
            reason: "must not be empty".into(),
        });
    }
    if key.len() > 64 {
        return Err(AppError::InvalidInput {
            field: "key",
            reason: format!("must be at most 64 bytes, got {}", key.len()),
        });
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(AppError::InvalidInput {
            field: "key",
            reason: "only ASCII letters, digits and . _ - are allowed".into(),
        });
    }
    Ok(())
}

#[tauri::command]
pub async fn get_setting(state: State<'_, AppState>, key: String) -> AppResult<Option<String>> {
    validate_key(&key)?;
    with_conn(&state.pool, move |conn| repo::setting::get(conn, &key)).await
}

#[tauri::command]
pub async fn set_setting(state: State<'_, AppState>, key: String, value: String) -> AppResult<()> {
    validate_key(&key)?;
    with_conn(&state.pool, move |conn| {
        repo::setting::set(conn, &key, &value).map(|_| ())
    })
    .await
}

/// 一次拉全部设置。前端启动时用这个，避免为每个键各跑一趟 IPC。
#[tauri::command]
pub async fn get_all_settings(state: State<'_, AppState>) -> AppResult<Vec<Setting>> {
    with_conn(&state.pool, |conn| repo::setting::get_all(conn)).await
}

#[tauri::command]
pub async fn delete_setting(state: State<'_, AppState>, key: String) -> AppResult<bool> {
    validate_key(&key)?;
    with_conn(&state.pool, move |conn| repo::setting::delete(conn, &key)).await
}

/// 运行时信息。「设置 → 关于」与状态栏要用，
/// 同时也让前端能自检 schema 版本是否落后。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    /// 数据库文件绝对路径
    pub db_path: String,
    /// 当前库已应用的迁移版本
    pub schema_version: i64,
    /// 代码里定义的最新迁移版本
    pub latest_schema_version: i64,
    pub app_version: String,
}

#[tauri::command]
pub async fn get_app_info(state: State<'_, AppState>) -> AppResult<AppInfo> {
    let db_path = state.db_path.to_string_lossy().to_string();
    let version = state.pool.get()?.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migration",
        [],
        |r| r.get::<_, i64>(0),
    )?;

    Ok(AppInfo {
        db_path,
        schema_version: version,
        latest_schema_version: migrate::latest_version(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}
