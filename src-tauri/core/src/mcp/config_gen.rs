//! MCP 客户端配置片段生成（design.md §8.1）。
//!
//! 这里只负责拼字符串，不猜客户端要什么 —— 两种格式的差异在设计阶段就消化完了：
//! JSON 一家（WorkBuddy / Claude Code / Cursor，`mcpServers`），
//! YAML 一家（Hermes Agent，`mcp_servers`，多带 trust/timeout）。
//!
//! **路径统一输出正斜杠**，这不是审美问题：Windows 的 `C:\Users\...` 在 YAML 双引号里
//! `\U` 会被当成转义序列，而 YAML 里根本不允许 `\U` 这个转义 → 解析直接失败。
//! 更坏的情况是 `\t` `\n` 这类**恰好合法**的转义：路径被静默改写成带制表符的字符串，
//! 配置文件看起来没问题，进程却找不到文件。而这段片段的用途就是让用户原样粘进配置文件，
//! 一旦转义被理解错，用户没有任何线索。正斜杠在 Windows 上被普遍接受，代价为零。

use std::path::Path;

use serde_json::json;

/// JSON 配置里 `mcpServers` 下的键名。Hermes 会给它加上 `mcp_` 前缀。
const SERVER_KEY: &str = "reqly";

/// 启动 MCP Server 需要的两个路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPaths {
    command: String,
    db: String,
}

impl McpPaths {
    /// 收两个路径并规范化成正斜杠。
    ///
    /// 刻意**不做大小写规范化**：Windows 路径不区分大小写，强行统一大小写只会让
    /// 用户在配置文件里看到的路径与他手上那份对不上，反而增加排查成本。
    pub fn new(command: &Path, db: &Path) -> Self {
        Self {
            command: to_forward_slashes(command),
            db: to_forward_slashes(db),
        }
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn db(&self) -> &str {
        &self.db
    }
}

/// WorkBuddy / Claude Code / Cursor 的 `mcpServers` 片段。
pub fn json_snippet(paths: &McpPaths) -> String {
    let entry = json!({
        "command": paths.command,
        "args": ["--db", paths.db],
        "env": {},
    });
    let doc = json!({ "mcpServers": { SERVER_KEY: entry } });

    // 序列化一个自建的 `Value` 不可能失败（没有自定义 Serialize 实现、也没有非字符串键）
    serde_json::to_string_pretty(&doc).expect("serializing a literal JSON object cannot fail")
}

/// Hermes Agent 的 `mcp_servers` 片段（写进 `~/.hermes/config.yaml`）。
///
/// `trust: untrusted` 是**刻意的**：Hermes 在这档下会按每个 tool 的
/// `readOnlyHint` 注解决定是否弹审批 —— 只读的免确认、写操作（尤其
/// `delete_request`）必须用户点头。这正是我们要的行为，不要为了"省事"调高它。
pub fn yaml_snippet(paths: &McpPaths) -> String {
    // 用双引号包路径 + 输出已转正斜杠的路径 → YAML 里不存在反斜杠转义问题
    format!(
        "mcp_servers:\n  {SERVER_KEY}:\n    command: \"{command}\"\n    args: [\"--db\", \"{db}\"]\n    enabled: true\n    timeout: 120\n    connect_timeout: 60\n    trust: untrusted\n",
        command = paths.command,
        db = paths.db,
    )
}

fn to_forward_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> McpPaths {
        McpPaths::new(
            Path::new(r"C:\Program Files\reqly\reqly-mcp.exe"),
            Path::new(r"C:\Users\example\AppData\Roaming\com.reqly.desktop\reqly.db"),
        )
    }

    #[test]
    fn paths_are_normalised_to_forward_slashes() {
        let paths = sample();
        assert_eq!(paths.command(), "C:/Program Files/reqly/reqly-mcp.exe");
        assert!(paths.db().ends_with("com.reqly.desktop/reqly.db"));
        assert!(!paths.command().contains('\\'));
        assert!(!paths.db().contains('\\'));
    }

    #[test]
    fn json_snippet_round_trips_and_keeps_the_shape_clients_expect() {
        let snippet = json_snippet(&sample());
        let doc: serde_json::Value = serde_json::from_str(&snippet).expect("must be valid JSON");

        let entry = &doc["mcpServers"]["reqly"];
        assert_eq!(entry["command"], "C:/Program Files/reqly/reqly-mcp.exe");
        assert_eq!(entry["args"][0], "--db");
        assert_eq!(
            entry["args"][1],
            "C:/Users/example/AppData/Roaming/com.reqly.desktop/reqly.db"
        );
        assert!(entry["env"].is_object());
        assert!(entry["env"]
            .as_object()
            .expect("env is an object")
            .is_empty());
    }

    #[test]
    fn json_snippet_does_not_escape_backslashes_because_there_are_none() {
        // 守住「正斜杠」这条不变式：JSON 里出现 `\\` 说明规范化漏了
        let snippet = json_snippet(&sample());
        assert!(
            !snippet.contains(r"\\"),
            "JSON 片段里不该出现反斜杠：{snippet}"
        );
    }

    #[test]
    fn yaml_snippet_has_no_backslash_at_all() {
        // YAML 单/双引号里都可能被转义理解错，所以这里直接断言"一个反斜杠都没有"
        let snippet = yaml_snippet(&sample());
        assert!(
            !snippet.contains('\\'),
            "YAML 片段里不该出现反斜杠：{snippet}"
        );
    }

    #[test]
    fn yaml_snippet_keeps_hermes_keys_and_untrusted_by_default() {
        let snippet = yaml_snippet(&sample());
        assert!(snippet.starts_with("mcp_servers:\n  reqly:\n"));
        assert!(snippet.contains("    command: \"C:/Program Files/reqly/reqly-mcp.exe\"\n"));
        assert!(snippet.contains("    args: [\"--db\", \"C:/Users/"));
        assert!(snippet.contains("    enabled: true\n"));
        assert!(snippet.contains("    timeout: 120\n"));
        assert!(snippet.contains("    connect_timeout: 60\n"));
        assert!(snippet.contains("    trust: untrusted\n"));
    }

    #[test]
    fn a_path_with_spaces_stays_quoted_in_yaml() {
        // 空格是这里唯一会咬人的字符：不带引号的 YAML 标量会在空格处断掉，
        // 变成 command 只到 "C:/Program" —— 而"配置文件看起来对"正是最难查的那种
        let snippet = yaml_snippet(&sample());
        assert!(snippet.contains("\"C:/Program Files/reqly/reqly-mcp.exe\""));
    }
}
