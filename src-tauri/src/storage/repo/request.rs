//! 接口请求仓储。
//!
//! 这个仓储比其他的麻烦一点：`params` / `headers` / `auth` / `settings`
//! 四列都是 JSON，读出时要反序列化、写入时要序列化。
//!
//! 因此这里**不用 `query_map`**，改用显式的 `rows.next()` 循环 ——
//! `query_map` 的映射闭包只能返回 `rusqlite::Result`，
//! 而 JSON 解析失败要冒泡成我们自己的错误类型（见 `parse_json_column` 的说明）。

use std::collections::BTreeMap;

use rusqlite::{Connection, Row};
use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::storage::models::{ApiRequest, AuthConfig, BodyType, KeyValueItem, RequestSettings};
use crate::storage::now_iso;
use crate::storage::repo::{opt_text, parse_json_column, to_json_column, ORDER_BY_SORT};

const COLUMNS: &str = "id, folder_id, workspace_id, name, method, url, params, headers, \
     body_type, body, body_by_type, body_rows, auth, settings, pre_script, test_script, \
     sort_order, created_at, updated_at, deleted_at";

fn map_row(row: &Row<'_>) -> AppResult<ApiRequest> {
    let params_raw: String = row.get("params")?;
    let headers_raw: String = row.get("headers")?;
    let auth_raw: String = row.get("auth")?;
    let settings_raw: String = row.get("settings")?;
    let body_type_raw: String = row.get("body_type")?;
    let body_by_type_raw: String = row.get("body_by_type")?;
    let body_rows_raw: String = row.get("body_rows")?;

    Ok(ApiRequest {
        id: row.get("id")?,
        folder_id: row.get("folder_id")?,
        workspace_id: row.get("workspace_id")?,
        name: row.get("name")?,
        method: row.get("method")?,
        url: row.get("url")?,
        params: parse_json_column::<Vec<KeyValueItem>>(&params_raw, "request.params")?,
        headers: parse_json_column::<Vec<KeyValueItem>>(&headers_raw, "request.headers")?,
        body_type: BodyType::parse(&body_type_raw),
        body: row.get("body")?,
        body_by_type: parse_json_column::<BTreeMap<String, String>>(
            &body_by_type_raw,
            "request.body_by_type",
        )?,
        body_rows: parse_json_column::<Vec<KeyValueItem>>(&body_rows_raw, "request.body_rows")?,
        // 宽松解析：存量值是 `{}`（P2 起 auth 一直只做透传），见 AuthConfig 的注释
        auth: AuthConfig::from_json(&parse_json_column::<Value>(&auth_raw, "request.auth")?),
        settings: parse_json_column(&settings_raw, "request.settings")?,
        pre_script: row.get("pre_script")?,
        test_script: row.get("test_script")?,
        sort_order: row.get("sort_order")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        deleted_at: opt_text(row, "deleted_at")?,
    })
}

/// 逐行收集。抽出来是因为本文件每个查询都要走同一套「自建错误类型」的映射。
fn collect(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> AppResult<Vec<ApiRequest>> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query(params)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(map_row(row)?);
    }
    Ok(out)
}

pub fn list_by_workspace(conn: &Connection, workspace_id: i64) -> AppResult<Vec<ApiRequest>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM request WHERE workspace_id = ?1 AND deleted_at IS NULL{ORDER_BY_SORT}"
    );
    collect(conn, &sql, &[&workspace_id])
}

/// 某个文件夹下的存活请求。`folder_id = None` 表示「未分类」。
pub fn list_by_folder(conn: &Connection, folder_id: Option<i64>) -> AppResult<Vec<ApiRequest>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM request WHERE folder_id IS ?1 AND deleted_at IS NULL{ORDER_BY_SORT}"
    );
    collect(conn, &sql, &[&folder_id])
}

pub fn list_deleted(conn: &Connection, workspace_id: i64) -> AppResult<Vec<ApiRequest>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM request
         WHERE workspace_id = ?1 AND deleted_at IS NOT NULL
         ORDER BY deleted_at DESC, id DESC"
    );
    collect(conn, &sql, &[&workspace_id])
}

pub fn get(conn: &Connection, id: i64) -> AppResult<ApiRequest> {
    let sql = format!("SELECT {COLUMNS} FROM request WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([id])?;
    match rows.next()? {
        Some(row) => map_row(row),
        None => Err(AppError::NotFound {
            entity: "request",
            id,
        }),
    }
}

/// 新建请求。除名字外一切走列默认值（method=GET、url=''、bodyType=none…），
/// 保证「新建一个请求」是最省事的一条路径。
pub fn create(
    conn: &Connection,
    workspace_id: i64,
    folder_id: Option<i64>,
    name: &str,
) -> AppResult<ApiRequest> {
    let now = now_iso();
    let next_order: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM request
         WHERE workspace_id = ?1 AND folder_id IS ?2 AND deleted_at IS NULL",
        rusqlite::params![workspace_id, folder_id],
        |r| r.get(0),
    )?;

    conn.execute(
        "INSERT INTO request (workspace_id, folder_id, name, sort_order, settings, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        rusqlite::params![
            workspace_id,
            folder_id,
            name,
            next_order,
            to_json_column(&RequestSettings::default(), "request.settings")?,
            now
        ],
    )?;
    get(conn, conn.last_insert_rowid())
}

/// 全量保存。UI 侧持有整个请求对象，逐字段 patch 反而更容易漏字段。
///
/// `id` / `created_at` 不受调用方影响，其余字段一律以传入值为准。
pub fn save(conn: &Connection, req: &ApiRequest) -> AppResult<ApiRequest> {
    let affected = conn.execute(
        "UPDATE request SET
           folder_id    = ?2,
           name         = ?3,
           method       = ?4,
           url          = ?5,
           params       = ?6,
           headers      = ?7,
           body_type    = ?8,
           body         = ?9,
           body_by_type = ?10,
           body_rows    = ?11,
           auth         = ?12,
           settings     = ?13,
           pre_script   = ?14,
           test_script  = ?15,
           sort_order   = ?16,
           updated_at   = ?17
         WHERE id = ?1",
        rusqlite::params![
            req.id,
            req.folder_id,
            req.name,
            req.method,
            req.url,
            to_json_column(&req.params, "request.params")?,
            to_json_column(&req.headers, "request.headers")?,
            req.body_type.as_db_str(),
            req.body,
            to_json_column(&req.body_by_type, "request.body_by_type")?,
            to_json_column(&req.body_rows, "request.body_rows")?,
            serde_json::to_string(&req.auth)
                .map_err(|e| AppError::Internal(format!("request.auth: {e}")))?,
            to_json_column(&req.settings, "request.settings")?,
            req.pre_script,
            req.test_script,
            req.sort_order,
            now_iso(),
        ],
    )?;

    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "request",
            id: req.id,
        });
    }
    get(conn, req.id)
}

pub fn rename(conn: &Connection, id: i64, name: &str) -> AppResult<ApiRequest> {
    let affected = conn.execute(
        "UPDATE request SET name = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![id, name, now_iso()],
    )?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "request",
            id,
        });
    }
    get(conn, id)
}

/// 复制成一个新请求：内容全抄，落在**同一个文件夹**的末尾。
///
/// `name` 由调用方给（前端用 `t("collections.copySuffix")` 拼），
/// 后端不生成「副本」这类面向用户的文案 —— 名字是数据，文案归前端（见 `crate::error` 的契约）。
///
/// 不复制的东西：`id`（新行）、`created_at` / `updated_at`（新行自己的时间）、
/// `deleted_at`（新行是存活的）、`sort_order`（追加到末尾）。
pub fn duplicate(conn: &Connection, id: i64, name: &str) -> AppResult<ApiRequest> {
    let src = get(conn, id)?;
    let now = now_iso();

    let next_order: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM request
         WHERE workspace_id = ?1 AND folder_id IS ?2 AND deleted_at IS NULL",
        rusqlite::params![src.workspace_id, src.folder_id],
        |r| r.get(0),
    )?;

    conn.execute(
        "INSERT INTO request
           (folder_id, workspace_id, name, method, url, params, headers,
            body_type, body, body_by_type, body_rows,
            auth, settings, pre_script, test_script,
            sort_order, created_at, updated_at)
         SELECT folder_id, workspace_id, ?2, method, url, params, headers,
                body_type, body, body_by_type, body_rows,
                auth, settings, pre_script, test_script,
                ?3, ?4, ?4
         FROM request WHERE id = ?1",
        rusqlite::params![id, name, next_order, now],
    )?;

    get(conn, conn.last_insert_rowid())
}

/// 在分类间移动。`folder_id = None` 表示移出到「未分类」。
pub fn move_to_folder(conn: &Connection, id: i64, folder_id: Option<i64>) -> AppResult<ApiRequest> {
    let req = get(conn, id)?;
    let next_order: i64 = conn.query_row(
        "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM request
         WHERE workspace_id = ?1 AND folder_id IS ?2 AND deleted_at IS NULL",
        rusqlite::params![req.workspace_id, folder_id],
        |r| r.get(0),
    )?;

    conn.execute(
        "UPDATE request SET folder_id = ?2, sort_order = ?3, updated_at = ?4 WHERE id = ?1",
        rusqlite::params![id, folder_id, next_order, now_iso()],
    )?;
    get(conn, id)
}

pub fn soft_delete(conn: &Connection, id: i64) -> AppResult<()> {
    let affected = conn.execute(
        "UPDATE request SET deleted_at = ?2, updated_at = ?2 WHERE id = ?1 AND deleted_at IS NULL",
        rusqlite::params![id, now_iso()],
    )?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "request",
            id,
        });
    }
    Ok(())
}

pub fn restore(conn: &Connection, id: i64) -> AppResult<ApiRequest> {
    let req = get(conn, id)?;
    let now = now_iso();

    // 父文件夹还在回收站的话，还原到「未分类」，避免出现看不见的孤儿
    let folder_id = match req.folder_id {
        Some(fid) => {
            let alive: i64 = conn.query_row(
                "SELECT COUNT(*) FROM folder WHERE id = ?1 AND deleted_at IS NULL",
                [fid],
                |r| r.get(0),
            )?;
            if alive > 0 {
                Some(fid)
            } else {
                None
            }
        }
        None => None,
    };

    conn.execute(
        "UPDATE request SET deleted_at = NULL, folder_id = ?2, updated_at = ?3 WHERE id = ?1",
        rusqlite::params![id, folder_id, now],
    )?;
    get(conn, id)
}

/// 彻底删除
pub fn purge(conn: &Connection, id: i64) -> AppResult<()> {
    let affected = conn.execute("DELETE FROM request WHERE id = ?1", [id])?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "request",
            id,
        });
    }
    Ok(())
}

pub fn count_alive(conn: &Connection, workspace_id: i64) -> AppResult<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM request WHERE workspace_id = ?1 AND deleted_at IS NULL",
        [workspace_id],
        |r| r.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrate;
    use crate::storage::now_iso;

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

    fn folder(conn: &Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO folder (workspace_id, name, created_at, updated_at)
             VALUES (1, ?1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [name],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn create_uses_column_defaults() {
        let conn = db();
        let r = create(&conn, 1, None, "health").unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.url, "");
        assert_eq!(r.body_type, BodyType::None);
        assert!(r.params.is_empty());
        assert!(r.headers.is_empty());
        assert_eq!(r.settings, RequestSettings::default());
        // 建表默认是 `'{}'`，读出来是「不注入授权」而不是报错
        assert_eq!(r.auth, AuthConfig::Inherit);
        // 003 的两列默认是**空快照**：前端读到 `{}` 会用 body 兜底填当前类型
        assert!(r.body_by_type.is_empty());
        assert!(r.body_rows.is_empty());
    }

    #[test]
    fn save_roundtrips_every_json_column() {
        let conn = db();
        let f = folder(&conn, "auth");
        let mut r = create(&conn, 1, Some(f), "login").unwrap();

        r.method = "POST".into();
        r.url = "https://example.com/login".into();
        r.params = vec![KeyValueItem::new("page", "1")];
        r.headers = vec![KeyValueItem::new("Content-Type", "application/json")];
        r.body_type = BodyType::Json;
        r.body = r#"{"user":"a"}"#.into();
        // 分槽快照与键值行：两者都是 JSON 列，必须与 params / headers 一样能原样往返
        r.body_by_type = BTreeMap::from([
            ("json".to_string(), r#"{"user":"a"}"#.to_string()),
            ("raw".to_string(), "plain text".to_string()),
        ]);
        r.body_rows = vec![KeyValueItem::new("grant_type", "password")];
        r.auth = AuthConfig::Bearer {
            token: "{{token.access}}".into(),
        };
        r.settings.timeout_ms = 5_000;
        r.settings.verify_tls = false;
        r.settings.proxy = Some("http://127.0.0.1:7890".into());
        r.pre_script = "console.log(1)".into();

        let saved = save(&conn, &r).unwrap();

        assert_eq!(saved.method, "POST");
        assert_eq!(saved.body_type, BodyType::Json);
        assert_eq!(saved.params, r.params);
        assert_eq!(saved.headers, r.headers);
        assert_eq!(saved.body_by_type, r.body_by_type);
        assert_eq!(saved.body_rows, r.body_rows);
        assert_eq!(saved.auth, r.auth);
        assert_eq!(saved.settings, r.settings);
        assert_eq!(saved.pre_script, "console.log(1)");
    }

    /// 空快照读回来是**空 map / 空 vec**，不是报错 ——
    /// 存量库那条路径（`{}` / `[]`）必须能正常读出请求，否则升级即白屏
    #[test]
    fn empty_body_snapshot_columns_read_back_as_empty() {
        let conn = db();
        let r = create(&conn, 1, None, "legacy").unwrap();
        let again = get(&conn, r.id).unwrap();

        assert!(again.body_by_type.is_empty());
        assert!(again.body_rows.is_empty());
    }

    #[test]
    fn list_separates_foldered_and_unfiled() {
        let conn = db();
        let f = folder(&conn, "auth");
        create(&conn, 1, Some(f), "login").unwrap();
        create(&conn, 1, None, "root-level").unwrap();

        assert_eq!(list_by_folder(&conn, Some(f)).unwrap().len(), 1);
        assert_eq!(list_by_folder(&conn, None).unwrap().len(), 1);
        assert_eq!(list_by_workspace(&conn, 1).unwrap().len(), 2);
        assert_eq!(count_alive(&conn, 1).unwrap(), 2);
    }

    #[test]
    fn soft_delete_then_restore_roundtrip() {
        let conn = db();
        let r = create(&conn, 1, None, "x").unwrap();
        soft_delete(&conn, r.id).unwrap();

        assert!(list_by_workspace(&conn, 1).unwrap().is_empty());
        assert_eq!(list_deleted(&conn, 1).unwrap().len(), 1);
        // 重复软删除应当报 NotFound，而不是静默成功
        assert!(matches!(
            soft_delete(&conn, r.id).unwrap_err(),
            AppError::NotFound { .. }
        ));

        let back = restore(&conn, r.id).unwrap();
        assert!(back.deleted_at.is_none());
        assert_eq!(list_by_workspace(&conn, 1).unwrap().len(), 1);
    }

    /// 父文件夹还在回收站时，还原的请求落到「未分类」
    #[test]
    fn restore_detaches_from_trashed_folder() {
        let conn = db();
        let f = folder(&conn, "auth");
        let r = create(&conn, 1, Some(f), "login").unwrap();

        conn.execute(
            "UPDATE folder SET deleted_at = '2026-01-01T00:00:00.000Z' WHERE id = ?1",
            [f],
        )
        .unwrap();

        let restored = restore(&conn, r.id).unwrap();
        assert_eq!(
            restored.folder_id, None,
            "must not stay under a trashed folder"
        );
    }

    #[test]
    fn move_to_folder_renumbers() {
        let conn = db();
        let a = folder(&conn, "a");
        let b = folder(&conn, "b");
        let r = create(&conn, 1, Some(a), "x").unwrap();

        let moved = move_to_folder(&conn, r.id, Some(b)).unwrap();
        assert_eq!(moved.folder_id, Some(b));
        assert_eq!(moved.sort_order, 0);

        let unfiled = move_to_folder(&conn, r.id, None).unwrap();
        assert_eq!(unfiled.folder_id, None);
    }

    /// 复制：内容全抄、落在同一个文件夹末尾、名字用传入的那个
    #[test]
    fn duplicate_copies_content_into_the_same_folder() {
        let conn = db();
        let f = folder(&conn, "auth");
        let mut src = create(&conn, 1, Some(f), "login").unwrap();
        src.method = "POST".into();
        src.url = "https://example.com/login".into();
        src.params = vec![KeyValueItem::new("page", "1")];
        src.headers = vec![KeyValueItem::new("X-Trace", "1")];
        src.body_type = BodyType::Json;
        src.body = r#"{"u":"a"}"#.into();
        src.body_by_type = BTreeMap::from([("json".to_string(), r#"{"u":"a"}"#.to_string())]);
        src.body_rows = vec![KeyValueItem::new("k", "v")];
        src.settings.timeout_ms = 1_234;
        save(&conn, &src).unwrap();

        // 同文件夹里先放一个别的，验证副本排在**末尾**
        create(&conn, 1, Some(f), "zzz").unwrap();

        let copy = duplicate(&conn, src.id, "login 副本").unwrap();

        assert_ne!(copy.id, src.id);
        assert_eq!(copy.name, "login 副本");
        assert_eq!(copy.folder_id, Some(f));
        assert_eq!(copy.method, "POST");
        assert_eq!(copy.url, "https://example.com/login");
        assert_eq!(copy.params, src.params);
        assert_eq!(copy.headers, src.headers);
        assert_eq!(copy.body_type, BodyType::Json);
        assert_eq!(copy.body, r#"{"u":"a"}"#);
        // 分槽快照与键值行也得跟着抄 —— 漏了的话副本一切正常，
        // 唯独切一次请求体类型就把别的类型的内容丢了，很难往「复制」上想
        assert_eq!(copy.body_by_type, src.body_by_type);
        assert_eq!(copy.body_rows, src.body_rows);
        assert_eq!(copy.settings.timeout_ms, 1_234);
        assert!(copy.deleted_at.is_none(), "副本必须是存活的");
        assert_eq!(copy.sort_order, 2, "追加到该文件夹末尾");
    }

    #[test]
    fn purge_removes_row() {
        let conn = db();
        let r = create(&conn, 1, None, "x").unwrap();
        purge(&conn, r.id).unwrap();
        assert!(matches!(
            get(&conn, r.id).unwrap_err(),
            AppError::NotFound {
                entity: "request",
                ..
            }
        ));
        assert!(matches!(
            purge(&conn, r.id).unwrap_err(),
            AppError::NotFound { .. }
        ));
    }

    /// 时间戳必须带 T 和 Z，否则前端 new Date() 会按本地时区解析
    #[test]
    fn timestamps_are_utc_iso8601() {
        let conn = db();
        let r = create(&conn, 1, None, "x").unwrap();
        assert!(r.created_at.contains('T'), "missing T: {}", r.created_at);
        assert!(r.created_at.ends_with('Z'), "missing Z: {}", r.created_at);
        assert!(!r.created_at.is_empty());
        let _ = now_iso();
    }
}
