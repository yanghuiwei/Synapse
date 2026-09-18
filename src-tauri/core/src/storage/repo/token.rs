//! Token 源与缓存仓储（design.md §4.3）。
//!
//! 注意 `token_source` / `token_cache` 两张表都**没有 `deleted_at`**：
//! Token 配置属于「机器用的配置」而不是「用户的内容」，
//! 进回收站只会让 P4 的刷新逻辑要处理一堆边界情况。删除即硬删。

use rusqlite::{Connection, Row};

use crate::error::{AppError, AppResult};
use crate::storage::models::{ExpiryUnit, TokenCache, TokenSource};
use crate::storage::now_iso;
use crate::storage::repo::{bool_to_int, opt_text, ORDER_BY_NAME};

const SOURCE_COLUMNS: &str = "id, workspace_id, folder_id, name, fetch_request_id, \
     access_path, refresh_path, expires_in_path, expires_in_unit, refresh_request_id, \
     auto_refresh, skew_seconds, created_at, updated_at";

const CACHE_COLUMNS: &str =
    "id, token_source_id, access_token, refresh_token, expires_at, raw_response, updated_at";

fn map_source(row: &Row<'_>) -> rusqlite::Result<TokenSource> {
    let unit_raw: String = row.get("expires_in_unit")?;
    Ok(TokenSource {
        id: row.get("id")?,
        workspace_id: row.get("workspace_id")?,
        folder_id: row.get("folder_id")?,
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

fn collect_sources(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> AppResult<Vec<TokenSource>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, map_source)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn list_by_workspace(conn: &Connection, workspace_id: i64) -> AppResult<Vec<TokenSource>> {
    let sql =
        format!("SELECT {SOURCE_COLUMNS} FROM token_source WHERE workspace_id = ?1{ORDER_BY_NAME}");
    collect_sources(conn, &sql, &[&workspace_id])
}

pub fn list_by_folder(conn: &Connection, folder_id: i64) -> AppResult<Vec<TokenSource>> {
    let sql =
        format!("SELECT {SOURCE_COLUMNS} FROM token_source WHERE folder_id = ?1{ORDER_BY_NAME}");
    collect_sources(conn, &sql, &[&folder_id])
}

/// 沿「最近的在前」的文件夹链找第一个 Token 源。
///
/// 判定依据是 `token_source` 表里**真的有没有行**，而不是文件夹上的
/// `is_token_source` 标记 —— 那个标记只是 UI 的入口引导（建 Token 源时顺手打上），
/// 拿它当硬条件会让「先建源、后打标记」这条顺序把源变成隐形的。
pub fn first_in_chain(conn: &Connection, chain: &[i64]) -> AppResult<Option<TokenSource>> {
    for folder_id in chain {
        if let Some(source) = list_by_folder(conn, *folder_id)?.into_iter().next() {
            return Ok(Some(source));
        }
    }
    Ok(None)
}

/// 工作区的**全局** Token 源 —— 挂在系统分类（「token刷新」）下的那一个。
///
/// 用途是 `first_in_chain` 之后的**兜底**：Token 来源固定配在系统分类下，
/// 而绝大多数请求并不在这个分类里（甚至没挂任何分类），
/// 只沿分类链找的话 `{{token.access}}` 会永远解析不出来。
///
/// 语义上是「链上没配就用全局那一个」——链上配了仍然以链上为准，
/// 所以老的「按分类配 Token」用法完全不受影响。
pub fn system_source(conn: &Connection) -> AppResult<Option<TokenSource>> {
    let Some(workspace) = crate::storage::repo::workspace::active(conn)? else {
        return Ok(None);
    };
    let Some(folder) = crate::storage::repo::folder::system_folder(conn, workspace.id)? else {
        return Ok(None);
    };
    Ok(list_by_folder(conn, folder.id)?.into_iter().next())
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

pub fn create(
    conn: &Connection,
    workspace_id: i64,
    folder_id: i64,
    name: &str,
    fetch_request_id: i64,
) -> AppResult<TokenSource> {
    let now = now_iso();
    conn.execute(
        "INSERT INTO token_source (workspace_id, folder_id, name, fetch_request_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        rusqlite::params![workspace_id, folder_id, name, fetch_request_id, now],
    )?;
    get(conn, conn.last_insert_rowid())
}

/// 全量保存。
///
/// `folder_id` 要指向被标记为 `is_token_source` 的文件夹 ——
/// 这里**只校验文件夹存在**，不校验那个标记：标记是 UI 层的引导，
/// 不该做成数据库层的硬约束（否则先建 Token 源再打标记的流程会失败）。
pub fn save(conn: &Connection, source: &TokenSource) -> AppResult<TokenSource> {
    // 指纹变了 ⇒ 自己名下那行缓存立刻作废。
    //
    // 两个理由，缺一个都不够：
    // 1. **旧 token 的来历说不清了** —— 改了 `access_path` 之后那份 token 是从
    //    另一个字段里提的，接着用等于拿一份对不上配置的凭据去发请求。
    // 2. 它可能正是原来那一组的**代表行**。配置一改它就不属于那一组了，
    //    那行缓存会变成谁也不读的孤儿：占着库，还会在未来某次「改回原配置」
    //    时被当成有效缓存读出来。
    //
    // ⚠️ 只在**指纹**变化时清。改个名字就丢掉 token，代价是用户莫名其妙多打
    // 一次取 token 请求 —— 而名字根本不影响 token 是什么。
    let before = get(conn, source.id)?;
    let key_changed = before.share_key() != source.share_key();

    let affected = conn.execute(
        "UPDATE token_source SET
           folder_id          = ?2,
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
            source.folder_id,
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

    if key_changed {
        // ⚠️ 这里刻意传 `source.id` 而**不是** `cache_owner(...)`：
        // 要清的是「这条来源名下那一行」——它可能正是原组的代表行（那就等于
        // 把整组那份清掉，正确），也可能什么也没有（因为它本来就不是代表，
        // 缓存挂在别人名下 —— 那种情况下**不该**去清，那条缓存对新配置无关，
        // 对原组却仍然有效）。
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

/// 这份来源的 token 缓存**该读写在哪一行上**。
///
/// 判据不是「哪条来源记录」，而是 [`TokenSource::share_key`] ——「取 token 的那套
/// 配置」。于是：
///
/// * 两条来源配的是同一条取 token 请求、同一套提取路径 ⇒ 同一个代表 ⇒
///   同一行缓存 ⇒ 发送时只打一次网络、界面上只有一个倒计时；
/// * 配置只要有一处不同 ⇒ 各自的代表 ⇒ 各自独立。
///
/// ## 代表取组内 id 最小的那个
///
/// * **稳定**：不随查询顺序变。若改成「按名字排序取第一个」，用户给来源改个名
///   就会把缓存挪到另一个来源名下 —— 看起来就是「改完名字倒计时重置了」。
/// * **单调**：后建的同配置来源不会把已有缓存从先建的来源手里抢走。
///
/// ## 为什么不用一条 SQL 表达
///
/// 指纹的构成（哪些字段、怎么拼接）必须**只有 [`TokenSource::share_key`] 一处**。
/// 在 SQL 里再拼一遍 `fetch_request_id = ? AND access_path = ? AND …`，
/// 就是同一件事的第二个定义，迟早会有一处改了另一处没改 —— 那种漂移的表现是
/// 「缓存按一套规则共享、单飞锁按另一套」，最难查。这里改用「拉回工作区的来源
/// （个位数）再按指纹比」，代价可以忽略。
///
/// 组为空时退化成 `source.id` 自己。正常走不到（来源一定在自己的组里），
/// 只有「这一行刚被删掉、而某个执行链还攥着它的副本」才会命中 ——
/// 那种情况下按最老的语义（按来源自己一行）读写，绝不 panic。
pub fn cache_owner(conn: &Connection, source: &TokenSource) -> AppResult<i64> {
    let key = source.share_key();
    Ok(list_by_workspace(conn, source.workspace_id)?
        .iter()
        .filter(|other| other.share_key() == key)
        .map(|other| other.id)
        .min()
        .unwrap_or(source.id))
}

/// 读某个**代表行**的缓存。
///
/// ⚠️ `owner_id` 必须是 [`cache_owner`] 给的那个 id，**不是**随手一条来源的 id。
/// 传错的后果是静默的：`None` ⇒ 上层当成「从没取过」⇒ 界面显示「待刷新」，
/// 而库里其实躺着一份能用的 token。参数特意不叫 `token_source_id`，
/// 就是为了让调用点读起来像「我算过归属了吗」。
pub fn get_cache(conn: &Connection, owner_id: i64) -> AppResult<Option<TokenCache>> {
    let sql = format!("SELECT {CACHE_COLUMNS} FROM token_cache WHERE token_source_id = ?1");
    match conn.query_row(&sql, [owner_id], map_cache) {
        Ok(c) => Ok(Some(c)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// 写入 / 覆盖某个**代表行**的缓存。每个共享组只保留一行（唯一索引兜底）。
///
/// `owner_id` 的约束同 [`get_cache`]。
pub fn upsert_cache(
    conn: &Connection,
    owner_id: i64,
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
            owner_id,
            access_token,
            refresh_token,
            expires_at,
            raw_response,
            now_iso(),
        ],
    )?;

    get_cache(conn, owner_id)?.ok_or_else(|| {
        AppError::Internal(format!(
            "token_cache row vanished after upsert (owner #{owner_id})"
        ))
    })
}

/// 清掉某个代表行的缓存。
///
/// `owner_id` 的约束同 [`get_cache`]。**只用于「配置变了」与「删来源」两条路径**：
/// 它删的是整个共享组的那一份 token，组里每一条来源都会因此重新取一次。
pub fn clear_cache(conn: &Connection, owner_id: i64) -> AppResult<bool> {
    let affected = conn.execute(
        "DELETE FROM token_cache WHERE token_source_id = ?1",
        [owner_id],
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
             INSERT INTO folder (id, workspace_id, name, is_token_source, created_at, updated_at)
               VALUES (1, 1, 'auth', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO folder (id, workspace_id, name, is_token_source, created_at, updated_at)
               VALUES (2, 1, '业务分类', 0, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO request (id, folder_id, workspace_id, name, created_at, updated_at)
               VALUES (1, 1, 1, 'login', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn create_uses_design_defaults() {
        let conn = db();
        let s = create(&conn, 1, 1, "iPSA-OAuth", 1).unwrap();
        assert_eq!(s.access_path, "$.access_token");
        assert_eq!(s.refresh_path, "$.refresh_token");
        assert_eq!(s.expires_in_path, "$.expires_in");
        assert_eq!(s.expires_in_unit, ExpiryUnit::Seconds);
        assert_eq!(s.skew_seconds, 30);
        assert!(s.auto_refresh);
        assert_eq!(s.refresh_request_id, None);
    }

    #[test]
    fn folder_and_name_must_be_unique() {
        let conn = db();
        create(&conn, 1, 1, "dup", 1).unwrap();
        assert!(
            create(&conn, 1, 1, "dup", 1).is_err(),
            "unique index on (folder_id, name) must reject duplicates"
        );
    }

    #[test]
    fn save_updates_every_configurable_field() {
        let conn = db();
        let mut s = create(&conn, 1, 1, "oauth", 1).unwrap();
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
        let s = create(&conn, 1, 1, "oauth", 1).unwrap();

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

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        let _ = "token-a";
    }

    /// ⚠️ 这条测试在 2026-09-18 换了语义。
    ///
    /// 原来它叫 `cache_is_scoped_per_source`，钉的是「缓存按**来源记录**隔离」——
    /// 于是两条配着**同一条**取 token 请求的来源会各刷一次、各走一个倒计时。
    /// 现在规则是「按**配置**共享」：同指纹共用一个代表行。
    #[test]
    fn sources_with_the_same_config_share_one_cache_row() {
        let conn = db();
        // `create` 只吃「名字 + 取 token 请求」，其余落到列默认值 ——
        // 所以 a 与 b 的配置**逐字段相同**，这正是「配置相同」的最小例子
        let a = create(&conn, 1, 1, "a", 1).unwrap();
        let b = create(&conn, 1, 1, "b", 1).unwrap();
        assert_ne!(a.id, b.id, "先确认这是两条不同的来源记录");
        assert_eq!(a.share_key(), b.share_key(), "配置逐字段相同 ⇒ 同一个指纹");

        assert_eq!(cache_owner(&conn, &a).unwrap(), a.id, "代表取组内最小的 id");
        assert_eq!(cache_owner(&conn, &b).unwrap(), a.id);

        upsert_cache(
            &conn,
            cache_owner(&conn, &b).unwrap(),
            "shared",
            None,
            None,
            None,
        )
        .unwrap();

        // 缓存只落在代表那一行上：b 自己名下没有行，但 b 读得到同一份
        assert!(get_cache(&conn, b.id).unwrap().is_none());
        let seen = get_cache(&conn, cache_owner(&conn, &b).unwrap())
            .unwrap()
            .expect("b 应该读到组里那一行");
        assert_eq!(seen.access_token, "shared");
        assert_eq!(seen.token_source_id, a.id);

        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "两条来源只该有一行缓存");
    }

    /// 配置只要有一处不同，就不再共享 —— 各自一行，互不覆盖。
    #[test]
    fn sources_with_different_configs_keep_separate_cache_rows() {
        let conn = db();
        let a = create(&conn, 1, 1, "a", 1).unwrap();
        let mut b = create(&conn, 1, 1, "b", 1).unwrap();
        assert_eq!(
            cache_owner(&conn, &a).unwrap(),
            cache_owner(&conn, &b).unwrap(),
            "先确认这两条本来是同一组"
        );

        b.access_path = "$.data.token".into();
        let b = save(&conn, &b).unwrap();

        assert_ne!(
            cache_owner(&conn, &a).unwrap(),
            cache_owner(&conn, &b).unwrap(),
            "解析路径不同 ⇒ 取到的 token 可能不是一个东西，必须分开"
        );

        upsert_cache(
            &conn,
            cache_owner(&conn, &a).unwrap(),
            "ta",
            None,
            None,
            None,
        )
        .unwrap();
        upsert_cache(
            &conn,
            cache_owner(&conn, &b).unwrap(),
            "tb",
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            get_cache(&conn, cache_owner(&conn, &a).unwrap())
                .unwrap()
                .unwrap()
                .access_token,
            "ta"
        );
        assert_eq!(
            get_cache(&conn, cache_owner(&conn, &b).unwrap())
                .unwrap()
                .unwrap()
                .access_token,
            "tb"
        );
    }

    /// 清缓存清掉的是**整个共享组**那一份 —— 组里每条来源下次都得重新取。
    /// 只有两条路径会走到它：配置变更（`save`）与删除来源（外键级联）。
    #[test]
    fn clearing_the_owner_row_drops_the_whole_group() {
        let conn = db();
        let a = create(&conn, 1, 1, "a", 1).unwrap();
        let b = create(&conn, 1, 1, "b", 1).unwrap();
        let owner = cache_owner(&conn, &b).unwrap();
        upsert_cache(&conn, owner, "t", None, None, None).unwrap();

        assert!(clear_cache(&conn, owner).unwrap());
        assert!(get_cache(&conn, cache_owner(&conn, &a).unwrap())
            .unwrap()
            .is_none());
        assert!(get_cache(&conn, cache_owner(&conn, &b).unwrap())
            .unwrap()
            .is_none());
    }

    /// 名字、所在分类、自动刷新开关、提前量**都不是**「token 是什么」。
    ///
    /// 改它们不能把一条来源踢出它原来的共享组 —— 否则「同一个接口按业务拆成
    /// 两条来源、分放在两个分类下」这条最自然的用法就又变回各刷一次。
    #[test]
    fn fields_that_do_not_change_the_token_do_not_split_the_group() {
        let conn = db();
        let a = create(&conn, 1, 1, "a", 1).unwrap();
        let owner = cache_owner(&conn, &a).unwrap();

        let mut b = create(&conn, 1, 1, "b", 1).unwrap();
        b.name = "renamed".into();
        b.folder_id = 2;
        b.auto_refresh = !b.auto_refresh;
        b.skew_seconds = 600;
        let b = save(&conn, &b).unwrap();

        assert_eq!(b.folder_id, 2, "先确认位置真的改了");
        assert_eq!(cache_owner(&conn, &b).unwrap(), owner, "这些字段不该换代表");
        assert_eq!(b.share_key(), a.share_key());
    }

    /// 指纹取的是**有效的**刷新请求：`None`（复用取 token 那个）与
    /// 「显式填成同一个」在行为上完全一样，不能拆成两组。
    #[test]
    fn an_explicit_refresh_request_equal_to_the_fetch_one_is_the_same_config() {
        let conn = db();
        let a = create(&conn, 1, 1, "a", 1).unwrap();
        let mut b = create(&conn, 1, 1, "b", 1).unwrap();

        b.refresh_request_id = Some(b.fetch_request_id);
        let b = save(&conn, &b).unwrap();

        assert_eq!(a.share_key(), b.share_key());
        assert_eq!(
            cache_owner(&conn, &a).unwrap(),
            cache_owner(&conn, &b).unwrap()
        );
    }

    /// 配置变了 ⇒ 自己名下那行缓存立刻作废：旧 token 是从另一个字段里提的，
    /// 接着用等于拿一份对不上配置的凭据去发请求。
    #[test]
    fn changing_the_config_drops_the_cache_row() {
        let conn = db();
        let mut a = create(&conn, 1, 1, "a", 1).unwrap();
        upsert_cache(
            &conn,
            cache_owner(&conn, &a).unwrap(),
            "stale",
            None,
            None,
            None,
        )
        .unwrap();
        assert!(get_cache(&conn, a.id).unwrap().is_some());

        a.access_path = "$.other".into();
        save(&conn, &a).unwrap();

        assert!(
            get_cache(&conn, a.id).unwrap().is_none(),
            "换了解析路径，旧 token 必须丢掉"
        );
    }

    /// 对照组：只改名字**不该**丢 token —— 名字不影响 token 是什么，
    /// 白白多打一次取 token 请求是用户看得见的代价。
    #[test]
    fn renaming_keeps_the_cache_row() {
        let conn = db();
        let mut a = create(&conn, 1, 1, "a", 1).unwrap();
        upsert_cache(
            &conn,
            cache_owner(&conn, &a).unwrap(),
            "keep",
            None,
            None,
            None,
        )
        .unwrap();

        a.name = "new name".into();
        a.skew_seconds = 90;
        save(&conn, &a).unwrap();

        let kept = get_cache(&conn, cache_owner(&conn, &a).unwrap())
            .unwrap()
            .expect("改名字不该丢缓存");
        assert_eq!(kept.access_token, "keep");
    }

    #[test]
    fn delete_source_cascades_to_cache() {
        let conn = db();
        let s = create(&conn, 1, 1, "oauth", 1).unwrap();
        upsert_cache(&conn, s.id, "t", None, None, None).unwrap();

        delete(&conn, s.id).unwrap();
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM token_cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
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
