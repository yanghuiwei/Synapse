//! 请求执行流水线（design.md §4.1 的第 1–3 步）。
//!
//! ```text
//!   用户点 Send
//!     ├─1 取请求定义 + 当前环境（激活环境的变量）
//!     ├─2 按请求 id 取「上下文」：Token 源
//!     ├─3 需要 token 就先确保就绪  ← 这一步可能打一次刷新请求（单飞锁在这儿）
//!     ├─4 解析所有字符串字段上的占位符
//!     └─5 交给 http::execute 发送
//! ```
//!
//! ## 为什么先把「要不要 token」扫一遍，而不是直接解析
//!
//! 解析 `{{token.access}}` 需要 token 已经就绪，而拿 token 是一个**网络动作**。
//! 所以顺序只能是「先扫出来要不要 → 要就去拿 → 再解析」。
//! 扫描用的是 `template::resolver::scan_spec`，与解析**共用同一份字段清单**——
//! 两份清单一旦漂移，就会出现「扫描说不用 token、解析时却发现用上了」，
//! 表现为请求带着 `{{token.access}}` 原样发出去。
//!
//! ## `{{resp.*}}` 是「上一条成功响应的 JSON」
//!
//! 由 `AppState.last_response` 提供（进程级、最近一次）。响应不是 JSON 时它会被
//! **清空**而不是留着上一次的值 —— 留着就会出现「你看到的响应和刚发的那次无关」
//! 这种最难查的错。

use std::collections::HashMap;

use chrono::{Local, Utc};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::error::AppResult;
use crate::http::client::ClientCache;
use crate::http::execute::RequestSpec;
use crate::http::token::{remaining_seconds, TokenDeps, TokenRefresher, TokenUsage};
use crate::storage::models::TokenSource;
use crate::storage::pool::SqlitePool;
use crate::storage::{repo, with_conn};
use crate::template::functions::Entropy;
use crate::template::resolver::{
    resolve_spec, scan_spec, Namespace, TemplateContext, TokenValues, UnresolvedPlaceholder,
};

/// 流水线的依赖。抽成结构而不是直接吃 `AppState`：单测只要凑齐这三样。
pub struct Pipeline<'a> {
    pub pool: &'a SqlitePool,
    pub clients: &'a ClientCache,
    pub tokens: &'a TokenRefresher,
}

/// 本次发送的 Token 上下文：解析要用的值 + 给用户看的使用报告。
///
/// 合成一个结构而不是让 `token_context` 返回元组：两半一个进模板、一个进响应，
/// 元组解构时写反了编译器不拦（都是 `Option<...>` 起的名字），
/// 症状是「响应里报的 Token 来源和实际用的不是一个」。
struct TokenContext {
    /// 解析 `{{token.*}}` 用的值（含 access / refresh / expiresAt）
    values: TokenValues,
    /// 回给界面的那份报告
    usage: TokenUsage,
}

/// 一份可以发出去的请求 + 本次的占位符报告 + Token 使用报告。
#[derive(Debug)]
pub struct Prepared {
    pub spec: RequestSpec,
    pub unresolved: Vec<UnresolvedPlaceholder>,
    /// 本次发送的 Token 使用情况。`None` 有两种含义，对使用者是同一件事：
    /// ① 这条请求压根没写 `{{token.*}}`；② 写了但文件夹链上没有 Token 源
    /// （后者会同时报一条 `noTokenSource` 的未解析项，用户看得见该去配）。
    pub token: Option<TokenUsage>,
}

impl<'a> Pipeline<'a> {
    pub fn new(pool: &'a SqlitePool, clients: &'a ClientCache, tokens: &'a TokenRefresher) -> Self {
        Self {
            pool,
            clients,
            tokens,
        }
    }

    /// 把「前端传来的草稿」变成「可以直接发送的请求」。
    ///
    /// `resp` 是 `{{resp.$.path}}` 的来源（上一条成功响应）。
    /// `cancel` 会贯穿到刷新请求上 —— 用户按取消时，正在跑的刷新也该停。
    pub async fn prepare(
        &self,
        spec: &RequestSpec,
        resp: Option<&Value>,
        cancel: Option<&CancellationToken>,
    ) -> AppResult<Prepared> {
        let env = self.active_env().await?;

        let token_ctx = self.token_context(spec, &env, cancel).await?;

        let mut ctx = TemplateContext {
            env: &env,
            token: token_ctx.as_ref().map(|t| &t.values),
            resp,
            now: Local::now(),
            entropy: Entropy::from_system(),
        };
        let resolved = resolve_spec(spec, &mut ctx);

        Ok(Prepared {
            spec: resolved.spec,
            unresolved: resolved.unresolved,
            token: token_ctx.map(|t| t.usage),
        })
    }

    /// 激活环境的变量表。
    ///
    /// 工作区由后端自己查（这一版只有一个），前端不必把它塞进请求定义里 ——
    /// 它是「当前会话的状态」而不是「这条请求的属性」。
    async fn active_env(&self) -> AppResult<HashMap<String, String>> {
        let pool = self.pool;
        with_conn(pool, |conn| {
            let Some(workspace) = repo::workspace::active(conn)? else {
                // 还没播种工作区（首次启动的极短窗口）→ 空环境，
                // `{{env.x}}` 会照常报「变量不存在」，不会把请求搞坏
                return Ok(HashMap::new());
            };
            repo::env::active_vars(conn, workspace.id)
        })
        .await
    }

    /// 判断这次发送用不用得上 token，用得上就先确保就绪。
    ///
    /// **用不上时不碰 Token 源**：仅仅因为「这个文件夹里有 Token 源」就去打一次
    /// 刷新请求，会让每个不相关的请求都背上一次网络往返。
    async fn token_context(
        &self,
        spec: &RequestSpec,
        env: &HashMap<String, String>,
        cancel: Option<&CancellationToken>,
    ) -> AppResult<Option<TokenContext>> {
        let wants_token = scan_spec(spec)
            .iter()
            .any(|hit| hit.namespace == Namespace::Token);
        if !wants_token {
            return Ok(None);
        }

        let source = self.find_token_source(spec.request_id).await?;
        let Some(source) = source else {
            // 没有 Token 源**不是错误**：`{{token.access}}` 会被原样保留并报
            // 「没有可用 Token 源」，用户看得见也就知道该去配了。
            // 这与「刷新失败要中止请求」是两回事。
            return Ok(None);
        };

        let deps = TokenDeps {
            pool: self.pool,
            clients: self.clients,
            source: &source,
            env,
            cancel,
        };
        // ★ 刷新失败在这里冒泡上去 → 中止整条请求（plan.md §4 验收第 5 条）。
        // 错误里带着 sourceId / sourceName，前端据此把卡片指回 Token 源配置。
        let outcome = self.tokens.ensure(&deps, false).await?;

        // 生效的取 Token 接口：与 `fetch` 的选法**必须一致**
        // （`refresh_request_id` 优先，见 `http::token::fetch`）——
        // 这里是「界面上说刷的是哪个」，那边是「实际刷的是哪个」，
        // 两处算错一个就会让用户照着界面去改错误的那个接口。
        let endpoint_request_id = source.refresh_request_id.unwrap_or(source.fetch_request_id);

        let usage = TokenUsage {
            source_id: source.id,
            source_name: source.name.clone(),
            refreshed: outcome.refreshed,
            endpoint_request_id,
            endpoint_name: self.endpoint_name(endpoint_request_id).await,
            remaining_seconds: remaining_seconds(outcome.values.expires_at.as_deref(), Utc::now()),
            expires_at: outcome.values.expires_at.clone(),
            auto_refresh: source.auto_refresh,
        };

        Ok(Some(TokenContext {
            values: outcome.values,
            usage,
        }))
    }

    /// 那个取 Token 的接口叫什么名字。**读不出来就是 `None`**。
    ///
    /// 为什么不冒泡错误：它只服务于界面上的一行说明。为了多显示一个接口名，
    /// 把一次本来能成功的业务请求打回去，代价完全不成比例 ——
    /// 「这个源配得好好的、就因为它引用的接口刚被删掉，所有请求都发不出去了」
    /// 是最糟的一类连带失败。
    async fn endpoint_name(&self, request_id: i64) -> Option<String> {
        let pool = self.pool;
        with_conn(pool, move |conn| repo::request::get(conn, request_id))
            .await
            .ok()
            .map(|request| request.name)
    }

    /// 这次发送用哪个 Token 源：**就是这条请求自己配的那一份**。
    ///
    /// 2026-09-18 起的规则（plan.md §35）：Token 源逐请求一份，**没有**分类继承、
    /// 也**没有**全局兜底。以前的两级查找（先沿文件夹链找最近的、再退到系统分类
    /// 「token刷新」下的那一个）被整个删掉了 —— 它会让「A 接口的 token 悄悄被
    /// B 接口用上」，而那正是最难看出来的那类串号。迁移见 `008`。
    ///
    /// `request_id` 为空 → `None`：未保存的草稿就是这样。这时
    /// `{{token.access}}` 会被原样保留并报「没有可用 Token 源」，
    /// 与「新请求还没配源」表现一致，不会静默借用别人的 token。
    async fn find_token_source(&self, request_id: Option<i64>) -> AppResult<Option<TokenSource>> {
        let Some(request_id) = request_id else {
            return Ok(None);
        };
        let pool = self.pool;
        with_conn(pool, move |conn| repo::token::by_request(conn, request_id)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::{KeyValueItem, RequestSettings};
    use crate::storage::testkit::{seed_token_workspace, TestDb};
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    struct Counter {
        port: u16,
        hits: Arc<AtomicUsize>,
    }

    fn token_server() -> Counter {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);

        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let _ = read_request(&mut stream);
                counter.fetch_add(1, Ordering::SeqCst);
                let body = r#"{"access_token":"tok","expires_in":3600}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        Counter { port, hits }
    }

    fn read_request(stream: &mut TcpStream) -> String {
        use std::io::Read;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    }

    fn spec() -> RequestSpec {
        RequestSpec {
            method: "GET".into(),
            url: "https://api.example.com/x".into(),
            params: Vec::new(),
            headers: Vec::new(),
            body_type: crate::storage::models::BodyType::None,
            body: String::new(),
            settings: RequestSettings::default(),
            request_id: None,
            form_fields: Vec::new(),
        }
    }

    fn pipeline_ctx(tag: &str) -> (TestDb, TokenRefresher) {
        let db = TestDb::new(tag);
        seed_token_workspace(&db, "http://127.0.0.1:1/placeholder");
        (db, TokenRefresher::new())
    }

    #[tokio::test]
    async fn resolves_env_placeholders_from_the_active_environment() {
        let (db, tokens) = pipeline_ctx("pipe-env");
        db.exec(
            "INSERT INTO environment (id, workspace_id, name, is_active, created_at, updated_at)
               VALUES (1, 1, 'dev', 1, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO env_var (environment_id, key, value, is_secret, enabled, sort_order)
               VALUES (1, 'baseUrl', 'https://dev.example.com', 0, 1, 0),
                      (1, 'gone', 'should-not-resolve', 0, 0, 1);",
        );

        let clients = ClientCache::new();
        let pipeline = Pipeline::new(&db.pool, &clients, &tokens);
        let mut s = spec();
        s.url = "{{env.baseUrl}}/a/{{env.gone}}".into();

        let prepared = pipeline.prepare(&s, None, None).await.expect("prepare");
        assert_eq!(prepared.spec.url, "https://dev.example.com/a/{{env.gone}}");
        assert_eq!(
            prepared.unresolved.len(),
            1,
            "停用的变量必须报出来，而不是解析成功"
        );
        assert_eq!(prepared.unresolved[0].reason, "envNotFound");
    }

    /// 这条是「用不上就不碰 Token 源」的钉子：这条请求上有 Token 源，
    /// 但请求里没有 `{{token.*}}` → 一次刷新请求都不该发
    #[tokio::test]
    async fn does_not_touch_the_token_source_when_no_token_placeholder_is_used() {
        let server = token_server();
        let (db, tokens) = pipeline_ctx("pipe-no-token");
        db.exec(&format!(
            "UPDATE request SET url = 'http://127.0.0.1:{}/t' WHERE id = 1",
            server.port
        ));

        let clients = ClientCache::new();
        let pipeline = Pipeline::new(&db.pool, &clients, &tokens);
        let mut s = spec();
        s.request_id = Some(2); // 它名下就是 token_source id=1

        let prepared = pipeline.prepare(&s, None, None).await.expect("prepare");
        assert_eq!(prepared.spec.url, "https://api.example.com/x");
        assert_eq!(server.hits.load(Ordering::SeqCst), 0, "不该有刷新请求");
    }

    #[tokio::test]
    async fn injects_the_token_from_the_requests_own_source() {
        let server = token_server();
        let (db, tokens) = pipeline_ctx("pipe-token");
        db.exec(&format!(
            "UPDATE request SET url = 'http://127.0.0.1:{}/t' WHERE id = 1",
            server.port
        ));

        let clients = ClientCache::new();
        let pipeline = Pipeline::new(&db.pool, &clients, &tokens);
        let mut s = spec();
        s.request_id = Some(2);
        s.headers = vec![KeyValueItem::new(
            "Authorization",
            "Bearer {{token.access}}",
        )];

        let prepared = pipeline.prepare(&s, None, None).await.expect("prepare");
        assert_eq!(prepared.spec.headers[0].value, "Bearer tok");
        assert!(prepared.unresolved.is_empty());
        assert_eq!(server.hits.load(Ordering::SeqCst), 1);
    }

    /// ★ 逐请求隔离的钉子。
    ///
    /// request 2 和 request 3 **在同一个分类里**，而 Token 源挂在 request 2 名下。
    /// 008 之前这里是「沿分类链向上找 + 系统分类兜底」，request 3 会**静默**
    /// 拿到 request 2 的那份 token；现在它必须一无所获。
    ///
    /// 少了这一条，把 `by_request` 换成任何"宽松"的实现（按分类找、取第一条找到的）
    /// 都能让上面两条测试照样绿 —— 而症状是「B 接口用了 A 接口的凭据」，
    /// 只在服务端 401 / 越权时才暴露，界面上看不出任何异常。
    #[tokio::test]
    async fn a_neighbouring_request_in_the_same_folder_does_not_inherit_the_source() {
        let server = token_server();
        let (db, tokens) = pipeline_ctx("pipe-neighbour");
        db.exec(&format!(
            "UPDATE request SET url = 'http://127.0.0.1:{}/t' WHERE id = 1",
            server.port
        ));

        let clients = ClientCache::new();
        let pipeline = Pipeline::new(&db.pool, &clients, &tokens);
        let mut s = spec();
        s.request_id = Some(3); // 与 request 2 同属分类 1，但自己没配源
        s.headers = vec![KeyValueItem::new(
            "Authorization",
            "Bearer {{token.access}}",
        )];

        let prepared = pipeline.prepare(&s, None, None).await.expect("不该失败");
        assert_eq!(
            prepared.spec.headers[0].value, "Bearer {{token.access}}",
            "同分类的另一条请求的 token 绝不能被继承过来"
        );
        assert_eq!(prepared.unresolved.len(), 1);
        assert_eq!(prepared.unresolved[0].reason, "noTokenSource");
        assert_eq!(server.hits.load(Ordering::SeqCst), 0, "不该替它去拿 token");
    }

    /// 没有 Token 源时 `{{token.access}}` 不该让请求失败，只留一条警告
    #[tokio::test]
    async fn missing_token_source_yields_a_warning_not_an_error() {
        let (db, tokens) = pipeline_ctx("pipe-no-source");
        let clients = ClientCache::new();
        let pipeline = Pipeline::new(&db.pool, &clients, &tokens);

        let mut s = spec();
        // request_id 为 None → 未保存的草稿 → 找不到 Token 源
        s.headers = vec![KeyValueItem::new(
            "Authorization",
            "Bearer {{token.access}}",
        )];

        let prepared = pipeline.prepare(&s, None, None).await.expect("不该失败");
        assert_eq!(prepared.spec.headers[0].value, "Bearer {{token.access}}");
        assert_eq!(prepared.unresolved.len(), 1);
        assert_eq!(prepared.unresolved[0].reason, "noTokenSource");
    }

    /// 刷新失败必须**中止**整条请求，并带上指向配置的信息
    #[tokio::test]
    async fn refresh_failure_aborts_the_request() {
        let (db, tokens) = pipeline_ctx("pipe-refresh-fail");
        // url 指向一个必然连不上的端口
        db.exec("UPDATE request SET url = 'http://127.0.0.1:1/t' WHERE id = 1");

        let clients = ClientCache::new();
        let pipeline = Pipeline::new(&db.pool, &clients, &tokens);
        let mut s = spec();
        s.request_id = Some(2);
        s.url = "{{token.access}}".into();

        let err = pipeline.prepare(&s, None, None).await.unwrap_err();
        assert_eq!(err.code(), crate::error::codes::TOKEN_REFRESH);
        let params = err.params().unwrap();
        assert_eq!(params["sourceId"], 1);
        assert_eq!(params["sourceName"], "oauth", "前端靠它把卡片指回配置面板");
    }

    #[tokio::test]
    async fn resp_namespace_reads_the_last_response() {
        let (db, tokens) = pipeline_ctx("pipe-resp");
        let clients = ClientCache::new();
        let pipeline = Pipeline::new(&db.pool, &clients, &tokens);

        let mut s = spec();
        s.url = "https://api.example.com/{{resp.$.data.id}}".into();
        let resp = serde_json::json!({ "data": { "id": 42 } });

        let prepared = pipeline
            .prepare(&s, Some(&resp), None)
            .await
            .expect("prepare");
        assert_eq!(prepared.spec.url, "https://api.example.com/42");

        // 没有响应时给的是「还没有响应」而不是「路径不存在」—— 两种提示指向的动作不同
        let prepared = pipeline.prepare(&s, None, None).await.expect("prepare");
        assert_eq!(
            prepared.spec.url,
            "https://api.example.com/{{resp.$.data.id}}"
        );
        assert_eq!(prepared.unresolved[0].reason, "noResponse");
    }
}
