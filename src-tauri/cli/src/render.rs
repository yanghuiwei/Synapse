//! CLI 的人读输出与错误文案。
//!
//! ## 为什么这些中文写在这里而不是后端
//!
//! 后端契约是**只给错误码、不给文案**（见 `reqly_core::error` 的模块头）。
//! 桌面端那一份在 `src/lib/i18n/zh-CN.ts`，CLI 这一份在这儿 —— 两边都是入口层，
//! 各自对各自的用户说话。把中文塞回后端会让「后端不认识 UI 语言」这条边界塌掉。
//!
//! ## 表格为什么自己算宽度
//!
//! 请求名、文件夹名里都是中文。按 `chars().count()` 对齐会让中文列短一截
//! （终端里一个汉字占两格），整个表歪掉。所以这里有一个最小的 East Asian Width
//! 判断 —— 为它引一个 `unicode-width` 依赖不值得，那张表的核心区间一共十来段。

use std::fmt::Write as _;

use reqly_core::error::AppError;
use reqly_core::service::ops::{
    DeleteRequestResponse, EnvironmentDto, FolderSummary, ImportCurlResponse,
    ListEnvironmentsResponse, ListFoldersResponse, ListRequestsResponse, RefreshTokenResponse,
    RequestDetail, RequestSummary, RunLoadTestResponse, SendRequestResponse,
};

/// 终端里这个字符占几格。只处理「中日韩越全角」与「其余算一格」两种。
fn char_width(c: char) -> usize {
    match c as u32 {
        0x1100..=0x115F      // 谚文字母
        | 0x2E80..=0x303E    // 部首扩展 / 康熙部首 / 中日韩符号
        | 0x3041..=0x33FF    // 假名 / 注音 / 中日韩兼容
        | 0x3400..=0x4DBF    // 扩展 A
        | 0x4E00..=0x9FFF    // 基本区
        | 0xA000..=0xA4CF    // 彝文
        | 0xAC00..=0xD7A3    // 谚文音节
        | 0xF900..=0xFAFF    // 兼容表意文字
        | 0xFE30..=0xFE6F    // 兼容形式
        | 0xFF00..=0xFF60    // 全角
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x3FFFD => 2,
        // 零宽 / 组合符不占格，近似成 1 也不影响可读性
        _ => 1,
    }
}

fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// 一个朴素的对齐表格。列多的时候比逐行 `key: value` 好读得多。
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return "（空）".to_string();
    }

    let cols = headers.len();
    let mut widths: Vec<usize> = headers.iter().map(|h| display_width(h)).collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(slot) = widths.get_mut(index) {
                *slot = (*slot).max(display_width(cell));
            }
        }
    }

    let render_row = |cells: &[String]| -> String {
        let mut line = String::new();
        for (index, cell) in cells.iter().enumerate().take(cols) {
            line.push_str(cell);
            if index + 1 < cols {
                let width = widths[index];
                line.push_str(&" ".repeat(width.saturating_sub(display_width(cell)) + 2));
            }
        }
        line.trim_end().to_string()
    };

    let header_cells: Vec<String> = headers.iter().map(|h| (*h).to_string()).collect();
    let mut out = render_row(&header_cells);
    for row in rows {
        let mut cells = row.clone();
        cells.resize(cols, String::new());
        out.push('\n');
        out.push_str(&render_row(&cells));
    }
    out
}

fn section(title: &str, body: &str) -> String {
    if body.is_empty() {
        return String::new();
    }
    format!("{title}:\n{body}\n")
}

/// 字节数给人看的样子。`bodySize` 是**截断前**的真实大小，偶尔会很大。
fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let value = bytes as f64;
    if value < KIB {
        format!("{bytes} B")
    } else if value < KIB * KIB {
        format!("{:.1} KB", value / KIB)
    } else {
        format!("{:.1} MB", value / (KIB * KIB))
    }
}

// ============================================================
// 各操作的输出
// ============================================================

pub fn folders(response: &ListFoldersResponse) -> String {
    let rows: Vec<Vec<String>> = response
        .folders
        .iter()
        .map(|folder| {
            vec![
                folder.id.to_string(),
                folder
                    .parent_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "-".into()),
                folder.name.clone(),
                if folder.is_token_source { "是" } else { "" }.to_string(),
            ]
        })
        .collect();

    let mut out = format!("工作区 #{}\n", response.workspace_id);
    out.push_str(&table(&["ID", "PARENT", "名称", "TOKEN源"], &rows));
    out.push('\n');
    out
}

/// `folders create` 出来的那一个文件夹。形状与列表里的一行一致。
pub fn folder_created(folder: &FolderSummary) -> String {
    format!(
        "#{} {}（parent {}）\n",
        folder.id,
        folder.name,
        folder
            .parent_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "无（工作区根）".into())
    )
}

pub fn requests(response: &ListRequestsResponse) -> String {
    let mut out = format!("工作区 #{}\n", response.workspace_id);
    out.push_str(&table(
        &["ID", "METHOD", "名称", "URL", "FOLDER"],
        &response
            .requests
            .iter()
            .map(request_row)
            .collect::<Vec<_>>(),
    ));
    out.push('\n');
    out
}

fn request_row(request: &RequestSummary) -> Vec<String> {
    vec![
        request.id.to_string(),
        request.method.clone(),
        request.name.clone(),
        request.url.clone(),
        request
            .folder_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "-".into()),
    ]
}

fn key_value_rows(items: &[reqly_core::service::ops::KeyValueDto]) -> Vec<Vec<String>> {
    items
        .iter()
        .map(|item| {
            vec![
                if item.enabled { "" } else { "×" }.to_string(),
                item.key.clone(),
                item.value.clone(),
            ]
        })
        .collect()
}

pub fn request_detail(request: &RequestDetail) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "#{} {} {}", request.id, request.method, request.name);
    let _ = writeln!(out, "  url:       {}", request.url);
    let _ = writeln!(
        out,
        "  folder:    {}",
        request
            .folder_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "（工作区根）".into())
    );
    let _ = writeln!(out, "  auth:      {}", request.auth.r#type);
    let _ = writeln!(
        out,
        "  settings:  重定向={} 上限={} 校验TLS={} 超时={}ms 版本={} 代理={}",
        request.settings.follow_redirects,
        request.settings.max_redirects,
        request.settings.verify_tls,
        request.settings.timeout_ms,
        request.settings.http_version,
        if request.settings.has_proxy {
            "有"
        } else {
            "无"
        },
    );
    out.push('\n');

    if !request.params.is_empty() {
        out.push_str(&section(
            "Query 参数",
            &table(&["", "KEY", "VALUE"], &key_value_rows(&request.params)),
        ));
    }
    if !request.headers.is_empty() {
        out.push_str(&section(
            "请求头",
            &table(&["", "KEY", "VALUE"], &key_value_rows(&request.headers)),
        ));
    }
    if request.body_type != "none" || !request.body.is_empty() {
        out.push_str(&section(
            &format!("请求体（{}）", request.body_type),
            &request.body,
        ));
    }
    if !request.pre_script.is_empty() {
        out.push_str(&section("前置脚本", &request.pre_script));
    }
    if !request.test_script.is_empty() {
        out.push_str(&section("测试脚本", &request.test_script));
    }
    out
}

pub fn environments(response: &ListEnvironmentsResponse) -> String {
    let mut out = format!("工作区 #{}\n", response.workspace_id);
    if response.environments.is_empty() {
        // 与表格的空态保持一致：说清「这里是空的」，而不是留一片空白
        // 让人怀疑是不是命令没跑对。
        out.push_str("（还没有环境）\n");
        return out;
    }
    for environment in &response.environments {
        out.push_str(&environment_block(environment));
    }
    out.push('\n');
    out
}

pub fn environment_block(environment: &EnvironmentDto) -> String {
    let mut out = format!(
        "环境 #{}{}\n",
        environment.id,
        if environment.is_active {
            "（激活）"
        } else {
            ""
        }
    );
    let rows: Vec<Vec<String>> = environment
        .vars
        .iter()
        .map(|var| {
            vec![
                var.key.clone(),
                var.value.clone(),
                if var.is_secret { "密文" } else { "" }.to_string(),
                if var.enabled { "" } else { "×" }.to_string(),
            ]
        })
        .collect();
    out.push_str(&table(&["KEY", "VALUE", "类型", ""], &rows));
    out.push('\n');
    out
}

pub fn response(response: &SendRequestResponse) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} {} {}",
        response.http_version, response.status, response.status_text
    );
    let _ = writeln!(out, "{}", response.url);
    // 回显这次**实际**用的请求体类型：是 `none` 就说明一个字节都没发。
    // 这是「body 静默没发出去」最后的观测点 —— 出了偏差在这里一眼能看见，
    // 不必去怀疑服务端。
    let _ = writeln!(out, "请求体类型: {}", response.body_type);
    let _ = writeln!(
        out,
        "总耗时 {}ms（TTFB {}ms / 下载 {}ms）{}",
        response.total_ms,
        response.ttfb_ms,
        response.download_ms,
        if response.redirected {
            " · 发生过重定向"
        } else {
            ""
        }
    );
    out.push('\n');

    let headers: Vec<Vec<String>> = response
        .headers
        .iter()
        .map(|header| vec![header.name.clone(), header.value.clone()])
        .collect();
    out.push_str(&section("响应头", &table(&["NAME", "VALUE"], &headers)));

    if !response.unresolved_placeholders.is_empty() {
        let rows: Vec<Vec<String>> = response
            .unresolved_placeholders
            .iter()
            .map(|hit| vec![hit.raw.clone(), hit.reason.clone()])
            .collect();
        out.push_str(&section(
            "没解析掉的占位符（原文已原样发出）",
            &table(&["原文", "原因"], &rows),
        ));
    }

    let size = if response.body_truncated {
        format!(
            "响应体（已截断：只给前 {} / 共 {}）",
            human_bytes(response.body.len() as u64),
            human_bytes(response.body_size)
        )
    } else {
        format!("响应体（{}）", human_bytes(response.body_size))
    };
    let mut body = response.body.clone();
    if body.is_empty() {
        body = "（空）".into();
    }
    out.push_str(&section(&size, &body));
    out
}

pub fn loadtest(response: &RunLoadTestResponse) -> String {
    let mut out = format!("压测轮次 #{}\n", response.run_id);
    let _ = writeln!(
        out,
        "  总计 {} · 成功 {} · 失败 {}",
        response.counts.total, response.counts.succeeded, response.counts.failed
    );
    out.push('\n');
    // 概要本身就是后端算好的 JSON（分位、错误分布、吞吐），原样给出去
    match serde_json::to_string_pretty(&response.summary) {
        Ok(text) => out.push_str(&section("概要", &text)),
        Err(_) => out.push_str(&section("概要", &response.summary.to_string())),
    }
    out
}

pub fn refresh_token(response: &RefreshTokenResponse) -> String {
    let mut out = format!("#{} {}\n", response.source_id, response.source_name);
    match (&response.expires_at, response.expires_in_seconds) {
        (Some(at), Some(in_seconds)) => {
            let _ = writeln!(out, "  过期时间: {at}（还有 {in_seconds} 秒）");
        }
        (None, _) => out.push_str("  过期时间: 永不过期\n"),
        _ => out.push_str("  过期时间: 未知\n"),
    }
    let _ = writeln!(
        out,
        "  refresh token: {}",
        if response.has_refresh_token {
            "有"
        } else {
            "没有"
        }
    );
    out
}

pub fn import_curl(response: &ImportCurlResponse) -> String {
    let mut out = format!("格式 {}\n", response.format);
    let _ = writeln!(
        out,
        "新建 {} · 更新 {} · 跳过 {} · 移除 {}",
        response.counts.created,
        response.counts.updated,
        response.counts.skipped,
        response.counts.removed
    );
    out.push('\n');

    if !response.created.is_empty() {
        out.push_str(&section(
            "本次新建",
            &table(
                &["ID", "METHOD", "名称", "URL", "FOLDER"],
                &response.created.iter().map(request_row).collect::<Vec<_>>(),
            ),
        ));
    }
    if !response.warnings.is_empty() {
        let rows: Vec<Vec<String>> = response
            .warnings
            .iter()
            .map(|warning| vec![warning.clone()])
            .collect();
        out.push_str(&section("警告", &table(&["内容"], &rows)));
    }
    out
}

pub fn move_request(request: &RequestSummary) -> String {
    format!(
        "#{} {} 已移到文件夹 {}\n",
        request.id,
        request.name,
        request
            .folder_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "（工作区根）".into())
    )
}

pub fn delete_request(response: &DeleteRequestResponse) -> String {
    let suffix = if response.restorable {
        "（进了回收站，桌面端可还原）"
    } else {
        ""
    };
    format!("#{} 已删除{suffix}\n", response.request_id)
}

// ============================================================
// 错误文案
// ============================================================

/// `AppError` → 一句中文。
///
/// 按**用户的下一个动作**分类，而不是按异常类型 —— 这也是后端把 DNS / 连接被拒 /
/// TLS 拆成三个码的理由（见 `reqly_core::error::codes` 的注释）。
/// 认不出的码退化成「错误码 + debug」，不编一句含糊的话糊过去。
pub fn describe_error(error: &AppError) -> String {
    use reqly_core::error::codes;
    let params = error.params().unwrap_or(serde_json::Value::Null);

    match error.code() {
        codes::DB => "数据库操作失败".into(),
        codes::DB_LOCKED => "数据库被占用（桌面端可能正在写，稍后重试）".into(),
        codes::MIGRATION => format!(
            "数据库迁移失败（版本 {}）",
            params["version"].as_i64().unwrap_or_default()
        ),
        codes::NOT_FOUND => format!(
            "找不到 {} #{}",
            params["entity"].as_str().unwrap_or("记录"),
            params["id"].as_i64().unwrap_or_default()
        ),
        codes::INVALID_INPUT => format!(
            "参数不合法：{}",
            params["field"].as_str().unwrap_or("未知字段")
        ),
        codes::IO => "文件读写失败".into(),
        codes::INTERNAL => "内部错误".into(),
        codes::DNS => format!(
            "域名解析失败：{}",
            params["host"].as_str().unwrap_or("（未提供主机名）")
        ),
        codes::CONN_REFUSED => "连接被拒绝（服务没起，或端口不对）".into(),
        codes::TLS => "TLS 握手或证书校验失败".into(),
        codes::TIMEOUT => "请求超时".into(),
        codes::REDIRECT => "重定向次数超过上限".into(),
        codes::DECODE => "响应体解码失败".into(),
        codes::URL => format!(
            "请求地址不合法：{}",
            params["raw"].as_str().unwrap_or("（未提供）")
        ),
        codes::REQUEST => "请求构造失败".into(),
        codes::BODY_NOT_JSON => "请求体不是合法 JSON".into(),
        codes::CANCELLED => "请求被取消".into(),
        codes::NETWORK => "网络错误".into(),
        codes::NOT_IMPLEMENTED => format!(
            "该功能还没实现：{}",
            params["feature"].as_str().unwrap_or("未知")
        ),
        codes::READ_ONLY => format!(
            "只读模式：写操作 `{}` 被拒绝",
            params["tool"].as_str().unwrap_or("未知")
        ),
        codes::TOKEN_SOURCE => "Token 源配置不成立".into(),
        codes::TOKEN_REFRESH => "刷新 Token 的请求本身失败了".into(),
        codes::TOKEN_EXTRACT => "取到了响应，但按配置的路径提不出 token".into(),
        codes::TOKEN_EXPIRED => "没有可用缓存，且该 Token 源关闭了自动刷新".into(),
        codes::IMPORT_DETECT => "没能从内容里认出格式".into(),
        codes::IMPORT_JSON => "内容是 JSON，但不合法或不是我们认得的那种".into(),
        codes::IMPORT_PARSE => "格式认出来了，但里面某一条解不动".into(),
        other => format!("未知错误（{other}）"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_takes_two_columns() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("用户列表"), 8);
        // 中英混排：中文按两格，ASCII 按一格（"GET " 是 4，"用户" 是 4）
        assert_eq!(display_width("GET 用户"), 8);
    }

    #[test]
    fn a_table_pads_columns_by_display_width() {
        let text = table(
            &["NAME", "值"],
            &[
                vec!["用户列表".into(), "1".into()],
                vec!["ab".into(), "2".into()],
            ],
        );
        let lines: Vec<&str> = text.lines().collect();

        // 两行的第二列必须起始在**同一显示列**上 —— 这正是按字符数对齐会错的地方。
        // ⚠️ 不能用 `rfind` 的下标直接比：那返回的是**字节**偏移，
        // 中文两行会差出好几位（这条测试第一版就是这么写错的）。
        let column_of_second_cell = |line: &str| -> usize {
            let byte = line
                .rfind(|c: char| c.is_ascii_digit())
                .expect("每行都该有一个数字");
            display_width(&line[..byte])
        };
        assert_eq!(
            column_of_second_cell(lines[1]),
            column_of_second_cell(lines[2]),
            "第二列没对齐：\n{text}"
        );
    }

    #[test]
    fn an_empty_table_says_so_instead_of_printing_a_header() {
        assert_eq!(table(&["A"], &[]), "（空）");
    }

    #[test]
    fn errors_are_described_by_the_next_action() {
        let not_found = AppError::NotFound {
            entity: "request",
            id: 7,
        };
        assert_eq!(describe_error(&not_found), "找不到 request #7");

        let refused = AppError::http(reqly_core::error::codes::CONN_REFUSED, "connect");
        assert!(describe_error(&refused).contains("连接被拒绝"));

        let read_only = AppError::ReadOnly {
            tool: "delete_request",
        };
        assert!(describe_error(&read_only).contains("delete_request"));
    }

    #[test]
    fn dns_failures_name_the_host() {
        let error = AppError::http_with(
            reqly_core::error::codes::DNS,
            "dns lookup failed",
            serde_json::json!({ "host": "api.example.com" }),
        );
        assert_eq!(describe_error(&error), "域名解析失败：api.example.com");
    }

    #[test]
    fn an_unknown_code_falls_back_instead_of_inventing_a_sentence() {
        let error = AppError::http("E_SOMETHING_NEW", "boom");
        assert_eq!(describe_error(&error), "未知错误（E_SOMETHING_NEW）");
    }

    #[test]
    fn bytes_are_shown_in_a_readable_unit() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(3 * 1024 * 1024), "3.0 MB");
    }
}
