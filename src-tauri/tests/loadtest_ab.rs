//! P6-11 验收（本机）：把压测引擎的延迟分位对着 ApacheBench 校准。
//!
//! 对照 `docs/plan.md` §3 的 P6 硬指标：
//!   `ab -n 500 -c 50` 打同一个本机 mock server，**P95 偏差 < 10%**。
//!
//! 默认 `#[ignore]`：它要求
//!   1. 机器上有 `ab`（可用 `AB_BIN` 指到别处）；
//!   2. 计时不被别的重活干扰 —— 塞进常规 `cargo test` 只会随机翻红。
//!
//! ```text
//! set AB_BIN=C:\path\to\ab.exe
//! cargo test --test loadtest_ab -- --ignored --nocapture --test-threads=1
//! ```
//!
//! ## 为什么 mock server 的延迟是**确定性**的
//!
//! 这个测试要回答的问题很窄：「同一条延迟曲线，两个客户端量出来的 P95 一不一样」。
//! 一旦服务端自己带随机性，两次运行的分位数就会因为**样本不同**而分开，
//! 那时候测出来的是抽样噪声而不是测量精度 —— 两个工具对不上也说不清是谁的错。
//!
//! 所以延迟取 `delay_ms(序号)` 这个纯函数：ab 跑一遍、我们跑一遍，
//! 看到的**是同一个多重集**，唯一剩下的变量就只有两边各自的计时与分位算法。
//! 这也正是最该被钉住的东西 —— 见 `loadtest::latency` 的模块说明：
//! 「算错的 P95 和算对的 P95 长得一模一样」。
//!
//! ## 两套延迟曲线，各钉一个点
//!
//! * `uniform` —— 80..119ms 均匀铺开。分位落在"平台"中间，考**分位数值**准不准。
//! * `cliff` —— 九成 40ms、一成 160ms。P95 落在 160 那一段，P90 正好卡在台阶上，
//!   顺带把"两边取第几个样本"这个**下标差**放大成可见的数字（见下一节）。
//!
//! ## 分位口径：两边**差一个秩**（实测钉出来的，不是抄文档）
//!
//! 理论推演这边走过一次弯路。先按"ab 取 `⌊n·q⌋ - 1`"推，于是断言写成"逐位相等"；
//! 第一次跑就被 126 vs 131 打回来。把 `cliff` 曲线的 ab 原始输出摊开才看清：
//!
//! ```text
//! 66% 48   75% 48   80% 49   90% 162
//! ```
//!
//! `cliff` 恰好是「450 个快 + 50 个慢」，快慢分界落在下标 450。ab 的 90% 报的是
//! **162（慢组第一个）**，而不是 49（快组最后一个）—— 说明 ab 取的 0 基下标是
//! `⌊q·n⌋`，**比我们的最近秩（`⌈q·n⌉ - 1`）正好高一位**。
//!
//! 这个差一位在 `uniform` 上看不出来（值域只占 1ms，相邻两名常常同值），
//! `cliff` 把它放大成 49 vs 162。**留着 cliff 就是为了钉这一条**：
//! 它是本次验收里唯一一个"看着像 bug 其实是口径"的数字。
//!
//! ## 2×2 配对：为什么四个格子都要跑
//!
//! `ab` 默认**每个请求新建连接**（`-k` 才复用），而引擎默认 `keep_alive = true`。
//! 于是「拿 ab 的默认对引擎的默认」和「拿 ab 的默认对引擎的 `keep_alive=false`」
//! 是**两件不同的事**，实测差得不小。所以每个曲线跑满 2×2：
//!
//! | | 我们 `keep_alive=true`（默认） | 我们 `keep_alive=false` |
//! |---|---|---|
//! | `ab -n 500 -c 50`（默认，每请求建连） | **准绳口径** | 两边都新建连接 |
//! | `ab -k`（连接复用） | 同策略 | 交叉 |
//!
//! 断言只用**前两格**（准绳口径 + 同策略）。后两格的偏差已知更大，
//! 原因在下一节，属于"被量出来的事实"而不是"分位算错了"。
//!
//! ## 已知偏差：我们客户端在"每次新建连接"这条路上比 ab 重（5~15ms，浮动）
//!
//! 实测（`cliff` 曲线，`Connection Times` 的 `mean`，四轮）：
//!
//! | 配对 | ab | 我们 | 差 |
//! |---|---|---|---|
//! | 连接复用 vs `-k` | 60.0 | 60.2 / 60.5 / 60.8 / 61.0 | **≤ +1.0** |
//! | 每请求建连 vs 无 `-k` | 58.0 / 58.0 / 64.0 | 63.4 / 71.0 / 72.7 / 74.9 | **+5.4 ~ +14.7** |
//!
//! 复用那一路稳定对得上（1ms 以内），所以差异只可能出在**建连路径**上：
//! `keep_alive = false` 在实现上就是 `pool_max_idle_per_host(0)`
//! （见 `http::client::PoolProfile`），于是每个请求都要过一次
//! hyper 的连接建立 —— 而 ab 的建连是一次阻塞 `connect()`。回环上真握手不到 0.1ms，
//! 所以这是**客户端侧的开销**（DNS 解析走 `spawn_blocking` 那条路最可疑），不是服务端慢。
//! 它的绝对量还会随机器负载在 5~15ms 之间浮动，所以这一格的允差放到
//! [`TOLERANCE_KNOWN_DRIFT`]，而**不是**把 P95 的 10% 标准挪走。
//!
//! 另一头也有一处不对称，只是方向相反：`Total` 的 **max**，ab 稳定比我们高一大截
//! （`cliff` 上 213~277 vs 183~190）。ab 的 `-c 50` 是 50 个阻塞线程，
//! 被系统调度晾一下就是几十毫秒；我们这边全是异步任务，尾部反而收得住。
//! 地板我们差、天花板 ab 差 —— 两边的偏差各有各的来源，不是同一个数被算错两遍。
//!
//! 它不影响连接复用（引擎默认，也是真实压测用的那一路），
//! 留在账上：`loadtest::engine::LoadtestConfig::keep_alive` 的文档本来就写着
//! 关掉它"只在刻意验证连接建立开销时才关"。待办见 `docs/plan.md` §14。

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

use reqly_lib::http::{ClientCache, RequestSpec};
use reqly_lib::loadtest::{engine, LoadtestConfig, LoadtestDeps, RunOutcome};
use reqly_lib::storage::models::{AuthConfig, BodyType, HttpVersion, RequestSettings};

const REQUESTS: u64 = 500;
const CONCURRENCY: u32 = 50;

/// 响应体固定 11 字节：内容不重要，重要的是别让两个客户端在"读多少字节"上产生差异
const BODY: &[u8] = b"{\"ok\":true}";

/// 准绳口径与同策略口径的允差（plan.md §3 的硬指标）
const TOLERANCE: f64 = 10.0;

/// 已知偏差格子的允差。见模块头：我们客户端在"每次新建连接"时比 ab 重 5~15ms，
/// 在 165ms 量级的 P95 上就是约 +3% ~ +9%，机器忙的时候还能再多一点。
/// 放宽不等于放过 —— 每格的实际偏差都会打印出来，超了这一格也会报出来。
const TOLERANCE_KNOWN_DRIFT: f64 = 20.0;

// ============================================================
// 延迟曲线
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Profile {
    /// 80..119ms 均匀铺开
    Uniform,
    /// 九成 40ms、一成 160ms。快慢分界落在下标 450，正好把"差一位"放大
    Cliff,
}

impl Profile {
    fn name(self) -> &'static str {
        match self {
            Profile::Uniform => "uniform",
            Profile::Cliff => "cliff",
        }
    }

    /// 第 `n` 个（从 0 数）请求要睡多久。**纯函数**，见模块头。
    fn delay_ms(self, n: u64) -> u64 {
        match self {
            Profile::Uniform => 80 + (n % 40),
            Profile::Cliff => {
                if n % 10 == 9 {
                    160
                } else {
                    40
                }
            }
        }
    }
}

// ============================================================
// mock server
// ============================================================

/// 本机 mock server。
///
/// 跑在**自己的线程 + 自己的 tokio runtime** 上：和被测客户端共用一个 runtime 的话，
/// 服务端的定时器与客户端的任务会在同一批 worker 上抢调度，
/// 那等于让"被测对象"和"测量工具"互相干扰 —— 而这正是压测最不该有的东西。
struct MockServer {
    addr: SocketAddr,
    hits: Arc<AtomicU64>,
    cancel: CancellationToken,
    thread: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    fn start(profile: Profile) -> Self {
        // 端口交给内核挑：固定端口在连续跑两轮时会撞上 TIME_WAIT
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定本机端口");
        std_listener
            .set_nonblocking(true)
            .expect("listener 设成非阻塞");
        let addr = std_listener.local_addr().expect("取本机端口");
        let hits = Arc::new(AtomicU64::new(0));
        let cancel = CancellationToken::new();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            // 服务端几乎全是定时器等待，4 个 worker 足够；
            // 开多了反而会和客户端的线程抢 CPU
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("建 mock server runtime");

        let thread = {
            let hits = Arc::clone(&hits);
            let cancel = cancel.clone();
            thread::spawn(move || {
                runtime.block_on(async move {
                    let listener =
                        TcpListener::from_std(std_listener).expect("转成 tokio listener");
                    serve(listener, profile, hits, cancel).await;
                });
            })
        };

        Self {
            addr,
            hits,
            cancel,
            thread: Some(thread),
        }
    }

    fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn serve(
    listener: TcpListener,
    profile: Profile,
    hits: Arc<AtomicU64>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let hits = Arc::clone(&hits);
                    tokio::spawn(async move {
                        // 一条连接上的任何错误都只影响这条连接：
                        // ab 会在收到响应后主动断开，那边的 RST 不该把 accept 循环带走
                        let _ = handle_conn(stream, profile, hits).await;
                    });
                }
                Err(_) => break,
            },
        }
    }
}

async fn handle_conn(
    stream: TcpStream,
    profile: Profile,
    hits: Arc<AtomicU64>,
) -> std::io::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    loop {
        // ---- 请求行 ----
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let request_line = line.trim_end();
        if request_line.is_empty() {
            continue;
        }
        // ab 默认发 HTTP/1.0（每个请求一条连接），`-k` 才发 HTTP/1.1 + keep-alive。
        // 两种都要能收：这个测试本来就要跑 2×2 四格。
        let http_11 = request_line.ends_with("HTTP/1.1");

        // ---- 头 ----
        let mut connection_header: Option<bool> = None;
        let mut content_length: usize = 0;
        loop {
            line.clear();
            if reader.read_line(&mut line).await? == 0 {
                return Ok(());
            }
            let header = line.trim_end();
            if header.is_empty() {
                break;
            }
            let Some((name, value)) = header.split_once(':') else {
                continue;
            };
            let value = value.trim().to_ascii_lowercase();
            match name.trim().to_ascii_lowercase().as_str() {
                "connection" => connection_header = Some(value.contains("keep-alive")),
                "content-length" => content_length = value.parse().unwrap_or(0),
                _ => {}
            }
        }
        if content_length > 0 {
            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).await?;
        }

        // ---- 模拟服务端处理耗时 ----
        let nth = hits.fetch_add(1, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(profile.delay_ms(nth))).await;

        // ---- 响应 ----
        let keep_alive = connection_header.unwrap_or(http_11);
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: {}\r\n\r\n",
            BODY.len(),
            if keep_alive { "keep-alive" } else { "close" },
        );
        write_half.write_all(head.as_bytes()).await?;
        write_half.write_all(BODY).await?;
        write_half.flush().await?;

        if !keep_alive {
            return Ok(());
        }
    }
}

// ============================================================
// ab
// ============================================================

/// `Connection Times (ms)` 里 `Total:` 那一行：min / mean / median / max。
///
/// 用来解释"为什么两边的地板和台阶不一样"—— 单看分位表看不出这件事。
#[derive(Debug, Clone, Copy)]
struct AbTotal {
    min_ms: u64,
    mean_ms: f64,
    median_ms: u64,
    max_ms: u64,
}

#[derive(Debug, Default)]
struct AbReport {
    complete: u64,
    failed: u64,
    /// `Percentage of the requests served within a certain time (ms)` 那段
    percentiles: Vec<(u32, u64)>,
    total: Option<AbTotal>,
    raw: String,
}

impl AbReport {
    fn at(&self, pct: u32) -> Option<u64> {
        self.percentiles
            .iter()
            .find(|(p, _)| *p == pct)
            .map(|(_, ms)| *ms)
    }

    fn parse(output: &str) -> Self {
        let mut report = AbReport {
            raw: output.to_string(),
            ..AbReport::default()
        };
        let mut in_table = false;
        // `Total:` 在 ab 的输出里出现三次（连接耗时表、传输量、"Total body sent"），
        // 只认连接耗时表里的那一次 —— 所以得先看到 `Connection Times (ms)` 才开窗
        let mut in_times = false;

        for line in output.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("Connection Times") {
                in_times = true;
                continue;
            }
            if trimmed.starts_with("Percentage of the requests") {
                in_table = true;
                // 百分位表排在连接耗时表之后，这里顺手把窗口关掉
                in_times = false;
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("Complete requests:") {
                report.complete = rest.trim().parse().unwrap_or(0);
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("Failed requests:") {
                report.failed = rest.trim().parse().unwrap_or(0);
                continue;
            }

            if in_times {
                if let Some(rest) = trimmed.strip_prefix("Total:") {
                    // min  mean[+/-sd] median  max —— 五列，**sd 那一列要跳过**。
                    // 早期版本按四列读，把 sd 当成了 median、把 median 当成了 max：
                    // 于是"ab 的 max"打印出来刚好等于它的中位数，看着像 ab 出了错。
                    // 分位表里的 90% 明明比这个"max"大一大截，这才露的馅。
                    let cells: Vec<&str> = rest.split_whitespace().collect();
                    if cells.len() >= 5 {
                        report.total = Some(AbTotal {
                            min_ms: cells[0].parse().unwrap_or(0),
                            mean_ms: cells[1].parse().unwrap_or(0.0),
                            median_ms: cells[3].parse().unwrap_or(0),
                            max_ms: cells[4].parse().unwrap_or(0),
                        });
                    }
                }
                continue;
            }

            if !in_table {
                continue;
            }
            // 形如 `  95%     119` 或 ` 100%     119 (longest request)`
            let Some((pct, ms)) = trimmed.split_once('%') else {
                continue;
            };
            let (Ok(pct), Some(ms)) = (pct.trim().parse::<u32>(), ms.split_whitespace().next())
            else {
                continue;
            };
            let Ok(ms) = ms.parse::<u64>() else {
                continue;
            };
            report.percentiles.push((pct, ms));
        }
        report
    }
}

fn run_ab(url: &str, keep_alive: bool) -> AbReport {
    let bin = std::env::var("AB_BIN").unwrap_or_else(|_| "ab".to_string());
    let mut command = Command::new(&bin);
    command.args(["-n", &REQUESTS.to_string(), "-c", &CONCURRENCY.to_string()]);
    if keep_alive {
        command.arg("-k");
    }
    command.arg(url);

    let output = command
        .output()
        .unwrap_or_else(|e| panic!("跑不起 `{bin}`：{e}\n把 ab 的路径放进 AB_BIN 再试"));

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        output.status.success(),
        "`{bin}` 退出码 {:?}\n--- stdout ---\n{stdout}\n--- stderr ---\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    AbReport::parse(&stdout)
}

// ============================================================
// 被测方：我们的引擎
// ============================================================

// 与 `ab -n 500 -c 50` 对齐：**不加预热**。
// 预热会削掉首批请求里的建连开销，而 ab 那边没有这一步 —— 加了就不叫对照。
fn engine_config(keep_alive: bool) -> LoadtestConfig {
    LoadtestConfig {
        concurrency: CONCURRENCY,
        total: REQUESTS,
        timeout_ms: 30_000,
        follow_redirects: false,
        keep_alive,
        think_time_ms: 0,
        http_version: HttpVersion::Auto,
        warmup_requests: 0,
    }
}

fn run_engine(runtime: &Runtime, url: &str, keep_alive: bool) -> RunOutcome {
    let settings = RequestSettings {
        timeout_ms: 30_000,
        follow_redirects: false,
        ..RequestSettings::default()
    };
    let client = ClientCache::new()
        .get_for_loadtest(&settings, CONCURRENCY, keep_alive)
        .expect("建压测 client");

    let spec = RequestSpec {
        method: "GET".into(),
        url: url.to_string(),
        params: Vec::new(),
        headers: Vec::new(),
        body_type: BodyType::None,
        body: String::new(),
        form_fields: Vec::new(),
        auth: AuthConfig::None,
        settings,
        folder_id: None,
    };

    let outcome = runtime
        .block_on(engine::run(LoadtestDeps {
            client,
            spec,
            config: engine_config(keep_alive),
            cancel: CancellationToken::new(),
            on_progress: None,
        }))
        .expect("压测应当跑完");

    assert_eq!(
        outcome.summary.total, REQUESTS,
        "引擎没跑满：{:?}",
        outcome.summary
    );
    assert_eq!(
        outcome.summary.failed, 0,
        "引擎出现了失败：{:?}",
        outcome.summary
    );
    outcome
}

/// 把样本按 10ms 一桶铺开，用来解释"两边地板/台阶为什么不一样"。
/// 只看汇总数字的话，`cliff` 的 P90 差 46% 会像是个 bug。
fn print_histogram(durations: &[i64]) {
    let mut buckets = BTreeMap::<i64, u64>::new();
    for ms in durations {
        *buckets.entry(ms / 10 * 10).or_insert(0) += 1;
    }
    println!("  ours 样本分布（10ms 一桶）");
    for (floor, count) in buckets {
        let bar = "#".repeat((count / 2).min(60) as usize);
        println!("    {floor:>4}..{:>4}ms  {count:>4}  {bar}", floor + 9);
    }
}

fn deviation(ours: u64, theirs: u64) -> f64 {
    (ours as f64 - theirs as f64).abs() / theirs as f64 * 100.0
}

// ============================================================
// 主对照
// ============================================================

/// 一格的结论。
struct Cell {
    label: &'static str,
    /// ab 的 P95 与我们的 P95
    ab_p95: u64,
    ours_p95: u64,
    deviation: f64,
    tolerance: f64,
}

fn compare(profile: Profile, runtime: &Runtime) {
    let server = MockServer::start(profile);
    let url = server.url();

    // ab 是外部进程，启动/退出都要时间。四轮串行，先跑完 ab 再跑引擎，
    // 避免"ab 正在冷启动时引擎已经在计时"这种错位。
    let ab_default = run_ab(&url, false);
    let ab_keepalive = run_ab(&url, true);
    assert_eq!(
        ab_default.complete, REQUESTS,
        "ab(默认) 没跑满：\n{}",
        ab_default.raw
    );
    assert_eq!(
        ab_keepalive.complete, REQUESTS,
        "ab(-k) 没跑满：\n{}",
        ab_keepalive.raw
    );
    assert_eq!(
        ab_default.failed, 0,
        "ab(默认) 报了失败：\n{}",
        ab_default.raw
    );
    assert_eq!(
        ab_keepalive.failed, 0,
        "ab(-k) 报了失败：\n{}",
        ab_keepalive.raw
    );

    let ours_default = run_engine(runtime, &url, true);
    let ours_no_keepalive = run_engine(runtime, &url, false);

    println!(
        "\n===== {:?} （{} 请求 / {} 并发）=====",
        profile.name(),
        REQUESTS,
        CONCURRENCY
    );

    // 四格：准绳口径 / 同策略 / 两个交叉格
    let cells = vec![
        Cell {
            label: "准绳口径   ab -n 500 -c 50      vs 引擎默认(keep-alive)",
            ab_p95: ab_default.at(95).expect("ab 输出里没有 95%"),
            ours_p95: ours_default.summary.latency.p95_ms,
            deviation: 0.0,
            tolerance: TOLERANCE,
        },
        Cell {
            label: "同策略     ab -k                 vs keep_alive=true",
            ab_p95: ab_keepalive.at(95).expect("ab 输出里没有 95%"),
            ours_p95: ours_default.summary.latency.p95_ms,
            deviation: 0.0,
            tolerance: TOLERANCE,
        },
        Cell {
            label: "两边新建   ab -n 500 -c 50      vs keep_alive=false",
            ab_p95: ab_default.at(95).expect("ab 输出里没有 95%"),
            ours_p95: ours_no_keepalive.summary.latency.p95_ms,
            deviation: 0.0,
            tolerance: TOLERANCE_KNOWN_DRIFT,
        },
        Cell {
            label: "交叉       ab -k                 vs keep_alive=false",
            ab_p95: ab_keepalive.at(95).expect("ab 输出里没有 95%"),
            ours_p95: ours_no_keepalive.summary.latency.p95_ms,
            deviation: 0.0,
            tolerance: TOLERANCE_KNOWN_DRIFT,
        },
    ];
    let cells: Vec<Cell> = cells
        .into_iter()
        .map(|mut cell| {
            cell.deviation = deviation(cell.ours_p95, cell.ab_p95);
            cell
        })
        .collect();

    println!("  P95 配对矩阵");
    for cell in &cells {
        println!(
            "    {}  ab={:>4}  ours={:>4}  偏差 {:>6.2}%  (允差 {:.0}%)",
            cell.label, cell.ab_p95, cell.ours_p95, cell.deviation, cell.tolerance,
        );
    }

    // P50 / P90 / P99 只报告不断言：P90 在 cliff 上本来就该差一大截（见模块头），
    // 拿它做断言只会把一个口径事实伪装成回归。
    println!("  其余分位（仅报告）");
    for (pct, pick) in [
        (
            50u32,
            (|s: &RunOutcome| s.summary.latency.p50_ms) as fn(&RunOutcome) -> u64,
        ),
        (90, |s| s.summary.latency.p90_ms),
        (99, |s| s.summary.latency.p99_ms),
    ] {
        let ab_default_ms = ab_default.at(pct).expect("ab 输出里没有该分位");
        let ab_ka_ms = ab_keepalive.at(pct).expect("ab 输出里没有该分位");
        let ours_default_ms = pick(&ours_default);
        let ours_nk_ms = pick(&ours_no_keepalive);
        println!(
            "    P{pct:<3} ab 默认={ab_default_ms:>4} (-k={ab_ka_ms:>4})   ours 默认={ours_default_ms:>4} (no-ka={ours_nk_ms:>4})   偏差 {:.2}% / {:.2}%",
            deviation(ours_default_ms, ab_default_ms),
            deviation(ours_nk_ms, ab_default_ms),
        );
    }

    // 地板与均值分两个策略各看一次 —— 模块头那条「+12ms 建连开销」就是从这里读出来的
    println!("  Connection Times 的 Total 行（同策略两两对）");
    for (label, total, outcome) in [
        ("ab -k  vs 我们默认   ", ab_keepalive.total, &ours_default),
        (
            "ab 默认 vs 我们新建  ",
            ab_default.total,
            &ours_no_keepalive,
        ),
    ] {
        if let Some(total) = total {
            println!(
                "    {label} min ab={:>4} ours={:>4}   mean ab={:>5.1} ours={:>5.1}   med ab={:>4} ours={:>4}   max ab={:>4} ours={:>4}",
                total.min_ms,
                outcome.summary.latency.min_ms,
                total.mean_ms,
                outcome.summary.latency.mean_ms,
                total.median_ms,
                outcome.summary.latency.p50_ms,
                total.max_ms,
                outcome.summary.latency.max_ms,
            );
        }
    }
    println!(
        "  引擎耗时 {}/{}ms · 服务端共收到 {} 个请求（ab 1000 + 我们 1000）",
        ours_default.summary.elapsed_ms,
        ours_no_keepalive.summary.elapsed_ms,
        server.hits(),
    );

    if std::env::var("LT_TRACE").is_ok() {
        println!("----- ab 原始输出（默认）-----\n{}", ab_default.raw);
        println!("----- ab 原始输出（-k）-----\n{}", ab_keepalive.raw);
        let durations: Vec<i64> = ours_default.samples.iter().map(|s| s.duration_ms).collect();
        print_histogram(&durations);
    }

    // ---- 断言 ----
    for cell in &cells {
        assert!(
            cell.deviation < cell.tolerance,
            "P95 偏差 {:.2}% ≥ {:.0}%（{:?} / {}）：ab={}ms，我们={}ms",
            cell.deviation,
            cell.tolerance,
            profile.name(),
            cell.label,
            cell.ab_p95,
            cell.ours_p95,
        );
    }
}

#[test]
#[ignore = "需要本机装 ab，且不能被其它重活干扰计时"]
fn p95_agrees_with_apachebench() {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    // 客户端 runtime 的 worker 数会影响测量地板：50 个任务同时被唤醒时，
    // 任务要排队等一个 worker 来 poll，这段等待会**算进被测延迟**（它发生在
    // `Instant::now()` 之后）。默认给的是 `num_cpus`，也就是线上用真身跑时的配置；
    // 想验证这个解释就 `set LT_CLIENT_THREADS=16` 再跑，看快组尾部收不收敛。
    if let Ok(threads) = std::env::var("LT_CLIENT_THREADS") {
        if let Ok(threads) = threads.parse::<usize>() {
            builder.worker_threads(threads);
        }
    }
    let runtime = builder.build().expect("建客户端 runtime");

    compare(Profile::Uniform, &runtime);
    compare(Profile::Cliff, &runtime);
}
