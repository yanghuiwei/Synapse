//! 压测引擎：派发、计时、取消、采样。
//!
//! ## 三个刻意的设计选择
//!
//! **1. worker 池，而不是「为每个请求 spawn 一个 task」。**
//!
//! `design.md` §7.2 画的是 `for i in 0..N spawn task { acquire permit; ... }`。
//! 并发语义（同时在飞数 = C）两者**完全一致**，但任务数是 O(C) 与 O(N) 的差别：
//! 单轮 50 万请求时，后者要先造 50 万个 task（几十 MB + 一轮调度风暴），
//! 而压测最不能容忍的就是「客户端自己在忙」。
//! 这里开 C 个 worker，每个 worker 从共享计数器领号，同时最多一个在飞请求 ——
//! 并发数由结构本身保证，不需要信号量。
//!
//! **2. 耗时在**这一层**测，不用响应里的 `total_ms`。**
//!
//! `HttpResponsePayload` 有 `total_ms`，但它只在**成功**时存在 ——
//! 失败（DNS / 超时 / 连不上）返回的是 `Err`，没有耗时。
//! 只记成功样本的耗时，等于把最糟的那一段（超时的那批）从分位里删掉，
//! P95 会好看得不像话。所以这里用同一个 `Instant` 把成功与失败**放在同一个时钟上**，
//! 代价是多了任务调度的那几微秒（对所有样本一致，不影响对比）。
//!
//! **3. 占位符只在开工前解析一次。**
//!
//! 解析要查库（环境变量、Token 源、文件夹链上的授权继承）。放到每个请求上，
//! 那就是给每个请求前面挂 1ms 级的数据�库读 —— 对一个 5ms 的接口，
//! 量出来的东西一半是 Reqly 自己。
//! 代价是 `{{$uuid}}` 这类动态函数会被**冻结**，所以引擎会把它们报进
//! [`LoadtestSummary::frozen_functions`]，由界面如实告诉用户。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppResult};
use crate::http::{execute, HttpResponsePayload, RequestSpec};
use crate::loadtest::stats::{LoadtestSummary, ProgressSnapshot, StatsAccumulator};
use crate::storage::models::{HttpVersion, LoadtestSample, RequestSettings};
use crate::template::resolver::{scan_spec, Namespace};

/// 进度事件的间隔（design.md §7.2）。
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

/// 采样 channel 的缓冲深度。
///
/// 这个数字就是「收集得比派发慢」时允许堆积的样本数。定成 8192：
/// 50 并发 × 每个约 5ms ⇒ 约 1 万请求/秒，够收集任务喘息 0.8 秒
/// （而它每个 tick 只做 O(1) 的记账）。满了就是**背压**：
/// worker 在 `send().await` 上停下，宁可少打几个请求，
/// 也不能让样本在内存里堆成一个无界队列。
const SAMPLE_BUFFER: usize = 8192;

/// 单轮压测的请求数上限。
///
/// 存在的理由是**样本要在内存里留一份**（见 [`RunOutcome::samples`]）：
/// 50 万条 × 约 56 字节 ≈ 28MB，这是可接受的峰值。
/// 再多就该用专业工具了 —— design.md §7.5 也把长稳测试列为明确不做的事。
pub const MAX_TOTAL: u64 = 500_000;

/// 并发数上限。见 `http::client::MAX_LOADTEST_IDLE_PER_HOST` 的说明。
pub const MAX_CONCURRENCY: u32 = 1_000;

// ============================================================
// 配置
// ============================================================

/// 一次压测的配置（design.md §7.1）。
///
/// 会原样存进 `loadtest_run.config`，所以字段名走 camelCase。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadtestConfig {
    pub concurrency: u32,
    pub total: u64,
    /// 单请求超时。会覆盖请求自身的设置 —— 压测要一次定死超时，
    /// 否则「哪些请求算超时」在不同请求之间不一致，分位数就没法解释
    pub timeout_ms: u64,
    pub follow_redirects: bool,
    /// 复用连接（默认 true）。关掉之后每个请求都要重做握手 ——
    /// 那是在测握手而不是测接口，只在刻意验证连接建立开销时才关
    pub keep_alive: bool,
    /// 每个 worker 两轮之间的间隔
    #[serde(default)]
    pub think_time_ms: u64,
    #[serde(default)]
    pub http_version: HttpVersion,
    /// 预热请求数，不计入统计
    #[serde(default)]
    pub warmup_requests: u64,
}

impl Default for LoadtestConfig {
    fn default() -> Self {
        Self {
            concurrency: 10,
            total: 100,
            timeout_ms: 30_000,
            follow_redirects: false,
            keep_alive: true,
            think_time_ms: 0,
            http_version: HttpVersion::Auto,
            warmup_requests: 0,
        }
    }
}

impl LoadtestConfig {
    /// 校验。放在引擎里而不是只靠前端 —— 前端能被绕过，
    /// 而一个 `concurrency = 0` 的配置会让 worker 池开不出任何 worker：
    /// 派发数 0、耗时 0、报告全 0，看起来像"接口秒回"。
    pub fn validate(&self) -> AppResult<()> {
        if self.concurrency == 0 {
            return Err(AppError::InvalidInput {
                field: "concurrency",
                reason: "至少为 1：并发 0 会一个请求都不发".into(),
            });
        }
        if self.concurrency > MAX_CONCURRENCY {
            return Err(AppError::InvalidInput {
                field: "concurrency",
                reason: format!("上限 {MAX_CONCURRENCY}：再高会先把本机端口与 fd 用光"),
            });
        }
        if self.total == 0 {
            return Err(AppError::InvalidInput {
                field: "total",
                reason: "至少为 1".into(),
            });
        }
        if self.total > MAX_TOTAL {
            return Err(AppError::InvalidInput {
                field: "total",
                reason: format!("上限 {MAX_TOTAL}：单轮样本要在内存里留一份"),
            });
        }
        if self.warmup_requests > MAX_TOTAL {
            return Err(AppError::InvalidInput {
                field: "warmupRequests",
                reason: format!("上限 {MAX_TOTAL}"),
            });
        }
        if self.timeout_ms == 0 {
            return Err(AppError::InvalidInput {
                field: "timeoutMs",
                reason: "不能为 0：那等于永不超时，单轮压测会挂住".into(),
            });
        }
        Ok(())
    }

    /// 压测真正使用的请求设置：以**请求自身**的设置打底（代理、TLS 校验这些
    /// 属于"这个接口怎么连"，压测配置管不着），再覆盖压测明确声明的那几项。
    pub fn apply_to(&self, base: &RequestSettings) -> RequestSettings {
        RequestSettings {
            timeout_ms: self.timeout_ms,
            follow_redirects: self.follow_redirects,
            http_version: self.http_version,
            ..base.clone()
        }
    }
}

// ============================================================
// 入口
// ============================================================

/// 引擎跑一轮需要的东西。
///
/// 刻意**不**接收 `AppState`：引擎不该认识数据库、也不该认识 Tauri。
/// 拿到的已经是一个解析好、可发送的 spec（由命令层走 `Pipeline::prepare` 得到）。
pub struct LoadtestDeps {
    pub client: reqwest::Client,
    /// 已经解析完占位符的请求
    pub spec: RequestSpec,
    pub config: LoadtestConfig,
    pub cancel: CancellationToken,
    /// 每 200ms 一次的进度回调。
    ///
    /// 用回调而不是 channel：进度是「最新的才有意义」，
    /// 排队只会让界面显示过期的数字，而这个数字正是用户盯着看的那一个。
    pub on_progress: Option<Box<dyn FnMut(ProgressSnapshot) + Send>>,
}

pub struct RunOutcome {
    pub summary: LoadtestSummary,
    /// 全部样本，按完成顺序。
    ///
    /// 留在内存里一次交给调用方，而不是在引擎里边跑边写库：
    /// 收集任务一旦要等数据库（哪怕只有几毫秒），那几毫秒里
    /// worker 就会被背压顶住 —— 也就是说**磁盘速度会被算进被测延迟**。
    /// 先收完再写，写库完全落在测量区间之外。
    /// 代价是内存占用有界（见 [`MAX_TOTAL`]），这个代价是明码标价的。
    pub samples: Vec<LoadtestSample>,
}

/// 跑一轮压测。
///
/// 取消语义（design.md §7.2）：`cancel` 一发出**立刻停止派发**，
/// 已经在飞的请求**不打断、自然结束**。所以返回是"这一轮到此为止"，
/// 不是"请求全被砍了" —— 超时设成 30 秒时，取消后最多还要等 30 秒。
/// 这样做的理由：被强行中断的请求会在样本里留下一批
/// "取消"错误，把错误分布和分位数一起污染掉。
pub async fn run(deps: LoadtestDeps) -> AppResult<RunOutcome> {
    let LoadtestDeps {
        client,
        spec,
        config,
        cancel,
        mut on_progress,
    } = deps;
    config.validate()?;

    // ---- 预热 ----
    //
    // 放在计时开始**之前**、样本直接丢掉：首轮请求里混着 DNS + TCP + TLS，
    // 把它们算进 P99 会让一个 20ms 的接口报出几百毫秒的 P99。
    //
    // 预热也走并发，而且是**同样的 C 路并发**：连接池是按
    // `pool_max_idle_per_host = C` 预热的，串行跑 C 次只会把第一条连接
    // 反复用热，剩下 C-1 条照样是冷的 —— 那样预热等于没做。
    if config.warmup_requests > 0 {
        dispatch(
            &client,
            &spec,
            &config,
            DispatchBatch {
                seq: Arc::new(AtomicU64::new(0)),
                count: config.warmup_requests,
                base: Instant::now(),
            },
            &cancel,
            None,
        )
        .await;
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
    }

    // ---- 正式计时从这里开始 ----
    let started = Instant::now();
    let (tx, rx) = mpsc::channel::<LoadtestSample>(SAMPLE_BUFFER);

    // 收集任务：唯一的累积器所有者（所以直方图不需要锁，见 stats.rs）
    let collector = tokio::spawn(collect(rx, started, on_progress.take()));

    dispatch(
        &client,
        &spec,
        &config,
        DispatchBatch {
            seq: Arc::new(AtomicU64::new(0)),
            count: config.total,
            base: started,
        },
        &cancel,
        Some(&tx),
    )
    .await;

    // 关掉发送端，收集任务把剩下的样本吃完后退出
    drop(tx);
    let (accumulator, samples) = collector
        .await
        .map_err(|e| AppError::Internal(format!("loadtest collector task failed: {e}")))?;

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let mut summary = accumulator.summarize(elapsed_ms, cancel.is_cancelled());
    summary.frozen_functions = frozen_functions(&spec);

    Ok(RunOutcome { summary, samples })
}

// ============================================================
// 派发
// ============================================================

/// 一批派发任务的账本：谁在领号、领到几为止、时间原点在哪。
///
/// 打包成一个结构体而不是三个平铺参数，一是让 `dispatch` 的参数回到 7 个以内，
/// 二是这三样东西**只在同一批内自洽** —— 拿 A 批的 `seq` 配 B 批的 `count`
/// 是没法从类型上看出来的错误，而捏成一个 `&mut`-free 的值之后，
/// 要么整批都对，要么整批都不传。
struct DispatchBatch {
    /// 共享领号器。所有 worker 从它取 `index`，取到 `count` 就收工
    seq: Arc<AtomicU64>,
    count: u64,
    /// 这一批的时间原点：`started_offset_ms` 相对它计算
    base: Instant,
}

/// 开 C 个 worker 领号派发，返回实际派发数。
///
/// `sink` 为 `None` 时样本直接丢掉（预热用）。
///
/// `seq` 收 `Arc<AtomicU64>` 而不是 `&AtomicU64`：每个 worker 都是一个
/// `tokio::spawn` 出来的任务，而 spawn 要求 `'static` —— 借来的引用进不去。
/// 明明 `dispatch` 会 await 完所有 worker 才返回，作用域上借得住，
/// 但 tokio 没有稳定的 scoped spawn（`tokio::task::scope` 还没进 stable），
/// 所以只能退一步用 `Arc`。代价是几次原子引用计数，对一次几十秒的压测可以忽略。
async fn dispatch(
    client: &reqwest::Client,
    spec: &RequestSpec,
    config: &LoadtestConfig,
    batch: DispatchBatch,
    cancel: &CancellationToken,
    sink: Option<&mpsc::Sender<LoadtestSample>>,
) -> u64 {
    let think = Duration::from_millis(config.think_time_ms);
    let mut handles = Vec::with_capacity(config.concurrency as usize);

    for _ in 0..config.concurrency {
        let client = client.clone();
        let spec = spec.clone();
        let cancel = cancel.clone();
        let sink = sink.cloned();
        let seq = Arc::clone(&batch.seq);
        // Copy 的两个值，直接拷进任务里 —— 让 worker 的循环体保持扁平
        let count = batch.count;
        let base = batch.base;

        handles.push(tokio::spawn(async move {
            let mut dispatched = 0u64;
            loop {
                // 取消检查放在**领号之前**：领了号再发现被取消，
                // 等于白白跳掉一个序号，日志里会出现解释不清的空洞
                if cancel.is_cancelled() {
                    break;
                }
                let index = seq.fetch_add(1, Ordering::Relaxed);
                if index >= count {
                    break;
                }

                let started_offset_ms = base.elapsed().as_millis() as i64;
                // 同一个时钟量成功与失败，见模块头第 2 条
                let clock = Instant::now();
                let result = execute(&client, &spec).await;
                let duration_ms = clock.elapsed().as_millis() as i64;
                dispatched += 1;

                if let Some(tx) = sink.as_ref() {
                    let sample = to_sample(index as i64, started_offset_ms, duration_ms, result);
                    // 这里 await 是**有意的背压**，见 SAMPLE_BUFFER 的说明
                    if tx.send(sample).await.is_err() {
                        // 接收端没了（收集任务已退出），继续派发没有意义
                        break;
                    }
                }

                if !think.is_zero() {
                    tokio::time::sleep(think).await;
                }
            }
            dispatched
        }));
    }

    let mut total = 0u64;
    for handle in handles {
        // worker 里只有 HTTP 调用，理论上不会 panic；真 panic 了也不能让整轮压测
        // 直接失败 —— 少几个样本比"报告没了"好，而且派发数会如实体现在 total 里
        if let Ok(n) = handle.await {
            total += n;
        }
    }
    total
}

/// 一次执行的结果 → 一条样本。
///
/// `duration_ms` 由调用方传入（引擎自己量的那个），**不用**响应里的 `total_ms`：
/// 失败路径没有响应，只有引擎的时钟对成功与失败都成立。
fn to_sample(
    seq: i64,
    started_offset_ms: i64,
    duration_ms: i64,
    result: AppResult<HttpResponsePayload>,
) -> LoadtestSample {
    match result {
        Ok(payload) => LoadtestSample {
            id: 0,
            // 落库时才由仓储填。`0` 不是"id 是 0"，是"还没有 id"
            run_id: 0,
            seq,
            status: Some(i64::from(payload.status)),
            duration_ms,
            // 用截断前的实际字节数：截断是**展示**策略，
            // 传输量该是多少就是多少
            bytes: Some(payload.body_size as i64),
            error: None,
            started_offset_ms,
        },
        Err(err) => LoadtestSample {
            id: 0,
            run_id: 0,
            seq,
            // 拿不到响应就是 None。用 0 当哨兵的话，
            // 前端得记住"0 是特殊的"，而 `null` 在类型上就逼着它处理
            status: None,
            duration_ms,
            bytes: None,
            // 存**错误码**不存文案（全局契约，见 crate::error）
            error: Some(err.code().to_string()),
            started_offset_ms,
        },
    }
}

// ============================================================
// 收集
// ============================================================

/// 唯一的累积器所有者：吃样本、每 200ms 推一次进度。
///
/// 返回 `(累积器, 全部样本)`。
async fn collect(
    mut rx: mpsc::Receiver<LoadtestSample>,
    started: Instant,
    mut on_progress: Option<Box<dyn FnMut(ProgressSnapshot) + Send>>,
) -> (StatsAccumulator, Vec<LoadtestSample>) {
    let mut accumulator = StatsAccumulator::new();
    let mut samples: Vec<LoadtestSample> = Vec::new();

    let mut ticker = tokio::time::interval(PROGRESS_INTERVAL);
    // 某一轮 tick 被拖慢时**推迟**而不是补发：补发会在卡顿之后
    // 一次性涌出好几个进度事件，界面上就是"数字原地跳三下"
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut last_tick = Instant::now();
    let mut last_sent = 0u64;

    loop {
        tokio::select! {
            // biased：样本优先于进度。反过来会让进度事件在
            // 高吞吐下不断插队，样本越堆越多直到触发背压
            biased;

            incoming = rx.recv() => match incoming {
                Some(sample) => {
                    accumulator.record(&sample);
                    samples.push(sample);
                }
                // 所有发送端都关了，且队列已空
                None => break,
            },

            _ = ticker.tick() => {
                let now = Instant::now();
                let elapsed = now.duration_since(last_tick).as_secs_f64();
                let sent = accumulator.sent();
                // "当前 RPS" 要的是**这一刻快不快**，所以按两次 tick 之间的
                // 增量算，而不是从开工到现在的平均值 —— 后者在压测中途
                // 会越来越钝，看不出接口正在劣化
                let current_rps = if elapsed > 0.0 {
                    (sent - last_sent) as f64 / elapsed
                } else {
                    0.0
                };
                last_sent = sent;
                last_tick = now;

                if let Some(callback) = on_progress.as_mut() {
                    callback(accumulator.snapshot(
                        started.elapsed().as_millis() as u64,
                        current_rps,
                    ));
                }
            }
        }
    }

    (accumulator, samples)
}

// ============================================================
// 冻结的占位符
// ============================================================

/// 找出「整轮压测里所有请求都会用同一个值」的动态函数占位符。
///
/// 见模块头第 3 条：占位符只解析一次，所以 `{{$uuid}}` 这类本该每请求
/// 都不同的东西会固定成同一个值。这不一定错（压同一个接口本来就不需要
/// 唯一键），但用户必须知道 —— 否则他会以为压测打的是 500 个不同的幂等键。
fn frozen_functions(spec: &RequestSpec) -> Vec<String> {
    let mut out: Vec<String> = scan_spec(spec)
        .into_iter()
        // 只报动态函数。`{{env.*}}` / `{{token.*}}` 冻结是**正确**的：
        // 那本来就该在整轮里保持一致，变了反而说明环境被改了
        .filter(|hit| hit.namespace == Namespace::Func)
        .map(|hit| hit.raw)
        .collect();
    // 去重 + 排序：同一份配置导出两次要能 diff（和统计里错误分组的理由一样）
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::BodyEncoding;

    fn spec_with(url: &str) -> RequestSpec {
        RequestSpec {
            method: "GET".into(),
            url: url.into(),
            params: Vec::new(),
            headers: Vec::new(),
            body_type: Default::default(),
            body: String::new(),
            auth: Default::default(),
            settings: RequestSettings::default(),
            folder_id: None,
            form_fields: Vec::new(),
        }
    }

    fn payload(status: u16, body_size: u64) -> HttpResponsePayload {
        HttpResponsePayload {
            status,
            status_text: "OK".into(),
            http_version: "HTTP/1.1".into(),
            headers: Vec::new(),
            url: "https://x/y".into(),
            redirected: false,
            body: String::new(),
            body_encoding: BodyEncoding::Text,
            body_size,
            truncated: false,
            ttfb_ms: 1,
            download_ms: 1,
            total_ms: 2,
            body_file: None,
            unresolved_placeholders: Vec::new(),
            // 压测不关心 Token 使用报告（它自己就是「拿一份现成 spec 打 N 次」）
            token_usage: None,
            // 也不需要 curl 回显（那是桌面端「看一眼上次发了什么」的入口）
            curl: None,
            curl_plain: None,
        }
    }

    // ---------------- 配置校验 ----------------

    #[test]
    fn default_config_is_valid() {
        LoadtestConfig::default().validate().unwrap();
    }

    #[test]
    fn zero_concurrency_is_rejected() {
        // 并发 0 会让 worker 池开不出任何 worker：派发 0、耗时 0、报告全 0，
        // 看起来像"接口秒回"。这种"静默的成功"必须被挡在门外
        let c = LoadtestConfig {
            concurrency: 0,
            ..LoadtestConfig::default()
        };
        assert!(matches!(
            c.validate().unwrap_err(),
            AppError::InvalidInput {
                field: "concurrency",
                ..
            }
        ));
    }

    #[test]
    fn zero_total_is_rejected() {
        let c = LoadtestConfig {
            total: 0,
            ..LoadtestConfig::default()
        };
        assert!(matches!(
            c.validate().unwrap_err(),
            AppError::InvalidInput { field: "total", .. }
        ));
    }

    #[test]
    fn zero_timeout_is_rejected_because_the_run_would_never_end() {
        let c = LoadtestConfig {
            timeout_ms: 0,
            ..LoadtestConfig::default()
        };
        assert!(matches!(
            c.validate().unwrap_err(),
            AppError::InvalidInput {
                field: "timeoutMs",
                ..
            }
        ));
    }

    #[test]
    fn oversized_runs_are_rejected_with_the_field_name() {
        for (field, mutate) in [
            (
                "total",
                Box::new(|c: &mut LoadtestConfig| c.total = MAX_TOTAL + 1)
                    as Box<dyn Fn(&mut LoadtestConfig)>,
            ),
            (
                "concurrency",
                Box::new(|c: &mut LoadtestConfig| c.concurrency = MAX_CONCURRENCY + 1),
            ),
            (
                "warmupRequests",
                Box::new(|c: &mut LoadtestConfig| c.warmup_requests = MAX_TOTAL + 1),
            ),
        ] {
            let mut c = LoadtestConfig::default();
            mutate(&mut c);
            let err = c.validate().unwrap_err();
            assert!(
                matches!(&err, AppError::InvalidInput { field: f, .. } if *f == field),
                "期望 field = {field}，实际 {err}"
            );
        }
    }

    #[test]
    fn config_round_trips_as_camel_case_json() {
        let c = LoadtestConfig {
            concurrency: 50,
            total: 500,
            timeout_ms: 5000,
            follow_redirects: false,
            keep_alive: true,
            think_time_ms: 10,
            http_version: HttpVersion::Http1,
            warmup_requests: 20,
        };
        let json = serde_json::to_value(&c).unwrap();
        assert_eq!(json["concurrency"], 50);
        assert_eq!(json["thinkTimeMs"], 10);
        assert_eq!(json["httpVersion"], "http1");
        assert_eq!(json["warmupRequests"], 20);

        let back: LoadtestConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back, c);
    }

    /// 老配置（P1 写的表里那套字段）反序列化时不该炸
    #[test]
    fn unknown_or_missing_fields_fall_back_to_defaults() {
        let c: LoadtestConfig = serde_json::from_value(serde_json::json!({
            "concurrency": 5,
            "total": 10,
            "timeoutMs": 1000,
            "followRedirects": false,
            "keepAlive": true,
            "somethingFromTheFuture": 1
        }))
        .unwrap();
        assert_eq!(c.think_time_ms, 0);
        assert_eq!(c.warmup_requests, 0);
        assert_eq!(c.http_version, HttpVersion::Auto);
    }

    // ---------------- 设置覆盖 ----------------

    #[test]
    fn loadtest_config_overrides_only_what_it_declares() {
        let base = RequestSettings {
            proxy: Some("http://127.0.0.1:7890".into()),
            verify_tls: false,
            max_redirects: 3,
            ..RequestSettings::default()
        };

        let c = LoadtestConfig {
            timeout_ms: 1234,
            follow_redirects: true,
            http_version: HttpVersion::Http2,
            ..LoadtestConfig::default()
        };
        let merged = c.apply_to(&base);

        assert_eq!(merged.timeout_ms, 1234);
        assert!(merged.follow_redirects);
        assert_eq!(merged.http_version, HttpVersion::Http2);
        // 代理与 TLS 校验属于「这个接口怎么连」，压测配置管不着
        assert_eq!(merged.proxy.as_deref(), Some("http://127.0.0.1:7890"));
        assert!(!merged.verify_tls);
        assert_eq!(merged.max_redirects, 3);
    }

    // ---------------- 结果 → 样本 ----------------

    #[test]
    fn a_successful_execution_becomes_a_sample_with_status_and_bytes() {
        let s = to_sample(7, 120, 34, Ok(payload(201, 4096)));
        assert_eq!(s.seq, 7);
        assert_eq!(s.started_offset_ms, 120);
        assert_eq!(s.duration_ms, 34);
        assert_eq!(s.status, Some(201));
        assert_eq!(s.bytes, Some(4096));
        assert_eq!(s.error, None);
        assert_eq!(s.run_id, 0, "落库时才由仓储填");
    }

    /// 失败样本必须**带着耗时** —— 这是模块头第 2 条那个坑的回归测试
    #[test]
    fn a_failed_execution_keeps_its_duration() {
        let err = AppError::http(crate::error::codes::TIMEOUT, "timed out");
        let s = to_sample(1, 5000, 30_000, Err(err));

        assert_eq!(s.status, None);
        assert_eq!(s.error.as_deref(), Some(crate::error::codes::TIMEOUT));
        assert_eq!(s.bytes, None);
        // 丢掉的耗时会让一次 30 秒的超时从分位里消失，P95 假性变好
        assert_eq!(
            s.duration_ms, 30_000,
            "失败样本的耗时不能丢，否则最糟的那一段不计入分位"
        );
    }

    #[test]
    fn failure_samples_carry_the_error_code_not_a_sentence() {
        let err = AppError::http(crate::error::codes::DNS, "dns lookup failed for api.x");
        let s = to_sample(0, 0, 1, Err(err));
        assert_eq!(s.error.as_deref(), Some("E_DNS"));
    }

    // ---------------- 冻结的占位符 ----------------

    #[test]
    fn no_dynamic_placeholders_means_no_frozen_functions() {
        let spec = spec_with("https://api.example.com/users?page=1");
        assert!(frozen_functions(&spec).is_empty());
    }

    #[test]
    fn dynamic_function_placeholders_are_reported_verbatim() {
        let spec = spec_with("https://api.example.com/orders/{{$uuid}}?t={{$timestampMs}}");
        let frozen = frozen_functions(&spec);

        assert_eq!(frozen.len(), 2);
        // 报原文而不是名字：界面要能直接在请求里定位到它是哪一处
        assert!(frozen.contains(&"{{$uuid}}".to_string()), "{frozen:?}");
        assert!(
            frozen.contains(&"{{$timestampMs}}".to_string()),
            "{frozen:?}"
        );
    }

    /// `{{env.*}}` / `{{token.*}}` 冻结是**正确**的，不该报成警告
    #[test]
    fn env_and_token_placeholders_are_not_reported() {
        let spec = spec_with("{{env.baseUrl}}/users");
        assert!(frozen_functions(&spec).is_empty());
    }

    #[test]
    fn duplicate_placeholders_are_reported_once() {
        let spec = spec_with("https://x/y?trace={{$uuid}}&id={{$uuid}}");
        let frozen = frozen_functions(&spec);
        assert_eq!(frozen.len(), 1);
        assert_eq!(frozen[0], "{{$uuid}}");
    }

    #[test]
    fn frozen_functions_are_sorted_for_stable_json_diffs() {
        let spec = spec_with("https://x/y?a={{$randomInt}}&b={{$uuid}}");
        assert_eq!(
            frozen_functions(&spec),
            vec!["{{$randomInt}}".to_string(), "{{$uuid}}".to_string()]
        );
    }
}
