//! Token 源的获取与自动刷新（design.md §4.3）。
//!
//! ## 单飞锁（single-flight）到底锁的是什么
//!
//! 锁的是「刷新这个动作」，**不是**锁住 token 的值。
//!
//! 结构是 `HashMap<TokenSourceId, Arc<tokio::sync::Mutex<()>>>` —— 每个 Token 源一把
//! 「航班锁」，然后**两次检查缓存**：
//!
//! ```text
//!   ensure(force = false)
//!     ├─ 检查一（不持锁）：缓存有效 → 直接返回，连锁都不碰
//!     └─ gate.lock().await          ← 5 个并发请求在这里排队
//!          └─ 检查二（持锁）：缓存有效 → 直接返回，零额外请求  ★
//!               └─ 都没有 → 打一次取 token 请求 → 写 token_cache
//! ```
//!
//! 关键在检查二：**后到者醒来时，第一个已经把新缓存写进库了**，
//! 于是它们全部命中、一个请求都不发。plan.md §4 验收第 3 条
//! （并发 5 个请求，mock server 只收到 1 次刷新）靠的就是这四行。
//!
//! 为什么不用「往 map 里存 `Shared<Future>`」那种写法：那需要把 Future 存成
//! `'static` 并把错误类型擦掉，还要处理「谁负责删 entry」。这里后到者**从库里**
//! 读结果，值本身不经过内存传递 —— 真源始终是 `token_cache` 那一行，
//! 内存里只串行化动作。代价是后到者多读一次库（几微秒），换来的是
//! 一套没有生命周期纠缠、也没有「内存态与库不一致」可能的实现。
//!
//! ## 为什么失败**不**合并
//!
//! 刷新失败时不写缓存、直接把错误抛给这个人，锁随之释放 ——
//! 后面的等待者醒来会各自重试一次。这是刻意的：把一次网络抖动放大成
//! 「所有人共用同一个错误」更糟，而且用户点一下重试就好了。
//! 验收第 3 条只约束**成功**路径的合并，这条与它不冲突。
//!
//! ## 为什么 entry 不清理
//!
//! entry 的个数 = Token 源的个数（个位数），常驻可忽略。清理反而要回答
//! 「谁来删、什么时候删」——而删早了会让两个请求拿到两把不同的锁，
//! 单飞锁就静默失效了。宁可留着。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Local, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::error::{codes, AppError, AppResult};
use crate::http::client::ClientCache;
use crate::http::execute::{execute_with_cancel, RequestSpec};
use crate::storage::models::{ExpiryUnit, TokenCache, TokenSource};
use crate::storage::pool::SqlitePool;
use crate::storage::{repo, with_conn};
use crate::template::functions::Entropy;
use crate::template::jsonpath;
use crate::template::resolver::{resolve_spec, TemplateContext, TokenValues};

/// 取一个 Token 值所需的全部外部依赖。
///
/// 抽成显式结构而不是直接吃 `AppState`：单测要能塞一个「进程内极简 TCP server」
/// 的地址和一个临时库，而不是搭一整套 tauri 状态。
pub struct TokenDeps<'a> {
    pub pool: &'a SqlitePool,
    pub clients: &'a ClientCache,
    pub source: &'a TokenSource,
    /// 激活环境的变量。取 token 的那个请求自己也可能写 `{{env.clientId}}`
    pub env: &'a HashMap<String, String>,
    pub cancel: Option<&'a CancellationToken>,
}

/// 一次「确保就绪」的结果：拿到的值 + 本次到底刷没刷。
///
/// ## 为什么要把这件事显式带出来
///
/// 缓存命中与真刷新在**结果**上完全一样 —— 都是一个能用的 token，
/// 所以在此之前没有任何地方能区分它们。但用户排查问题时这两者天差地别：
/// 「我以为它刷了，其实一直在用一小时前那个」是最典型的误判，而且从界面上
/// 完全看不出来（请求成功、token 也是对的，只是旧的）。
///
/// 这个标记由**执行期**产生，一路回传到响应的 `token_usage` 里。
/// 不让前端拿 `updatedAt` 前后比对：那种推断在并发（或 MCP/CLI 也在刷）
/// 时会把别人的刷新算到这一次头上，结论看着自洽却是错的。
#[derive(Debug)]
pub struct TokenOutcome {
    pub values: TokenValues,
    /// `true` = 本次真的打了一次取 Token 请求；`false` = 命中缓存（零网络）
    pub refreshed: bool,
}

/// 「Token 源 id」→ 一把航班锁。
///
/// 键就是来源自己的 id：**一个接口一份配置、一份 token、一个倒计时**，
/// 所以"谁和谁该排同一队"不需要任何推导 —— 两条来源要么是同一条，
/// 要么互不相干。
///
/// （2026-09-18 之前这里的键是配置指纹，为的是让"配着同一套取 token 配置的
/// 两条来源只打一次网络"。那套合并已撤掉：同配置的两个接口也要各刷各的。）
#[derive(Default)]
pub struct TokenRefresher {
    flights: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
}

impl TokenRefresher {
    pub fn new() -> Self {
        Self::default()
    }

    /// 取某个 Token 源本次可用的值。`force = true` 时忽略缓存（「立即刷新」）。
    pub async fn ensure(&self, deps: &TokenDeps<'_>, force: bool) -> AppResult<TokenOutcome> {
        // ── 检查一：不持锁。缓存有效时（绝大多数请求）在这里就返回了 ──
        if !force {
            if let Some(values) = read_fresh(deps).await? {
                return Ok(TokenOutcome {
                    values,
                    refreshed: false,
                });
            }
        }

        let gate = self.flight_for(deps.source.id);
        let _guard = gate.lock().await;

        // ── 检查二：持锁。★ 这一步让「并发只打一次刷新」成立 ──
        if !force {
            if let Some(values) = read_fresh(deps).await? {
                // 后到者在这里醒来，`refreshed = false`：它拿到的是**第一个人**
                // 刷出来的值，自己一个请求都没发。界面要说的是「本次没刷」——
                // 说「刷了」会把并发时的请求数报成 5 次，而服务端只收到 1 次。
                return Ok(TokenOutcome {
                    values,
                    refreshed: false,
                });
            }
        }

        // 关掉自动刷新时**不偷偷用过期 token**：服务端返回的 401 会把
        // 「token 过期」伪装成「权限不足」，排查成本远高于我们直说。
        if !deps.source.auto_refresh && !force {
            return Err(source_error(
                codes::TOKEN_EXPIRED,
                format!(
                    "token source #{} has no usable cache and auto refresh is off",
                    deps.source.id
                ),
                deps.source,
            ));
        }

        Ok(TokenOutcome {
            values: fetch(deps).await?,
            refreshed: true,
        })
    }

    /// 「立即刷新」：忽略缓存，强制打一次（design.md §4.3 的最后一行）。
    ///
    /// `force = true` 会跳过两次缓存检查，所以结果里 `refreshed` 必然是 `true`。
    pub async fn refresh_now(&self, deps: &TokenDeps<'_>) -> AppResult<TokenOutcome> {
        self.ensure(deps, true).await
    }

    /// 当前登记的航班数。测试用 —— 它应当等于**用过的** Token 源个数，
    /// 且不会因为两条来源配置相同就少登记一把锁。
    #[cfg(test)]
    fn flight_count(&self) -> usize {
        lock(&self.flights).len()
    }

    /// 取（或创建）某个 Token 源的航班锁。
    ///
    /// `std::sync::Mutex` 的持有时间只有一次哈希查找，不会成为热点 ——
    /// 真正的等待发生在后面那把 `tokio` 的异步锁上。
    fn flight_for(&self, source_id: i64) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.flights).entry(source_id).or_default().clone()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // 持锁的代码不会 panic（只有哈希查找），所以中毒在这里是不可能状态；
    // 真中了的话继续用，好过让整个功能永久瘫掉。
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// 读缓存；有效就返回，不存在或已过期返回 `None`。
///
/// 缓存挂在**这条来源自己**名下（`token_source.id` ↔ `token_cache.token_source_id`
/// 一一对应），中间没有任何归属推导。
async fn read_fresh(deps: &TokenDeps<'_>) -> AppResult<Option<TokenValues>> {
    let source_id = deps.source.id;
    let cache = with_conn(deps.pool, move |conn| {
        repo::token::get_cache(conn, source_id)
    })
    .await?;

    let Some(cache) = cache else { return Ok(None) };
    if is_expired(&cache, deps.source.skew_seconds, Utc::now()) {
        return Ok(None);
    }

    Ok(Some(TokenValues {
        access_token: cache.access_token,
        refresh_token: cache.refresh_token,
        expires_at: cache.expires_at,
    }))
}

/// 缓存是否已过期。`skew_seconds` 是「提前多少秒开始视为过期」。
///
/// 时间戳解析不出来时按**已过期**处理：宁可多刷一次，也不要拿着一个
/// 自己都读不懂的过期时间去发请求。
pub fn is_expired(cache: &TokenCache, skew_seconds: i64, now: DateTime<Utc>) -> bool {
    let Some(raw) = cache.expires_at.as_deref() else {
        // `expires_at = None` 的语义是「永不过期」（见 TokenCache 的注释）：
        // 用户没配 `expires_in_path` 时的显式选择
        return false;
    };

    match parse_expiry(raw) {
        Some(at) => now + Duration::seconds(skew_seconds.max(0)) >= at,
        None => true,
    }
}

/// 距离过期还剩多少秒。负数 = 已经过去多久；`None` = 永不过期或时间戳读不懂。
///
/// ⚠️ **只用于显示**，别拿它判「算不算过期」——「提前量」(`skew_seconds`)
/// 不在这个数字里，判定走 [`is_expired`]。签名收 `Option<&str>` 而不是
/// `&TokenCache`：响应里带出去的那份只是一段 ISO 字符串，没有整行缓存。
///
/// 与 `is_expired` 共用同一份解析（[`parse_expiry`]），否则会出现
/// 「剩余秒数说明明还有 30 秒、判定却说已过期」这种自相矛盾的展示。
pub fn remaining_seconds(expires_at: Option<&str>, now: DateTime<Utc>) -> Option<i64> {
    let at = parse_expiry(expires_at?)?;
    Some((at - now).num_seconds())
}

/// RFC3339 → UTC。读不懂就是 `None`，由调用方决定偏保守的那一侧。
fn parse_expiry(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// 一次业务请求里 Token 的**实际使用情况**（由流水线填进响应回给前端）。
///
/// 这是「发送前能看见」与「发送后知道到底用没用」两件事的共同数据源：
/// 前者要来源名与倒计时，后者要 `refreshed`（本次刷没刷）与 `endpointName`
/// （刷的是哪个接口）。由后端一次算清，前端只负责翻译成文案。
///
/// 字段刻意不含任何给人读的句子 —— 文案归前端查表（契约同错误码）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub source_id: i64,
    pub source_name: String,
    /// ★ 本次真的打了一次取 Token 请求（`false` = 命中缓存、零网络）
    pub refreshed: bool,
    /// 生效的取 Token 接口 id。选法与 `fetch` 一致：`refresh_request_id` 优先
    pub endpoint_request_id: i64,
    /// 那个接口的名字。**可能为 `None`** —— 它刚被删掉、或名字读不出来时
    /// 宁可少显示一截，也不该让整条响应因为一个展示字段而失败
    pub endpoint_name: Option<String>,
    /// 缓存里的过期时间（ISO8601）；`None` = 永不过期
    pub expires_at: Option<String>,
    /// 还剩多少秒（负数 = 已经过去多久）。**不含** `skew_seconds`，只看显示
    pub remaining_seconds: Option<i64>,
    /// 该来源是否开着自动刷新 —— 关掉时「过期了也不刷」是预期行为，不是故障
    pub auto_refresh: bool,
}

/// 真正打一次取 token 请求、按路径提取、写缓存。
async fn fetch(deps: &TokenDeps<'_>) -> AppResult<TokenValues> {
    let source = deps.source;

    // 刷新的接口可以与取 token 的接口不同（design.md §4.3 的「刷新接口」一行）
    let request_id = source.refresh_request_id.unwrap_or(source.fetch_request_id);
    let spec = load_fetch_spec(deps, request_id).await?;

    let client = deps.clients.get(&spec.settings)?;
    let payload = execute_with_cancel(&client, &spec, deps.cancel)
        .await
        .map_err(|e| {
            // 把「取 token 的请求失败」与「业务请求失败」在错误码上分开：
            // 前者的处置动作是去修那个 Token 接口
            source_error(
                codes::TOKEN_REFRESH,
                format!("token request `{}` failed: {e}", spec.label()),
                source,
            )
        })?;

    // `execute` 对非 2xx 是**正常返回**（4xx/5xx 也是有效响应），
    // 所以这里必须自己判 —— 否则我们会拿着一个 401 的错误体当 token 用
    if !(200..300).contains(&payload.status) {
        return Err(AppError::http_with(
            codes::TOKEN_REFRESH,
            format!(
                "token endpoint `{}` returned HTTP {}",
                spec.label(),
                payload.status
            ),
            source_params(source, Some(payload.status)),
        ));
    }

    let body: Value = serde_json::from_str(&payload.body).map_err(|e| {
        source_error(
            codes::TOKEN_EXTRACT,
            format!("token response is not valid JSON: {e}"),
            source,
        )
    })?;

    let access_token = extract_required(&body, &source.access_path, "access_token", source)?;

    let refresh_token = if source.refresh_path.trim().is_empty() {
        None
    } else {
        // refresh_token 允许取不到（有些接口不返回它，也就没法续期）
        jsonpath::extract(&body, &source.refresh_path)
            .filter(|v| !v.is_null())
            .map(jsonpath::to_placeholder_string)
            .filter(|s| !s.is_empty())
    };

    let expires_at = compute_expires_at_field(&body, source, Utc::now())?;

    let cache = {
        let source_id = source.id;
        let access = access_token.clone();
        let refresh = refresh_token.clone();
        let expires = expires_at.clone();
        with_conn(deps.pool, move |conn| {
            // `raw_response` 传 None：原始响应默认**不落盘**（design.md §4.3），
            // 它可能含 refresh_token 之类的长期凭据，留在库里只会扩大暴露面
            repo::token::upsert_cache(
                conn,
                source_id,
                &access,
                refresh.as_deref(),
                expires.as_deref(),
                None,
            )
        })
        .await?
    };

    Ok(TokenValues {
        access_token: cache.access_token,
        refresh_token: cache.refresh_token,
        expires_at: cache.expires_at,
    })
}

/// 把「取 token 的那个请求」从库里读出来，解析掉 env 占位符，变成可发送的 spec。
async fn load_fetch_spec(deps: &TokenDeps<'_>, request_id: i64) -> AppResult<RequestSpec> {
    let request = with_conn(deps.pool, move |conn| repo::request::get(conn, request_id)).await?;

    let mut ctx = TemplateContext {
        env: deps.env,
        // ★ 防递归：取 token 的请求**不能**再用 `{{token.*}}`。
        // 否则 token 源 A 的取 token 请求里引用了 token，就是自己刷新自己。
        token: None,
        // 同理，也不给 `{{resp.*}}` —— 刷新发生在业务请求真正发出之前
        resp: None,
        now: Local::now(),
        entropy: Entropy::from_system(),
    };

    Ok(resolve_spec(&RequestSpec::from(&request), &mut ctx).spec)
}

/// 按路径取一个必需的字段，取不到就报「提取失败」。
fn extract_required(
    body: &Value,
    path: &str,
    label: &str,
    source: &TokenSource,
) -> AppResult<String> {
    if path.trim().is_empty() {
        return Err(source_error(
            codes::TOKEN_EXTRACT,
            format!("`{label}` path is not configured"),
            source,
        ));
    }

    jsonpath::extract(body, path)
        .filter(|v| !v.is_null())
        .map(jsonpath::to_placeholder_string)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            source_error(
                codes::TOKEN_EXTRACT,
                format!("`{path}` did not yield a {label}"),
                source,
            )
        })
}

/// `expires_in_path` → 绝对过期时间（ISO8601）。
///
/// 两种「没有过期时间」要分清楚：
/// * 路径**没配**（空串）→ `None`，语义是「永不过期」，这是用户的显式选择
/// * 配了但**取不到** → 报错。静默退化成「永不过期」会让 token 永远不刷新，
///   而用户以为配好了 —— 这种静默失败最难查
fn compute_expires_at_field(
    body: &Value,
    source: &TokenSource,
    now: DateTime<Utc>,
) -> AppResult<Option<String>> {
    let path = source.expires_in_path.trim();
    if path.is_empty() {
        return Ok(None);
    }

    let raw = jsonpath::extract(body, path)
        .filter(|v| !v.is_null())
        .ok_or_else(|| {
            source_error(
                codes::TOKEN_EXTRACT,
                format!("`{path}` did not yield an expiry value"),
                source,
            )
        })?;

    let at = absolute_expiry(raw, source.expires_in_unit, now).ok_or_else(|| {
        source_error(
            codes::TOKEN_EXTRACT,
            format!(
                "`{path}` = {raw} is not usable as `{}`",
                source.expires_in_unit.as_db_str()
            ),
            source,
        )
    })?;

    Ok(Some(at.to_rfc3339_opts(SecondsFormat::Millis, true)))
}

/// 把提取到的值按单位换算成绝对时间。
///
/// `absolute` 分支同时接受 ISO8601 字符串与数字（`1e12` 以上的数字按毫秒算）——
/// 内网接口两种都见过，只认其中一种会让人怀疑是我们算错了。
fn absolute_expiry(raw: &Value, unit: ExpiryUnit, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match unit {
        ExpiryUnit::Seconds => as_i64(raw).map(|secs| now + Duration::seconds(secs)),
        ExpiryUnit::Millis => as_i64(raw).map(|ms| now + Duration::milliseconds(ms)),
        ExpiryUnit::Absolute => match raw {
            Value::String(s) => DateTime::parse_from_rfc3339(s.trim())
                .ok()
                .map(|at| at.with_timezone(&Utc)),
            Value::Number(_) => {
                let n = as_i64(raw)?;
                Some(if n > 100_000_000_000 {
                    // > 1e11 只可能是毫秒（1e11 秒 = 公元 5138 年）
                    DateTime::from_timestamp_millis(n)?
                } else {
                    DateTime::from_timestamp(n, 0)?
                })
            }
            _ => None,
        },
    }
}

fn as_i64(raw: &Value) -> Option<i64> {
    match raw {
        Value::Number(n) => n.as_i64(),
        // 有些接口把秒数包成字符串
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn source_params(source: &TokenSource, status: Option<u16>) -> Value {
    let mut params = serde_json::json!({
        "sourceId": source.id,
        "sourceName": source.name,
    });
    if let Some(status) = status {
        params["status"] = serde_json::json!(status);
    }
    params
}

/// Token 相关的错误都带上 sourceId / sourceName。
///
/// 前端靠这两个值把错误卡片**指回** Token 源配置面板 ——
/// 只说「刷新失败」，用户还得自己去猜是哪一个 Token 源。
fn source_error(code: &'static str, debug: String, source: &TokenSource) -> AppError {
    AppError::http_with(code, debug, source_params(source, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::testkit::{seed_token_workspace, TestDb};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    // ============================================================
    // 进程内极简 token 服务
    // ============================================================
    //
    // 不引 wiremock（项目既有做法）。这里要断言的恰恰是**收到几次请求**，
    // 用一个会数数的真 socket 最直接。

    struct TokenServer {
        port: u16,
        hits: Arc<AtomicUsize>,
    }

    impl TokenServer {
        fn url(&self) -> String {
            format!("http://127.0.0.1:{}/oauth/token", self.port)
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }
    }

    /// 起一个永远回同一个 body 的 token 服务，并数它被打了多少次。
    fn token_server(body: &'static str) -> TokenServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let _ = read_request(&mut stream);
                counter.fetch_add(1, Ordering::SeqCst);
                let response = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: application/json\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        TokenServer { port, hits }
    }

    /// 回一个指定状态码的服务（错误路径用）
    fn error_server(status: &'static str) -> TokenServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let _ = read_request(&mut stream);
                counter.fetch_add(1, Ordering::SeqCst);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                     Content-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        TokenServer { port, hits }
    }

    fn read_request(stream: &mut TcpStream) -> String {
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            let Some(head_end) = find_subslice(&buf, b"\r\n\r\n") else {
                continue;
            };
            let head = String::from_utf8_lossy(&buf[..head_end]).to_lowercase();
            let want: usize = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            if buf.len() - (head_end + 4) >= want {
                break;
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack.windows(needle.len()).position(|w| w == needle)
    }

    // ============================================================
    // 临时库
    // ============================================================

    /// 一个装好工作区/文件夹/请求/Token 源的临时库。
    /// fetch 请求的 url 先占位成「必然连不上」的地址，需要时再用 `set_fetch_url` 改。
    fn db(tag: &str) -> TestDb {
        let db = TestDb::new(tag);
        seed_token_workspace(&db, "http://127.0.0.1:1/placeholder");
        db
    }

    fn set_fetch_url(pool: &SqlitePool, url: &str) {
        let conn = pool.get().expect("conn");
        conn.execute(
            "UPDATE request SET url = ?1 WHERE id = 1",
            rusqlite::params![url],
        )
        .expect("update url");
    }

    fn set_source(pool: &SqlitePool, sql_set: &str) {
        let conn = pool.get().expect("conn");
        conn.execute_batch(&format!("UPDATE token_source SET {sql_set} WHERE id = 1"))
            .expect("update source");
    }

    fn source(pool: &SqlitePool) -> TokenSource {
        let conn = pool.get().expect("conn");
        repo::token::get(&conn, 1).expect("source")
    }

    /// 写一条**已过期**的缓存，模拟「token 用完了」这一刻
    fn seed_expired_cache(pool: &SqlitePool) {
        let conn = pool.get().expect("conn");
        repo::token::upsert_cache(
            &conn,
            1,
            "stale-token",
            Some("stale-refresh"),
            Some("2020-01-01T00:00:00.000Z"),
            None,
        )
        .expect("seed cache");
    }

    fn seed_fresh_cache(pool: &SqlitePool) {
        let conn = pool.get().expect("conn");
        let far = (Utc::now() + Duration::hours(2)).to_rfc3339_opts(SecondsFormat::Millis, true);
        repo::token::upsert_cache(&conn, 1, "cached-token", None, Some(&far), None)
            .expect("seed cache");
    }

    fn env() -> HashMap<String, String> {
        HashMap::new()
    }

    // ============================================================
    // ★ 硬指标：并发只打一次刷新
    // ============================================================

    /// plan.md §4 验收第 3 条。这条是本阶段最核心的正确性验证。
    #[tokio::test]
    async fn concurrent_expired_requests_trigger_exactly_one_refresh() {
        let server = token_server(r#"{"access_token":"fresh","expires_in":3600}"#);
        let db = db("single-flight");
        set_fetch_url(&db.pool, &server.url());
        seed_expired_cache(&db.pool);

        let source = source(&db.pool);
        let env = env();
        let refresher = TokenRefresher::new();
        let clients = ClientCache::new();
        let deps = TokenDeps {
            pool: &db.pool,
            clients: &clients,
            source: &source,
            env: &env,
            cancel: None,
        };

        let (a, b, c, d, e) = tokio::join!(
            refresher.ensure(&deps, false),
            refresher.ensure(&deps, false),
            refresher.ensure(&deps, false),
            refresher.ensure(&deps, false),
            refresher.ensure(&deps, false),
        );

        let outcomes: Vec<TokenOutcome> = [a, b, c, d, e]
            .into_iter()
            .map(|result| result.expect("每次调用都该成功"))
            .collect();

        for (index, outcome) in outcomes.iter().enumerate() {
            assert_eq!(
                outcome.values.access_token, "fresh",
                "第 {index} 个并发请求拿到的 token 不对"
            );
        }
        assert_eq!(
            server.hits(),
            1,
            "并发 5 个请求只该触发 1 次刷新 —— 多了说明单飞锁没生效"
        );
        // ★ 只有一个人真的刷了：其余四个在检查二醒来时命中了它写下的缓存。
        // 这是「服务端只收到 1 次请求」在**结果**上的体现，也是界面敢说
        // 「本次命中缓存」的依据 —— 把五个人都报成「刷新过」会在
        // 并发场景下凭空多报四次网络往返。
        assert_eq!(
            outcomes.iter().filter(|o| o.refreshed).count(),
            1,
            "五个并发请求里只该有一个标记为「真刷新」"
        );
        assert_eq!(refresher.flight_count(), 1, "同一套配置只该有一把航班锁");
    }

    // ============================================================
    // ★ 一个接口一份：配置相同也各刷各的
    // ============================================================

    /// 再插一条来源给**另一个接口**（`create` 之外的列全落默认值）。
    ///
    /// 于是它与 id=1 那条**逐字段相同**，只有归属不同 —— 这正是
    /// 「两个接口配着同一条取 token 接口」这个最常见的形状。
    fn add_source_for_request_3(pool: &SqlitePool) -> TokenSource {
        let conn = pool.get().expect("conn");
        conn.execute(
            "INSERT INTO token_source (workspace_id, request_id, name, fetch_request_id,
                                       created_at, updated_at)
             VALUES (1, 3, 'twin', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        )
        .expect("insert twin");
        repo::token::get(&conn, conn.last_insert_rowid()).expect("twin")
    }

    fn deps<'a>(
        pool: &'a SqlitePool,
        clients: &'a ClientCache,
        source: &'a TokenSource,
        env: &'a HashMap<String, String>,
    ) -> TokenDeps<'a> {
        TokenDeps {
            pool,
            clients,
            source,
            env,
            cancel: None,
        }
    }

    /// 🔴 **两条逐字段相同、只是归属不同**的来源，各打一次网络、各存一份 token。
    ///
    /// 这条测试的语义在 2026-09-18 反过来了：原来它叫
    /// `sources_sharing_one_config_fetch_once_and_share_the_token`，钉的是
    /// 「配置相同就合并成一组」。现在的规则是**每个接口各自一份** ——
    /// 所以同样两行数据，期望值从「1 次网络 + 1 行缓存」变成
    /// 「2 次网络 + 2 行缓存」，且两个倒计时互不影响。
    ///
    /// 少了这一条，"各刷各的"可能被一个**永远只打一次**的实现蒙对 ——
    /// 而这种实现会让一个接口的刷新结果静默盖到另一个接口的头上。
    #[tokio::test]
    async fn requests_with_identical_configs_fetch_separately() {
        let server = token_server(r#"{"access_token":"fresh","expires_in":3600}"#);
        let db = db("same-config-two-requests");
        set_fetch_url(&db.pool, &server.url());
        seed_expired_cache(&db.pool); // id=1 那份缓存已经过期

        let a = source(&db.pool); // 归属 request 2
        let b = add_source_for_request_3(&db.pool); // 归属 request 3，配置逐字段相同
        assert_eq!(
            a.token_shape(),
            b.token_shape(),
            "两条来源的取 token 配置必须完全一致 —— 这是这条测试的前提"
        );

        let env = env();
        let refresher = TokenRefresher::new();
        let clients = ClientCache::new();

        // deps 必须先绑成变量：`tokio::join!` 里直接写 `&deps(...)` 的话，
        // 那个临时值在语句结束时就析构了，而 join 产生的 future 还借着它。
        let deps_a = deps(&db.pool, &clients, &a, &env);
        let deps_b = deps(&db.pool, &clients, &b, &env);
        let (first, second) = tokio::join!(
            refresher.ensure(&deps_a, false),
            refresher.ensure(&deps_b, false),
        );
        let first = first.expect("a");
        let second = second.expect("b");

        assert_eq!(first.values.access_token, "fresh");
        assert_eq!(second.values.access_token, "fresh");
        assert_eq!(
            server.hits(),
            2,
            "两个接口各有一份 token —— 只打一次说明被合并了，而合并是不要的行为"
        );
        assert_eq!(refresher.flight_count(), 2, "两条来源 = 两把航班锁");

        // 缓存各落各的行，谁也不读谁的
        let conn = db.pool.get().expect("conn");
        let row_a = repo::token::get_cache(&conn, a.id)
            .expect("read")
            .expect("a 名下该有缓存");
        let row_b = repo::token::get_cache(&conn, b.id)
            .expect("read")
            .expect("b 名下该有缓存");
        assert_eq!(row_a.token_source_id, a.id);
        assert_eq!(row_b.token_source_id, b.id);
    }

    /// 配置不同 ⇒ 走不同的提取路径，各自拿到自己那份值。
    ///
    /// 与上一条的区别在**断言的对象**：那条看"各打几次网络"，这条看
    /// "两条来源真的读到了不同的字段" —— 一个共用缓存的实现能让上一条
    /// 也看起来合理，但绕不过相邻两条来源拿到不同 token 这一关。
    #[tokio::test]
    async fn different_configs_read_different_paths() {
        let server = token_server(r#"{"access_token":"fresh","tok2":"other","expires_in":3600}"#);
        let db = db("two-paths");
        set_fetch_url(&db.pool, &server.url());

        let a = source(&db.pool); // 用默认的 $.access_token
        let mut b = add_source_for_request_3(&db.pool);
        b.access_path = "$.tok2".into(); // 换一个字段 ⇒ 换一套配置
        b = {
            let conn = db.pool.get().expect("conn");
            repo::token::save(&conn, &b).expect("save")
        };

        let env = env();
        let refresher = TokenRefresher::new();
        let clients = ClientCache::new();

        let first = refresher
            .ensure(&deps(&db.pool, &clients, &a, &env), false)
            .await
            .expect("a");
        let second = refresher
            .ensure(&deps(&db.pool, &clients, &b, &env), false)
            .await
            .expect("b");

        assert_eq!(first.values.access_token, "fresh", "a 走 $.access_token");
        assert_eq!(second.values.access_token, "other", "b 走 $.tok2");
        assert_eq!(server.hits(), 2, "各取一次");
        assert_eq!(refresher.flight_count(), 2, "两条来源 = 两把航班锁");
    }

    /// 缓存有效 → 一次请求都不发（design.md §4.3 第一行）
    #[tokio::test]
    async fn fresh_cache_short_circuits_without_any_request() {
        let server = token_server(r#"{"access_token":"fresh","expires_in":3600}"#);
        let db = db("fresh");
        set_fetch_url(&db.pool, &server.url());
        seed_fresh_cache(&db.pool);

        let source = source(&db.pool);
        let env = env();
        let refresher = TokenRefresher::new();
        let clients = ClientCache::new();
        let deps = TokenDeps {
            pool: &db.pool,
            clients: &clients,
            source: &source,
            env: &env,
            cancel: None,
        };

        let outcome = refresher.ensure(&deps, false).await.expect("hit cache");
        assert_eq!(outcome.values.access_token, "cached-token");
        assert!(
            !outcome.refreshed,
            "命中缓存必须报「没刷」—— 报「刷了」等于在界面上编造一次网络往返"
        );
        assert_eq!(server.hits(), 0, "缓存有效时不该发任何请求");
        assert_eq!(refresher.flight_count(), 0, "连航班锁都不该建");
    }

    /// 「立即刷新」要忽略缓存
    #[tokio::test]
    async fn force_refresh_ignores_a_fresh_cache() {
        let server = token_server(r#"{"access_token":"forced","expires_in":3600}"#);
        let db = db("force");
        set_fetch_url(&db.pool, &server.url());
        seed_fresh_cache(&db.pool);

        let source = source(&db.pool);
        let env = env();
        let refresher = TokenRefresher::new();
        let clients = ClientCache::new();
        let deps = TokenDeps {
            pool: &db.pool,
            clients: &clients,
            source: &source,
            env: &env,
            cancel: None,
        };

        let outcome = refresher.refresh_now(&deps).await.expect("force");
        assert_eq!(outcome.values.access_token, "forced");
        assert!(outcome.refreshed, "「立即刷新」必须报「刷了」");
        assert_eq!(server.hits(), 1);
    }

    /// 刷新成功后缓存要真的被换掉（下次不再打请求）
    #[tokio::test]
    async fn refresh_replaces_the_cache_row() {
        let server =
            token_server(r#"{"access_token":"n1","refresh_token":"r1","expires_in":3600}"#);
        let db = db("replace");
        set_fetch_url(&db.pool, &server.url());
        seed_expired_cache(&db.pool);

        let source = source(&db.pool);
        let env = env();
        let refresher = TokenRefresher::new();
        let clients = ClientCache::new();
        let deps = TokenDeps {
            pool: &db.pool,
            clients: &clients,
            source: &source,
            env: &env,
            cancel: None,
        };

        let first = refresher.ensure(&deps, false).await.expect("refresh");
        assert_eq!(first.values.access_token, "n1");
        assert_eq!(first.values.refresh_token.as_deref(), Some("r1"));
        assert!(first.values.expires_at.is_some(), "过期时间必须被算出来");
        assert!(first.refreshed, "缓存已过期 → 这次一定是真刷新");

        let rows: i64 = db
            .pool
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM token_cache", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "每个 Token 源只留一行缓存");

        // 第二次走缓存，不再打请求
        let second = refresher.ensure(&deps, false).await.expect("cached");
        assert_eq!(second.values.access_token, "n1");
        assert!(!second.refreshed);
        assert_eq!(server.hits(), 1);
    }

    /// 未开自动刷新 + 无可用缓存 → 明确报「需要手动刷」，而不是拿旧 token 硬发
    #[tokio::test]
    async fn expired_without_auto_refresh_reports_a_typed_error() {
        let db = db("no-auto");
        seed_expired_cache(&db.pool);
        set_source(&db.pool, "auto_refresh = 0");

        let source = source(&db.pool);
        let env = env();
        let refresher = TokenRefresher::new();
        let clients = ClientCache::new();
        let deps = TokenDeps {
            pool: &db.pool,
            clients: &clients,
            source: &source,
            env: &env,
            cancel: None,
        };

        let err = refresher.ensure(&deps, false).await.unwrap_err();
        assert_eq!(err.code(), codes::TOKEN_EXPIRED);
        assert_eq!(err.params().unwrap()["sourceName"], "oauth");

        // 但「立即刷新」是用户的显式动作，仍然要放行
        set_fetch_url(&db.pool, "http://127.0.0.1:1/x");
        let forced = refresher.ensure(&deps, true).await.unwrap_err();
        assert_eq!(forced.code(), codes::TOKEN_REFRESH);
    }

    // ============================================================
    // 错误路径
    // ============================================================

    #[tokio::test]
    async fn non_2xx_token_response_is_a_refresh_failure() {
        let server = error_server("401 Unauthorized");
        let db = db("401");
        set_fetch_url(&db.pool, &server.url());
        seed_expired_cache(&db.pool);

        let source = source(&db.pool);
        let env = env();
        let refresher = TokenRefresher::new();
        let clients = ClientCache::new();
        let deps = TokenDeps {
            pool: &db.pool,
            clients: &clients,
            source: &source,
            env: &env,
            cancel: None,
        };

        let err = refresher.ensure(&deps, false).await.unwrap_err();
        assert_eq!(
            err.code(),
            codes::TOKEN_REFRESH,
            "非 2xx 必须当成刷新失败，而不是拿错误体当 token"
        );
        assert_eq!(err.params().unwrap()["status"], 401);
        assert!(!err.retryable(), "401 重试没用，不该给重试按钮");
    }

    /// 刷新失败**不写缓存**：下一次请求必须还能再试
    #[tokio::test]
    async fn failed_refresh_does_not_poison_the_cache() {
        let server = error_server("500 Internal Server Error");
        let db = db("fail-nocache");
        set_fetch_url(&db.pool, &server.url());
        seed_expired_cache(&db.pool);

        let source = source(&db.pool);
        let env = env();
        let refresher = TokenRefresher::new();
        let clients = ClientCache::new();
        let deps = TokenDeps {
            pool: &db.pool,
            clients: &clients,
            source: &source,
            env: &env,
            cancel: None,
        };

        let _ = refresher.ensure(&deps, false).await.unwrap_err();
        let conn = db.pool.get().unwrap();
        let cached = repo::token::get_cache(&conn, 1).unwrap().unwrap();
        assert_eq!(cached.access_token, "stale-token", "失败不该覆盖旧缓存");

        // 失败不合并：第二个请求会自己再试一次
        let _ = refresher.ensure(&deps, false).await.unwrap_err();
        assert_eq!(server.hits(), 2, "失败路径上每个请求各自重试");
    }

    #[tokio::test]
    async fn missing_access_path_is_an_extract_failure() {
        let server = token_server(r#"{"data":{"token":"nested"}}"#);
        let db = db("extract");
        set_fetch_url(&db.pool, &server.url());
        set_source(&db.pool, "access_path = '$.nope'");
        seed_expired_cache(&db.pool);

        let source = source(&db.pool);
        let env = env();
        let refresher = TokenRefresher::new();
        let clients = ClientCache::new();
        let deps = TokenDeps {
            pool: &db.pool,
            clients: &clients,
            source: &source,
            env: &env,
            cancel: None,
        };

        let err = refresher.ensure(&deps, false).await.unwrap_err();
        assert_eq!(err.code(), codes::TOKEN_EXTRACT);
        assert_eq!(server.hits(), 1, "提取失败之前请求已经发出去了");
    }

    /// 配了 `expires_in_path` 却取不到 → 必须报错，不能静默变成「永不过期」
    #[tokio::test]
    async fn configured_but_unresolvable_expiry_is_an_error() {
        let server = token_server(r#"{"access_token":"a"}"#);
        let db = db("expiry-miss");
        set_fetch_url(&db.pool, &server.url());
        set_source(&db.pool, "expires_in_path = '$.expires_in'");
        seed_expired_cache(&db.pool);

        let source = source(&db.pool);
        let env = env();
        let refresher = TokenRefresher::new();
        let clients = ClientCache::new();
        let deps = TokenDeps {
            pool: &db.pool,
            clients: &clients,
            source: &source,
            env: &env,
            cancel: None,
        };

        let err = refresher.ensure(&deps, false).await.unwrap_err();
        assert_eq!(
            err.code(),
            codes::TOKEN_EXTRACT,
            "静默退化成「永不过期」会让 token 永远不再刷新"
        );
    }

    /// 不配 `expires_in_path` 是「永不过期」的显式选择
    #[tokio::test]
    async fn empty_expiry_path_means_never_expires() {
        let server = token_server(r#"{"access_token":"eternal"}"#);
        let db = db("eternal");
        set_fetch_url(&db.pool, &server.url());
        set_source(&db.pool, "expires_in_path = ''");
        seed_expired_cache(&db.pool);

        let source = source(&db.pool);
        let env = env();
        let refresher = TokenRefresher::new();
        let clients = ClientCache::new();
        let deps = TokenDeps {
            pool: &db.pool,
            clients: &clients,
            source: &source,
            env: &env,
            cancel: None,
        };

        let outcome = refresher.ensure(&deps, false).await.expect("refresh");
        assert_eq!(outcome.values.access_token, "eternal");
        assert_eq!(outcome.values.expires_at, None);

        // 再取一次走缓存（expires_at = None → is_expired 恒 false）
        let again = refresher.ensure(&deps, false).await.expect("cached");
        assert_eq!(again.values.access_token, "eternal");
        assert_eq!(server.hits(), 1, "永不过期的缓存不该触发第二次请求");
    }

    /// 取 token 的请求里可以写 `{{env.x}}` —— 它要被解析
    #[tokio::test]
    async fn fetch_request_placeholders_are_resolved_with_env() {
        let server = token_server(r#"{"access_token":"from-env","expires_in":600}"#);
        let db = db("env-resolve");
        set_fetch_url(&db.pool, "{{env.tokenUrl}}");
        seed_expired_cache(&db.pool);

        let source = source(&db.pool);
        let env = HashMap::from([("tokenUrl".to_string(), server.url())]);
        let refresher = TokenRefresher::new();
        let clients = ClientCache::new();
        let deps = TokenDeps {
            pool: &db.pool,
            clients: &clients,
            source: &source,
            env: &env,
            cancel: None,
        };

        let outcome = refresher.ensure(&deps, false).await.expect("refresh");
        assert_eq!(outcome.values.access_token, "from-env");
        assert_eq!(server.hits(), 1);
    }

    // ============================================================
    // 纯函数：过期判定与换算
    // ============================================================

    fn cache_with(expires_at: Option<&str>) -> TokenCache {
        TokenCache {
            id: 1,
            token_source_id: 1,
            access_token: "t".into(),
            refresh_token: None,
            expires_at: expires_at.map(str::to_string),
            raw_response: None,
            updated_at: "2026-01-01T00:00:00.000Z".into(),
        }
    }

    fn at(iso: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(iso)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// 剩余秒数与 `is_expired` **同源解析**，读不懂就是 `None`。
    ///
    /// 这条钉的是「界面上的倒计时」：给它 `0` 会让倒计时显示成「马上刷新」，
    /// 而真实原因只是那个 ISO 串读不出来 —— 用户会一直等一个永远不会发生的刷新。
    #[test]
    fn remaining_seconds_is_derived_from_the_same_parse_as_expiry() {
        let now = at("2026-09-11T10:00:00Z");

        assert_eq!(
            remaining_seconds(Some("2026-09-11T10:05:00Z"), now),
            Some(300)
        );
        assert_eq!(
            remaining_seconds(Some("2026-09-11T09:59:00Z"), now),
            Some(-60),
            "已经过期给负数 —— 界面靠它说「已过期 1 分钟」"
        );
        assert_eq!(remaining_seconds(None, now), None, "永不过期没有倒计时");
        assert_eq!(
            remaining_seconds(Some("not-a-time"), now),
            None,
            "读不懂的时间戳不给数字，别报成「马上刷新」"
        );
    }

    #[test]
    fn skew_seconds_makes_a_token_expire_early() {
        let now = at("2026-09-11T10:00:00Z");

        // 还有 60 秒，提前 30 秒 → 仍有效
        assert!(!is_expired(
            &cache_with(Some("2026-09-11T10:01:00Z")),
            30,
            now
        ));
        // 还有 20 秒，提前 30 秒 → 已经算过期
        assert!(is_expired(
            &cache_with(Some("2026-09-11T10:00:20Z")),
            30,
            now
        ));
        // 恰好到 skew 边界（now + 30 == expires_at）→ 过期
        assert!(is_expired(
            &cache_with(Some("2026-09-11T10:00:30Z")),
            30,
            now
        ));
        // 已经过去了
        assert!(is_expired(
            &cache_with(Some("2026-09-11T09:59:00Z")),
            30,
            now
        ));
    }

    #[test]
    fn cache_without_expiry_never_expires() {
        assert!(!is_expired(&cache_with(None), 30, Utc::now()));
    }

    /// 读不懂的过期时间按过期处理 —— 宁可多刷一次
    #[test]
    fn unparseable_expiry_counts_as_expired() {
        assert!(is_expired(&cache_with(Some("not-a-time")), 0, Utc::now()));
    }

    #[test]
    fn relative_units_are_added_to_now() {
        let now = at("2026-09-11T10:00:00Z");
        assert_eq!(
            absolute_expiry(&serde_json::json!(3600), ExpiryUnit::Seconds, now),
            Some(at("2026-09-11T11:00:00Z"))
        );
        assert_eq!(
            absolute_expiry(&serde_json::json!(1500), ExpiryUnit::Millis, now),
            Some(at("2026-09-11T10:00:01.500Z"))
        );
        // 秒数被包成字符串也认
        assert_eq!(
            absolute_expiry(&serde_json::json!("60"), ExpiryUnit::Seconds, now),
            Some(at("2026-09-11T10:01:00Z"))
        );
    }

    #[test]
    fn absolute_unit_accepts_iso_string_and_numbers() {
        let now = at("2026-09-11T10:00:00Z");

        assert_eq!(
            absolute_expiry(
                &serde_json::json!("2026-09-11T12:00:00Z"),
                ExpiryUnit::Absolute,
                now
            ),
            Some(at("2026-09-11T12:00:00Z"))
        );

        let seconds = at("2026-09-11T12:00:00Z").timestamp();
        assert_eq!(
            absolute_expiry(&serde_json::json!(seconds), ExpiryUnit::Absolute, now),
            Some(at("2026-09-11T12:00:00Z"))
        );

        let millis = at("2026-09-11T12:00:00Z").timestamp_millis();
        assert_eq!(
            absolute_expiry(&serde_json::json!(millis), ExpiryUnit::Absolute, now),
            Some(at("2026-09-11T12:00:00Z"))
        );

        assert_eq!(
            absolute_expiry(&serde_json::json!("nope"), ExpiryUnit::Absolute, now),
            None
        );
    }
}
