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
    /// 这一份 token **落在哪条代表行上**（`repo::token::cache_owner`）。
    ///
    /// 配置逐字段相同的来源会返回同一个值 —— 界面据此知道「这两个来源共用一份
    /// token、一个倒计时」，于是刷新其一就能同步更新另一个，计数也不会把
    /// 一份 token 数成两份。**没有缓存行时这个值同样有效**（归属和有没有值是两件事）。
    pub cache_owner_id: i64,
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
    folder_id: i64,
    name: String,
    fetch_request_id: i64,
) -> AppResult<TokenSource> {
    let name = clean_name(&name, "name")?;
    with_conn(&state.pool, move |conn| {
        repo::token::create(conn, workspace_id, folder_id, &name, fetch_request_id)
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

/// 某条请求实际会用到哪个 Token 源。
///
/// 两级判定，与 `http::pipeline::find_token_source` 是**同一套规则的两份实现**：
/// 先沿文件夹链向上找第一个，再兜底到系统分类「token刷新」下的全局那一个。
/// 两份必须一起改 —— 界面显示的和发送时用的要是两个源，用户就会看到一个
/// 「明明配好了却不生效」的假象。
///
/// ⚠️ 请求**没有归属分类**时也要照常解析（`folder_id = None`）：Token 来源
/// 固定配在系统分类下，这条路径恰恰是最常见的。早先这里直接返回 `None`，
/// 界面上就永远显示「没有可用 Token 来源」。
#[tauri::command]
pub async fn resolve_token_source(
    state: State<'_, AppState>,
    folder_id: Option<i64>,
) -> AppResult<Option<TokenSource>> {
    with_conn(&state.pool, move |conn| {
        if let Some(folder_id) = folder_id {
            let chain = repo::folder::ancestry(conn, folder_id)?;
            let ids: Vec<i64> = chain.iter().map(|f| f.id).collect();
            if let Some(source) = repo::token::first_in_chain(conn, &ids)? {
                return Ok(Some(source));
            }
        }
        repo::token::system_source(conn)
    })
    .await
}

// ============================================================
// 状态与刷新
// ============================================================

#[tauri::command]
pub async fn get_token_status(state: State<'_, AppState>, id: i64) -> AppResult<TokenStatus> {
    let source = source_of(&state, id).await?;
    let (owner, cache) = cache_of(&state, &source).await?;

    Ok(status_of(&source, owner, cache.as_ref()))
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
    let (owner, cache) = cache_of(&state, &source).await?;
    Ok(status_of(&source, owner, cache.as_ref()))
}

// ============================================================
// 内部助手
// ============================================================

async fn source_of(state: &State<'_, AppState>, id: i64) -> AppResult<TokenSource> {
    let pool = state.pool.clone();
    with_conn(&pool, move |conn| repo::token::get(conn, id)).await
}

/// 这份来源的**缓存归属**（落在哪条代表行上）与那一行里的内容。
///
/// 两件事一起返回，不是图省事：分开两次查，两趟之间配置可能被改过，
/// 于是会出现「归属按新配置算、内容读的是旧那一行」这种半拉子状态。
///
/// 归属算法只有 `repo::token::cache_owner` 一处 —— 这里不自己拼 SQL，
/// 也不拿 `source.id` 顶替（那样就把「配置相同的来源共用一份 token」悄悄废掉了，
/// 而且症状只是「明明有 token 却显示没取过」，看不出根因）。
async fn cache_of(
    state: &State<'_, AppState>,
    source: &TokenSource,
) -> AppResult<(i64, Option<TokenCache>)> {
    let pool = state.pool.clone();
    let source = source.clone();
    with_conn(&pool, move |conn| {
        let owner = repo::token::cache_owner(conn, &source)?;
        let cache = repo::token::get_cache(conn, owner)?;
        Ok((owner, cache))
    })
    .await
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

fn status_of(source: &TokenSource, cache_owner_id: i64, cache: Option<&TokenCache>) -> TokenStatus {
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
            cache_owner_id,
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
        cache_owner_id,
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
            folder_id: 1,
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
        let status = status_of(&source(true), 7, None);
        assert!(!status.has_cache);
        assert!(status.expired, "从没取过就必须算过期，否则徽标会显示成有效");
        assert_eq!(status.remaining_seconds, None);
        assert_eq!(status.access_token, None);
    }

    #[test]
    fn remaining_seconds_is_computed_from_expires_at() {
        let at = (chrono::Utc::now() + chrono::Duration::seconds(100))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let status = status_of(&source(true), 7, Some(&cache(Some(&at))));

        let remaining = status.remaining_seconds.expect("算得出剩余秒数");
        assert!((96..=100).contains(&remaining), "剩余秒数不对: {remaining}");
        assert!(!status.expired, "还有 100 秒、skew 30 秒 → 仍然有效");
        assert!(status.has_cache);
    }

    /// 永不过期的缓存：`remaining` 为 `None` 且不算过期
    #[test]
    fn cache_without_expiry_is_valid_forever() {
        let status = status_of(&source(true), 7, Some(&cache(None)));
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
        let status = status_of(&source(true), 7, Some(&cache(Some(&at))));
        assert!(status.expired);
        assert!(status.remaining_seconds.unwrap() < 0);
    }

    #[test]
    fn status_mirrors_auto_refresh_flag() {
        assert!(status_of(&source(true), 7, None).auto_refresh);
        assert!(!status_of(&source(false), 7, None).auto_refresh);
    }

    /// 归属要**原样**回给前端，而且在「没有缓存」时同样有效 ——
    /// 界面靠它判断哪些来源共用一份 token（刷新其一要同步更新另一个、
    /// 计数也不能把一份 token 数成两份）。没有值时把它填成 `0` 或来源自己的 id，
    /// 都会让界面把同组的两条来源看成两套。
    #[test]
    fn cache_owner_id_is_reported_even_without_a_cache_row() {
        let owner = 3; // 组内的代表行，未必是这一条来源自己
        assert_eq!(status_of(&source(true), owner, None).cache_owner_id, owner);
        assert_eq!(
            status_of(&source(true), owner, Some(&cache(None))).cache_owner_id,
            owner
        );
    }
}
