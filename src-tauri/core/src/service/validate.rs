//! 入参校验。
//!
//! 抽出来是因为「名字」这条规则被文件夹、请求、环境、Token 源四处共用 ——
//! 各写一份的结果必然是某一天只有三处加上了长度上限。
//!
//! 它住在核心层而不是命令层：导入（`import`）与建请求这几条路径
//! MCP / CLI 也要走，长度上限必须是同一个数。

use crate::error::{AppError, AppResult};

/// 名字长度上限。不是产品限制，只是防住「粘贴一篇文章当名字」把
/// 侧栏和后续的搜索索引撑坏。
pub const MAX_NAME_LEN: usize = 256;

/// 名字：去掉首尾空白后必须非空，且不能超长。
///
/// 返回**裁好的**字符串，调用方必须用返回值写库 —— 否则会出现
/// 「名字看着是空的但入库成了空格」这种两头都不认的行。
pub fn clean_name(raw: &str, field: &'static str) -> AppResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidInput {
            field,
            reason: "name cannot be empty".into(),
        });
    }
    if trimmed.chars().count() > MAX_NAME_LEN {
        return Err(AppError::InvalidInput {
            field,
            reason: format!("name is longer than {MAX_NAME_LEN} characters"),
        });
    }
    Ok(trimmed.to_string())
}
