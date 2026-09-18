//! 仓储层。
//!
//! 所有函数都是**同步**的，接收 `&Connection`。异步边界统一由
//! [`crate::storage::with_conn`] 提供 —— 这样 SQL 代码可以直接写单测，
//! 不需要搭 tokio 运行时。
//!
//! 列名映射约定：DB 用 snake_case，模型用 camelCase，映射写在各自的 `map_row`。

pub mod env;
pub mod folder;
pub mod history;
pub mod loadtest;
pub mod request;
pub mod setting;
pub mod token;
pub mod workspace;

use rusqlite::Row;
use serde::de::DeserializeOwned;

use crate::error::{AppError, AppResult};

/// 读一列可空的 `TEXT`，NULL 映射成 `None`。
///
/// rusqlite 的 `Option<String>` 已经能处理 NULL，但显式包一层能让
/// 「这一列允许为空」这件事在 SQL 映射处一眼可见。
pub(crate) fn opt_text(row: &Row<'_>, idx: &str) -> rusqlite::Result<Option<String>> {
    row.get::<_, Option<String>>(idx)
}

/// 读一列可空的 `0/1` 布尔标记，NULL 映射成 `None`。
///
/// 三态而不是 `bool`：NULL 的意思是「这条记录早于加这一列的那次迁移」，
/// 与「明确是 false」不是一回事。合成 `false` 会让升级上来的老库里
/// 每一行都显示成一个确定的结论，而那个结论我们其实并不知道。
pub(crate) fn opt_flag(row: &Row<'_>, idx: &str) -> rusqlite::Result<Option<bool>> {
    Ok(row.get::<_, Option<i64>>(idx)?.map(|v| v != 0))
}

/// `ORDER BY sort_order, id` 的复用片段 —— 保证同 sort_order 时顺序稳定，
/// 否则列表在两次查询间可能交换位置，前端会出现「莫名重排」。
pub(crate) const ORDER_BY_SORT: &str = " ORDER BY sort_order ASC, id ASC";

/// 给**没有 sort_order 列**的表用的稳定排序（如 `environment`，见 design.md §3.2）。
/// 按名称排序对用户来说是可预期的；将来若要支持手工拖动排序，
/// 再加一条 002 迁移补 `sort_order` 列。
pub(crate) const ORDER_BY_NAME: &str = " ORDER BY name ASC, id ASC";

/// 把 JSON 列反序列化成具体类型。
///
/// 解析失败**不回退成默认值**：那等于静默丢数据（下一次保存就把坏数据覆盖掉了）。
/// 宁可报错并把列名带出来，让人能定位。
pub(crate) fn parse_json_column<T: DeserializeOwned>(
    raw: &str,
    column: &'static str,
) -> AppResult<T> {
    serde_json::from_str(raw).map_err(|e| {
        AppError::Internal(format!(
            "column `{column}` contains invalid JSON ({e}); raw value starts with: {}",
            raw.chars().take(64).collect::<String>()
        ))
    })
}

/// 把值序列化成 JSON 列。正常情况下不会失败。
pub(crate) fn to_json_column<T: serde::Serialize>(
    value: &T,
    column: &'static str,
) -> AppResult<String> {
    serde_json::to_string(value)
        .map_err(|e| AppError::Internal(format!("failed to serialize column `{column}`: {e}")))
}

/// 布尔 → SQLite 的 0/1。rusqlite 的 `ToSql for bool` 其实已经这么做了，
/// 这个助手只是让 `params![]` 里的意图更明确。
pub(crate) fn bool_to_int(v: bool) -> i64 {
    i64::from(v)
}
