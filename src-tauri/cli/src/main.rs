//! `reqly-cli` —— Reqly 的命令行入口。
//!
//! ## 为什么除了 MCP 还要有一个 CLI
//!
//! MCP 把工作台接进了「对话」这个场景，但它有三条**协议本身带来的**限制：
//!
//! 1. **响应体硬截 64KB** —— stdio 是一条共享的 JSON-RPC 通道，塞大响应会把通道堵死；
//! 2. **常驻开销** —— 13 个 tool 的 schema 与描述要一直占着模型上下文；
//! 3. **不可组合** —— 结果是协议帧，`| jq` 这种用法无从谈起。
//!
//! CLI 的 stdout 是干净的文本或 JSON，三条限制都不成立：可以 `--max-body-bytes 0`
//! 把大响应落成文件、可以只在需要时调用（不占上下文）、可以接进任何管道与脚本。
//! 两者不是替代关系 —— **查和聊走 MCP，批处理与管道走 CLI**。
//!
//! ## 操作只有一份实现
//!
//! 全部来自 `reqly_core::service::ops`（MCP tool 调的是同一个 `Ops`）。
//! 这里只做三件事：解析参数、调 `Ops`、把结果摆成人能读的样子。
//!
//! `Ops` 里一共 15 条，MCP 只暴露其中 13 条（那张 tool 表有测试钉着数量）——
//! 多出来的 `folders create` / `env create` 是 CLI 独有的管理类入口。
//! 它们同样收在 `Ops` 里而不是在 CLI 里另开一份实现：共用的编排不该分叉，
//! 「暴露哪几个给 AI」是 MCP 侧的决定。见 `ops.rs` 模块头「13 与 15」。
//!
//! ## 退出码
//!
//! * `0` —— 成功
//! * `1` —— 跑起来了但失败了（`AppError`，具体码见 stderr）
//! * `2` —— 命令行写错了（clap 的用法错误 / 缺 `--db`）
//!
//! 脚本按 `1` 与 `2` 区分「业务失败」与「调用方式不对」是很有用的 ——
//! 前者要改数据，后者要改命令。

mod render;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand};
use serde::Serialize;

use reqly_core::error::{AppError, AppResult};
use reqly_core::service::ops::{
    CreateEnvironmentRequest, CreateFolderRequest, CreateRequestRequest, DeleteRequestRequest,
    GetRequestRequest, ImportCurlRequest, KeyValueInput, ListEnvironmentsRequest,
    ListFoldersRequest, ListRequestsRequest, MoveRequestRequest, Ops, RefreshTokenRequest,
    RequestFields, RunLoadTestRequest, SendRequestRequest, SetEnvVarRequest, UpdateRequestRequest,
    DEFAULT_MAX_BODY_BYTES,
};
use reqly_core::AppState;

/// 业务失败的退出码。
const EXIT_FAILURE: u8 = 1;
/// 用法错误的退出码（缺 `--db`；clap 自己的用法错误也用 2）。
const EXIT_USAGE: u8 = 2;

/// 为什么这行提示要写在 `after_help` 里而不是文档：用户看到错误时正好需要它。
const EXAMPLES: &str = "\
示例：
  reqly-cli --db %APPDATA%/com.reqly.desktop/reqly.db folders
  reqly-cli --db %APPDATA%/com.reqly.desktop/reqly.db folders create --name RMS-报价权限
  reqly-cli --db %APPDATA%/com.reqly.desktop/reqly.db requests list --query 用户
  reqly-cli --db %APPDATA%/com.reqly.desktop/reqly.db requests get 12
  reqly-cli --db %APPDATA%/com.reqly.desktop/reqly.db send 12 --header \"X-Debug: 1\"
  reqly-cli --db %APPDATA%/com.reqly.desktop/reqly.db send --method POST --url https://httpbin.org/post --body \"{\\\"a\\\":1}\"
  reqly-cli --db %APPDATA%/com.reqly.desktop/reqly.db import curl --file request.txt

与 MCP 的分工：MCP 走一条共享的 JSON-RPC 通道（响应体硬截 64KB），适合「边聊边查」；
CLI 的 stdout 是干净文本或 JSON，适合管道、脚本、以及把大响应落成文件 ——
`--max-body-bytes 0` 这种用法只在 CLI 上成立。
";

#[derive(Parser)]
#[command(
    name = "reqly-cli",
    version,
    about = "Reqly 的命令行入口：同一个本地工作台，给脚本和 AI 用",
    after_help = EXAMPLES
)]
struct Cli {
    /// Reqly 的 SQLite 文件路径（也可以设环境变量 REQLY_DB）。
    /// 桌面端「设置」页能直接看到它的绝对路径。
    /// 文件不存在会新建一个空库（迁移会自动跑）—— 抄示例里的路径前先确认它真在那儿。
    #[arg(long, global = true, env = "REQLY_DB", value_name = "PATH")]
    db: Option<PathBuf>,

    /// 只读模式：11 个写操作一律返回 E_READ_ONLY，不会改到库
    #[arg(long, global = true)]
    read_only: bool,

    /// 机器可读输出：stdout 只出一份 JSON（与 MCP 的返回形状一致）
    #[arg(long, global = true)]
    json: bool,

    /// 响应体截断上限（字节）。0 = 不在这一层截（执行层仍有 8MB 上限）
    #[arg(long, global = true, value_name = "N", default_value_t = DEFAULT_MAX_BODY_BYTES)]
    max_body_bytes: usize,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 文件夹：列出（默认）或新建
    Folders(FoldersArgs),
    /// 请求的增删改查
    #[command(subcommand)]
    Requests(RequestsCommand),
    /// 环境与变量
    #[command(subcommand)]
    Env(EnvCommand),
    /// 发一次请求（可用已保存的，也可内联定义）
    Send(SendArgs),
    /// 压测
    #[command(subcommand)]
    Loadtest(LoadtestCommand),
    /// Token 源
    #[command(subcommand)]
    Token(TokenCommand),
    /// 从外部格式导入
    #[command(subcommand)]
    Import(ImportCommand),
}

/// `folders` 的参数与子命令。
///
/// `--workspace` 挂在**这一层**而不是各自的子命令里：`folders`（默认列出）与
/// `folders create` 都要用它，两边各声明一次会出现
/// 「`folders --workspace 1 create --workspace 2`」这种谁生效要靠实现细节定的形状。
#[derive(Args)]
struct FoldersArgs {
    /// 工作区 id；省略 = 当前（唯一）工作区
    #[arg(long)]
    workspace: Option<i64>,
    /// 省略子命令 = `list`。做成「可选子命令」而不是直接把 `folders` 改成分组，
    /// 是为了让 `reqly-cli folders` 这条既有用法一个字都不用改。
    #[command(subcommand)]
    command: Option<FoldersCommand>,
}

#[derive(Subcommand)]
enum FoldersCommand {
    /// 列出文件夹（平铺返回，按 parentId 自行组树）
    List,
    /// 新建一个文件夹
    Create {
        /// 文件夹名（必填）
        #[arg(long)]
        name: String,
        /// 父文件夹 id；省略 = 工作区根
        #[arg(long, value_name = "FOLDER_ID")]
        parent: Option<i64>,
    },
}

#[derive(Subcommand)]
enum RequestsCommand {
    /// 列出请求（按名称做大小写不敏感的子串匹配）
    List {
        /// 只看这个文件夹下的请求；省略 = 整个工作区的存活请求
        #[arg(long)]
        folder: Option<i64>,
        /// 按名称模糊搜
        #[arg(long)]
        query: Option<String>,
    },
    /// 取完整请求定义（auth 只报类型，不报值）
    Get {
        /// 请求 id
        request_id: i64,
    },
    /// 新建一条立刻可发送的请求
    Create(CreateArgs),
    /// 局部更新：只改给出的字段，没给出的原样保留
    Update(UpdateArgs),
    /// 把请求移动到另一个文件夹
    Move {
        /// 请求 id
        request_id: i64,
        /// 目标文件夹 id（必须与请求同工作区且未被删除）
        #[arg(long, value_name = "FOLDER_ID")]
        to: i64,
    },
    /// 软删除（进回收站，桌面端可还原）
    Delete {
        /// 请求 id
        request_id: i64,
    },
}

/// 键值对行，可重复给。
#[derive(Args)]
struct KeyValueArgs {
    /// 请求头，可重复：`--header "X-Trace: 1"`
    #[arg(long = "header", value_name = "K:V")]
    headers: Vec<String>,
    /// query 参数，可重复：`--param "page=1"`
    #[arg(long = "param", value_name = "K=V")]
    params: Vec<String>,
}

/// 请求体。`--body` 与 `--body-file` 二选一。
#[derive(Args)]
struct BodyArgs {
    /// none / json / form / urlencoded / raw / binary / graphql
    #[arg(long, value_name = "TYPE")]
    body_type: Option<String>,
    /// 请求体原文
    #[arg(long, conflicts_with = "body_file", value_name = "TEXT")]
    body: Option<String>,
    /// 从文件读请求体（`-` = stdin）。必须是 UTF-8 —— 请求体在库里就是文本
    #[arg(long, value_name = "PATH")]
    body_file: Option<PathBuf>,
}

/// 创建 / 更新 / 内联发送共用的可选字段。
#[derive(Args)]
struct FieldsArgs {
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    method: Option<String>,
    #[arg(long)]
    url: Option<String>,
    #[command(flatten)]
    key_values: KeyValueArgs,
    #[command(flatten)]
    body: BodyArgs,
}

#[derive(Args)]
struct CreateArgs {
    /// 请求名（必填）
    #[arg(long)]
    name: String,
    /// HTTP 方法（必填）
    #[arg(long)]
    method: String,
    /// 请求地址（必填）
    #[arg(long)]
    url: String,
    /// 落到哪个文件夹；省略 = 工作区根（未分类）
    #[arg(long)]
    folder: Option<i64>,
    #[arg(long)]
    workspace: Option<i64>,
    #[command(flatten)]
    key_values: KeyValueArgs,
    #[command(flatten)]
    body: BodyArgs,
}

#[derive(Args)]
struct UpdateArgs {
    /// 请求 id
    request_id: i64,
    #[command(flatten)]
    fields: FieldsArgs,
}

#[derive(Args)]
struct SendArgs {
    /// 已保存请求的 id。给了它，下面的内联字段会**覆盖**其上的对应字段；
    /// 不给就必须同时给 --method 与 --url
    request_id: Option<i64>,
    /// 内联定义挂到哪个文件夹。不是可有可无的元数据 ——
    /// Token 源与授权继承都要沿文件夹链向上找，不写它就解析不到 `{{token.access}}`
    #[arg(long)]
    folder: Option<i64>,
    /// 指定环境名；当前只支持「就是那个激活的环境」，传别的会返回 E_NOT_IMPLEMENTED
    #[arg(long)]
    environment: Option<String>,
    #[command(flatten)]
    fields: FieldsArgs,
}

#[derive(Subcommand)]
enum EnvCommand {
    /// 列出环境与变量（密文变量的值恒为 ••••，原文不会出来）
    List {
        #[arg(long)]
        workspace: Option<i64>,
    },
    /// 新建一个空环境（变量接着用 `env set` 写；激活要在桌面端切）
    Create {
        /// 环境名
        #[arg(long)]
        name: String,
        #[arg(long)]
        workspace: Option<i64>,
    },
    /// 写一个变量：同名改写，没有就追加（按非密文处理）
    Set {
        /// 环境 id
        environment_id: i64,
        /// 变量名
        key: String,
        /// 变量值。**省略则从 stdin 读一行** ——
        /// 写在命令行里的值会出现在进程列表里（Windows 上本机其它进程可见），
        /// 要传敏感值就走 `echo <value> | reqly-cli env set <id> <key>`
        value: Option<String>,
    },
}

#[derive(Subcommand)]
enum LoadtestCommand {
    /// 跑一轮压测（阻塞到结束，上限 120 秒；同一进程同时只允许一轮）
    Run {
        /// 请求 id
        request_id: i64,
        /// 并发数
        #[arg(long)]
        concurrency: u32,
        /// 总请求数
        #[arg(long)]
        total: u64,
        /// 单请求超时，默认 30 秒
        #[arg(long, value_name = "MS")]
        timeout_ms: Option<u64>,
        /// 预热请求数（不计入统计），默认 0
        #[arg(long)]
        warmup: Option<u64>,
    },
}

#[derive(Subcommand)]
enum TokenCommand {
    /// 强制刷新一个 Token 源（只回有效期与有没有 refresh token）
    Refresh {
        /// Token 源 id
        token_source_id: i64,
    },
}

#[derive(Subcommand)]
enum ImportCommand {
    /// 把一段 curl 命令原文导入成请求（走与界面导入完全相同的解析与落库路径）
    Curl {
        /// 落到哪个文件夹；省略 = 工作区根
        #[arg(long)]
        folder: Option<i64>,
        #[arg(long)]
        workspace: Option<i64>,
        /// 直接给 curl 原文
        #[arg(long, conflicts_with = "file", value_name = "CURL")]
        text: Option<String>,
        /// 从文件读（`-` = stdin）。都不给时读 stdin
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
    },
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    // 在运行时之前挡住缺 `--db`：这一条报的是「你命令没写全」，与「跑起来失败了」
    // 不是一回事，所以走 2 而不是 1（见本文件顶部的退出码说明）。
    let Some(db) = cli.db.clone() else {
        eprintln!(
            "reqly-cli: 缺少 --db（也可以设环境变量 REQLY_DB）\n\
             \n\
             桌面端「设置」页能直接看到库文件的绝对路径。"
        );
        return ExitCode::from(EXIT_USAGE);
    };

    match run(&cli, db).await {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            report(&cli, &error);
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

/// 失败怎么摆出来。两种形状对应两类消费者：
/// 人读要「一句中文 + 错误码 + 排查用的原文」，脚本要能 `jq` 出 `code`。
fn report(cli: &Cli, error: &AppError) {
    if cli.json {
        let payload = serde_json::json!({
            "code": error.code(),
            "params": error.params(),
            "debug": error.to_string(),
            // 中文那句是 CLI 自己加的（后端按契约只给码）
            "message": render::describe_error(error),
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(text) => eprintln!("{text}"),
            Err(_) => eprintln!("reqly-cli: {}", render::describe_error(error)),
        }
    } else {
        eprintln!("reqly-cli: {}", render::describe_error(error));
        eprintln!("  错误码: {} · 详情: {error}", error.code());
    }
}

async fn run(cli: &Cli, db: PathBuf) -> AppResult<String> {
    let state = Arc::new(AppState::new(db)?);
    // 0 表示「不在这一层截」：执行层自己还有一道 8MB 上限，那一道不归这里管。
    let limit = if cli.max_body_bytes == 0 {
        usize::MAX
    } else {
        cli.max_body_bytes
    };
    let ops = Ops::with_body_limit(state, cli.read_only, limit);
    let json = cli.json;

    match &cli.command {
        Command::Folders(args) => match &args.command {
            // 不给子命令 = list：`reqly-cli folders` 这条既有用法不变
            None | Some(FoldersCommand::List) => {
                let response = ops
                    .list_folders(ListFoldersRequest {
                        workspace_id: args.workspace,
                    })
                    .await?;
                emit(json, response, render::folders)
            }
            Some(FoldersCommand::Create { name, parent }) => {
                let folder = ops
                    .create_folder(CreateFolderRequest {
                        workspace_id: args.workspace,
                        parent_id: *parent,
                        name: name.clone(),
                    })
                    .await?;
                emit(json, folder, render::folder_created)
            }
        },

        Command::Requests(command) => match command {
            RequestsCommand::List { folder, query } => {
                let response = ops
                    .list_requests(ListRequestsRequest {
                        folder_id: *folder,
                        query: query.clone(),
                    })
                    .await?;
                emit(json, response, render::requests)
            }
            RequestsCommand::Get { request_id } => {
                let request = ops
                    .get_request(GetRequestRequest {
                        request_id: *request_id,
                    })
                    .await?;
                emit(json, request, render::request_detail)
            }
            RequestsCommand::Create(args) => {
                let request = ops
                    .create_request(CreateRequestRequest {
                        folder_id: args.folder,
                        workspace_id: args.workspace,
                        name: args.name.clone(),
                        method: args.method.clone(),
                        url: args.url.clone(),
                        params: parse_key_values(&args.key_values.params, '=', "param")?,
                        headers: parse_key_values(&args.key_values.headers, ':', "header")?,
                        body_type: args.body.body_type.clone(),
                        body: resolve_body(&args.body)?,
                    })
                    .await?;
                emit(json, request, render::request_detail)
            }
            RequestsCommand::Update(args) => {
                let request = ops
                    .update_request(UpdateRequestRequest {
                        request_id: args.request_id,
                        fields: fields_of(&args.fields)?,
                    })
                    .await?;
                emit(json, request, render::request_detail)
            }
            RequestsCommand::Move { request_id, to } => {
                let request = ops
                    .move_request(MoveRequestRequest {
                        request_id: *request_id,
                        target_folder_id: *to,
                    })
                    .await?;
                emit(json, request, render::move_request)
            }
            RequestsCommand::Delete { request_id } => {
                let response = ops
                    .delete_request(DeleteRequestRequest {
                        request_id: *request_id,
                    })
                    .await?;
                emit(json, response, render::delete_request)
            }
        },

        Command::Env(command) => match command {
            EnvCommand::Create { name, workspace } => {
                let environment = ops
                    .create_environment(CreateEnvironmentRequest {
                        workspace_id: *workspace,
                        name: name.clone(),
                    })
                    .await?;
                emit(json, environment, render::environment_block)
            }
            EnvCommand::List { workspace } => {
                let response = ops
                    .list_environments(ListEnvironmentsRequest {
                        workspace_id: *workspace,
                    })
                    .await?;
                emit(json, response, render::environments)
            }
            EnvCommand::Set {
                environment_id,
                key,
                value,
            } => {
                // 值省略时从 stdin 读：命令行参数会进进程列表，Windows 上
                // 同机其它进程（任务管理器、`wmic process`）都能看到。
                // 这不是理论风险 —— 这是 `curl -H "Authorization: ..."` 那一类
                // 长期被诟病的用法，能少一处是一处。
                let value = match value {
                    Some(value) => value.clone(),
                    None => read_line_from_stdin()?,
                };
                let environment = ops
                    .set_env_var(SetEnvVarRequest {
                        environment_id: *environment_id,
                        key: key.clone(),
                        value,
                    })
                    .await?;
                emit(json, environment, render::environment_block)
            }
        },

        Command::Send(args) => {
            let response = ops
                .send_request(SendRequestRequest {
                    request_id: args.request_id,
                    method: args.fields.method.clone(),
                    url: args.fields.url.clone(),
                    params: parse_key_values(&args.fields.key_values.params, '=', "param")?,
                    headers: parse_key_values(&args.fields.key_values.headers, ':', "header")?,
                    body_type: args.fields.body.body_type.clone(),
                    body: resolve_body(&args.fields.body)?,
                    folder_id: args.folder,
                    environment: args.environment.clone(),
                })
                .await?;
            emit(json, response, render::response)
        }

        Command::Loadtest(LoadtestCommand::Run {
            request_id,
            concurrency,
            total,
            timeout_ms,
            warmup,
        }) => {
            let response = ops
                .run_load_test(RunLoadTestRequest {
                    request_id: *request_id,
                    concurrency: *concurrency,
                    total: *total,
                    timeout_ms: *timeout_ms,
                    warmup_requests: *warmup,
                })
                .await?;
            emit(json, response, render::loadtest)
        }

        Command::Token(TokenCommand::Refresh { token_source_id }) => {
            let response = ops
                .refresh_token(RefreshTokenRequest {
                    token_source_id: *token_source_id,
                })
                .await?;
            emit(json, response, render::refresh_token)
        }

        Command::Import(ImportCommand::Curl {
            folder,
            workspace,
            text,
            file,
        }) => {
            // `--text` 优先；`--file` 次之（两者互斥，clap 已经挡掉同时给的情况）；
            // 都不给就读 stdin —— 这样 `pbpaste | reqly-cli import curl` 直接可用。
            let curl_text = match (text, file) {
                (Some(text), _) => text.clone(),
                (None, Some(path)) => read_text(path)?,
                (None, None) => read_text(Path::new("-"))?,
            };
            let response = ops
                .import_curl(ImportCurlRequest {
                    curl_text,
                    target_folder_id: *folder,
                    workspace_id: *workspace,
                })
                .await?;
            emit(json, response, render::import_curl)
        }
    }
}

/// 结果怎么出：`--json` 走序列化（形状与 MCP 一致），否则走人读渲染。
///
/// 两者都吃同一个 `T`，所以不会出现「JSON 有字段、表格里漏了」这种裂缝。
fn emit<T: Serialize>(json: bool, value: T, human: impl FnOnce(&T) -> String) -> AppResult<String> {
    if json {
        serde_json::to_string_pretty(&value)
            .map_err(|e| AppError::Internal(format!("cannot serialize the result: {e}")))
    } else {
        Ok(human(&value))
    }
}

fn fields_of(args: &FieldsArgs) -> AppResult<RequestFields> {
    Ok(RequestFields {
        name: args.name.clone(),
        method: args.method.clone(),
        url: args.url.clone(),
        params: parse_key_values(&args.key_values.params, '=', "param")?,
        headers: parse_key_values(&args.key_values.headers, ':', "header")?,
        body_type: args.body.body_type.clone(),
        body: resolve_body(&args.body)?,
    })
}

/// 解析 `--header "K: V"` / `--param "K=V"`。
///
/// 分隔符**按位置给**而不是统一成某一个：header 的值里常有 `=`（base64、`q=0.9`），
/// param 的值里常有 `:`（时间戳、URL）。统一按 `=` 切会把 header 撕坏，
/// 统一按 `:` 切会把 param 撕坏 —— 所以由调用方说清这一行是什么。
fn parse_key_values(
    raw: &[String],
    separator: char,
    field: &'static str,
) -> AppResult<Option<Vec<KeyValueInput>>> {
    if raw.is_empty() {
        // 空 => None，不是 Some(vec![])：None 在 core 里表示「不改这个字段」，
        // 而 `update` 会把 Some(vec![]) 理解成「清空全部头」。两者差别很大。
        return Ok(None);
    }

    let parsed = raw
        .iter()
        .map(|item| {
            item.split_once(separator)
                .map(|(key, value)| KeyValueInput {
                    key: key.trim().to_string(),
                    value: value.trim().to_string(),
                    enabled: true,
                    desc: None,
                })
                .ok_or_else(|| AppError::InvalidInput {
                    field,
                    reason: format!("`{item}` 里没有分隔符 `{separator}`"),
                })
        })
        .collect::<AppResult<Vec<_>>>()?;

    Ok(Some(parsed))
}

fn resolve_body(args: &BodyArgs) -> AppResult<Option<String>> {
    if let Some(inline) = &args.body {
        return Ok(Some(inline.clone()));
    }
    match &args.body_file {
        Some(path) => Ok(Some(read_text(path)?)),
        None => Ok(None),
    }
}

/// 读一个文本文件；`-` 表示 stdin。
///
/// 为什么不做二进制：`body` 在库里是 `TEXT`，请求体走的是同一条模板解析路径
/// （要能认出 `{{...}}`）。二进制体得先有存储层的表达方式，那是另一件事。
fn read_text(path: &Path) -> AppResult<String> {
    if path.as_os_str() == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(AppError::Io)?;
        return Ok(buffer);
    }
    std::fs::read_to_string(path).map_err(AppError::Io)
}

/// 从 stdin 读**一行**，去掉行尾换行。
///
/// 与 [`read_text`] 分开：那个读的是文件全文（请求体可以有换行），
/// 这里读的是「一个值」—— 多读进来的内容会被当成值的一部分写进库。
fn read_line_from_stdin() -> AppResult<String> {
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(AppError::Io)?;
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// clap 的声明式定义最容易出的错是「两个参数同名」或「全局参数与子命令冲突」，
    /// 而它们只在**运行时**展开才发现 —— `debug_assert` 会在解析时报 panic。
    /// 这条测试把整个命令树搭一遍，等于在 CI 里把那句话跑出来。
    #[test]
    fn the_command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("reqly-cli").chain(args.iter().copied()))
            .unwrap_or_else(|e| panic!("解析失败：{e}"))
    }

    /// 取 `send` 的字段参数（这一组测试里出现得最多）。返回所有权，
    /// 免得为了借用去 leak 一个 `Cli`。
    fn send_fields(args: &[&str]) -> FieldsArgs {
        match parse(args).command {
            Command::Send(send) => send.fields,
            _ => panic!("应该是 send"),
        }
    }

    #[test]
    fn global_flags_work_on_both_sides_of_the_subcommand() {
        let before = parse(&["--db", "a.db", "--json", "folders"]);
        let after = parse(&["folders", "--db", "a.db", "--json"]);
        assert!(before.json && after.json);
        assert_eq!(before.db, after.db);
    }

    #[test]
    fn the_body_limit_defaults_to_the_mcp_one_and_zero_means_no_limit() {
        assert_eq!(
            parse(&["folders"]).max_body_bytes,
            DEFAULT_MAX_BODY_BYTES,
            "默认必须与 MCP 那档一致，否则同一个响应两边看到的不一样"
        );
        assert_eq!(
            parse(&["folders", "--max-body-bytes", "0"]).max_body_bytes,
            0
        );
    }

    /// 分隔符按位置给的理由：header 的值里常有 `=`，param 的值里常有 `:`。
    /// 统一按某一个切，另一类就会被撕坏 —— 两条都钉住。
    #[test]
    fn headers_and_params_keep_their_own_separator() {
        let fields = send_fields(&["send", "1", "--header", "X-Q: q=0.9", "--param", "at=12:30"]);

        let headers = parse_key_values(&fields.key_values.headers, ':', "header")
            .unwrap()
            .unwrap();
        assert_eq!(headers[0].key, "X-Q");
        assert_eq!(headers[0].value, "q=0.9", "header 的值里有 `=`，不能被切掉");

        let params = parse_key_values(&fields.key_values.params, '=', "param")
            .unwrap()
            .unwrap();
        assert_eq!(params[0].key, "at");
        assert_eq!(params[0].value, "12:30", "param 的值里有 `:`，不能被切掉");
    }

    #[test]
    fn an_unparsable_pair_is_rejected_with_the_offending_text() {
        let error = parse_key_values(&["no-separator".to_string()], ':', "header").unwrap_err();
        assert_eq!(error.code(), reqly_core::error::codes::INVALID_INPUT);
        assert!(
            error.to_string().contains("no-separator"),
            "错误里必须带上出问题的那一段：{error}"
        );
    }

    #[test]
    fn an_empty_key_value_list_is_none_not_an_empty_vec() {
        // 这个区别很要紧：Some(vec![]) 在 update 里表示「把这些头全清掉」
        assert!(parse_key_values(&[], ':', "header").unwrap().is_none());
    }

    #[test]
    fn send_accepts_either_a_saved_request_or_an_inline_definition() {
        let saved = parse(&["send", "12"]);
        match &saved.command {
            Command::Send(args) => assert_eq!(args.request_id, Some(12)),
            _ => panic!("应该是 send"),
        }

        let inline = send_fields(&["send", "--method", "POST", "--url", "https://example.com"]);
        assert_eq!(inline.method.as_deref(), Some("POST"));
        assert_eq!(inline.url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn body_and_body_file_are_mutually_exclusive() {
        // 两个都给是用法错误：静默挑一个会让用户以为另一个生效了
        let result = Cli::try_parse_from([
            "reqly-cli",
            "send",
            "1",
            "--body",
            "{}",
            "--body-file",
            "a.json",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn import_curl_reads_stdin_when_neither_text_nor_file_is_given() {
        let cli = parse(&["import", "curl"]);
        match &cli.command {
            Command::Import(ImportCommand::Curl { text, file, .. }) => {
                assert!(text.is_none() && file.is_none());
            }
            _ => panic!("应该是 import curl"),
        }
    }
}
