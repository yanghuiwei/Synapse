//! 环境与变量命令（design.md §4.2 的 `{{env.KEY}}`）。
//!
//! 仓储（`storage::repo::env`）在 P1 就写好了并带单测，这里只补命令层：
//! **校验入参 → 调仓储 → 映射错误**，不写业务逻辑、不产生面向用户的文案。
//!
//! ## 「当前环境」的唯一真源是 `environment.is_active`
//!
//! `setting` 表里曾经有一个 `activeEnvironmentId` 键也表达同一件事，
//! P4 把它去掉了（`commands::settings::keys` 里留了说明）。理由：
//! * `is_active` 是**行内列**，`set_active` 在一个事务里保证「同一工作区只有一个 1」，
//!   这个不变量能在数据层被验证（`repo::env` 有单测钉着）
//! * 它天然带 `workspace_id`，多工作区一放开不会串
//! * setting 键没有工作区维度，一组多工作区就必然要先改它 —— 现在用它等于给自己挖坑
//!
//! `deleted_at` 与 `is_active` 的关系也在这里收口：软删环境会顺手把
//! `is_active` 置 0（见 `repo::env::soft_delete`），不存在「激活着一个回收站里的环境」。

use tauri::State;

use crate::commands::validate::clean_name;
use crate::error::{AppError, AppResult};
use crate::storage::models::{EnvVar, Environment};
use crate::storage::{repo, with_conn};
use crate::AppState;

/// 变量名的长度上限。比名字短得多：`{{env.x}}` 是要敲进 URL 里的。
const MAX_VAR_KEY_LEN: usize = 128;

// ============================================================
// 环境
// ============================================================

#[tauri::command]
pub async fn list_environments(
    state: State<'_, AppState>,
    workspace_id: i64,
) -> AppResult<Vec<Environment>> {
    with_conn(&state.pool, move |conn| {
        repo::env::list_by_workspace(conn, workspace_id)
    })
    .await
}

/// 当前激活的环境。没有激活项时返回 `null`（不是错误 —— 「不使用环境」是常态）。
#[tauri::command]
pub async fn get_active_environment(
    state: State<'_, AppState>,
    workspace_id: i64,
) -> AppResult<Option<Environment>> {
    with_conn(&state.pool, move |conn| {
        repo::env::get_active(conn, workspace_id)
    })
    .await
}

#[tauri::command]
pub async fn create_environment(
    state: State<'_, AppState>,
    workspace_id: i64,
    name: String,
) -> AppResult<Environment> {
    let name = clean_name(&name, "name")?;
    with_conn(&state.pool, move |conn| {
        repo::env::create(conn, workspace_id, &name)
    })
    .await
}

#[tauri::command]
pub async fn rename_environment(
    state: State<'_, AppState>,
    id: i64,
    name: String,
) -> AppResult<Environment> {
    let name = clean_name(&name, "name")?;
    with_conn(&state.pool, move |conn| repo::env::rename(conn, id, &name)).await
}

/// 切换激活环境。`id = null` 表示「不使用环境」。
///
/// 「同一工作区只有一个激活环境」由仓储在一个事务里保证（先全置 0 再置 1），
/// 所以这里不需要任何额外校验 —— 也不要在这里再写一遍，两处规则必然分叉。
#[tauri::command]
pub async fn set_active_environment(
    state: State<'_, AppState>,
    workspace_id: i64,
    id: Option<i64>,
) -> AppResult<()> {
    with_conn(&state.pool, move |conn| {
        repo::env::set_active(conn, workspace_id, id)
    })
    .await
}

/// 软删除（进回收站）。仓储会顺手把 `is_active` 置 0。
#[tauri::command]
pub async fn delete_environment(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    with_conn(&state.pool, move |conn| repo::env::soft_delete(conn, id)).await
}

#[tauri::command]
pub async fn restore_environment(state: State<'_, AppState>, id: i64) -> AppResult<Environment> {
    with_conn(&state.pool, move |conn| repo::env::restore(conn, id)).await
}

/// 彻底删除。**不可撤销**（变量由外键级联带走）。
#[tauri::command]
pub async fn purge_environment(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    with_conn(&state.pool, move |conn| repo::env::purge(conn, id)).await
}

// ============================================================
// 变量
// ============================================================

#[tauri::command]
pub async fn list_env_vars(
    state: State<'_, AppState>,
    environment_id: i64,
) -> AppResult<Vec<EnvVar>> {
    with_conn(&state.pool, move |conn| {
        repo::env::list_vars(conn, environment_id)
    })
    .await
}

/// 整表替换某个环境的变量。
///
/// 变量表编辑器是整表提交语义（design.md §6.3），所以这里没有「单行增删改」——
/// 少一套 API 就少一套需要保持一致的语义。仓储侧「先删后插」在一个事务里。
#[tauri::command]
pub async fn replace_env_vars(
    state: State<'_, AppState>,
    environment_id: i64,
    vars: Vec<EnvVar>,
) -> AppResult<Vec<EnvVar>> {
    let vars = clean_vars(vars)?;
    with_conn(&state.pool, move |conn| {
        repo::env::replace_vars(conn, environment_id, &vars)
    })
    .await
}

/// 变量名的校验与规范化。
///
/// 空白键直接**丢弃**而不是报错：界面上「加一行」的默认状态就是空行，
/// 为它弹一个错误框纯属打扰。但会把值一起丢掉 —— 一个没有名字的变量
/// 谁也引用不到，留着只是让它出现在下一次的替换里。
fn clean_vars(vars: Vec<EnvVar>) -> AppResult<Vec<EnvVar>> {
    let mut out = Vec::with_capacity(vars.len());

    for var in vars {
        let key = var.key.trim();
        if key.is_empty() {
            continue;
        }
        if key.chars().count() > MAX_VAR_KEY_LEN {
            return Err(AppError::InvalidInput {
                field: "key",
                reason: format!("key is longer than {MAX_VAR_KEY_LEN} characters"),
            });
        }
        out.push(EnvVar {
            // id / sort_order 由仓储按数组下标重排，这里给什么都不作数
            key: key.to_string(),
            ..var
        });
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(key: &str, value: &str) -> EnvVar {
        EnvVar {
            id: 0,
            environment_id: 0,
            key: key.into(),
            value: value.into(),
            is_secret: false,
            enabled: true,
            sort_order: 0,
        }
    }

    #[test]
    fn blank_keys_are_dropped_not_rejected() {
        let out = clean_vars(vec![var("  ", "x"), var(" a ", "1"), var("", "y")]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "a", "键要去空白");
        assert_eq!(out[0].value, "1");
    }

    #[test]
    fn overlong_key_is_rejected() {
        let long = "k".repeat(MAX_VAR_KEY_LEN + 1);
        assert!(clean_vars(vec![var(&long, "1")]).is_err());
        // 边界值放行
        let limit = "k".repeat(MAX_VAR_KEY_LEN);
        assert!(clean_vars(vec![var(&limit, "1")]).is_ok());
    }
}
