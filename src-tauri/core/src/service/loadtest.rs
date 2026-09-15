//! 一轮压测的**落库编排**（原 `commands/loadtest.rs` 的中段）。
//!
//! 为什么这三件事必须共用一份：`loadtest_run.config` 的形状是**跨入口的契约** ——
//! 桌面端读历史时要能解析出「当时压的是哪个地址」。MCP / CLI 建 run 行时
//! 若自己拼一遍 JSON，形状一旦漂移，症状是「AI 跑的那几轮在历史里没有目标地址」。

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::http::RequestSpec;
use crate::loadtest::engine::{LoadtestConfig, RunOutcome};
use crate::loadtest::stats::LoadtestSummary;
use crate::storage::models::LoadtestRun;
use crate::storage::pool::SqlitePool;
use crate::storage::{repo, with_conn};

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

/// 把「这一轮压的是什么 + 配置」压成落库用的 JSON。
///
/// 抽成独立函数而不是留在 `start_loadtest` 里内联：MCP 侧（`mcp::tools`）也要
/// 建 run 行，而**两边写进 `loadtest_run.config` 的形状必须一模一样** ——
/// 否则桌面端读历史时，MCP 跑出来的那几轮解析不出目标地址。
pub fn stored_run_config(
    spec: &RequestSpec,
    config: &LoadtestConfig,
) -> AppResult<serde_json::Value> {
    serde_json::to_value(StoredRunConfig {
        target: StoredTarget {
            method: &spec.method,
            url: &spec.url,
        },
        config,
    })
    .map_err(|e| AppError::Internal(format!("serialize loadtest run config: {e}")))
}

/// 落库：写概要 → 批量写采样 → 裁旧明细。
pub async fn persist_outcome(
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
///
/// MCP 侧复用（`mcp::tools`）：那边同样是「引擎没产出概要就把 run 行抹掉」，
/// 而收尾语义必须与桌面端一致 —— 一份实现，两处调用。
pub async fn discard_run(pool: &SqlitePool, run_id: i64) {
    if let Err(e) = with_conn(pool, move |conn| repo::loadtest::delete_run(conn, run_id)).await {
        eprintln!("[reqly] failed to discard empty loadtest run #{run_id}: {e}");
    }
}
