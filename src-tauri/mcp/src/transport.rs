//! MCP 的两条通道（design.md §8.1）。
//!
//! | 通道 | 谁在用 | 为什么 |
//! | --- | --- | --- |
//! | stdio | AI 客户端 spawn 这个进程（Claude Code / Cursor / WorkBuddy / Hermes） | 最稳：没有端口、没有监听、没有鉴权问题，进程活多久通道就活多久 |
//! | 回环 HTTP | 独立进程被同机多个客户端连 | 一次启动多个客户端共用；仍然只绑 `127.0.0.1` |
//!
//! ## stdout 是协议流，不是日志出口
//!
//! stdio 通道下 stdout 上跑的是一行一行的 JSON-RPC。往里写一个字节的非协议内容，
//! 客户端的解析器就会在那一行崩掉、握手失败，而报错信息通常只说「invalid JSON」
//! —— 排查起来像玄学。所以本模块与整个 `mcp` 模块的日志一律走 **stderr**
//! （`AppState::new` 里的启动日志已经是这么做的）。
//!
//! ## 为什么 HTTP 那边用 `axum::serve` 而不是自己写 hyper 循环
//!
//! `StreamableHttpService` 是一个 tower service，`axum` 已经把它需要的
//! 「监听 → 每连接一个 task → 优雅退出」做完了。自己接一遍只会多出一份
//! 可能忘记处理 `Accept`/`Content-Type` 的胶水代码（SSE 与 JSON 两条返回路径
//! 由那个 service 自己按请求头选）。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::ServiceExt;

use crate::server::ReqlyMcp;
use reqly_core::error::{AppError, AppResult};
use reqly_core::AppState;

/// 回环 HTTP 的默认端口与路径，与 design.md §8.1 写的一致。
pub const DEFAULT_HTTP_PORT: u16 = 7777;
pub const HTTP_PATH: &str = "/mcp";

/// 跑 stdio 通道，阻塞到客户端断开。
///
/// 返回值只在「握手就失败了」和「正常结束」两种情况出现 —— 前者要让进程以
/// 非零码退出（客户端会据此报「server 启动失败」），后者安静退出即可。
pub async fn serve_stdio(state: Arc<AppState>, read_only: bool) -> AppResult<()> {
    let service = ReqlyMcp::new(state, read_only)
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| AppError::Internal(format!("MCP stdio handshake failed: {e}")))?;

    // `waiting()` 等的是服务自己那个后台任务：客户端关掉 stdin 时它会收尾退出。
    service
        .waiting()
        .await
        .map_err(|e| AppError::Internal(format!("MCP stdio task panicked: {e}")))?;

    Ok(())
}

/// 跑回环 HTTP 通道，阻塞到进程被 Ctrl-C（或父进程）结束。
///
/// **只绑 `Ipv4Addr::LOCALHOST`**（design.md §8.3）：这条通道没有鉴权，
/// 一旦绑到 `0.0.0.0` 就等于把整个工作台（含所有已存的请求定义）交给同网段。
/// 所以这里不接受「绑定地址」参数，端口之外无从配置 —— 少一个能配错的东西。
pub async fn serve_http(state: Arc<AppState>, read_only: bool, port: u16) -> AppResult<()> {
    let mut config = StreamableHttpServerConfig::default();
    // 默认值本来就是回环三件套，这里显式覆盖一遍：它是 §8.3 的安全边界，
    // 不该依赖「上游的默认值将来不会被改」。
    config.allowed_hosts = vec!["127.0.0.1".into(), "localhost".into(), "::1".into()];

    // 每个会话一份 `ReqlyMcp`。`Tools` 里是 `Arc<AppState>` + bool，克隆很廉价；
    // 而会话之间共享同一个 `AppState` 正是我们要的（连接池、Token 航班锁、压测占坑
    // 都必须是进程级唯一，理由见 `lib.rs` 里 `AppState` 各字段的注释）。
    let factory = {
        let state = Arc::clone(&state);
        move || Ok(ReqlyMcp::new(Arc::clone(&state), read_only))
    };
    let service: StreamableHttpService<ReqlyMcp, LocalSessionManager> =
        StreamableHttpService::new(factory, Arc::new(LocalSessionManager::default()), config);

    let router = axum::Router::new().nest_service(HTTP_PATH, service);
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| with_context(e, format!("cannot bind MCP http listener on {addr}")))?;

    eprintln!("[reqly-mcp] listening on http://{addr}{HTTP_PATH} (loopback only)");

    axum::serve(listener, router)
        .await
        .map_err(|e| with_context(e, "MCP http server stopped".into()))?;

    Ok(())
}

/// 给 `io::Error` 补上「在做什么」的上下文，同时**保留 `ErrorKind`**。
///
/// `AppError::Io(#[from] std::io::Error)` 只留下原始错误，而
/// 「端口被占用」和「端口被防火墙拦」在 `ErrorKind` 上就已经不同了 ——
/// 把它换成 `Internal(String)` 会顺手把这个信息扔掉，错误码也从 `E_IO` 退化成 `E_INTERNAL`。
fn with_context(error: std::io::Error, what: String) -> AppError {
    AppError::Io(std::io::Error::new(
        error.kind(),
        format!("{what}: {error}"),
    ))
}
