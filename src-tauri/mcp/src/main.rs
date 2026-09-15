//! `reqly-mcp` —— 独立的 MCP Server 可执行文件（design.md §8.1）。
//!
//! 这个 package **不依赖 tauri**（只依赖 `reqly-core` + rmcp + axum），
//! 所以它编出来的 exe 只带核心层，不带整个 GUI 框架 —— 拆之前是 21.5 MB。
//!
//! ## 为什么是独立进程，而不是让桌面 App 兼做 MCP server
//!
//! AI 客户端（Claude Code / Cursor / WorkBuddy / Hermes）启动 MCP server 的方式是
//! **spawn 一个进程**。如果 server 必须由桌面 App 提供，用户就得先开着 App 才能用 AI
//! —— 而「不想开 App 也想让 AI 查一下接口」正是这个功能的存在理由。
//! 独立进程 + `--db` 直连同一个 SQLite（WAL 多进程并发读）把这两件事解耦了。
//!
//! ## 为什么手写参数解析而不是引 clap
//!
//! 一共四个参数、没有子命令、没有环境变量回退。为它引入一个（会连带拉起
//! `anstream`/`clap_lex` 等一串）依赖，换来的只是 `--help` 的排版。
//!
//! ## stdout 是协议流
//!
//! 这个程序的 stdout 从头到尾都归 JSON-RPC。参数错误、启动失败、运行日志
//! 一律走 **stderr**，最后以非零码退出 —— 客户端会把非零退出当作「server 起不来」
//! 报给用户，而那一行 stderr 就是唯一的线索，所以它必须说清楚是什么坏了。

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use reqly_core::AppState;
use reqly_mcp::transport::{serve_http, serve_stdio, DEFAULT_HTTP_PORT};

/// 参数错误的退出码。与「启动失败」区分开：2 = 你命令行写错了，
/// 1 = 参数没问题但起不来（库打不开、端口被占）。
const EXIT_USAGE: u8 = 2;
const EXIT_FAILURE: u8 = 1;

const USAGE: &str = "\
reqly-mcp —— 把 Reqly 工作台暴露成 MCP 工具集

用法：
  reqly-mcp --db <path> [--http] [--port <n>] [--read-only]

参数：
  --db <path>   必填。Reqly 的 SQLite 文件；桌面端「设置」页能直接看到它的绝对路径。
                文件不存在会新建一个空库（迁移会自动跑）。
  --http        走回环 HTTP 通道（默认是 stdio）。只绑 127.0.0.1，不对外。
  --port <n>    HTTP 端口，默认 7777。只有 --http 时有意义。
  --read-only   只读模式：所有写类 tool 直接返回 permission_denied。
  -h, --help    显示这段说明。

示例：
  reqly-mcp --db \"%APPDATA%/com.reqly.desktop/reqly.db\"
  reqly-mcp --db \"%APPDATA%/com.reqly.desktop/reqly.db\" --http --port 7777
  reqly-mcp --db \"%APPDATA%/com.reqly.desktop/reqly.db\" --read-only
";

#[derive(Debug)]
struct Options {
    db: PathBuf,
    http: bool,
    port: u16,
    read_only: bool,
}

/// 解析命令行。返回 `Err` 的是**给人看的一句话**（会打到 stderr 上），
/// 不是 `AppError` —— 这一类错误发生在运行时之前，用户需要的是「哪个参数写错了」。
fn parse_args(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut db: Option<PathBuf> = None;
    let mut http = false;
    let mut port = DEFAULT_HTTP_PORT;
    let mut read_only = false;

    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        // 同时接受 `--db <path>` 和 `--db=<path>`：两种写法在 CLI 生态里都常见，
        // 只认一种的话，用户看着配置文件里抄来的写法会以为程序坏了。
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag.to_string(), Some(value.to_string())),
            None => (arg, None),
        };

        match flag.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "--db" => {
                let value = take_value(&flag, inline, &mut args)?;
                if value.trim().is_empty() {
                    return Err("--db 需要一个非空路径".into());
                }
                db = Some(PathBuf::from(value));
            }
            "--port" => {
                let value = take_value(&flag, inline, &mut args)?;
                port = value
                    .parse::<u16>()
                    .map_err(|_| format!("--port 需要一个 1-65535 的端口号，收到 `{value}`"))?;
                if port == 0 {
                    return Err("--port 不能是 0".into());
                }
            }
            "--http" => {
                reject_value(&flag, inline)?;
                http = true;
            }
            "--read-only" => {
                reject_value(&flag, inline)?;
                read_only = true;
            }
            other => return Err(format!("不认识的参数 `{other}`")),
        }
    }

    let db = db.ok_or("缺少必填参数 --db")?;

    Ok(Options {
        db,
        http,
        port,
        read_only,
    })
}

fn take_value(
    flag: &str,
    inline: Option<String>,
    args: &mut impl Iterator<Item = String>,
) -> Result<String, String> {
    match inline {
        Some(value) => Ok(value),
        None => args.next().ok_or_else(|| format!("`{flag}` 后面缺少取值")),
    }
}

fn reject_value(flag: &str, inline: Option<String>) -> Result<(), String> {
    match inline {
        // `--http=yes` 这种写法：开关就是开关，给它赋值只会让人以为支持
        // 「关掉」的取值，进而写出 `--http=false` 并静默地不生效。
        Some(value) => Err(format!("`{flag}` 是开关，不接受取值（收到 `{value}`）")),
        None => Ok(()),
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) if message.is_empty() => {
            // `--help`：打到 stdout，退出码 0 —— 这是用户主动要的输出，不是错误
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("reqly-mcp: {message}\n\n{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };

    // 建池 → 迁移 → 保证默认工作区，与桌面端走同一条启动路径。
    // 它内部的日志全部走 stderr（见 `AppState::new` 的注释）。
    let state = match AppState::new(options.db) {
        Ok(state) => Arc::new(state),
        Err(err) => {
            eprintln!("reqly-mcp: cannot open the Reqly database: {err}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };

    let result = if options.http {
        serve_http(state, options.read_only, options.port).await
    } else {
        serve_stdio(state, options.read_only).await
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("reqly-mcp: {err}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        parse_args(args.iter().map(|a| (*a).to_string()))
    }

    #[test]
    fn database_path_is_required() {
        let message = parse(&[]).unwrap_err();
        assert!(message.contains("--db"), "{message}");
    }

    #[test]
    fn both_spellings_of_a_valued_flag_are_accepted() {
        let spaced = parse(&["--db", "/tmp/a.db"]).unwrap();
        let joined = parse(&["--db=/tmp/a.db"]).unwrap();

        assert_eq!(spaced.db, PathBuf::from("/tmp/a.db"));
        assert_eq!(joined.db, PathBuf::from("/tmp/a.db"));
    }

    #[test]
    fn defaults_are_stdio_and_writable() {
        let options = parse(&["--db", "/tmp/a.db"]).unwrap();
        assert!(!options.http);
        assert!(!options.read_only);
        assert_eq!(options.port, DEFAULT_HTTP_PORT);
    }

    #[test]
    fn http_port_can_be_overridden() {
        let options = parse(&["--db", "/tmp/a.db", "--http", "--port", "9001"]).unwrap();
        assert!(options.http);
        assert_eq!(options.port, 9001);
    }

    #[test]
    fn a_bad_port_is_rejected_with_the_offending_value() {
        let message = parse(&["--db", "/tmp/a.db", "--port", "70000"]).unwrap_err();
        assert!(message.contains("70000"), "{message}");
        assert!(parse(&["--db", "/tmp/a.db", "--port", "0"]).is_err());
    }

    #[test]
    fn a_boolean_flag_refuses_a_value() {
        // `--http=false` 必须报错而不是被当成「未指定」—— 后者会静默地按 stdio 起，
        // 用户以为 HTTP 开着却连不上。
        let message = parse(&["--db", "/tmp/a.db", "--http=no"]).unwrap_err();
        assert!(message.contains("--http"), "{message}");
    }

    #[test]
    fn unknown_flags_are_named_in_the_error() {
        let message = parse(&["--db", "/tmp/a.db", "--wat"]).unwrap_err();
        assert!(message.contains("--wat"), "{message}");
    }

    #[test]
    fn help_is_signalled_by_an_empty_error() {
        assert_eq!(parse(&["--help"]).unwrap_err(), "");
        assert_eq!(parse(&["-h"]).unwrap_err(), "");
    }

    #[test]
    fn a_valued_flag_at_the_end_reports_a_missing_value() {
        let message = parse(&["--db"]).unwrap_err();
        assert!(message.contains("--db"), "{message}");
    }
}
