//! 压测仓储。
//!
//! 采样表是**写入量最大**的表：一轮 5000 请求就是 5000 行。
//! 所以两个关键点：
//! * 批量写入必须放在**一个事务**里 —— 逐行自动提交在 WAL 下意味着
//!   5000 次 fsync，会把压测本身变成磁盘基准测试。
//! * 采样用 prepared statement 复用，避免每行重新编译 SQL。

use rusqlite::{Connection, Row};

use crate::error::{AppError, AppResult};
use crate::storage::models::{LoadtestRun, LoadtestSample};
use crate::storage::now_iso;
use crate::storage::repo::{opt_text, parse_json_column, to_json_column};

const RUN_COLUMNS: &str = "id, request_id, request_name, config, summary, started_at, finished_at";
const SAMPLE_COLUMNS: &str =
    "id, run_id, seq, status, duration_ms, bytes, error, started_offset_ms";

fn map_run(row: &Row<'_>) -> AppResult<LoadtestRun> {
    let config_raw: String = row.get("config")?;
    let summary_raw: String = row.get("summary")?;
    Ok(LoadtestRun {
        id: row.get("id")?,
        request_id: row.get("request_id")?,
        request_name: row.get("request_name")?,
        config: parse_json_column(&config_raw, "loadtest_run.config")?,
        summary: parse_json_column(&summary_raw, "loadtest_run.summary")?,
        started_at: row.get("started_at")?,
        finished_at: opt_text(row, "finished_at")?,
    })
}

fn map_sample(row: &Row<'_>) -> rusqlite::Result<LoadtestSample> {
    Ok(LoadtestSample {
        id: row.get("id")?,
        run_id: row.get("run_id")?,
        seq: row.get("seq")?,
        status: row.get("status")?,
        duration_ms: row.get("duration_ms")?,
        bytes: row.get("bytes")?,
        error: row.get("error")?,
        started_offset_ms: row.get("started_offset_ms")?,
    })
}

fn collect_runs(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> AppResult<Vec<LoadtestRun>> {
    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query(params)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(map_run(row)?);
    }
    Ok(out)
}

/// 开一轮压测。`config` 是前端传来的原始配置（并发数、时长等）。
///
/// `summary` 先写 `{}` 占位 —— 列是 NOT NULL，跑完再用 [`finish_run`] 覆盖。
pub fn create_run(
    conn: &Connection,
    request_id: Option<i64>,
    request_name: &str,
    config: &serde_json::Value,
) -> AppResult<LoadtestRun> {
    conn.execute(
        "INSERT INTO loadtest_run (request_id, request_name, config, summary, started_at)
         VALUES (?1, ?2, ?3, '{}', ?4)",
        rusqlite::params![
            request_id,
            request_name,
            to_json_column(config, "loadtest_run.config")?,
            now_iso(),
        ],
    )?;
    get_run(conn, conn.last_insert_rowid())
}

/// 收尾：写入统计汇总与结束时间。
///
/// 幂等 —— 重复调用只会覆盖 summary（压测中断后重试收尾是常见情况）。
pub fn finish_run(
    conn: &Connection,
    id: i64,
    summary: &serde_json::Value,
) -> AppResult<LoadtestRun> {
    let affected = conn.execute(
        "UPDATE loadtest_run SET summary = ?2, finished_at = ?3 WHERE id = ?1",
        rusqlite::params![
            id,
            to_json_column(summary, "loadtest_run.summary")?,
            now_iso()
        ],
    )?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "loadtest_run",
            id,
        });
    }
    get_run(conn, id)
}

/// 读单轮。
///
/// ⚠️ 这里**不能**用 `conn.query_row(sql, [id], map_run)` —— `query_row` 要求闭包
/// 返回 `rusqlite::Result`，而 `map_run` 返回 `AppResult`（它要把 JSON 解析失败
/// 变成带列名的 `E_INTERNAL`）。所以走显式 `rows.next()`，与 `request.rs::get` 一致。
pub fn get_run(conn: &Connection, id: i64) -> AppResult<LoadtestRun> {
    let sql = format!("SELECT {RUN_COLUMNS} FROM loadtest_run WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([id])?;
    match rows.next()? {
        Some(row) => map_run(row),
        None => Err(AppError::NotFound {
            entity: "loadtest_run",
            id,
        }),
    }
}

pub fn list_runs(conn: &Connection, limit: i64, offset: i64) -> AppResult<Vec<LoadtestRun>> {
    let sql = format!(
        "SELECT {RUN_COLUMNS} FROM loadtest_run
         ORDER BY started_at DESC, id DESC
         LIMIT ?1 OFFSET ?2"
    );
    collect_runs(conn, &sql, &[&limit, &offset])
}

/// 硬删除。采样由外键级联带走。
pub fn delete_run(conn: &Connection, id: i64) -> AppResult<()> {
    let affected = conn.execute("DELETE FROM loadtest_run WHERE id = ?1", [id])?;
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "loadtest_run",
            id,
        });
    }
    Ok(())
}

/// 批量写采样。返回写入行数。
///
/// 同一个事务 + 复用 prepared statement；5000 行的耗时通常在几十毫秒量级。
pub fn insert_samples(
    conn: &mut Connection,
    run_id: i64,
    samples: &[LoadtestSample],
) -> AppResult<usize> {
    if samples.is_empty() {
        return Ok(0);
    }

    let tx = conn.transaction()?;

    // 存在性检查放在事务内：避免「检查通过 → run 被删 → 插入报外键错误」
    let exists: i64 = tx.query_row(
        "SELECT COUNT(*) FROM loadtest_run WHERE id = ?1",
        [run_id],
        |r| r.get(0),
    )?;
    if exists == 0 {
        return Err(AppError::NotFound {
            entity: "loadtest_run",
            id: run_id,
        });
    }

    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO loadtest_sample
               (run_id, seq, status, duration_ms, bytes, error, started_offset_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for s in samples {
            stmt.execute(rusqlite::params![
                run_id,
                s.seq,
                s.status,
                s.duration_ms,
                s.bytes,
                s.error,
                s.started_offset_ms,
            ])?;
        }
    }

    tx.commit()?;
    Ok(samples.len())
}

pub fn list_samples(
    conn: &Connection,
    run_id: i64,
    limit: i64,
    offset: i64,
) -> AppResult<Vec<LoadtestSample>> {
    let sql = format!(
        "SELECT {SAMPLE_COLUMNS} FROM loadtest_sample
         WHERE run_id = ?1 ORDER BY seq ASC LIMIT ?2 OFFSET ?3"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![run_id, limit, offset], map_sample)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn count_samples(conn: &Connection, run_id: i64) -> AppResult<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM loadtest_sample WHERE run_id = ?1",
        [run_id],
        |r| r.get(0),
    )?)
}

/// 读一轮的**全部**采样。给导出用。
///
/// 单独开一个函数，而不是让调用方给 [`list_samples`] 传 `limit = -1`：
/// SQLite 里 `LIMIT -1` 确实是「不限」，但把这个魔法数散落在调用点上，
/// 读代码的人第一反应是「这里写错了」。一个具名函数把这个约定收在一处。
///
/// 上限由 [`crate::loadtest::MAX_TOTAL`] 保证（一轮最多 50 万条，约 30MB），
/// 所以这里可以放心一次性读进内存 —— 导出本来就是一次性动作。
pub fn list_all_samples(conn: &Connection, run_id: i64) -> AppResult<Vec<LoadtestSample>> {
    let sql = format!(
        "SELECT {SAMPLE_COLUMNS} FROM loadtest_sample
         WHERE run_id = ?1 ORDER BY seq ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([run_id], map_sample)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 默认保留多少轮的采样明细。
///
/// 为什么是「保留 N 轮**明细**」而不是「保留 N 轮」：两个表的量级差三个数量级 ——
/// `loadtest_run` 一行约 1KB（名字 + config + summary），而一轮 50 万请求的
/// `loadtest_sample` 约 30MB。design.md §7.4 又要「历史压测记录列表，可对比两次运行」，
/// 而对比用的是 run 行里的 summary，不是逐条采样。所以裁的只能是明细。
///
/// 20 轮的上限是这么来的：常见的一轮是几百到几千请求（几十 KB～几百 KB），
/// 20 轮也就几 MB；真有人连跑 20 轮 50 万请求，那是 600MB —— 属于
/// design.md §7.5 明确划到「不做」的长稳测试范畴，不在优化目标里。
const DEFAULT_KEEP_RUNS: i64 = 20;

/// 供命令层默认使用。见 [`DEFAULT_KEEP_RUNS`] 的说明。
pub fn default_keep_runs() -> i64 {
    DEFAULT_KEEP_RUNS
}

/// 裁掉旧轮次的采样明细，只留最近 [`DEFAULT_KEEP_RUNS`] 轮。
///
/// 返回删除的行数。
///
/// **不动 `loadtest_run`**：历史列表、跨轮对比、以及报告里的概要数字都要靠它。
/// 被裁过明细的那一轮会呈现为「概要齐全、图表和样本为空」，
/// 界面靠 `count_samples(run) == 0 && summary.total > 0` 认出来并说明白
/// —— 这比悄悄少一截数据好。
///
/// `keep_runs < 1` 直接拒绝：`LIMIT 0` 会把**刚跑完的这一轮**的明细也删掉，
/// 而那正是用户马上要看的报告。这种"参数写错就静默毁掉唯一有用的数据"
/// 必须在入口拦住。
pub fn prune_samples(conn: &Connection, keep_runs: i64) -> AppResult<u64> {
    if keep_runs < 1 {
        return Err(AppError::InvalidInput {
            field: "keepRuns",
            reason: "至少为 1：保留 0 轮会连刚跑完的那一轮明细一起删掉".into(),
        });
    }

    // `started_at` 精度到毫秒，同一批创建的 run 可能完全相同，用 id 兜底排序
    // —— 与 `list_runs` 保持同一套顺序，否则会出现「列表里还在、明细没了」
    // 的交叉状态。
    //
    // 这里不需要 `finished_at IS NOT NULL` 之类的保护：正在跑的那一轮一定是
    // 最新的（命令层保证同时只有一轮），因此必然落在 keep 集合里。
    let deleted = conn.execute(
        "DELETE FROM loadtest_sample
         WHERE run_id NOT IN (
           SELECT id FROM loadtest_run ORDER BY started_at DESC, id DESC LIMIT ?1
         )",
        [keep_runs],
    )?;
    Ok(deleted as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::migrate;
    use serde_json::json;

    fn db() -> Connection {
        let mut conn = Connection::open_in_memory().expect("db");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        migrate::run(&mut conn).expect("migrate");
        conn.execute_batch(
            "INSERT INTO workspace (id, name, created_at, updated_at)
               VALUES (1, 'w', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');
             INSERT INTO request (id, workspace_id, name, created_at, updated_at)
               VALUES (1, 1, 'health', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z');",
        )
        .unwrap();
        conn
    }

    fn sample(run_id: i64, seq: i64) -> LoadtestSample {
        LoadtestSample {
            id: 0,
            run_id,
            seq,
            status: Some(200),
            duration_ms: 30 + seq,
            bytes: Some(100),
            error: None,
            started_offset_ms: seq * 10,
        }
    }

    #[test]
    fn create_and_finish_run() {
        let conn = db();
        let run = create_run(&conn, Some(1), "health", &json!({ "concurrency": 10 })).unwrap();
        assert_eq!(run.request_name, "health");
        assert_eq!(run.config["concurrency"], 10);
        assert_eq!(run.summary, json!({}), "summary starts as an empty object");
        assert!(run.finished_at.is_none());

        let done = finish_run(&conn, run.id, &json!({ "rps": 120.5, "p95": 88 })).unwrap();
        assert_eq!(done.summary["rps"], 120.5);
        assert!(done.finished_at.is_some());
        assert!(done.finished_at.unwrap().ends_with('Z'));
    }

    #[test]
    fn insert_samples_bulk_and_read_back_in_order() {
        let mut conn = db();
        let run = create_run(&conn, Some(1), "health", &json!({})).unwrap();

        let batch: Vec<LoadtestSample> = (0..250).map(|i| sample(run.id, i)).collect();
        assert_eq!(insert_samples(&mut conn, run.id, &batch).unwrap(), 250);
        assert_eq!(count_samples(&conn, run.id).unwrap(), 250);

        let page = list_samples(&conn, run.id, 3, 10).unwrap();
        assert_eq!(page.len(), 3);
        assert_eq!(page[0].seq, 10, "samples must come back ordered by seq");
        assert_eq!(page[0].duration_ms, 40);
        assert_eq!(page[2].seq, 12);
    }

    #[test]
    fn empty_batch_is_a_noop() {
        let mut conn = db();
        let run = create_run(&conn, None, "x", &json!({})).unwrap();
        assert_eq!(insert_samples(&mut conn, run.id, &[]).unwrap(), 0);
    }

    /// 采样插入前必须确认 run 还在，否则会直接撞外键约束
    #[test]
    fn insert_samples_rejects_unknown_run_without_partial_write() {
        let mut conn = db();
        let run = create_run(&conn, None, "x", &json!({})).unwrap();
        delete_run(&conn, run.id).unwrap();

        assert!(matches!(
            insert_samples(&mut conn, run.id, &[sample(run.id, 0)]).unwrap_err(),
            AppError::NotFound {
                entity: "loadtest_run",
                ..
            }
        ));
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM loadtest_sample", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 0, "failed batch must not leave partial rows");
    }

    #[test]
    fn delete_run_cascades_to_samples() {
        let mut conn = db();
        let run = create_run(&conn, None, "x", &json!({})).unwrap();
        insert_samples(&mut conn, run.id, &[sample(run.id, 0), sample(run.id, 1)]).unwrap();

        delete_run(&conn, run.id).unwrap();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM loadtest_sample", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    /// request 被删后 run 仍在（ON DELETE SET NULL），历史压测报告不能丢
    #[test]
    fn run_survives_request_deletion() {
        let conn = db();
        let run = create_run(&conn, Some(1), "health", &json!({})).unwrap();
        conn.execute("DELETE FROM request WHERE id = 1", [])
            .unwrap();

        let after = get_run(&conn, run.id).unwrap();
        assert_eq!(after.request_id, None);
        assert_eq!(after.request_name, "health", "name snapshot must remain");
    }

    #[test]
    fn runs_are_listed_newest_first() {
        let conn = db();
        for name in ["a", "b", "c"] {
            create_run(&conn, None, name, &json!({})).unwrap();
        }
        let runs = list_runs(&conn, 10, 0).unwrap();
        assert_eq!(runs.len(), 3);
        // started_at 精度到毫秒，同一批里可能相同，靠 id DESC 兜底
        assert_eq!(runs[0].request_name, "c");
        assert_eq!(list_runs(&conn, 2, 0).unwrap().len(), 2);
    }

    #[test]
    fn missing_run_reports_not_found() {
        let conn = db();
        assert!(matches!(
            get_run(&conn, 77).unwrap_err(),
            AppError::NotFound {
                entity: "loadtest_run",
                ..
            }
        ));
        assert!(matches!(
            finish_run(&conn, 77, &json!({})).unwrap_err(),
            AppError::NotFound { .. }
        ));
    }

    // ---------------- 保留策略 ----------------

    /// 建 `n` 轮，每轮 `per_run` 条采样。返回按创建顺序排列的 run id。
    fn seed_runs(conn: &mut Connection, n: i64, per_run: i64) -> Vec<i64> {
        let mut ids = Vec::new();
        for i in 0..n {
            let run = create_run(conn, None, &format!("r{i}"), &json!({})).unwrap();
            let batch: Vec<LoadtestSample> = (0..per_run).map(|s| sample(run.id, s)).collect();
            insert_samples(conn, run.id, &batch).unwrap();
            ids.push(run.id);
        }
        ids
    }

    #[test]
    fn prune_keeps_only_the_newest_runs_samples() {
        let mut conn = db();
        let ids = seed_runs(&mut conn, 5, 4);

        // 留最近 2 轮：删掉 3 轮 × 4 条 = 12 条
        assert_eq!(prune_samples(&conn, 2).unwrap(), 12);

        for (i, id) in ids.iter().enumerate() {
            let left = count_samples(&conn, *id).unwrap();
            if i >= 3 {
                assert_eq!(left, 4, "第 {i} 轮应当被保留");
            } else {
                assert_eq!(left, 0, "第 {i} 轮的明细应当被裁掉");
            }
        }
    }

    /// 这条是保留策略的**核心安全属性**：刚跑完的那一轮明细必须还在。
    /// 用户跑完压测第一件事就是看图表，而图表的数据源正是采样明细。
    #[test]
    fn prune_never_touches_the_newest_run() {
        let mut conn = db();
        let ids = seed_runs(&mut conn, 6, 3);
        let newest = *ids.last().unwrap();

        prune_samples(&conn, 1).unwrap();

        assert_eq!(
            count_samples(&conn, newest).unwrap(),
            3,
            "最新一轮的明细被裁掉了 —— 报告会变成空的"
        );
    }

    /// run 行必须活下来：历史列表、跨轮对比、概要数字都靠它。
    /// 裁掉的只是明细，这跟「删掉一次压测记录」是两回事。
    #[test]
    fn prune_keeps_run_rows_so_history_still_lists_them() {
        let mut conn = db();
        seed_runs(&mut conn, 5, 2);

        prune_samples(&conn, 1).unwrap();

        let runs = list_runs(&conn, 50, 0).unwrap();
        assert_eq!(runs.len(), 5, "run 行不该被保留策略删掉");
        // 被裁明细的那几轮概要仍然读得出来
        assert_eq!(runs[0].request_name, "r4");
        assert_eq!(runs[4].request_name, "r0");
    }

    #[test]
    fn prune_is_a_noop_when_under_the_limit() {
        let mut conn = db();
        seed_runs(&mut conn, 3, 5);
        assert_eq!(prune_samples(&conn, 10).unwrap(), 0);
        assert_eq!(prune_samples(&conn, 3).unwrap(), 0);
    }

    /// `keep_runs = 0` 会连最新的那一轮一起删 —— 这类"参数写错就毁掉唯一有用数据"
    /// 的输入必须在入口拒掉，而不是靠调用方自觉。
    #[test]
    fn prune_refuses_zero_instead_of_emptying_everything() {
        let mut conn = db();
        let ids = seed_runs(&mut conn, 2, 3);

        for bad in [0, -1] {
            assert!(matches!(
                prune_samples(&conn, bad).unwrap_err(),
                AppError::InvalidInput {
                    field: "keepRuns",
                    ..
                }
            ));
        }
        // 拒绝之后一条都不能少
        for id in ids {
            assert_eq!(count_samples(&conn, id).unwrap(), 3);
        }
    }

    #[test]
    fn prune_on_an_empty_database_is_harmless() {
        let conn = db();
        assert_eq!(prune_samples(&conn, 5).unwrap(), 0);
    }
}
