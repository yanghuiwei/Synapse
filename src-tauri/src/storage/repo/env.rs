//! 环境与变量仓储。
//!
//! 两处需要事务的地方：
//! * [`set_active`] —— 「同一工作区只有一个激活环境」是跨行约束，
//!   必须在一个事务里先全部置 0 再置 1，否则中途失败会留下两个激活环境。
//! * [`replace_vars`] —— 变量表编辑器是整表提交语义，先删后插要在同一事务里。
//!
//! [`merge_vars`] 是个例外：它**故意不带事务**，因为调用方（导入命令）
//! 要把建环境、写变量、建请求收在同一个事务里。

use rusqlite::{Connection, Row};

use crate::error::{AppError, AppResult};
use crate::storage::models::{EnvVar, Environment};
use crate::storage::now_iso;
use crate::storage::repo::{bool_to_int, opt_text, ORDER_BY_NAME, ORDER_BY_SORT};

const ENV_COLUMNS: &str = "id, workspace_id, name, is_active, created_at, updated_at, deleted_at";
const VAR_COLUMNS: &str = "id, environment_id, key, value, is_secret, enabled, sort_order";

fn map_env(row: &Row<'_>) -> rusqlite::Result<Environment> {
    Ok(Environment {
        id: row.get("id")?,
        workspace_id: row.get("workspace_id")?,
        name: row.get("name")?,
        is_active: row.get("is_active")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        deleted_at: opt_text(row, "deleted_at")?,
    })
}

fn map_var(row: &Row<'_>) -> rusqlite::Result<EnvVar> {
    Ok(EnvVar {
        id: row.get("id")?,
        environment_id: row.get("environment_id")?,
        key: row.get("key")?,
        value: row.get("value")?,
        is_secret: row.get("is_secret")?,
        enabled: row.get("enabled")?,
        sort_order: row.get("sort_order")?,
    })
}

// ============================================================
// 环境
// ============================================================

pub fn list_by_workspace(conn: &Connection, workspace_id: i64) -> AppResult<Vec<Environment>> {
    // environment 表没有 sort_order 列（design.md §3.2），按名称排序
    let sql = format!(
        "SELECT {ENV_COLUMNS} FROM environment
         WHERE workspace_id = ?1 AND deleted_at IS NULL{ORDER_BY_NAME}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([workspace_id], map_env)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn list_deleted(conn: &Connection, workspace_id: i64) -> AppResult<Vec<Environment>> {
    let sql = format!(
        "SELECT {ENV_COLUMNS} FROM environment
         WHERE workspace_id = ?1 AND deleted_at IS NOT NULL
         ORDER BY deleted_at DESC, id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([workspace_id], map_env)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn get(conn: &Connection, id: i64) -> AppResult<Environment> {
    let sql = format!("SELECT {ENV_COLUMNS} FROM environment WHERE id = ?1");
    conn.query_row(&sql, [id], map_env).map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::NotFound {
            entity: "environment",
            id,
        },
        other => AppError::Db(other),
    })
}

/// 当前激活的环境。没有激活项时返回 `None`。
pub fn get_active(conn: &Connection, workspace_id: i64) -> AppResult<Option<Environment>> {
    let sql = format!(
        "SELECT {ENV_COLUMNS} FROM environment
         WHERE workspace_id = ?1 AND is_active = 1 AND deleted_at IS NULL
         LIMIT 1"
    );
    match conn.query_row(&sql, [workspace_id], map_env) {
        Ok(e) => Ok(Some(e)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// 当前激活环境的变量表：键 → 值，**只含 `enabled = 1` 的**。
///
/// 停用（`enabled = 0`）的变量必须在这里就被滤掉，而不是留给解析器 ——
/// 否则 `{{env.x}}` 会解析成功，用户却以为它已经停用了。
/// 没有激活环境时返回空表（不是错误：`{{env.x}}` 会照常报「变量不存在」）。
pub fn active_vars(
    conn: &Connection,
    workspace_id: i64,
) -> AppResult<std::collections::HashMap<String, String>> {
    let Some(active) = get_active(conn, workspace_id)? else {
        return Ok(std::collections::HashMap::new());
    };

    let mut vars = std::collections::HashMap::new();
    for var in list_vars(conn, active.id)? {
        if var.enabled && !var.key.trim().is_empty() {
            vars.insert(var.key, var.value);
        }
    }
    Ok(vars)
}

pub fn create(conn: &Connection, workspace_id: i64, name: &str) -> AppResult<Environment> {
    let now = now_iso();
    conn.execute(
        "INSERT INTO environment (workspace_id, name, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3)",
        rusqlite::params![workspace_id, name, now],
    )?;
    get(conn, conn.last_insert_rowid())
}

pub fn rename(conn: &Connection, id: i64, name: &str) -> AppResult<Environment> {
    let affected = conn.execute(
        "UPDATE environment SET name = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![id, name, now_iso()],
    )?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "environment",
            id,
        });
    }
    get(conn, id)
}

/// 设置激活环境。`id = None` 表示取消激活（回到「不使用环境」）。
pub fn set_active(conn: &mut Connection, workspace_id: i64, id: Option<i64>) -> AppResult<()> {
    let now = now_iso();
    let tx = conn.transaction()?;

    tx.execute(
        "UPDATE environment SET is_active = 0, updated_at = ?2
         WHERE workspace_id = ?1 AND is_active = 1",
        rusqlite::params![workspace_id, now],
    )?;

    if let Some(env_id) = id {
        let affected = tx.execute(
            "UPDATE environment SET is_active = 1, updated_at = ?2
             WHERE id = ?1 AND workspace_id = ?3 AND deleted_at IS NULL",
            rusqlite::params![env_id, now, workspace_id],
        )?;
        if affected == 0 {
            // 事务在这里回滚，不会留下「全部取消激活」的中间态
            return Err(AppError::NotFound {
                entity: "environment",
                id: env_id,
            });
        }
    }

    tx.commit()?;
    Ok(())
}

pub fn soft_delete(conn: &Connection, id: i64) -> AppResult<()> {
    let affected = conn.execute(
        "UPDATE environment SET deleted_at = ?2, is_active = 0, updated_at = ?2
         WHERE id = ?1 AND deleted_at IS NULL",
        rusqlite::params![id, now_iso()],
    )?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "environment",
            id,
        });
    }
    Ok(())
}

pub fn restore(conn: &Connection, id: i64) -> AppResult<Environment> {
    let affected = conn.execute(
        "UPDATE environment SET deleted_at = NULL, updated_at = ?2 WHERE id = ?1",
        rusqlite::params![id, now_iso()],
    )?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "environment",
            id,
        });
    }
    get(conn, id)
}

pub fn purge(conn: &Connection, id: i64) -> AppResult<()> {
    let affected = conn.execute("DELETE FROM environment WHERE id = ?1", [id])?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "environment",
            id,
        });
    }
    Ok(())
}

// ============================================================
// 变量
// ============================================================

pub fn list_vars(conn: &Connection, environment_id: i64) -> AppResult<Vec<EnvVar>> {
    let sql = format!("SELECT {VAR_COLUMNS} FROM env_var WHERE environment_id = ?1{ORDER_BY_SORT}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([environment_id], map_var)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 整表替换某个环境的变量。
///
/// 用「先删后插」而不是逐行 diff：变量表的规模是几十行，
/// 而且 UI 是整表提交的，diff 带来的复杂度换不到什么。
pub fn replace_vars(
    conn: &mut Connection,
    environment_id: i64,
    vars: &[EnvVar],
) -> AppResult<Vec<EnvVar>> {
    let tx = conn.transaction()?;
    ensure_env(&tx, environment_id)?;
    clear_vars(&tx, environment_id)?;
    insert_vars(&tx, environment_id, vars)?;
    tx.commit()?;
    list_vars(conn, environment_id)
}

/// 合并变量：同名的用传入值改写，库里没有的追加到末尾，**库里其余的保留**。
///
/// 与 [`replace_vars`] 的差别是**覆盖范围**而不是写法：那个是整表替换
/// （变量编辑器整表提交的语义），这个是给导入用的 ——
/// 导入一份集合不该把用户自己往上加的变量一起冲掉。
///
/// 刻意**不开事务**：调用方（导入命令）要把「建环境 + 写变量 + 建请求」
/// 收在同一个事务里，这里再开一个就成了嵌套事务，回滚边界会变得说不清。
pub fn merge_vars(
    conn: &Connection,
    environment_id: i64,
    incoming: &[EnvVar],
) -> AppResult<Vec<EnvVar>> {
    ensure_env(conn, environment_id)?;

    let mut merged = list_vars(conn, environment_id)?;
    for var in incoming {
        match merged.iter_mut().find(|slot| slot.key == var.key) {
            Some(slot) => {
                slot.value = var.value.clone();
                slot.is_secret = var.is_secret;
            }
            None => merged.push(var.clone()),
        }
    }

    clear_vars(conn, environment_id)?;
    insert_vars(conn, environment_id, &merged)?;
    list_vars(conn, environment_id)
}

fn ensure_env(conn: &Connection, environment_id: i64) -> AppResult<()> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM environment WHERE id = ?1",
        [environment_id],
        |r| r.get(0),
    )?;
    if exists == 0 {
        return Err(AppError::NotFound {
            entity: "environment",
            id: environment_id,
        });
    }
    Ok(())
}

fn clear_vars(conn: &Connection, environment_id: i64) -> AppResult<()> {
    conn.execute(
        "DELETE FROM env_var WHERE environment_id = ?1",
        [environment_id],
    )?;
    Ok(())
}

fn insert_vars(conn: &Connection, environment_id: i64, vars: &[EnvVar]) -> AppResult<()> {
    for (index, var) in vars.iter().enumerate() {
        conn.execute(
            "INSERT INTO env_var (environment_id, key, value, is_secret, enabled, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                environment_id,
                var.key,
                var.value,
                bool_to_int(var.is_secret),
                bool_to_int(var.enabled),
                index as i64,
            ],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrate;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().expect("db");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate::run(&mut conn).expect("migrate");
        conn.execute(
            "INSERT INTO workspace (id, name, created_at, updated_at)
             VALUES (1, 'w', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        )
        .unwrap();
        conn
    }

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
    fn create_and_list_environments() {
        let conn = db();
        create(&conn, 1, "dev").unwrap();
        create(&conn, 1, "prod").unwrap();
        assert_eq!(list_by_workspace(&conn, 1).unwrap().len(), 2);
        assert!(get_active(&conn, 1).unwrap().is_none());
    }

    #[test]
    fn set_active_is_exclusive() {
        let mut conn = db();
        let dev = create(&conn, 1, "dev").unwrap();
        let prod = create(&conn, 1, "prod").unwrap();

        set_active(&mut conn, 1, Some(dev.id)).unwrap();
        assert_eq!(get_active(&conn, 1).unwrap().unwrap().id, dev.id);

        set_active(&mut conn, 1, Some(prod.id)).unwrap();
        assert_eq!(get_active(&conn, 1).unwrap().unwrap().id, prod.id);

        let active_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM environment WHERE is_active = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active_rows, 1, "exactly one environment may be active");

        set_active(&mut conn, 1, None).unwrap();
        assert!(get_active(&conn, 1).unwrap().is_none());
    }

    /// set_active 指向不存在 / 已删除的环境时，不能留下「全都不是激活」的中间态
    #[test]
    fn failed_set_active_rolls_back() {
        let mut conn = db();
        let dev = create(&conn, 1, "dev").unwrap();
        set_active(&mut conn, 1, Some(dev.id)).unwrap();

        let err = set_active(&mut conn, 1, Some(9999)).unwrap_err();
        assert!(matches!(err, AppError::NotFound { .. }));

        assert_eq!(
            get_active(&conn, 1).unwrap().unwrap().id,
            dev.id,
            "previous active must survive a failed switch"
        );
    }

    #[test]
    fn soft_delete_deactivates_and_can_be_restored() {
        let mut conn = db();
        let dev = create(&conn, 1, "dev").unwrap();
        set_active(&mut conn, 1, Some(dev.id)).unwrap();

        soft_delete(&conn, dev.id).unwrap();
        assert!(list_by_workspace(&conn, 1).unwrap().is_empty());
        assert_eq!(list_deleted(&conn, 1).unwrap().len(), 1);
        assert!(
            get_active(&conn, 1).unwrap().is_none(),
            "trashed env must not stay active"
        );

        restore(&conn, dev.id).unwrap();
        assert_eq!(list_by_workspace(&conn, 1).unwrap().len(), 1);
        assert!(
            !get(&conn, dev.id).unwrap().is_active,
            "restore does not re-activate"
        );
    }

    #[test]
    fn replace_vars_is_full_replacement_and_reorders() {
        let mut conn = db();
        let dev = create(&conn, 1, "dev").unwrap();

        replace_vars(&mut conn, dev.id, &[var("a", "1"), var("b", "2")]).unwrap();
        let after = replace_vars(&mut conn, dev.id, &[var("b", "3")]).unwrap();

        assert_eq!(after.len(), 1, "old rows must be gone");
        assert_eq!(after[0].key, "b");
        assert_eq!(after[0].value, "3");
        assert_eq!(after[0].sort_order, 0, "order follows the input array");
    }

    /// 变量是 environment 的级联子表
    #[test]
    fn purge_environment_cascades_to_vars() {
        let mut conn = db();
        let dev = create(&conn, 1, "dev").unwrap();
        replace_vars(&mut conn, dev.id, &[var("a", "1")]).unwrap();

        purge(&conn, dev.id).unwrap();
        let vars: i64 = conn
            .query_row("SELECT COUNT(*) FROM env_var", [], |r| r.get(0))
            .unwrap();
        assert_eq!(vars, 0);
    }

    #[test]
    fn replace_vars_rejects_unknown_environment() {
        let mut conn = db();
        assert!(matches!(
            replace_vars(&mut conn, 999, &[var("a", "1")]).unwrap_err(),
            AppError::NotFound {
                entity: "environment",
                ..
            }
        ));
    }

    #[test]
    fn secret_and_enabled_flags_survive_roundtrip() {
        let mut conn = db();
        let dev = create(&conn, 1, "dev").unwrap();

        let mut secret = var("token", "s3cr3t");
        secret.is_secret = true;
        secret.enabled = false;

        let saved = replace_vars(&mut conn, dev.id, &[secret.clone()]).unwrap();
        assert!(saved[0].is_secret);
        assert!(!saved[0].enabled);
    }
}
