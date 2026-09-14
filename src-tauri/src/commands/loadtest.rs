//! 压测命令层（design.md §7）。
//!
//! ## 引擎跑在自己的任务里，命令只等结果
//!
//! `start_loadtest` 由前端 `await`，但它**不是**引擎的宿主 —— 引擎连同
//! 「落库 + 收尾」一起放在 `tokio::spawn` 出来的任务里，命令只 await 一个
//! 一次性通道。理由是界面切走（或用户按 F5）会让命令的 future 被丢掉：
//! 如果引擎跑在那个 future 里，一轮压测会半路死掉，库里留下一行
//! `finished_at` 永远为空的残骸 —— 而用户看到的是「接口跑着跑着没反应了」。
//!
//! 分成两个任务之后，收发两端断开只意味着「没人看结果了」，那一轮照样
//! 会被如实记完。代价是出错时命令侧只能从通道里拿到一个已成形的错误。
//!
//! ## 为什么用前端给的 `token` 而不是后端自增 id
//!
//! 与 P2 的 `send_request` / `cancel_request` 同一套约定：进度事件在跑的过程中
//! 就要带上一个标识，而数据库里的 `run.id` 要到建完行才有、且这一刻前端还不知道。
//! 让前端生成一个 `token` 贯穿全程，进度、取消、错误都能对上号。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};

use crate::error::{AppError, AppResult};
use crate::http::{Pipeline, RequestSpec};
use crate::loadtest::engine::{self, LoadtestConfig, LoadtestDeps, RunOutcome};
use crate::loadtest::report::{self, LoadtestChart, ReportLabels};
use crate::loadtest::stats::{LoadtestSummary, ProgressSnapshot};
use crate::loadtest::ActiveRun;
use crate::storage::models::{LoadtestRun, LoadtestSample};
use crate::storage::pool::SqlitePool;
use crate::storage::{repo, with_conn};
use crate::AppState;

/// 进度事件名（design.md §7.2）。
const PROGRESS_EVENT: &str = "reqly://loadtest-progress";

/// 一次历史查询最多返回多少轮。与 `history::list_history` 同一个量级：
/// 列表是给人翻的，不是给机器扫的。
const MAX_RUN_PAGE: i64 = 200;

// ============================================================
// 入参 / 出参
// ============================================================

/// 这一轮压的是哪个请求。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadtestTarget {
    /// 从哪条保存的请求发起。可空 —— 也支持对着未保存的草稿压测。
    /// 存进 `loadtest_run.request_id`（`ON DELETE SET NULL`），
    /// 所以请求被删之后历史记录里那次压测仍然在。
    #[serde(default)]
    pub request_id: Option<i64>,
    /// 请求的显示名，作为**快照**存进这一轮。
    ///
    /// 由前端给而不是后端查：请求随时可能改名或被删，而历史列表要显示的是
    /// 「当时压的是什么」。为空时后端退化成 `METHOD url`。
    #[serde(default)]
    pub request_name: Option<String>,
}

/// 存进 `loadtest_run.config` 的东西。
///
/// 刻意分两层而不是把 `url` 平铺进 [`LoadtestConfig`]：那个类型同时是**前端传进来的
/// 入参形状**，塞进「打了哪个地址」之后，前端每次都得记着把它一起填上 ——
/// 而它压根不是配置项，是这次运行的目标。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredRunConfig<'a> {
    target: StoredTarget<'a>,
    config: &'a LoadtestConfig,
}

#[derive(Debug, Serialize)]
struct StoredTarget<'a> {
    method: &'a str,
    url: &'a str,
}

/// 一轮压测的详情。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadtestRunDetail {
    pub run: LoadtestRun,
    /// 结构化概要。`None` = 这一轮还没跑完，或跑失败没产出概要。
    ///
    /// 单给 `run.summary`（一个裸 JSON）也能用，但前端每次都要自己判形状；
    /// 在这里解析一次，调用方拿到的是「有概要」或「没有」，不是一个要猜的对象。
    pub summary: Option<LoadtestSummary>,
    /// 库里还剩多少条明细。
    ///
    /// 前端靠它区分「图表有数据」和「明细已被保留策略裁掉」——
    /// 后者概要仍然齐全，但画不出图，不说明的话看起来像"没采到数据"。
    pub sample_count: i64,
}

/// 导出结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    /// 落盘的绝对路径。前端拿它去「打开所在目录」。
    pub path: String,
    pub bytes: u64,
    /// 实际用的格式（小写），与请求的 `format` 一致
    pub format: String,
}

/// 进度事件载荷。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent<'a> {
    /// 前端给的 `token`。
    ///
    /// 同时只允许一轮压测，所以它不是为了区分"哪一轮"—— 而是用来**丢掉过期事件**的：
    /// 取消之后可能还有一两个 tick 已经在飞，那时界面状态已经变了，
    /// 按 token 认出来丢掉，比让进度条回跳一下好。
    token: &'a str,
    snapshot: ProgressSnapshot,
}

// ============================================================
// 跑一轮
// ============================================================

/// 跑一轮压测，跑完落库，返回收尾后的详情。
///
/// 是**异步命令**：前端 `await` 它就能拿到最终结果，不需要轮询。
/// 过程数据走 [`PROGRESS_EVENT`] 事件。
///
/// 返回 [`LoadtestRunDetail`] 而不是裸的 `LoadtestRun`：与 `get_loadtest_run`
/// 同一个形状，前端只需要认一种结构（`run.summary` 是个裸 JSON，
/// 每次自己去判形状太容易漏）。
#[tauri::command]
pub async fn start_loadtest(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    token: String,
    target: LoadtestTarget,
    spec: RequestSpec,
    config: LoadtestConfig,
) -> AppResult<LoadtestRunDetail> {
    if token.trim().is_empty() {
        return Err(AppError::InvalidInput {
            field: "token",
            reason: "不能为空：进度事件与取消都要靠它对上号".into(),
        });
    }

    // 先校验再建 run 行。反过来的话，一个 concurrency = 0 的输入会在历史里
    // 留下一行永远没有概要的残骸 —— 用户看到的是"我的压测记录里多了个空的"。
    config.validate()?;

    let request_name = target
        .request_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{} {}", spec.method, spec.url));

    let config_json = serde_json::to_value(StoredRunConfig {
        target: StoredTarget {
            method: &spec.method,
            url: &spec.url,
        },
        config: &config,
    })
    .map_err(|e| AppError::Internal(format!("serialize loadtest run config: {e}")))?;

    let request_id = target.request_id;
    let run = with_conn(&state.pool, move |conn| {
        repo::loadtest::create_run(conn, request_id, &request_name, &config_json)
    })
    .await?;

    // 占坑。压测必须独占，理由见 `loadtest::registry` 的模块说明。
    //
    // 占不上就把刚建的那行删掉 —— 否则历史列表会攒下一堆
    // 「0 个请求、永远没有概要」的残骸，而起因只是用户手快点了两次。
    let run_id = run.id;
    let Some(active) = state.loadtests.begin(run_id) else {
        discard_run(&state.pool, run_id).await;
        return Err(AppError::InvalidInput {
            field: "loadtest",
            reason: "已经有一轮压测在跑：两轮同时打同一个接口，双方测到的都是对方造成的劣化".into(),
        });
    };

    // 模板解析在命令层，不在引擎里 —— 引擎不认识数据库（见 `engine` 的模块说明）。
    //
    // 解析**只做一次**，所以 `{{$uuid}}` 这类动态函数在整轮里会冻结成同一个值。
    // 引擎会把它们报进 summary 的 `frozenFunctions`，由界面如实告诉用户。
    let previous = state
        .last_response
        .lock()
        .ok()
        .and_then(|slot| slot.clone());

    let prepared = match Pipeline::new(&state.pool, &state.clients, &state.tokens)
        .prepare(&spec, previous.as_deref(), Some(active.token()))
        .await
    {
        Ok(prepared) => prepared,
        Err(err) => {
            // 解析失败说明这一轮一个请求都没发出去，同样不该留下记录
            discard_run(&state.pool, run_id).await;
            return Err(err);
        }
    };

    // 压测专用连接池：空闲连接数按并发数铺开。
    // 用日常那套（每 host 8 条空闲）跑 50 并发的话，每轮有 42 条连接要重做
    // TCP + TLS 握手 —— 量出来的「延迟」其实是「握手 + 请求」，
    // 连接池自己成了被测对象。
    let client = state.clients.get_for_loadtest(
        &prepared.spec.settings,
        config.concurrency,
        config.keep_alive,
    )?;

    let pool = state.pool.clone();
    let (tx, rx) = tokio::sync::oneshot::channel::<AppResult<LoadtestRunDetail>>();

    tokio::spawn(async move {
        let result = run_round(pool, app, token, active, client, prepared.spec, config).await;
        // 接收端可能已经不在了（界面切走导致命令 future 被丢）。
        // 那不算错误 —— 上面那条注释解释了为什么引擎必须活得比命令久。
        let _ = tx.send(result);
    });

    // `rx.await` 给的是 `Result<AppResult<_>, RecvError>`：外层是「任务还在不在」，
    // 内层是「那一轮跑成没跑成」。只有任务凭空消失才是内部错误。
    rx.await
        .map_err(|_| AppError::Internal("loadtest task disappeared before finishing".into()))?
}

/// 一轮的完整生命周期：跑 → 落库 → 收尾。
///
/// `active` 按值收进来，**活到函数结束** —— 它在 `Drop` 里让出占坑，
/// 提前丢掉的话用户就能在在飞请求还没收完时再开一轮。
async fn run_round(
    pool: SqlitePool,
    app: tauri::AppHandle,
    token: String,
    active: ActiveRun,
    client: reqwest::Client,
    spec: RequestSpec,
    config: LoadtestConfig,
) -> AppResult<LoadtestRunDetail> {
    let run_id = active.run_id();

    let outcome = engine::run(LoadtestDeps {
        client,
        spec,
        config,
        cancel: active.token().clone(),
        on_progress: Some(Box::new(move |snapshot: ProgressSnapshot| {
            // 发不出去就算了：进度事件是「最新的才有意义」的东西，
            // 为了它去记日志只会把日志刷满
            let _ = app.emit(
                PROGRESS_EVENT,
                ProgressEvent {
                    token: &token,
                    snapshot,
                },
            );
        })),
    })
    .await;

    match outcome {
        Ok(outcome) => persist_outcome(pool, run_id, outcome).await,
        Err(err) => {
            // 引擎没产出概要。采样的写入在**成功路径上**（见 `persist_outcome`），
            // 所以此刻库里一行明细都没有 —— 把 run 行也删掉，
            // 别在历史里留一条「永远跑不完」的记录。
            discard_run(&pool, run_id).await;
            Err(err)
        }
    }
}

/// 落库：写概要 → 批量写采样 → 裁旧明细。
async fn persist_outcome(
    pool: SqlitePool,
    run_id: i64,
    outcome: RunOutcome,
) -> AppResult<LoadtestRunDetail> {
    let RunOutcome { summary, samples } = outcome;
    let summary_json = serde_json::to_value(&summary)
        .map_err(|e| AppError::Internal(format!("serialize loadtest summary: {e}")))?;
    // 先数好再移进闭包 —— `samples` 是 move 进去的
    let sample_count = samples.len() as i64;

    let run = with_conn(&pool, move |conn| {
        // 顺序是刻意的：先 finish 再写采样。这样即使接着写采样时失败、
        // 或者进程被杀，历史里那一轮也是**概要齐全**的，
        // 而不是一行永远显示「进行中」的记录。
        let run = repo::loadtest::finish_run(conn, run_id, &summary_json)?;
        repo::loadtest::insert_samples(conn, run_id, &samples)?;

        // 裁旧明细。**失败不算这一轮失败**：压测成功了、报告也齐了，
        // 清理没做成只是库会大一点，不该让用户看到一条错误提示。
        if let Err(e) = repo::loadtest::prune_samples(conn, repo::loadtest::default_keep_runs()) {
            eprintln!("[reqly] loadtest prune failed: {e}");
        }

        Ok(run)
    })
    .await?;

    Ok(LoadtestRunDetail {
        run,
        summary: Some(summary),
        sample_count,
    })
}

/// 丢掉一轮没有产出的压测记录。
///
/// 删不掉不报错：留下一行 `finished_at` 为空的记录，界面会显示成
/// 「未完成」—— 那是实情，比把失败吞掉强。
async fn discard_run(pool: &SqlitePool, run_id: i64) {
    if let Err(e) = with_conn(pool, move |conn| repo::loadtest::delete_run(conn, run_id)).await {
        eprintln!("[reqly] failed to discard empty loadtest run #{run_id}: {e}");
    }
}

// ============================================================
// 取消与查询
// ============================================================

/// 取消当前这一轮。返回 `false` 表示本来就没在跑（**不是错误**）。
///
/// 不接收 `token`：同时只允许一轮压测（见 `loadtest::registry`），
/// 所以「取消谁」这个问题没有歧义 —— 多要一个参数只是多一个对不上的机会。
///
/// 取消后已经在飞的请求**不打断、自然结束**（`engine` 的模块说明里有理由），
/// 所以超时设成 30 秒时，最多还要等 30 秒这一轮才真的落地。
#[tauri::command]
pub async fn cancel_loadtest(state: State<'_, AppState>) -> AppResult<bool> {
    Ok(state.loadtests.cancel_current())
}

/// 历史压测列表，新的在前。
#[tauri::command]
pub async fn list_loadtest_runs(
    state: State<'_, AppState>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> AppResult<Vec<LoadtestRun>> {
    let limit = limit.unwrap_or(50).clamp(1, MAX_RUN_PAGE);
    let offset = offset.unwrap_or(0).max(0);

    with_conn(&state.pool, move |conn| {
        repo::loadtest::list_runs(conn, limit, offset)
    })
    .await
}

/// 一轮的详情：run 行 + 解析好的概要 + 明细条数。
#[tauri::command]
pub async fn get_loadtest_run(
    state: State<'_, AppState>,
    run_id: i64,
) -> AppResult<LoadtestRunDetail> {
    with_conn(&state.pool, move |conn| {
        let run = repo::loadtest::get_run(conn, run_id)?;
        Ok(LoadtestRunDetail {
            summary: parse_summary(&run),
            sample_count: repo::loadtest::count_samples(conn, run_id)?,
            run,
        })
    })
    .await
}

/// 一轮的采样明细（分页）。
#[tauri::command]
pub async fn get_loadtest_samples(
    state: State<'_, AppState>,
    run_id: i64,
    limit: Option<i64>,
    offset: Option<i64>,
) -> AppResult<Vec<LoadtestSample>> {
    let limit = limit.unwrap_or(500).clamp(1, 10_000);
    let offset = offset.unwrap_or(0).max(0);

    with_conn(&state.pool, move |conn| {
        repo::loadtest::list_samples(conn, run_id, limit, offset)
    })
    .await
}

/// 一轮的图表数据（时间线 / 分布 / 最慢 / 失败）。
///
/// **聚合在后端算**：一轮 50 万条明细整体走 IPC 是几十 MB 的 JSON，
/// 而图表真正需要的只是几十个聚合点（见 `loadtest::report::LoadtestChart`）。
/// 顺带也保证了 HTML 报告里的折线与面板里的折线是**同一个算法**算出来的。
#[tauri::command]
pub async fn get_loadtest_chart(
    state: State<'_, AppState>,
    run_id: i64,
) -> AppResult<LoadtestChart> {
    with_conn(&state.pool, move |conn| {
        // 先确认这一轮存在。明细为空有两种可能 ——「存在但被保留策略裁过」
        // 与「压根没有这一轮」，前者是正常状态，后者要报 NotFound
        repo::loadtest::get_run(conn, run_id)?;
        let samples = repo::loadtest::list_all_samples(conn, run_id)?;
        Ok(report::chart_data(&samples))
    })
    .await
}

/// 删掉一轮压测（含明细，靠外键级联）。
///
/// 正在跑的那一轮不让删：删了之后引擎的收尾会撞 `NotFound`，
/// 而它会顺手清掉自己的 run 行 —— 用户看到的是「取消一下，记录也没了」。
#[tauri::command]
pub async fn delete_loadtest_run(state: State<'_, AppState>, run_id: i64) -> AppResult<()> {
    if state.loadtests.current_run_id() == Some(run_id) {
        return Err(AppError::InvalidInput {
            field: "runId",
            reason: "这一轮正在跑：先取消，等它结束后再删".into(),
        });
    }

    with_conn(&state.pool, move |conn| {
        repo::loadtest::delete_run(conn, run_id)
    })
    .await
}

// ============================================================
// 导出
// ============================================================

/// 导出报告到文件，返回落盘路径。
///
/// `labels` 由前端传（最终来自 i18n）：报表是唯一的「后端产文本」场景，
/// 但文案来源仍然只有一个 —— 见 `loadtest::report` 的模块说明。
#[tauri::command]
pub async fn export_loadtest_report(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    run_id: i64,
    format: String,
    labels: Option<ReportLabels>,
) -> AppResult<ExportResult> {
    let labels = labels.unwrap_or_default();

    // 渲染与读库都在阻塞线程池上做：一轮 50 万样本的 CSV 要拼 20MB 字符串，
    // 放在异步线程上会把其它命令一起卡住。
    let (content, ext) = with_conn(&state.pool, move |conn| {
        let run = repo::loadtest::get_run(conn, run_id)?;
        match format.as_str() {
            // CSV 是**原始样本**，只要有明细就能导 —— 哪怕概要坏了。
            // 反过来说，明细被裁过的那一轮导出来是空表（只有表头），
            // 这是事实，不额外报错。
            "csv" => {
                let samples = repo::loadtest::list_all_samples(conn, run_id)?;
                Ok((report::render_csv(&samples), "csv"))
            }
            "json" => {
                let summary = parse_summary(&run).ok_or_else(|| no_summary_error(run_id))?;
                Ok((report::render_json(&summary)?, "json"))
            }
            "html" => {
                let summary = parse_summary(&run).ok_or_else(|| no_summary_error(run_id))?;
                let samples = repo::loadtest::list_all_samples(conn, run_id)?;
                Ok((
                    report::render_html(&run, &summary, &samples, &labels),
                    "html",
                ))
            }
            other => Err(AppError::InvalidInput {
                field: "format",
                reason: format!("{other} 不支持，只认 html / csv / json"),
            }),
        }
    })
    .await?;

    let path = export_path(&app, run_id, ext)?;
    std::fs::write(&path, content.as_bytes())?;

    Ok(ExportResult {
        path: path.display().to_string(),
        bytes: content.len() as u64,
        format: ext.to_string(),
    })
}

fn no_summary_error(run_id: i64) -> AppError {
    AppError::InvalidInput {
        field: "runId",
        reason: format!("第 {run_id} 轮没有可导出的概要（还没跑完，或者跑失败了）"),
    }
}

/// 导出落在哪。
///
/// 优先**下载**目录：这是「导出文件」约定俗成的落点，用户打开资源管理器就能看到。
/// 取不到才退到数据目录下的 `exports/` —— 那是个隐藏目录，拿到路径也找不到，
/// 所以只能是兜底。
fn export_path(app: &tauri::AppHandle, run_id: i64, ext: &str) -> AppResult<PathBuf> {
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().app_data_dir().map(|d| d.join("exports")))
        .map_err(|e| AppError::Internal(format!("cannot resolve export dir: {e}")))?;
    std::fs::create_dir_all(&dir)?;

    // 文件名带 run id + 时间戳：重复导出不会互相覆盖。
    // 覆盖掉一份已经发出去的报表是**没法挽回**的，而多出来的文件用户随手就能删。
    //
    // 时间戳要洗一遍：`now_iso()` 是 RFC3339（含 `:`），而 `:` 在 Windows
    // 文件名里非法 —— 不洗会得到一个建不出来的路径。
    let stamp: String = crate::storage::now_iso()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    Ok(dir.join(format!("reqly-loadtest-{run_id}-{stamp}.{ext}")))
}

/// 从 `loadtest_run.summary` 里解析结构化概要。
///
/// `{}`（还没跑完 / 跑失败）或形状对不上时返回 `None`，**不报错**：
/// 调用方按「这一轮没有概要」处理即可，那是个正常状态而不是故障。
fn parse_summary(run: &LoadtestRun) -> Option<LoadtestSummary> {
    serde_json::from_value(run.summary.clone()).ok()
}
