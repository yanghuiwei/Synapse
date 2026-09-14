//! reqwest `Client` 的构造与缓存。
//!
//! ## 为什么必须缓存
//!
//! `Client` 内部是连接池（keep-alive）。每次请求都 `Client::builder().build()` 的话，
//! 每个请求都要重新做一次 TCP + TLS 握手 —— 在「连续调试同一个接口」这个最常见的
//! 场景下，耗时能差 3～10 倍，而且压测（P6）会因为端口耗尽直接失败。
//!
//! ## 缓存键怎么定
//!
//! 用「影响 Client 构造的全部设置」的规范化签名。改超时、改代理、关证书校验都会
//! 得到另一个 key —— 这些东西是**不可变地烘进** Client 的，改一个就得重建。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use reqwest::redirect::Policy;
use reqwest::{Client, Proxy};

use crate::error::{codes, AppError, AppResult};
use crate::storage::models::{HttpVersion, RequestSettings};

/// 缓存条目上限。
///
/// 用户手动切过的设置组合是有限的（通常就一两套：直连 + 内网自签），
/// 给 16 条足够，超了就整体清空 —— 为这点数据做 LRU 属于过度设计。
const MAX_CACHED_CLIENTS: usize = 16;

/// 空闲连接回收时间。取 reqwest 默认值，显式写出来是为了以后调参有地方改。
const POOL_IDLE_TIMEOUT_SECS: u64 = 90;

/// 日常单发的每 host 空闲连接数。
///
/// 8 是个够用的手气数：界面上的连续调试最多也就几路并发。
/// **压测必须覆盖它**（见 [`PoolProfile::Loadtest`]）。
const INTERACTIVE_IDLE_PER_HOST: usize = 8;

/// 压测时空闲连接数的硬上限。
///
/// 用户能填的并发数上限是 1000（`loadtest::MAX_CONCURRENCY`），
/// 但连接池是**全局共享**的，压测结束后这些空闲连接会一直留到
/// `pool_idle_timeout`（90 秒）。所以这里再箍一道，别让一次手滑
/// 输入把 fd 占满到影响界面本身。
const MAX_LOADTEST_IDLE_PER_HOST: usize = 1_000;

/// 「这个 Client 是给谁用的」。
///
/// 存在的理由是一个**会静默拖慢压测的默认值**：日常路径的空闲连接数是 8，
/// 而压测要的是「等于并发数」。按 8 条连接跑 50 并发的话，每一轮里
/// 42 条连接都要重做 TCP + TLS 握手 —— 量出来的"延迟"其实是
/// 「握手 + 请求」，而连接池本身成了被测对象。这正是 design.md §7.2
/// 写明 `pool_max_idle_per_host = C` 要避免的事。
///
/// 参与缓存键的构造：两套空闲连接数是**烘焙进** Client 的，
/// 不分开缓存的话，先生成的那一份会被后一个场景静默复用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PoolProfile {
    /// 界面上的单次发送 / 取 token / 导入时的探测
    Interactive,
    /// 压测：空闲连接数跟着并发数走；`keep_alive = false` 时归零（即不复用）
    Loadtest { concurrency: u32, keep_alive: bool },
}

impl PoolProfile {
    fn idle_per_host(self) -> usize {
        match self {
            PoolProfile::Interactive => INTERACTIVE_IDLE_PER_HOST,
            PoolProfile::Loadtest {
                concurrency,
                keep_alive,
            } => {
                if !keep_alive {
                    // reqwest 没有「关掉 keep-alive」的开关，
                    // 「每个 host 一条空闲连接都不留」就是它的等价表达
                    return 0;
                }
                (concurrency as usize).clamp(1, MAX_LOADTEST_IDLE_PER_HOST)
            }
        }
    }

    fn cache_tag(self) -> String {
        match self {
            PoolProfile::Interactive => "i".to_string(),
            PoolProfile::Loadtest {
                concurrency,
                keep_alive,
            } => format!("l{concurrency}{}", if keep_alive { "k" } else { "n" }),
        }
    }
}

pub struct ClientCache {
    entries: Mutex<HashMap<String, Client>>,
}

impl Default for ClientCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// 取一个符合 `settings` 的 Client。命中缓存直接返回（`Client` 内部是 `Arc`，clone 很便宜）。
    pub fn get(&self, settings: &RequestSettings) -> AppResult<Client> {
        self.get_with_profile(settings, PoolProfile::Interactive)
    }

    /// 压测专用：空闲连接数按并发数铺开。
    ///
    /// 单独开一个入口而不是给 `RequestSettings` 加字段：
    /// `RequestSettings` 是**会存进库、会进历史快照**的请求属性，
    /// 而「连接池留几条空闲」是这一次压测的运行时参数，
    /// 混进去会让「同一个请求」在同一次压测前后拿到两个不同的配置指纹。
    pub fn get_for_loadtest(
        &self,
        settings: &RequestSettings,
        concurrency: u32,
        keep_alive: bool,
    ) -> AppResult<Client> {
        self.get_with_profile(
            settings,
            PoolProfile::Loadtest {
                concurrency,
                keep_alive,
            },
        )
    }

    fn get_with_profile(
        &self,
        settings: &RequestSettings,
        profile: PoolProfile,
    ) -> AppResult<Client> {
        let key = cache_key(settings, profile);

        if let Some(client) = lock(&self.entries).get(&key) {
            return Ok(client.clone());
        }

        // 故意在锁外构造：build() 里会读证书库，不是瞬间完成的，
        // 不该让其它请求跟着等锁。并发下可能同一个 key 被构造两次，无害（后写入的胜出）。
        let client = build(settings, profile)?;

        let mut entries = lock(&self.entries);
        if entries.len() >= MAX_CACHED_CLIENTS {
            entries.clear();
        }
        entries.insert(key, client.clone());
        Ok(client)
    }

    /// 缓存条目数。**仅供测试**观察复用行为。
    ///
    /// 故意不叫 `len()`：那会让人以为 `ClientCache` 是个集合，进而要求配一个
    /// `is_empty()`（clippy 的 `len_without_is_empty`）；这里其实只有一个"有几套配置"
    /// 的内部计数，叫 `cached_count` 更贴切。
    #[cfg(test)]
    pub fn cached_count(&self) -> usize {
        lock(&self.entries).len()
    }
}

/// 拿锁。`Mutex` 中毒说明之前持有锁的线程 panic 了，
/// 但这张表里只是一堆 Client，数据本身没有「被写坏」的概念，直接接着用。
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn cache_key(settings: &RequestSettings, profile: PoolProfile) -> String {
    // 结构体字段顺序固定，序列化结果稳定，直接当签名用。
    // 注意 timeout_ms 为 0 时会被下面 max(1) 改成 1ms，这里也要归一，否则
    // 「0」和「1」会各占一个缓存位却指向同一个配置。
    format!(
        "v2|t={}|r={}:{}|tls={}|px={}|hv={:?}|pool={}",
        settings.timeout_ms.max(1),
        settings.follow_redirects,
        settings.max_redirects,
        settings.verify_tls,
        settings.proxy.as_deref().unwrap_or("").trim(),
        settings.http_version,
        profile.cache_tag(),
    )
}

fn build(settings: &RequestSettings, profile: PoolProfile) -> AppResult<Client> {
    let mut builder = Client::builder()
        .user_agent(user_agent())
        .pool_idle_timeout(Duration::from_secs(POOL_IDLE_TIMEOUT_SECS))
        .pool_max_idle_per_host(profile.idle_per_host())
        // 0 会被 reqwest 解读成「无限等待」，那是最糟的行为（界面假死、压测永不结束）
        .timeout(Duration::from_millis(settings.timeout_ms.max(1)))
        // 只在用户**明确关掉**校验时才关。默认是校验。
        .danger_accept_invalid_certs(!settings.verify_tls);

    builder = builder.redirect(if settings.follow_redirects {
        Policy::limited(settings.max_redirects as usize)
    } else {
        Policy::none()
    });

    match settings.http_version {
        HttpVersion::Auto => {}
        HttpVersion::Http1 => builder = builder.http1_only(),
        // ⚠️ reqwest **没有**「强制 https 走 h2」的开关：`http2_prior_knowledge()`
        // 是给明文 h2c 用的，拿去连 https 会在握手阶段直接失败。
        // 所以这里保持默认：ALPN 协商时 https 本来就优先 h2，
        // 服务端不支持就退回 1.1 —— 实际用的版本会如实回报给前端。
        HttpVersion::Http2 => {}
    }

    match settings.proxy.as_deref().map(str::trim) {
        Some(proxy) if !proxy.is_empty() => {
            let parsed = Proxy::all(proxy).map_err(|e| AppError::InvalidInput {
                field: "proxy",
                reason: format!("{proxy} 不是合法的代理地址：{e}"),
            })?;
            builder = builder.proxy(parsed);
        }
        _ => {
            // 显式**关掉系统代理探测**。
            //
            // reqwest 默认会读 HTTP_PROXY / HTTPS_PROXY / ALL_PROXY 这些环境变量。
            // 对一个「界面上写着『不使用代理』」的工具来说，隐式走代理是排查噩梦：
            // 用户从终端启动（带着 export 的代理变量）和从开始菜单启动，
            // 行为会不一样，而界面上完全看不出来。
            // 需要走代理就在 UI 里填，一次配置、所见即所得。
            builder = builder.no_proxy();
        }
    }

    builder
        .build()
        .map_err(|e| AppError::http(codes::INTERNAL, format!("构造 HTTP 客户端失败：{e}")))
}

fn user_agent() -> String {
    format!("Reqly/{}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> RequestSettings {
        RequestSettings::default()
    }

    #[test]
    fn same_settings_reuse_one_client() {
        let cache = ClientCache::new();
        let _first = cache.get(&settings()).expect("client a");
        let _second = cache.get(&settings()).expect("client b");

        // `Client` 内部是 Arc（clone 共享连接池），所以这里只能从缓存条目数上验证复用了。
        // reqwest 没有提供「两个 Client 是否同源」的公开 API —— 不为了测试去 hack 它。
        assert_eq!(cache.cached_count(), 1, "同一套设置只该建一个 Client");
    }

    #[test]
    fn different_settings_get_their_own_client() {
        let cache = ClientCache::new();
        cache.get(&settings()).expect("默认");

        let mut slower = settings();
        slower.timeout_ms = 1234;
        cache.get(&slower).expect("改超时");

        let mut no_tls = settings();
        no_tls.verify_tls = false;
        cache.get(&no_tls).expect("关证书校验");

        let mut proxied = settings();
        proxied.proxy = Some("http://127.0.0.1:7890".into());
        cache.get(&proxied).expect("走代理");

        assert_eq!(cache.cached_count(), 4, "四套不同设置 = 四个 Client");
    }

    #[test]
    fn zero_timeout_is_normalised_so_it_shares_the_cache_slot() {
        let cache = ClientCache::new();
        let mut zero = settings();
        zero.timeout_ms = 0;
        cache.get(&zero).expect("0ms");

        let mut one = settings();
        one.timeout_ms = 1;
        cache.get(&one).expect("1ms");

        // 0 会被归一成 1ms（否则等于永不超时），所以两者是同一套配置
        assert_eq!(cache.cached_count(), 1);
    }

    #[test]
    fn invalid_proxy_is_rejected_with_the_field_name() {
        // 注意「这不是地址」**不算**非法输入：URL 解析会按 IDN 规则把它转成 punycode
        // （http://xn--ihqq6tnb086gx92c/），于是成了一个合法的代理主机名。
        // 所以要拿结构上就坏掉的串来测。
        for bad in [
            "http://",          // 空主机名
            "http://[::1",      // 方括号没闭合
            "1.2.3.4:缺端口号", // 缺 scheme，也解析不出 base
        ] {
            let cache = ClientCache::new();
            let mut s = settings();
            s.proxy = Some(bad.to_string());

            let err = cache.get(&s).unwrap_err();
            assert!(
                matches!(&err, AppError::InvalidInput { field: "proxy", .. }),
                "`{bad}` 应当被拒，实际：{err}"
            );
        }
    }

    #[test]
    fn cache_does_not_grow_without_bound() {
        let cache = ClientCache::new();
        for i in 0..(MAX_CACHED_CLIENTS + 4) {
            let mut s = settings();
            s.timeout_ms = 100 + i as u64;
            cache.get(&s).expect("client");
        }
        assert!(
            cache.cached_count() <= MAX_CACHED_CLIENTS,
            "缓存必须有上限，实际 {}",
            cache.cached_count()
        );
    }

    #[test]
    fn http1_only_client_can_be_built() {
        let cache = ClientCache::new();
        let mut s = settings();
        s.http_version = HttpVersion::Http1;
        s.follow_redirects = false;
        s.max_redirects = 0;
        cache.get(&s).expect("http1 + 不跟随重定向");
    }

    /// 默认设置下**不能**走系统/环境代理。
    ///
    /// 这条不是洁癖：reqwest 默认会读 `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY`，
    /// 结果是「从终端启动（shell 里 export 过代理）」和「从开始菜单启动」行为不一致，
    /// 而界面上的设置明明写着「不使用代理」—— 用户一点线索都没有。
    ///
    /// 这个断言是被真事逼出来的：本机 `HTTP_PROXY=http://127.0.0.1:64242` 时，
    /// 「连 127.0.0.1:1 应当报 E_CONN_REFUSED」会变成「代理回了个 502」，
    /// 看上去像错误分类写错了，其实是 Client 把请求交给代理了。
    #[test]
    fn default_settings_do_not_pick_up_env_proxy() {
        // reqwest 没有公开「你用了哪些代理」的接口，但 `Debug` 里有 —— 退一步用它。
        // 注意 `Debug` 只在**非空**时才打 `proxies: [...]` 这一段（reqwest 的实现是
        // `if !self.proxies.is_empty()`），所以「没有代理」表现为这个字段压根不存在。
        let debug = format!("{:?}", ClientCache::new().get(&settings()).expect("client"));
        assert!(
            !debug.contains("proxies:"),
            "默认设置不该带任何代理（当前环境 HTTP_PROXY={:?}）：{debug}",
            std::env::var("HTTP_PROXY").ok()
        );

        // 反证：显式配了代理时这段一定会出现。
        // 否则万一哪天上游改了 `Debug` 格式，上面那条断言会变成「永远绿」的摆设。
        let mut proxied = settings();
        proxied.proxy = Some("http://127.0.0.1:7890".into());
        let with_proxy = format!(
            "{:?}",
            ClientCache::new().get(&proxied).expect("proxied client")
        );
        assert!(
            with_proxy.contains("proxies:"),
            "显式配了代理就该出现在 Debug 里：{with_proxy}"
        );
    }

    /// 压测的 Client 与日常的**不能共用缓存位**：空闲连接数是烘焙进 Client 的，
    /// 混用会让压测静默拿到只有 8 条空闲连接的池子。
    #[test]
    fn loadtest_client_does_not_share_the_interactive_slot() {
        let cache = ClientCache::new();
        cache.get(&settings()).expect("交互");
        cache.get_for_loadtest(&settings(), 50, true).expect("压测");

        assert_eq!(cache.cached_count(), 2, "两套连接池语义 = 两个 Client");
    }

    /// 同并发数同 keep_alive 的压测请求复用同一个 Client
    #[test]
    fn identical_loadtest_profiles_reuse_one_client() {
        let cache = ClientCache::new();
        cache.get_for_loadtest(&settings(), 64, true).expect("a");
        cache.get_for_loadtest(&settings(), 64, true).expect("b");

        assert_eq!(cache.cached_count(), 1);
    }

    /// 并发数不同 = 连接池大小不同 = 必须分开
    #[test]
    fn different_concurrency_gets_its_own_pool() {
        let cache = ClientCache::new();
        cache.get_for_loadtest(&settings(), 10, true).expect("10");
        cache.get_for_loadtest(&settings(), 100, true).expect("100");
        cache
            .get_for_loadtest(&settings(), 10, false)
            .expect("10 不复用");

        assert_eq!(cache.cached_count(), 3);
    }

    #[test]
    fn idle_per_host_follows_concurrency_and_is_clamped() {
        let lt = |c: u32, keep: bool| {
            PoolProfile::Loadtest {
                concurrency: c,
                keep_alive: keep,
            }
            .idle_per_host()
        };

        assert_eq!(lt(50, true), 50, "压测的空闲连接数要等于并发数");
        assert_eq!(lt(1, true), 1);
        // 关掉连接复用时一条空闲连接都不留
        assert_eq!(lt(50, false), 0);
        // 并发数再大也不能超过硬上限，否则压测结束后一堆空闲 fd 会赖着不走
        assert_eq!(lt(u32::MAX, true), MAX_LOADTEST_IDLE_PER_HOST);
        assert_eq!(PoolProfile::Interactive.idle_per_host(), 8);
    }
}
