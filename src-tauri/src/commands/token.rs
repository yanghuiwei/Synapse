//! Token 源命令（design.md §4.3）。
//!
//! 与 `commands::env` 一样只做「校验入参 → 调仓储 / 引擎 → 映射错误」。
//! 唯一带点编排的是 [`refresh_token`] 与 [`get_token_status`]：它们要用
//! 「激活环境的变量」去解析取 token 的那个请求，所以得先自己把环境捞出来。
//!
//! ## 为什么状态查询是命令而不是前端算
//!
//! 「还有多少秒过期」取决于 `skew_seconds` 与缓存的 `expires_at`，
//! 判定逻辑在 `http::token::is_expired` 里**只有一份**。前端自己拿
//! `expires_at` 与 `skew` 再算一遍，两边迟早会在边界上分叉，表现为
//! 「徽标说还有 5 秒，后端说已经过期了」。

use std::collections::HashMap;

use serde::Serialize;
use tauri::State;

use crate::commands::validate::clean_name;
use crate::error::{AppError, AppResult};
use crate::http::token::{is_expired, remaining_seconds, TokenDeps};
use crate::storage::models::{TokenCache, TokenSource};
use crate::storage::{repo, with_conn};
use crate::AppState;

/// Token 源的运行状态。给徽标（剩余有效期）与「立即刷新」用。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenStatus {
    /// 有没有缓存行。`false` = 从没取过
    pub has_cache: bool,
    /// 已经过期（含 `skew_seconds` 提前量）
    pub expired: bool,
    /// 过期时间的绝对值（ISO8601）；`null` = 永不过期
    pub expires_at: Option<String>,
    /// 还剩多少秒。负数表示已经过去多久；`null` = 永不过期或没有缓存
    pub remaining_seconds: Option<i64>,
    /// 缓存最后一次写入的时间
    pub updated_at: Option<String>,
    /// 当前缓存的 access token。**由前端负责打码显示** ——
    /// 与 `env_var.is_secret` 同一套约定（后端给真值，打码是展示层的事）
    pub access_token: Option<String>,
    pub has_refresh_token: bool,
    /// 该 Token 源是否开启自动刷新
    pub auto_refresh: bool,
}

// ============================================================
// 配置 CRUD
// ============================================================

#[tauri::command]
pub async fn list_token_sources(
    state: State<'_, AppState>,
    workspace_id: i64,
) -> AppResult<Vec<TokenSource>> {
    with_conn(&state.pool, move |conn| {
        repo::token::list_by_workspace(conn, workspace_id)
    })
    .await
}

#[tauri::command]
pub async fn create_token_source(
    state: State<'_, AppState>,
    workspace_id: i64,
    request_id: i64,
    name: String,
    fetch_request_id: i64,
) -> AppResult<TokenSource> {
    let name = clean_name(&name, "name")?;
    with_conn(&state.pool, move |conn| {
        repo::token::create(conn, workspace_id, request_id, &name, fetch_request_id)
    })
    .await
}

/// 全量保存。仓储只校验文件夹存在，不校验 `is_token_source` 标记 ——
/// 标记是 UI 的引导，不该做成数据库层的硬约束（先建源后打标记是合法顺序）。
#[tauri::command]
pub async fn save_token_source(
    state: State<'_, AppState>,
    source: TokenSource,
) -> AppResult<TokenSource> {
    let name = clean_name(&source.name, "name")?;
    if source.skew_seconds < 0 {
        return Err(AppError::InvalidInput {
            field: "skewSeconds",
            reason: "cannot be negative".into(),
        });
    }

    with_conn(&state.pool, move |conn| {
        repo::token::save(conn, &TokenSource { name, ..source })
    })
    .await
}

/// 硬删除（Token 配置属于「机器用的配置」，不进回收站；缓存行由外键带走）。
#[tauri::command]
pub async fn delete_token_source(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    with_conn(&state.pool, move |conn| repo::token::delete(conn, id)).await
}

/// 一条请求自己那份 Token 源（没有就 `None`）。
///
/// 与 `http::pipeline::find_token_source` 必须给出**同一个答案**：界面显示的和
/// 发送时用的要是两个源，用户就会看到一个「明明配好了却不生效」的假象。
///
/// 2026-09-18 之前这里是两级判定（先沿文件夹链找最近的，再兜底到系统分类
/// 「token刷新」下的全局那一个）。008 之后 Token 源逐请求一份，两级都没有了 ——
/// 这个函数只剩一次 `by_request` 查询，与流水线那侧逐字对应。
#[tauri::command]
pub async fn resolve_token_source(
    state: State<'_, AppState>,
    request_id: Option<i64>,
) -> AppResult<Option<TokenSource>> {
    let Some(request_id) = request_id else {
        // 未保存的草稿：没有身份 ⇒ 没有源。这与「新建请求还没配源」表现一致，
        // 不会静默借用别处的 token。
        return Ok(None);
    };
    with_conn(&state.pool, move |conn| {
        repo::token::by_request(conn, request_id)
    })
    .await
}

// ============================================================
// 状态与刷新
// ============================================================

/// 一条 Token 源的运行状态，**带上它是谁**。
///
/// 为什么不直接返回 `HashMap<i64, TokenStatus>`：JSON 的对象键只能是
/// 字符串，前端的 `Record<number, …>` 得靠隐式转换对上。一个显式的 `sourceId`
/// 字段让两边都不必猜。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenStatusRow {
    pub source_id: i64,
    /// `flatten` 而不是 `status: TokenStatus`：前端拿到的形状与
    /// `get_token_status` 的返回**逐字段相同**，两处不必各写一套解析。
    #[serde(flatten)]
    pub status: TokenStatus,
}

/// 一次拿回某个工作区**全部** Token 源的状态。
///
/// ## 为什么不是前端逐条调 `get_token_status`
///
/// 008 之后 Token 源是逐请求一份的 —— 这个库里有一百多条。逐条调就是一百多趟
/// IPC，而每次调用都要过一遍 `with_conn`（异步 → 阻塞的边界）。
/// 合成一条命令之后仍然是一百多次点查，但那是**同一个连接上的主键查找**，
/// 快得可以忽略；省掉的是 IPC 与序列化那一百多个来回。
///
/// 这里没有单独的测试：它只是 `list_by_workspace` /
/// `get_cache` / `status_of` 三个已测过的东西拼起来，
/// 再写一遍只会把它们各自的断言拄一遍。
#[tauri::command]
pub async fn list_token_statuses(
    state: State<'_, AppState>,
    workspace_id: i64,
) -> AppResult<Vec<TokenStatusRow>> {
    with_conn(&state.pool, move |conn| {
        let sources = repo::token::list_by_workspace(conn, workspace_id)?;
        let mut rows = Vec::with_capacity(sources.len());
        for source in sources {
            let cache = repo::token::get_cache(conn, source.id)?;
            rows.push(TokenStatusRow {
                source_id: source.id,
                status: status_of(&source, cache.as_ref()),
            });
        }
        Ok(rows)
    })
    .await
}

#[tauri::command]
pub async fn get_token_status(state: State<'_, AppState>, id: i64) -> AppResult<TokenStatus> {
    let source = source_of(&state, id).await?;
    let cache = cache_of(&state, &source).await?;

    Ok(status_of(&source, cache.as_ref()))
}

/// 强制刷新，忽略缓存（design.md §4.3 的「立即刷新」）。
///
/// 走的是同一个单飞锁：如果此刻正好有 5 个业务请求也在等刷新，
/// 这次强制刷新**不会**变成第 6 次网络请求 —— 它排在同一个航班上。
#[tauri::command]
pub async fn refresh_token(state: State<'_, AppState>, id: i64) -> AppResult<TokenStatus> {
    let source = source_of(&state, id).await?;
    let env = active_env(&state).await?;

    let deps = TokenDeps {
        pool: &state.pool,
        clients: &state.clients,
        source: &source,
        env: &env,
        cancel: None,
    };
    state.tokens.refresh_now(&deps).await?;

    // 刷完再把库里的那一行读回来作状态 —— 不直接拿刷新返回值拼，
    // 免得「显示的状态」与「真的落库了什么」有第二条路径
    let cache = cache_of(&state, &source).await?;
    Ok(status_of(&source, cache.as_ref()))
}

// ============================================================
// 内部助手
// ============================================================

async fn source_of(state: &State<'_, AppState>, id: i64) -> AppResult<TokenSource> {
    let pool = state.pool.clone();
    with_conn(&pool, move |conn| repo::token::get(conn, id)).await
}

/// 这份来源自己的那行缓存。
///
/// 直接按 `source.id` 读，**没有归属算法**。008 之前这里要先算出「缓存落在哪条
/// 代表行上」（配置相同的来源共用一份 token），所以还得读一次 `cache_owner`；
/// 现在一个源一行缓存，`token_cache.token_source_id` 上的唯一索引就是全部规则。
async fn cache_of(
    state: &State<'_, AppState>,
    source: &TokenSource,
) -> AppResult<Option<TokenCache>> {
    let pool = state.pool.clone();
    let id = source.id;
    with_conn(&pool, move |conn| repo::token::get_cache(conn, id)).await
}

/// 激活环境的变量。取 token 的那个请求自己也可能写 `{{env.clientId}}`。
async fn active_env(state: &State<'_, AppState>) -> AppResult<HashMap<String, String>> {
    let pool = state.pool.clone();
    with_conn(&pool, |conn| {
        let Some(workspace) = repo::workspace::active(conn)? else {
            return Ok(HashMap::new());
        };
        repo::env::active_vars(conn, workspace.id)
    })
    .await
}

fn status_of(source: &TokenSource, cache: Option<&TokenCache>) -> TokenStatus {
    let Some(cache) = cache else {
        return TokenStatus {
            has_cache: false,
            expired: true,
            expires_at: None,
            remaining_seconds: None,
            updated_at: None,
            access_token: None,
            has_refresh_token: false,
            auto_refresh: source.auto_refresh,
        };
    };

    let now = chrono::Utc::now();

    TokenStatus {
        has_cache: true,
        // 过期判定与剩余秒数**取同一个 `now`**：两处各调一次 `Utc::now()`
        // 会在「还剩 0.4 秒过期」这种边界上给出自相矛盾的组合
        // （remaining 说还有 0 秒、expired 说已经过期）。
        expired: is_expired(cache, source.skew_seconds, now),
        expires_at: cache.expires_at.clone(),
        // 剩余秒数走 `http::token` 那一份实现：界面上这个数字与响应里
        // `tokenUsage.remainingSeconds` 是同一个数，两处各算一遍迟早会差几秒 ——
        // 那种差异会让人怀疑「刚刷完的 token 怎么倒计时不一样」
        remaining_seconds: remaining_seconds(cache.expires_at.as_deref(), now),
        updated_at: Some(cache.updated_at.clone()),
        access_token: Some(cache.access_token.clone()),
        has_refresh_token: cache.refresh_token.is_some(),
        auto_refresh: source.auto_refresh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::TokenCache;

    fn source(auto_refresh: bool) -> TokenSource {
        TokenSource {
            id: 1,
            workspace_id: 1,
            request_id: 2,
            name: "oauth".into(),
            fetch_request_id: 1,
            access_path: "$.a".into(),
            refresh_path: "$.r".into(),
            expires_in_path: "$.e".into(),
            expires_in_unit: crate::storage::models::ExpiryUnit::Seconds,
            refresh_request_id: None,
            auto_refresh,
            skew_seconds: 30,
            created_at: "c".into(),
            updated_at: "u".into(),
        }
    }

    fn cache(expires_at: Option<&str>) -> TokenCache {
        TokenCache {
            id: 1,
            token_source_id: 1,
            access_token: "tok".into(),
            refresh_token: None,
            expires_at: expires_at.map(str::to_string),
            raw_response: None,
            updated_at: "2026-01-01T00:00:00.000Z".into(),
        }
    }

    #[test]
    fn no_cache_is_reported_as_expired() {
        let status = status_of(&source(true), None);
        assert!(!status.has_cache);
        assert!(status.expired, "从没取过就必须算过期，否则徽标会显示成有效");
        assert_eq!(status.remaining_seconds, None);
        assert_eq!(status.access_token, None);
    }

    #[test]
    fn remaining_seconds_is_computed_from_expires_at() {
        let at = (chrono::Utc::now() + chrono::Duration::seconds(100))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let status = status_of(&source(true), Some(&cache(Some(&at))));

        let remaining = status.remaining_seconds.expect("算得出剩余秒数");
        assert!((96..=100).contains(&remaining), "剩余秒数不对: {remaining}");
        assert!(!status.expired, "还有 100 秒、skew 30 秒 → 仍然有效");
        assert!(status.has_cache);
    }

    /// 永不过期的缓存：`remaining` 为 `None` 且不算过期
    #[test]
    fn cache_without_expiry_is_valid_forever() {
        let status = status_of(&source(true), Some(&cache(None)));
        assert!(status.has_cache);
        assert!(!status.expired);
        assert_eq!(status.remaining_seconds, None);
        assert_eq!(status.expires_at, None);
    }

    /// 已经过去的过期时间 → 过期，且剩余是负数
    #[test]
    fn past_expiry_shows_negative_remaining() {
        let at = (chrono::Utc::now() - chrono::Duration::seconds(60))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let status = status_of(&source(true), Some(&cache(Some(&at))));
        assert!(status.expired);
        assert!(status.remaining_seconds.unwrap() < 0);
    }

    #[test]
    fn status_mirrors_auto_refresh_flag() {
        assert!(status_of(&source(true), None).auto_refresh);
        assert!(!status_of(&source(false), None).auto_refresh);
    }
}
