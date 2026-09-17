//! P2 验收（真实公网）：把「一次请求闭环」对着 httpbin.org 走一遍。
//!
//! 对照 `docs/plan.md` §3 的 P2 验收标准：
//!   1. `GET https://httpbin.org/get?x=1` → 200 / 耗时合理 / JSON 能解析
//!   2. `POST /post` 带 JSON body → 200，回显里能看到我们发出去的内容
//!   3. 不存在的域名 → `E_DNS`
//!   4. 取消能真的打断一个卡住的请求
//!
//! 默认 `#[ignore]`：这几条依赖公网可达性和 httpbin 的响应速度，
//! 塞进常规 `cargo test` 只会让 CI 随机翻红。要跑就显式来：
//!
//! ```text
//! cargo test --test http_live -- --ignored --test-threads=1
//! ```
//!
//! 为什么已经有进程内单测了还要这一层：单测里的"服务器"是我们自己写的，
//! 它证明得了「组装 / 解析 / 错误分类」的逻辑对，证明不了
//! 「真的连上了一个外部站点、真的走完了 TLS、真的读回了 chunked 响应」。

use std::time::{Duration, Instant};

use reqly_lib::error::codes;
use reqly_lib::http::{execute, execute_with_cancel, BodyEncoding, ClientCache, RequestSpec};
use reqly_lib::storage::models::{AuthConfig, BodyType, KeyValueItem, RequestSettings};
use tokio_util::sync::CancellationToken;

/// 公网请求一律给足时间：httpbin 偶尔要好几秒，因为它慢而报红是假警报
fn settings() -> RequestSettings {
    RequestSettings {
        timeout_ms: 25_000,
        ..RequestSettings::default()
    }
}

fn spec(method: &str, url: &str) -> RequestSpec {
    RequestSpec {
        method: method.to_string(),
        url: url.to_string(),
        params: Vec::new(),
        headers: Vec::new(),
        body_type: BodyType::None,
        body: String::new(),
        form_fields: Vec::new(),
        auth: AuthConfig::None,
        settings: settings(),
        folder_id: None,
    }
}

fn client(settings: &RequestSettings) -> reqwest::Client {
    ClientCache::new().get(settings).expect("构造 client")
}

/// httpbin 回显出来的 header 名大小写不保证，按大小写无关地找
fn find_header<'a>(headers: &'a serde_json::Value, name: &str) -> &'a str {
    headers
        .as_object()
        .and_then(|map| {
            map.iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value)
        })
        .and_then(|value| value.as_str())
        .unwrap_or_default()
}

#[tokio::test]
#[ignore = "依赖公网（httpbin.org）"]
async fn get_against_httpbin_succeeds() {
    let settings = settings();
    let client = client(&settings);

    let mut request = spec("GET", "https://httpbin.org/get");
    request.params = vec![KeyValueItem::new("x", "1")];

    let started = Instant::now();
    let out = execute(&client, &request).await.expect("请求应当成功");
    let wall = started.elapsed();

    assert_eq!(out.status, 200, "响应体：{}", out.body);
    assert_eq!(out.status_text, "OK");
    assert_eq!(out.body_encoding, BodyEncoding::Text);
    assert!(!out.truncated);
    assert!(
        out.http_version.starts_with("HTTP/"),
        "协议版本实际：{}",
        out.http_version
    );

    // 三段计时必须自洽（毫秒取整允许 1ms 误差），并且和"墙上时间"不能差太远
    assert!(out.total_ms >= out.ttfb_ms, "{out:?}");
    assert!(out.ttfb_ms + out.download_ms + 1 >= out.total_ms, "{out:?}");
    assert!(wall < Duration::from_secs(30), "墙上时间 {wall:?}");

    let json: serde_json::Value = serde_json::from_str(&out.body).expect("响应应当是 JSON");
    // query 参数确实发出去了 —— 这比"看 URL 拼对了没"可信得多
    assert_eq!(json["args"]["x"], "1", "响应体：{}", out.body);
    // 我们的 UA 也应当出现在回显里
    assert!(
        find_header(&json["headers"], "user-agent").starts_with("Reqly/"),
        "响应体：{}",
        out.body
    );
    assert!(
        json["headers"].get("Host").is_some(),
        "响应体：{}",
        out.body
    );
}

#[tokio::test]
#[ignore = "依赖公网（httpbin.org）"]
async fn post_json_body_against_httpbin_succeeds() {
    let settings = settings();
    let client = client(&settings);

    let mut request = spec("POST", "https://httpbin.org/post");
    request.body_type = BodyType::Json;
    request.body = r#"{"name":"reqly","n":7}"#.to_string();

    let out = execute(&client, &request).await.expect("请求应当成功");
    assert_eq!(out.status, 200, "响应体：{}", out.body);

    let json: serde_json::Value = serde_json::from_str(&out.body).expect("响应应当是 JSON");
    // httpbin 把 body 原样回显到 `data`，解析成功的对象放在 `json`
    assert_eq!(json["json"]["name"], "reqly", "响应体：{}", out.body);
    assert_eq!(json["json"]["n"], 7, "响应体：{}", out.body);
    // content-type 必须自动补上 application/json，否则服务端不会当 JSON 解析
    assert!(
        find_header(&json["headers"], "content-type").contains("application/json"),
        "响应体：{}",
        out.body
    );
}

#[tokio::test]
#[ignore = "依赖公网（httpbin.org）"]
async fn unreachable_host_is_reported_as_dns() {
    let settings = settings();
    let client = client(&settings);

    // `.invalid` 是 RFC 2606 保留的顶级域，保证解析不出来
    let err = execute(
        &client,
        &spec("GET", "https://reqly-does-not-exist.invalid/get"),
    )
    .await
    .expect_err("域名不该解析成功");

    assert_eq!(err.code(), codes::DNS, "实际：{err}");
    assert_eq!(
        err.params().expect("DNS 错误要带 host")["host"],
        "reqly-does-not-exist.invalid"
    );
}

#[tokio::test]
#[ignore = "依赖公网（httpbin.org）"]
async fn slow_request_can_be_cancelled() {
    let settings = settings();
    let client = client(&settings);

    // `/delay/10` 会真的挂十秒 —— 正好拿来验证取消能不能把它从 await 里拽出来
    let request = spec("GET", "https://httpbin.org/delay/10");

    let token = CancellationToken::new();
    let canceller = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        canceller.cancel();
    });

    let started = Instant::now();
    let err = execute_with_cancel(&client, &request, Some(&token))
        .await
        .expect_err("应当被取消");

    assert_eq!(err.code(), codes::CANCELLED, "实际：{err}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "取消该立刻生效（不用等服务端那 10 秒），实际 {:?}",
        started.elapsed()
    );
}

/// 走代理这条路也必须是通的 —— 企业内网网关是主要使用场景。
///
/// 拿环境里的 `HTTP_PROXY` 当靶子：没有这个变量就直接跳过
/// （本地开发机上通常不设）。
#[tokio::test]
#[ignore = "依赖公网（httpbin.org）"]
async fn proxy_setting_is_actually_used() {
    let proxy = std::env::var("HTTP_PROXY")
        .or_else(|_| std::env::var("http_proxy"))
        .or_else(|_| std::env::var("HTTPS_PROXY"))
        .or_else(|_| std::env::var("https_proxy"));

    let Ok(proxy) = proxy else {
        println!("跳过：环境里没有 HTTP_PROXY / HTTPS_PROXY");
        return;
    };

    let mut settings = settings();
    settings.proxy = Some(proxy.clone());
    let client = client(&settings);

    let out = execute(&client, &spec("GET", "https://httpbin.org/get"))
        .await
        .unwrap_or_else(|e| panic!("走代理 {proxy} 应当成功，实际：{e}"));

    assert_eq!(out.status, 200, "响应体：{}", out.body);
}
