//! Token 源与缓存仓储（design.md §4.3）。
//!
//! 注意 `token_source` / `token_cache` 两张表都**没有 `deleted_at`**：
//! Token 配置属于「机器用的配置」而不是「用户的内容」，
//! 进回收站只会让 P4 的刷新逻辑要处理一堆边界情况。删除即硬删。
//!
//! ## 归属：一个接口一份（008 起）
//!
//! `token_source.request_id` 就是归属列，`UNIQUE` 索引保证一个接口至多一份。
//! 缓存行、单飞锁、界面上的倒计时全都按**这一行的 id** 独立 —— 没有"共享组"
//! 这回事。（曾经有过：按配置指纹把多条来源并成一组共用 token 与倒计时，
//! 2026-09-18 撤掉。理由见 `models.rs` 的 `token_shape`。）

use rusqlite::{Connection, Row};

use crate::error::{AppError, AppResult};
use crate::storage::models::{ExpiryUnit, TokenCache, TokenSource};
use crate::storage::now_iso;
use crate::storage::repo::{bool_to_int, opt_text, ORDER_BY_NAME};

const SOURCE_COLUMNS: &str = "id, workspace_id, request_id, name, fetch_request_id, \
     access_path, refresh_path, expires_in_path, expires_in_unit, refresh_request_id, \
     auto_refresh, skew_seconds, created_at, updated_at";

const CACHE_COLUMNS: &str =
    "id, token_source_id, access_token, refresh_token, expires_at, raw_response, updated_at";

fn map_source(row: &Row<'_>) -> rusqlite::Result<TokenSource> {
    let unit_raw: String = row.get("expires_in_unit")?;
    Ok(TokenSource {
        id: row.get("id")?,
        workspace_id: row.get("workspace_id")?,
        request_id: row.get("request_id")?,
        name: row.get("name")?,
        fetch_request_id: row.get("fetch_request_id")?,
        access_path: row.get("access_path")?,
        refresh_path: row.get("refresh_path")?,
        expires_in_path: row.get("expires_in_path")?,
        expires_in_unit: ExpiryUnit::parse(&unit_raw),
        refresh_request_id: row.get("refresh_request_id")?,
        auto_refresh: row.get("auto_refresh")?,
        skew_seconds: row.get("skew_seconds")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn map_cache(row: &Row<'_>) -> rusqlite::Result<TokenCache> {
    Ok(TokenCache {
        id: row.get("id")?,
        token_source_id: row.get("token_source_id")?,
        access_token: row.get("access_token")?,
        refresh_token: row.get("refresh_token")?,
        expires_at: opt_text(row, "expires_at")?,
        raw_response: opt_text(row, "raw_response")?,
        updated_at: row.get("updated_at")?,
    })
}

/// **某个接口**的 Token 源。`None` = 这个接口没配（大多数接口就是这样）。
///
/// 这是运行时的唯一入口：发送时拿的就是这条请求自己的配置，
/// 不再沿分类链向上找、也没有工作区级的兜底。
pub fn by_request(conn: &Connection, request_id: i64) -> AppResult<Option<TokenSource>> {
    let sql = format!("SELECT {SOURCE_COLUMNS} FROM token_source WHERE request_id = ?1");
    match conn.query_row(&sql, [request_id], map_source) {
        Ok(source) => Ok(Some(source)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn list_by_workspace(conn: &Connection, workspace_id: i64) -> AppResult<Vec<TokenSource>> {
    let sql =
        format!("SELECT {SOURCE_COLUMNS} FROM token_source WHERE workspace_id = ?1{ORDER_BY_NAME}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([workspace_id], map_source)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn get(conn: &Connection, id: i64) -> AppResult<TokenSource> {
    let sql = format!("SELECT {SOURCE_COLUMNS} FROM token_source WHERE id = ?1");
    conn.query_row(&sql, [id], map_source).map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => AppError::NotFound {
            entity: "token_source",
            id,
        },
        other => AppError::Db(other),
    })
}

/// 给某个接口建一份 Token 配置。
///
/// ⚠️ 调用方**必须**紧接着 `save` 一次全量字段：这条只写「归属 + 名字 + 取 token
/// 的请求」，提取路径与提前量落到列默认值上。写窄 INSERT 而不是收一堆参数，
/// 是因为 `save` 早就在往同一张表写同一批字段 —— 两条路径维护同一份数据，
/// 迟早会有一份忘了改（`store/token.ts::create` 里有这段历史）。
pub fn create(
    conn: &Connection,
    workspace_id: i64,
    request_id: i64,
    name: &str,
    fetch_request_id: i64,
) -> AppResult<TokenSource> {
    let now = now_iso();
    conn.execute(
        "INSERT INTO token_source
           (workspace_id, request_id, name, fetch_request_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        rusqlite::params![workspace_id, request_id, name, fetch_request_id, now],
    )?;
    get(conn, conn.last_insert_rowid())
}

/// 全量保存。
pub fn save(conn: &Connection, source: &TokenSource) -> AppResult<TokenSource> {
    // 配置变了 ⇒ 自己名下那行缓存立刻作废。
    //
    // 理由只有一个但足够：**旧 token 的来历说不清了** —— 改了 `access_path`
    // 之后那份 token 是从另一个字段里提的，改了 `fetch_request_id` 之后它压根是
    // 另一个接口发的。接着用等于拿一份对不上配置的凭据去发请求。
    //
    // ⚠️ 只在**取值语义**变化时清。改个名字就丢掉 token，代价是用户莫名其妙
    // 多打一次取 token 请求 —— 而名字根本不影响 token 是什么。
    let before = get(conn, source.id)?;
    let shape_changed = before.token_shape() != source.token_shape();

    let affected = conn.execute(
        "UPDATE token_source SET
           request_id         = ?2,
           name               = ?3,
           fetch_request_id   = ?4,
           access_path        = ?5,
           refresh_path       = ?6,
           expires_in_path    = ?7,
           expires_in_unit    = ?8,
           refresh_request_id = ?9,
           auto_refresh       = ?10,
           skew_seconds       = ?11,
           updated_at         = ?12
         WHERE id = ?1",
        rusqlite::params![
            source.id,
            source.request_id,
            source.name,
            source.fetch_request_id,
            source.access_path,
            source.refresh_path,
            source.expires_in_path,
            source.expires_in_unit.as_db_str(),
            source.refresh_request_id,
            bool_to_int(source.auto_refresh),
            source.skew_seconds,
            now_iso(),
        ],
    )?;

    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "token_source",
            id: source.id,
        });
    }

    if shape_changed {
        clear_cache(conn, source.id)?;
    }

    get(conn, source.id)
}

/// 硬删除。缓存行由 `token_cache` 的外键级联带走。
pub fn delete(conn: &Connection, id: i64) -> AppResult<()> {
    let affected = conn.execute("DELETE FROM token_source WHERE id = ?1", [id])?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "token_source",
            id,
        });
    }
    Ok(())
}

// ============================================================
// 缓存
// ============================================================

/// 读某个 Token 源的缓存。
///
/// 缓存与来源**一一对应**（`idx_token_cache_source` 上的唯一索引），
/// 所以参数就叫 `token_source_id` —— 这里没有任何"代谁读"的间接层。
pub fn get_cache(conn: &Connection, token_source_id: i64) -> AppResult<Option<TokenCache>> {
    let sql = format!("SELECT {CACHE_COLUMNS} FROM token_cache WHERE token_source_id = ?1");
    match conn.query_row(&sql, [token_source_id], map_cache) {
        Ok(c) => Ok(Some(c)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// 写入 / 覆盖某个 Token 源的缓存（刷新即 upsert）。
pub fn upsert_cache(
    conn: &Connection,
    token_source_id: i64,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: Option<&str>,
    raw_response: Option<&str>,
) -> AppResult<TokenCache> {
    conn.execute(
        "INSERT INTO token_cache
           (token_source_id, access_token, refresh_token, expires_at, raw_response, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(token_source_id) DO UPDATE SET
           access_token  = excluded.access_token,
           refresh_token = excluded.refresh_token,
           expires_at    = excluded.expires_at,
           raw_response  = excluded.raw_response,
           updated_at    = excluded.updated_at",
        rusqlite::params![
            token_source_id,
            access_token,
            refresh_token,
            expires_at,
            raw_response,
            now_iso(),
        ],
    )?;

    get_cache(conn, token_source_id)?.ok_or_else(|| {
        AppError::Internal(format!(
            "token_cache row vanished after upsert (source #{token_source_id})"
        ))
    })
}

/// 清掉某个 Token 源的缓存。**只用于「配置变了」与「删来源」两条路径**，
/// 两条都是"这份 token 已经不该被读到了"。
pub fn clear_cache(conn: &Connection, token_source_id: i64) -> AppResult<bool> {
    let affected = conn.execute(
        "DELETE FROM token_cache WHERE token_source_id = ?1",
        [token_source_id],
    )?;
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrate;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().expect("db");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate::run(&mut conn).expect("migrate");
        conn.execute_batch(
            "INSERT INTO workspace (id, name, created_at, updated_at)
               VALUES (1, 'w', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO folder (id, workspace_id, name, created_at, updated_at)
               VALUES (1, 1, 'TS', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO request (id, folder_id, workspace_id, name, created_at, updated_at)
               VALUES (1, 1, 1, 'login', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                      (2, 1, 1, 'me',    '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                      (3, 1, 1, 'login', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');",
        )
        .unwrap();
        conn
    }

    fn cache_rows(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM token_cache", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn create_uses_design_defaults() {
        let conn = db();
        let s = create(&conn, 1, 1, "iPSA-OAuth", 3).unwrap();
        assert_eq!(s.request_id, 1);
        assert_eq!(s.access_path, "$.access_token");
        assert_eq!(s.refresh_path, "$.refresh_token");
        assert_eq!(s.expires_in_path, "$.expires_in");
        assert_eq!(s.expires_in_unit, ExpiryUnit::Seconds);
        assert_eq!(s.skew_seconds, 30);
        assert!(s.auto_refresh);
        assert_eq!(s.refresh_request_id, None);
    }

    /// 一个接口至多一份配置 —— 这才是 008 换来的硬约束。
    /// 008 之前这里钉的是 UNIQUE(folder_id, name)：同一个分类下一份，
    /// 于是全工作区只能有一份 Token 配置。
    #[test]
    fn a_request_can_only_have_one_source() {
        let conn = db();
        create(&conn, 1, 1, "a", 3).unwrap();

        let dup = create(&conn, 1, 1, "另一个名字", 3);
        assert!(dup.is_err(), "同一个接口不该允许第二份 Token 配置");
    }

    /// 反过来：**不同接口同名**完全正常（这在 008 之前会撞唯一键）。
    #[test]
    fn different_requests_may_share_a_name() {
        let conn = db();
        let a = create(&conn, 1, 1, "login", 3).unwrap();
        let b = create(&conn, 1, 3, "login", 3).unwrap();
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn by_request_only_finds_that_requests_own_source() {
        let conn = db();
        let a = create(&conn, 1, 1, "a", 3).unwrap();
        create(&conn, 1, 2, "b", 3).unwrap();

        assert_eq!(by_request(&conn, 1).unwrap().unwrap().id, a.id);
        assert!(by_request(&conn, 2).unwrap().is_some());
        assert!(by_request(&conn, 42).unwrap().is_none());
    }

    #[test]
    fn save_updates_every_configurable_field() {
        let conn = db();
        let mut s = create(&conn, 1, 1, "oauth", 3).unwrap();
        s.access_path = "$.data.token".into();
        s.expires_in_unit = ExpiryUnit::Millis;
        s.skew_seconds = 120;
        s.auto_refresh = false;

        let saved = save(&conn, &s).unwrap();
        assert_eq!(saved.access_path, "$.data.token");
        assert_eq!(saved.expires_in_unit, ExpiryUnit::Millis);
        assert_eq!(saved.skew_seconds, 120);
        assert!(!saved.auto_refresh);
    }

    #[test]
    fn cache_upsert_keeps_a_single_row() {
        let conn = db();
        let s = create(&conn, 1, 1, "oauth", 3).unwrap();

        upsert_cache(
            &conn,
            s.id,
            "token-a",
            Some("r1"),
            Some("2026-01-01T00:00:00.000Z"),
            None,
        )
        .unwrap();
        let updated =
            upsert_cache(&conn, s.id, "token-b", None, None, Some("{\"raw\":true}")).unwrap();

        assert_eq!(updated.access_token, "token-b");
        assert_eq!(
            updated.refresh_token, None,
            "refresh token is overwritten, not merged"
        );
        assert_eq!(updated.expires_at, None);
        assert_eq!(updated.raw_response.as_deref(), Some("{\"raw\":true}"));
        assert_eq!(cache_rows(&conn), 1);
    }

    /// 🔴 **两个接口配着一模一样的取 token 配置，也各存各的缓存。**
    ///
    /// 这条钉的是 2026-09-18 撤掉的那套机制：曾经配置指纹相同的来源会被并成一组、
    /// 共用一行缓存与一个倒计时（`share_key()` + `cache_owner()`）。现在的规则是
    /// 「每个接口各自一份」，所以两条逐字段相同的配置必须落在**两行**上 ——
    /// 否则两个接口的倒计时会一起跳，而那是明确不要的行为。
    #[test]
    fn identical_configs_on_two_requests_still_keep_separate_cache_rows() {
        let conn = db();
        // 建两条来源，除了归属之外逐字段相同：`create` 只吃「归属 + 名字 + 取 token
        // 请求」，其余落到列默认值 —— 这正是"配置相同"的最小例子
        let a = create(&conn, 1, 1, "a", 3).unwrap();
        let b = create(&conn, 1, 2, "b", 3).unwrap();
        assert_eq!(
            a.token_shape(),
            b.token_shape(),
            "先确认这是两条配置逐字段相同的来源"
        );

        upsert_cache(&conn, a.id, "ta", None, None, None).unwrap();
        upsert_cache(&conn, b.id, "tb", None, None, None).unwrap();

        assert_eq!(cache_rows(&conn), 2, "一个接口一行，不合并");
        assert_eq!(get_cache(&conn, a.id).unwrap().unwrap().access_token, "ta");
        assert_eq!(get_cache(&conn, b.id).unwrap().unwrap().access_token, "tb");
    }

    /// 配置变了 ⇒ 自己名下那行缓存立刻作废：旧 token 是从另一个字段里提的，
    /// 接着用等于拿一份对不上配置的凭据去发请求。
    #[test]
    fn changing_the_config_drops_the_cache_row() {
        let conn = db();
        let mut a = create(&conn, 1, 1, "a", 3).unwrap();
        upsert_cache(&conn, a.id, "stale", None, None, None).unwrap();
        assert!(get_cache(&conn, a.id).unwrap().is_some());

        a.access_path = "$.other".into();
        save(&conn, &a).unwrap();

        assert!(
            get_cache(&conn, a.id).unwrap().is_none(),
            "换了解析路径，旧 token 必须丢掉"
        );
    }

    /// 换取 token 的接口同理 —— 那份 token 压根是另一个接口发的。
    #[test]
    fn swapping_the_fetch_request_drops_the_cache_row() {
        let conn = db();
        let mut a = create(&conn, 1, 1, "a", 3).unwrap();
        upsert_cache(&conn, a.id, "stale", None, None, None).unwrap();

        a.fetch_request_id = 2;
        save(&conn, &a).unwrap();

        assert!(get_cache(&conn, a.id).unwrap().is_none());
    }

    /// 对照组：只改名字 / 提前量**不该**丢 token —— 它们不影响 token 是什么，
    /// 白白多打一次取 token 请求是用户看得见的代价。
    #[test]
    fn renaming_keeps_the_cache_row() {
        let conn = db();
        let mut a = create(&conn, 1, 1, "a", 3).unwrap();
        upsert_cache(&conn, a.id, "keep", None, None, None).unwrap();

        a.name = "new name".into();
        a.skew_seconds = 90;
        a.auto_refresh = false;
        save(&conn, &a).unwrap();

        let kept = get_cache(&conn, a.id).unwrap().expect("改名字不该丢缓存");
        assert_eq!(kept.access_token, "keep");
    }

    /// 清缓存只清这一条来源的 —— 另一个接口的那份不该被误伤。
    #[test]
    fn clearing_one_source_leaves_the_others_alone() {
        let conn = db();
        let a = create(&conn, 1, 1, "a", 3).unwrap();
        let b = create(&conn, 1, 2, "b", 3).unwrap();
        upsert_cache(&conn, a.id, "ta", None, None, None).unwrap();
        upsert_cache(&conn, b.id, "tb", None, None, None).unwrap();

        assert!(clear_cache(&conn, a.id).unwrap());
        assert!(get_cache(&conn, a.id).unwrap().is_none());
        assert_eq!(get_cache(&conn, b.id).unwrap().unwrap().access_token, "tb");
    }

    #[test]
    fn delete_source_cascades_to_cache() {
        let conn = db();
        let s = create(&conn, 1, 1, "oauth", 3).unwrap();
        upsert_cache(&conn, s.id, "t", None, None, None).unwrap();

        delete(&conn, s.id).unwrap();
        assert_eq!(cache_rows(&conn), 0);
    }

    #[test]
    fn missing_source_reports_not_found() {
        let conn = db();
        assert!(matches!(
            get(&conn, 42).unwrap_err(),
            AppError::NotFound {
                entity: "token_source",
                ..
            }
        ));
        assert!(matches!(
            delete(&conn, 42).unwrap_err(),
            AppError::NotFound { .. }
        ));
    }
}
