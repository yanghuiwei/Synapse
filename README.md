# Synapse

> 轻量级 **Agent 原生** API 工作台 —— 安装包 5.2MB、本地优先、无账号、MCP Server + CLI 双通道的桌面 API 客户端。

替代 Postman 的日常使用场景（集合管理 / 发请求 / 环境变量 / 历史 / 压测），同时让 AI（WorkBuddy / Hermes / Claude Code / Cursor）能**直接操作你的接口集合** —— 不是把请求贴进对话框，而是通过 MCP 或 CLI 让 AI 去找接口、改参数、发请求、看响应。

**应用内不内置 AI**（2026-09-15 决定）：不配 provider、不做对话面板。AI 一律从外部 Agent 进来，本工具负责的只是**干净的数据 + 干净的两个通道**。

---

## 为什么是桌面应用，而不是网页版

这不是「想做桌面应用」，是**必须**。下面每一条都是浏览器里的硬墙：

| 能力 | 浏览器 | 桌面（Rust 侧） |
|---|---|---|
| 跨域请求 | 被 CORS 拦死 | `reqwest` 无同源限制 |
| 超时控制 | `fetch` 粒度粗，无法精确 | 连接 / 读 / 总超时可分别设 |
| 并发上限 | HTTP/1.1 单域名 6 连接 | 连接池可控到数百 |
| HTTP/2 | 浏览器不暴露协商细节 | 原生支持 |
| 大响应体 | 全部进内存，几百 MB 就崩 | 流式读取，可落盘 |
| 并发压测 | 基本不可行 | `Semaphore` 精确控并发 |
| 自签证书 | 无法配置 | 可配 `danger_accept_invalid_certs` |
| 与 AI 共用一份数据 | 拿不到本地库文件 | MCP / CLI 三个入口读写同一个 SQLite |

---

## 已实现的能力（P1 – P7 + P8 已落地的几项）

| 模块 | 说明 |
|---|---|
| **集合树** | Workspace → Folder（无限层级自引用）→ Request；拖拽移动、右键移动对话框；未归类请求单独成组 |
| **请求闭环** | 多方法；查询参数 / 请求头 / 表单 / 原始体编辑器；DNS、连接、超时、TLS、HTTP 状态**六类错误分开报**，后端只回错误码不回文案 |
| **响应面板** | 状态码 / 耗时 / 大小；格式化 JSON；响应头；原始报文 |
| **历史与回收站** | 每次发送落 `history`；全局软删除（`deleted_at`），回收站可还原 / 彻底删除 |
| **查看 curl**（P8） | 响应区「cURL」页签：`curl -X … -H …` 一眼看清**当时真实发出**的是什么（占位符已展开、Token 已注入，发送时落库，事后可回看）；还没跑过的请求可按当前编辑内容推演，两种来源在界面上分开标注。凭据默认明文（核对 401 就是要看它），可一键打码。生成的命令只用了导入器认得的参数，能直接粘回「导入 curl」闭环 |
| **变量与 Token** | 环境（Environment）→ 变量；`{{var}}` 占位符；标记 secret 的变量不回显；Token 源绑定到 Folder，命名空间占位符 `{{token.access}}`，同源并发刷新走单飞锁 |
| **导入器** | `curl` 命令；Postman v2.1 集合（apipost 导出走这条兼容路径）；**HAR 1.2**（浏览器 DevTools / Charles 抓包导出，按域名分文件夹、自动滤掉图片/JS/CSS 这类静态资源）；本工具自有 JSON 的导入导出。**OpenAPI 3.x 主动裁剪**（2026-09-15 决定不做，按需再迭代 —— 见路线图 P5） |
| **压测** | 常驻 worker + 原子领号控并发；成功率 / P50 / P90 / P95 / P99 / TPS / 有效 RPS；延迟 + RPS 双轴折线、延迟直方图、最近 50 条失败样本；**一键导出 HTML 报表**并在系统程序中打开 |
| **MCP Server**（P7） | 13 个 tool（snake_case 命名 + `readOnlyHint` 注解），stdio 与回环 HTTP 两种传输，只读模式，一键生成 WorkBuddy / Claude Code / Cursor 的 JSON 片段与 Hermes 的 YAML 片段 |
| **命令行 CLI**（P8-A） | 15 条叶子命令，覆盖集合 / 请求 / 环境 / 发请求 / 压测 / Token / 导入；`--json` 输出与 MCP 返回形状一致，可直接 `\| jq` |
| **外部改动自动刷新**（P8-A） | AI 在另一个进程里改了库（MCP 或 CLI），桌面端自动发现并刷新列表（1 秒轮询 + 1.2 秒去抖）—— 不用切板块、不用重启 |
| **主题** | 浅色 / 深色 / 跟随系统，纯 CSS 变量双主题 |

### 压测口径（三件必须先说清的事）

1. **成功** = 拿到响应且状态码 `< 400`（3xx 算成功）；**拿不到响应一律失败**，`status` 记为 NULL。
2. **`tps`**（实际吞吐，含失败）与 **`throughput.avg_rps`**（有效吞吐，只数成功）**不是一个数**。
3. **峰值 RPS** 只统计完整的 1 秒窗口；不足 1 秒时按比例折算，并置 `peak_window_is_partial` 标志。

分位数用**最近秩法**直接算精确值（满足「累计计数 ≥ ⌈q × n⌉」的最小耗时），按毫秒索引，内存有界、零依赖 —— 不是近似直方图。

---

## 给 AI 的两个通道：MCP 与 CLI

两个入口共用 `reqly-core` 里同一份操作实现（`service/ops.rs`），所以行为一致；差异只在**传输形态**，按场景选：

| | MCP Server | CLI |
|---|---|---|
| 适合 | 边聊边查：让 AI 自己找接口、改参数、发请求 | 管道、脚本、一次性批处理 |
| 传输 | 一条共享的 JSON-RPC 通道（stdio / HTTP） | 进程 stdout |
| 响应体 | 硬截 **64KB**（活着一整条通道，大响应会堵住对话） | 可 `--max-body-bytes 0` 不截，适合把大响应落成文件 |
| 上下文成本 | 13 个 tool 的 schema 常驻对话上下文 | 零 —— 需要时 `--help` 现查 |
| 结果可加工 | 协议帧，不能直接 `\| jq` | `--json` 是干净 JSON，随便 `\| jq` |
| 写操作保护 | 只读模式下 4 个只读 tool 免审批、写操作按 `readOnlyHint` 弹确认 | `--read-only` 让 11 个写操作返回 `E_READ_ONLY` |

### MCP

桌面端「设置 → MCP」面板直接给出可以粘进配置文件的片段（JSON 一家、YAML 一家），以及它指向的可执行文件绝对路径。传输支持 stdio 与回环 HTTP（只监听 `127.0.0.1`）。

13 个 tool 的完整清单（`read_only_hint` 为真的 4 个在只读模式下免审批；`delete_request` 是唯一 `destructive_hint = true` 的 —— 软删除虽然能还原，但它把请求从用户视野里拿掉了，审批提示必须弹）：

| tool | 注解 | 做什么 |
|---|---|---|
| `list_folders` | 只读 | 列文件夹（平铺返回，按 `parentId` 自己组树），挂着 Token 源的会标出来 |
| `list_requests` | 只读 | 列请求；给 `folderId` 只看一层，`query` 按名称做大小写不敏感的子串匹配 |
| `get_request` | 只读 | 按 id 取完整定义；`authorization` 只回类型（inherit/none/bearer/basic/apiKey），不回值 |
| `list_environments` | 只读 | 列环境与变量；`isSecret` 的值**恒为 `••••`**，原文不经过通道 |
| `create_request` | 写 | 新建一条**立刻可发送**的请求（不是空壳），name/method/url 必填 |
| `update_request` | 写 | 局部更新：只改给出的字段，没给的**原样保留** |
| `set_env_var` | 写 | 写一个环境变量，同名改写、没有就追加（按非密文处理） |
| `send_request` | 写 | 真实发出一次请求；可用 id 也可内联 method+url；响应体超 **64KB 截断并标记** |
| `run_load_test` | 写 | 对一条已保存请求跑一轮压测，阻塞到结束（上限 120s），返回 `runId` 与完整概要 |
| `refresh_token` | 写 | 强制刷新一个 Token 源；只回剩余有效期与有无 refresh token，**不回 token 本身** |
| `import_curl` | 写 | 导入一段 curl 原文，走与界面导入**完全相同**的解析与落库路径 |
| `move_request` | 写 | 把请求移到另一个分类（目标必须在同一工作区且未删除） |
| `delete_request` | 写 · **destructive** | 软删除进回收站，不会真抹掉数据 |

### CLI

```powershell
# 桌面端「设置」页能看到库文件的绝对路径，也可以设 REQLY_DB 环境变量
reqly-cli --db "%APPDATA%\com.reqly.desktop\reqly.db" folders
reqly-cli --db "%APPDATA%\com.reqly.desktop\reqly.db" requests list --query 用户
reqly-cli --db "%APPDATA%\com.reqly.desktop\reqly.db" requests get 12
reqly-cli --db "%APPDATA%\com.reqly.desktop\reqly.db" send 12 --header "X-Debug: 1"
# ⚠️ PowerShell 会给原生程序重新拼一次参数，`--body '{"a":1}'` 里的双引号会被吃掉
# （实测变成 `{a:1}`）。转义一下，或干脆用 --body-file。
reqly-cli --db "%APPDATA%\com.reqly.desktop\reqly.db" send --method POST --url https://httpbin.org/post --body '{\"a\":1}'
reqly-cli --db "%APPDATA%\com.reqly.desktop\reqly.db" import curl --file request.txt --json | jq '.created'
```

命令树（**15 条叶子命令** + 分组节点）：`folders {list,create}` · `requests {list,get,create,update,move,delete}` · `env {create,list,set}` · `send` · `loadtest run` · `token refresh` · `import curl`。

`folders` 不给子命令时等于 `folders list`（这条用法没变）。CLI 比 MCP 多两条 —— 编排（`service/ops.rs`）是同一份 15 个操作，MCP 只把其中 13 个开给 AI。

只想单独构建两个入口、不为整包等一次 release 编译：

```powershell
pnpm mcp:build      # = cargo build -p reqly-mcp
pnpm cli:build      # = cargo build -p reqly-cli
```

几条刻意的设计：

- **不进 argv 的密钥**：`env set 1 API_KEY`（值省略）会从 stdin 读一行 —— Windows 上命令行参数在进程列表里可见。
- **退出码两档**：`1` = 业务失败（带 `E_*` 错误码）、`2` = 用法错误。不给「网络错」单独一档，因为网络错本来就带 `E_DNS` / `E_CONN_REFUSED` / `E_TLS` 这类细分码，判 `code` 比判数字精确。
- **分隔符按位置定**：`--header "K: V"` 按 `:` 切、`--param "K=V"` 按 `=` 切（header 值里常有 `=`，param 值里常有 `:`）。
- **`--body` 不必再配 `--body-type`**：给了请求体、没给类型时按 `json` 发出去，并回显**实际用的**那个类型（`--json` 的 `bodyType`、人读输出的「请求体类型」）。反过来，`bodyType` 是 `none` 却带着非空 body 会**直接报错** `E_INVALID_INPUT`（`field: bodyType`）—— 这一对组合过去是静默把 body 丢掉，现场看起来像服务端坏了。`bodyType = none` + **空** body 仍然是合法的「确实不发请求体」。
- **建目录 / 建环境也能从 CLI 做**：`folders create --name <名字> [--parent <id>]`、`env create --name <名字>`。这两条是 CLI 独有的管理类入口，MCP 的 tool 表刻意不含它们（见下）。

安装包会把 `reqly-cli.exe` 放进 `%LOCALAPPDATA%\reqly\`（安装模式是 `currentUser`，不写 Program Files），**并把它加进当前用户的 PATH** —— 装完重开一个终端就能直接敲 `reqly-cli`。两条刻意的边界（都在 `src-tauri/nsis/path-hook.nsh`）：

- **卸载时摘掉**，不在注册表里留垃圾。
- **PATH 太长就不动它**：NSIS 的字符串上限是 1024，实测一条 ≥1024 字符的 PATH 读回来是**空串**而不是截断。照那个空串写回去，等于把用户整条 PATH 换成我们一个目录 —— 别人的命令会集体失踪，而且一声不吭。所以这种情况直接放弃并提示手工添加。少一条 PATH 记录和毁掉别人的 PATH，代价不对称。

  碰到这条提示时，先别急着手工加：能把 PATH 撑到 1024 以上的，通常都攒了重复项和失效项。`scripts/clean-user-path.ps1` 会把它们分三类列出来 —— **重复的**和**目录已不存在的**建议删，**断链的 junction**（名字在、目标没了）只报告，由你自己判断是不是某个应用暂时坏了。默认只报告不落盘，加 `-Apply` 才写，写之前先把你当前的 PATH 备份到桌面。开发机上实测 **1139 → 929** 字符。

想手工补上的话，**这两条都别用**：`setx` 有同样的 1024 字符截断；`[Environment]::SetEnvironmentVariable(...,'User')` 会把值降级成 `REG_SZ`，PATH 里原本的 `%USERPROFILE%\...` 会变成字面量。用注册表 API 并显式保持 `ExpandString`：

```powershell
$d   = "$env:LOCALAPPDATA\reqly"
$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
$raw = $key.GetValue('Path', $null, 'DoNotExpandEnvironmentNames')
if ($raw -notlike "*$d*") { $key.SetValue('Path', "$raw;$d", [Microsoft.Win32.RegistryValueKind]::ExpandString) }
$key.Close()   # 重开终端生效
```

手工加进去的这条**卸载时不会被自动摘掉** —— 卸载器读同一条长 PATH 也读不回来，按同一套取舍它选择不动。卸完自己删掉即可（PATH 里留一个指向已删目录的条目只是噪声，不影响命令解析）。

还有一个容易漏的点：**改注册表不会通知系统，已经开着的终端拿不到新 PATH，必须重开**。程序化改动的话记得广播 `WM_SETTINGCHANGE`（`SendMessageTimeoutW` + `SMTO_ABORTIFHUNG`）—— 安装钩子内部就是这么做的，所以装完重开终端就生效。

`reqly-mcp.exe` 与这条完全无关：MCP 客户端按配置文件里的绝对路径拉起它，全程不碰 PATH。

---

## 技术栈

> 下面的版本**不是凭印象写的**：前端抄 `package.json`，Rust 抄 `src-tauri/Cargo.lock` 里解析后的**实际版本**（2026-09-15 实测）。

| 层 | 选型 | 实际版本 | 说明 |
|---|---|---|---|
| 应用外壳 | Tauri | 2.11.5 | **只有桌面 crate 带它** —— 这条线是三个入口共用同一份数据的前提 |
| 前端框架 | React / React DOM | 19.1.0 | |
| 语言 | TypeScript | 7.0.2 | TS 7 是原生编译器，`tsc --noEmit` 很快，改完必跑 |
| 构建 | Vite / `@vitejs/plugin-react` | 8.0.16 / 6.0.2 | |
| 样式 | Tailwind CSS / `@tailwindcss/vite` | 4.3.3 | 颜色**唯一**来源是 `src/styles/theme.css`，业务组件禁止硬编码色值 |
| 状态 | zustand | 5.0.15 | |
| 长列表 | @tanstack/react-virtual | 3.14.11 | 集合树 / 历史虚拟滚动 |
| 分栏 | react-resizable-panels | 4.12.4 | |
| 图标 | lucide-react | 1.43.0 | |
| 图表 | uPlot | 1.6.32 | 压测的延迟 + RPS 双轴 |
| 校验 | zod | 4.6.1 | |
| 前端桥 | @tauri-apps/api / plugin-opener / cli | 2.11.1 / 2.5.5 / 2.11.4 | |
| 后端语言 | Rust（edition 2021，`x86_64-pc-windows-msvc`） | rustc 1.98.1 | |
| HTTP | reqwest | 0.12.28 | 刻意 `default-features = false`：不拉 native-tls/OpenSSL，**目标机不需要装 OpenSSL** |
| 存储 | rusqlite（`bundled`）/ r2d2 / r2d2_sqlite | 0.40.2 / 0.8.10 / 0.35.0 | SQLite 由 crate 自带编译（libsqlite3-sys 0.38.2），**目标机不需要装 SQLite** |
| 异步 | tokio / tokio-util | 1.53.1 / 0.7.19 | |
| 序列化 | serde / serde_json | 1.0.229 / 1.0.151 | |
| 错误 | thiserror | 2.0.20 | 契约：后端只回错误码，不回文案 |
| 时间 | chrono | 0.4.45 | |
| MCP 协议 | rmcp | 3.3.0 | MCP 官方 Rust SDK，对齐 MCP 规范 2026-07-28 |
| MCP HTTP 传输 | axum | 0.8.9 | 只给回环 HTTP 通道套一层 listener |
| JSON Schema | schemars | 1.2.2 | 出入参只定义一份，**版本必须与 rmcp 对齐**，否则是两个 `JsonSchema` trait |
| CLI 解析 | clap | 4.6.6 | `default-features = false`（不拉 anstream 给 `--help` 上色） |
| 静态 CRT | static_vcruntime（build-dep） | 3.0.0 | **只给两个 sidecar**；桌面端由 `tauri-build` 内嵌同一套逻辑 —— 见「目标机的运行时依赖」 |
| 包管理 | pnpm | 11.22.0 | Node 版本下限由 Vite 8 决定：`^20.19.0 \|\| >=22.12.0` |

**没有** eslint / prettier：格式化交给 `cargo fmt` 与 TS 编译器本身。

---

## 架构

四个 crate，靠「**谁都不依赖 tauri**」这条线切开 —— 桌面端才带 tauri，MCP 与 CLI 都是独立的瘦二进制。

```
┌──────────────────────────────────────────────────────────┐
│  WebView  (React 19 + TS + Vite + Tailwind v4)           │
│  侧栏（集合树 / 历史 / 回收站）· 请求面板 · 响应面板      │
│  底部抽屉：环境变量 / Token / 压测 / 导入 / 设置          │
└────────────────────────┬─────────────────────────────────┘
                         │  Tauri IPC（只传错误码，不传文案）
┌────────────────────────▼─────────────────────────────────┐
│  src-tauri/      ·  reqly        桌面外壳 + 命令层        │
└────────────────────────┬─────────────────────────────────┘
┌────────────────────────▼─────────────────────────────────┐
│  src-tauri/core/ ·  reqly-core   —— 不含 tauri            │
│    三个入口唯一共用的实现                                  │
│                                                            │
│    storage/   SQLite（WAL · 外键 · 软删除）· 迁移 · 仓储   │
│    http/      连接池 · 六类错误分流 · 模板渲染 · Token 单飞│
│    loadtest/  并发引擎 · 统计 · 分位 · 报表                │
│    importer/  curl · postman · har · reqly                │
│    template/  变量 / Token 占位符解析                      │
│    service/   15 个操作的编排层（ops.rs）                  │
│    mcp/       客户端配置片段生成                           │
└──────┬──────────────────────────────────┬────────────────┘
       │                                  │
┌──────▼───────────────┐   ┌──────────────▼────────────────┐
│ src-tauri/mcp/       │   │ src-tauri/cli/                │
│ reqly-mcp            │   │ reqly-cli                     │
│ 13 tool · 只读模式   │   │ 15 条命令 · --json            │
│ 7.4 MB               │   │ 5.5 MB                        │
└──────────────────────┘   └───────────────────────────────┘
```

**为什么 bin 要拆到独立 crate**：Cargo 的 `bin` target 会继承它所在 package 的**全部**依赖。`reqly-mcp` 原本挂在桌面 package 下，于是一路链上 tauri，二进制 21.5 MB —— 拆出来之后是 7.4 MB（−65%）。

**数据模型层级**

```
Workspace ─┬─ Folder ─┬─ Folder …（自引用，无限层级）
           │          └─ Request
           ├─ Environment ── EnvVar（可标 secret）
           ├─ TokenSource（绑定到 Folder）
           ├─ History（软删除 → 回收站）
           └─ LoadTestRun ── LoadTestSample
```

单文件 SQLite，开 `journal_mode=WAL` + `foreign_keys=ON` + `synchronous=NORMAL`。

数据文件位置：`%APPDATA%\com.reqly.desktop\reqly.db`（macOS：`~/Library/Application Support/com.reqly.desktop/reqly.db`），桌面端「设置」页会显示绝对路径。

> **从 0.1.0 之前的版本升级**：早期版本用的应用标识是 `com.reqly.app`，而 Tauri 拿它当数据目录名。首次启动时如果新目录里还没有库、旧目录里有，会**自动复制一份过去**（含 `-wal` / `-shm`）。旧目录**保留不删**，确认无误后可以自己清理。

---

## 数据与隐私

- **无账号、无云同步、无遥测**。所有数据在本机一个 SQLite 文件里。
- 标记为 secret 的变量与 Token 凭据不下发前端、不进日志、不进 CLI 的 `--json` 输出。
- MCP 的 HTTP 通道只监听 `127.0.0.1`。
- 删掉的请求先进回收站，不立刻落盘消失。
- AI 通过 MCP / CLI 改库后，桌面端会刷新列表 —— 但**不会**重拉你正在编辑的那条请求，避免静默覆盖草稿。

---

## 环境依赖

### 1. 开发态（源码跑起来、跑测试）

| 工具 | 要求 | 本机实测（2026-09-15） | 校验命令 |
|---|---|---|---|
| Node.js | Vite 8 要求 `^20.19.0 \|\| >=22.12.0` | v22.22.2 | `node --version` |
| pnpm | 10+ | 11.22.0 | `pnpm --version` |
| Rust | stable，edition 2021 | rustc 1.98.1 / cargo 1.98.1 | `rustc --version` |
| Rust target | `x86_64-pc-windows-msvc` | 已装 | `rustup target list --installed` |
| MSVC 生成工具 | VS 2022「使用 C++ 的桌面开发」 | VS 17.14（装在 `E:\vs2022`） | 见下 |
| Windows SDK | 10 / 11 任一 | `E:\Windows Kits\10\` | 见下 |
| WebView2 运行时 | Win11 自带 | 152.0.4191.66 | 见下 |

MSVC 与 Windows SDK **不要靠「在不在 PATH 里」判断**（不在是正常的），查注册表/官方 `vswhere`：

```powershell
# MSVC C++ 生成工具装在哪
& "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe" `
  -latest -products * `
  -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
  -property installationPath

# Windows SDK 根目录
(Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots").KitsRoot10

# WebView2 运行时版本（缺这个键 = 没装）
$g = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
(Get-ItemProperty "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\$g").pv
```

装（公司网络下 winget 通常可用）：

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools -e `
  --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id OpenJS.NodeJS.LTS -e
npm install -g pnpm
winget install --id Rustlang.Rustup -e
rustup default stable-x86_64-pc-windows-msvc
```

### 2. 起项目

```powershell
git clone https://github.com/yanghuiwei/h_req.git
cd h_req
pnpm install
pnpm tauri dev          # 会先跑 beforeDevCommand: pnpm dev（Vite 起在 1420）
```

### 3. 打安装包（NSIS）

```powershell
pnpm tauri build                      # 按 targets 打包（Windows 已收敛为只出 NSIS）
pnpm tauri build --bundles nsis       # 临时只要 NSIS 时用
```

三条**必须知道**的前置与边界：

1. **NSIS 工具链是 tauri CLI 现场下载的，不在系统里。** 落点 `%LOCALAPPDATA%\tauri\NSIS`（本机已有 nsis-3.11 + `nsis_tauri_utils.dll` v0.5.3，两个 SHA1 与 CLI 内嵌值一致 → 之后再构建不会再联网）。
   企业网络下 GitHub 常连不上，这时**手工预置**：URL 与 SHA1 能从 CLI 里 grep 出来，不用猜 ——
   ```powershell
   Select-String -Path "node_modules\.pnpm\@tauri-apps+cli-win32-x64-msvc@*\node_modules\@tauri-apps\cli-win32-x64-msvc\cli.win32-x64-msvc.node" `
     -Pattern "nsis-3\.|nsis_tauri_utils" -AllMatches | ForEach-Object { $_.Line }
   ```
   校验 SHA1 一致后解到 `%LOCALAPPDATA%\tauri\NSIS` 即可。实测可用镜像：`ghproxy.net` / `ghfast.top`（`gh-proxy.com` 403，`gitmirror` / `llkk.cc` 502）。
2. **sidecar 必须先 staging**（正常构建已自动，无需手动）。`pnpm sidecar:stage` = `node scripts/stage-sidecars.mjs`，把两个兄弟二进制复制成 `src-tauri/binaries/<name>-<target-triple>.exe`，再由 `tauri.conf.json` 的 `bundle.externalBin` 收进包；它挂在 `beforeBuildCommand`（`pnpm build:app`）上。
   **手工拷进去没用** —— 少一个 target triple 后缀，编译期直接报 `resource path ... doesn't exist`。（存在性检查在**编译期**，所以「构建成功」本身就等于「sidecar 都找到了」。）
3. **`src-tauri/nsis/*.nsh` 必须 UTF-8 带 BOM。** 无 BOM 时 makensis 按 ACP(GBK) 解码中文注释，直接报 `Bad text encoding`。修复一行：
   ```powershell
   [System.IO.File]::WriteAllText($f, $t, [Text.UTF8Encoding]::new($true))
   ```

**验包内容不用真装**（零副作用，比装一遍快）：

```powershell
& "C:\Program Files\7-Zip\7z.exe" l -slt src-tauri\target\release\bundle\nsis\reqly_0.1.0_x64-setup.exe
```

**Windows 上 `targets` 收敛成只剩 NSIS**（见 `src-tauri/tauri.windows.conf.json`）：脚手架默认的 `"all"` 在 Windows 上等于 NSIS + MSI，而 MSI 还要额外拉一套 WiX 工具链（同样是现场从 GitHub 下载，企业网络下经常直接失败）—— 我们只发 NSIS，就不去碰它。写成平台专属配置文件而不是改主文件，是为了让 macOS / Linux 继续沿用 `"all"`。

产物：`src-tauri/target/release/bundle/nsis/reqly_0.1.0_x64-setup.exe`（安装目录里是 `reqly.exe` + `reqly-mcp.exe` + `reqly-cli.exe` 三个可执行文件）。

### 4. 目标机的运行时依赖（把安装包发给别人时装什么）

安装包**不捆绑运行时**。实测三个 release 二进制的 PE 导入表，结论是**只需要 Windows 自带的组件**：

| 依赖 | 谁需要 | 处理方式 |
|---|---|---|
| **WebView2 运行时** | `reqly.exe`（GUI） | **安装器自动补**：`installer.nsi` 的 `Section WebView2` 查注册表 `…\EdgeUpdate\Clients\{F3017226-…}` 的 `pv`，缺就走 `downloadBootstrapper` 从微软官方链接下载并静默安装。Win11 自带。 |
| UCRT（`api-ms-win-crt-*.dll`） | 三个二进制都要 | **Win10+ 自带**，不用管 |
| VC++ 可再发行包 | 谁都不需要 | 三个二进制的导入表里**都没有 `VCRUNTIME140.dll`** |
| SQLite | 谁都不需要 | `rusqlite` 开 `bundled`，SQLite 源码随 crate 编进来 |
| OpenSSL | 谁都不需要 | `reqwest` 关 default features、走 rustls + 系统根证书（企业内网拿公司根证书做中间人代理的网关也能过） |

「不依赖 `vcruntime140.dll`」这条**曾经是坏的**，值得留个记录 —— 它背后有一条容易复发的结构性原因：

**`tauri-build` 只给桌面主程序做静态 CRT。** 它内嵌了 `static_vcruntime` 那套链接参数
（`/NODEFAULTLIB:vcruntime.lib` + `/DEFAULTLIB:libvcruntime.lib`，还会造一个空的 `msvcrt.lib`
去遮蔽 Rust std 硬编码链接的那个动态导入库），所以 `reqly.exe` 天然干净。
而 MCP / CLI 是**独立 crate、不跑 `tauri-build`**，于是走 rustc 默认的动态 CRT ——
产出硬导入 `VCRUNTIME140.dll` 的二进制：干净机器上 GUI 能起来、这两个会在**进程入口处直接失败**，
连错误码都出不来。

这正是「拆 crate 拿体积」的连带代价：拆出去的同时也脱离了桌面 package 的构建脚本，
**凡是 `tauri-build` 顺手做的事，新入口都得自己补一遍。**
修法：给两个 sidecar 各加一份 `build.rs` 调 `static_vcruntime::metabuild()`，与主程序同形
（C 启动文件 + VC 运行时静态，**UCRT 仍动态** —— UCRT 是 Windows 组件，动态链接才是对的；
`+crt-static` 会连它一起静态化，没必要）。代价是两个二进制各涨约 42 KB。

判据不是「构建成功」，而是**解析 PE 导入表**：`IMPORT`(数据目录 1) 与 `DELAY_IMPORT`(数据目录 13) 逐条读出来。
⚠️ 别用「在文件里搜 DLL 名字符串」代替它 —— 同一套启发式给出过自相矛盾的结论（它对 delay-load 会漏），
**它说「没有」的时候不可信**。

---

## 开发与验证

改完代码按这张表跑一遍，全绿才算完：

| 检查项 | 命令 |
|---|---|
| 后端测试 | `cargo test --workspace`（在 `src-tauri/` 下） |
| 后端 lint | `cargo clippy --workspace --all-targets -- -D warnings` |
| 后端格式 | `cargo fmt --all -- --check` |
| 前端类型 | `pnpm exec tsc --noEmit` |
| 前端构建 | `pnpm build` |
| sidecar staging | `pnpm sidecar:stage`（只有要打包时才需要） |
| 安装钩子回归 | `powershell -ExecutionPolicy Bypass -File scripts\nsis-path-hook-test.ps1`（23 个用例，只动一个测试用注册表键） |

**当前基线**

| 指标 | 值 |
|---|---|
| `cargo test --workspace` | **488 passed / 0 failed**（6 ignored = `http_live` 5 + `loadtest_ab` 1，按设计需公网 / 需 `ab`） |
| `clippy -D warnings` | 0 告警 |
| `tsc --noEmit` | 0 错误 |
| `pnpm build` | 2025 modules；CSS 37.40 kB / JS 625.36 kB（gzip 191.87 kB） |
| release 体积 | `reqly.exe` 9.5 MB · `reqly-mcp.exe` 7.5 MB · `reqly-cli.exe` 5.6 MB · NSIS 安装包 5.28 MB |
| 目标机依赖 | 三个二进制的导入表里**没有 `VCRUNTIME140.dll`**、没有 `MSVCP140.dll`；只有 Win10+ 自带的 UCRT 与系统 DLL（见「目标机的运行时依赖」） |

**压测的硬指标验收**（对 `ApacheBench` 校准 P95，偏差 < 10%）：需要本机有 `ab`，用例默认 `#[ignore]`：

```powershell
# 在 src-tauri/ 下执行
$env:AB_BIN = "C:\path\to\ab.exe"     # ab 不在 PATH 时指定
cargo test --test loadtest_ab -- --ignored --nocapture --test-threads=1
```

辅助环境变量：`LT_TRACE=1` 打印 ab 原始输出与样本直方图；`LT_CLIENT_THREADS=16` 调客户端 runtime 线程数。

---

## 项目结构

```
reqly/
├── src/                        前端
│   ├── features/               collection collections env history import
│   │                           layout loadtest mcp request response token trash ui
│   ├── lib/                    ipc 封装 · i18n · 外部改动订阅
│   ├── store/                  zustand
│   └── styles/                 theme.css（唯一颜色来源）· global.css
├── src-tauri/                  后端（Cargo workspace，四个 crate）
│   ├── src/                    reqly：桌面端 tauri 外壳 + 命令层
│   │   ├── commands/           #[tauri::command] 薄壳，实现体在 core
│   │   ├── external_changes.rs 外部进程改库 → 通知前端刷新
│   │   └── lib.rs              数据目录解析（含旧 identifier 迁移）· 命令注册
│   ├── tauri.conf.json         应用配置
│   ├── tauri.windows.conf.json Windows 专属覆盖（targets 只留 NSIS）
│   ├── core/                   reqly-core：不含 tauri，三入口共用
│   │   ├── migrations/         001_init · 002_folder_auth · 003_body_by_type
│   │   └── src/                storage http importer loadtest template service mcp
│   ├── mcp/                    reqly-mcp：MCP Server（lib + bin）
│   │   └── build.rs            sidecar 静态链接 VC 运行时（桌面端由 tauri-build 代劳）
│   ├── cli/                    reqly-cli：命令行（clap）
│   │   └── build.rs            同上
│   ├── binaries/               sidecar staging 中转（产物已 gitignore）
│   ├── nsis/                   path-hook.nsh（安装钩子：把 CLI 加进用户 PATH）
│   │                           path-hook.selftest.nsi（钩子单测夹具，UTF-8 带 BOM）
│   └── tests/                  startup.rs · http_live.rs · loadtest_ab.rs
├── scripts/                    stage-sidecars.mjs · nsis-path-hook-test.ps1
└── docs/                       设计文档 · 实施计划 · 校准留档
```

---

## 文档索引

| 文件 | 内容 |
|---|---|
| [`docs/design.md`](docs/design.md) | 设计文档：定位、7 项已确认决策、数据模型、各模块设计、Rust 侧技术根因 |
| [`docs/plan.md`](docs/plan.md) | 实施计划 + **§7–§21 各阶段实测记录**（完成范围、验收命令与结果、坑、路线图欠账） |
| [`docs/ab-calibration-p6.txt`](docs/ab-calibration-p6.txt) | P6 分位数对 ab 校准的原始输出留档 |

---

## 路线图

| 阶段 | 内容 | 状态 |
|---|---|---|
| P-1 / P0 | 环境准备、脚手架、主题与体积验证 | ✅ |
| P1 | 集合树与未归类分组 | ✅ |
| P2 | 请求闭环与六类错误分离 | ✅ |
| P3 | 历史、软删除与回收站 | ✅ |
| P4 | 环境变量与 Token 源 | ✅ |
| P5 | 导入器：curl ✅ · Postman v2.1 ✅ · **HAR 1.2 ✅** · OpenAPI 3.x 主动裁剪（见「非目标」） | ✅ |
| P6 | 并发压测与报表导出 | ✅ |
| P7 | MCP Server：13 个 tool、stdio + 回环 HTTP、配置片段生成、只读模式 | ✅ |
| P8-A | 拆 crate（core / mcp / cli）+ CLI 通道 + 外部改动自动刷新 + 两个 sidecar 进安装包 | ✅ |
| P8-② | 应用标识改 `com.reqly.desktop` + 老数据目录一次性迁移（含 WAL）+ Windows 打包只出 NSIS | ✅ |
| P8-④ | 修 9 处旧数据目录口径（`--help` 教人连旧库）+ NSIS 钩子把 CLI 加进用户 PATH | ✅ |
| P8-⑤ | 「查看 curl」：把**已解析**的请求（占位符展开、Token 注入之后）导出成可执行命令，随响应回传并落历史；`preview_curl` 支持按当前编辑内容推演 | ✅ |
| P8 | 打磨：命令面板 `Ctrl+K`、整包导入导出、自动更新、图标、关于页、体积优化 | ⏳ |

---

## 非目标（本期明确不做）

- 云同步 / 团队协作 / 账号体系
- gRPC / WebSocket / MQTT / Kafka（架构预留，不实现）
- 抓包代理
- 专业级压测（ramp-up、RPS 曲线、分布式加压）
- 插件市场
- **应用内置 AI**（原 P9 AI Provider 配置层 + P10 内置助手）—— 2026-09-15 裁剪。
  本工具的 AI 能力**只从外部 Agent 进来**：MCP（13 个 tool）与 CLI（15 条叶子命令）。
  理由：内置 provider / 对话面板要额外维护认证、自签证书、网关兼容、提示词一整套，
  而「让 AI 去找接口、改参数、发请求」这件事外部 Agent 做得更好 ——
  该由工具提供的是**干净的数据与通道**，不是另一个聊天框。
  仍为 AI 保留的：MCP 只读模式、外部改动自动刷新、配置片段生成。
- **OpenAPI 3.x / 2.0 导入** —— 2026-09-15 裁剪，按需再迭代。
  已有 curl / Postman v2.1 / HAR 1.2 三条路径覆盖日常。真要恢复：`core/src/importer/`
  加一个解析器即可，但指纹必须排在 Postman 的 `info` 分支**之前**，且要先定 YAML 取舍（见 `docs/plan.md` §22）。

---

## 界面语言

本期只做中文。所有文案走 `t('域.语义')` 查表，不写死在组件里，为后续多语言留出口。CLI 的错误文案同理，集中在 `cli/src/render.rs`。

---

## 许可证

仓库暂未指定开源许可证。
