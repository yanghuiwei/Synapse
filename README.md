# Reqly

> 轻量级 **AI 原生** API 工作台 —— 安装包 5.2MB、本地优先、无账号、MCP Server + CLI 双通道的桌面 API 客户端。

替代 Postman 的日常使用场景（集合管理 / 发请求 / 环境变量 / 历史 / 压测），同时让 AI（WorkBuddy / Hermes / Claude Code / Cursor）能**直接操作你的接口集合** —— 不是把请求贴进对话框，而是通过 MCP 或 CLI 让 AI 去找接口、改参数、发请求、看响应。

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

## 已实现的能力（P1 – P7 + P8-A）

| 模块 | 说明 |
|---|---|
| **集合树** | Workspace → Folder（无限层级自引用）→ Request；拖拽移动、右键移动对话框；未归类请求单独成组 |
| **请求闭环** | 多方法；查询参数 / 请求头 / 表单 / 原始体编辑器；DNS、连接、超时、TLS、HTTP 状态**六类错误分开报**，后端只回错误码不回文案 |
| **响应面板** | 状态码 / 耗时 / 大小；格式化 JSON；响应头；原始报文 |
| **历史与回收站** | 每次发送落 `history`；全局软删除（`deleted_at`），回收站可还原 / 彻底删除 |
| **变量与 Token** | 环境（Environment）→ 变量；`{{var}}` 占位符；标记 secret 的变量不回显；Token 源绑定到 Folder，命名空间占位符 `{{token.access}}`，同源并发刷新走单飞锁 |
| **导入器** | `curl` 命令；Postman v2.1 集合（apipost 导出走这条兼容路径）；本工具自有 JSON 的导入导出。**OpenAPI 3.x 与 HAR 1.2 尚未实现**（`core/src/importer/` 里确实没有这两个解析器，见路线图 P5 的说明） |
| **压测** | 常驻 worker + 原子领号控并发；成功率 / P50 / P90 / P95 / P99 / TPS / 有效 RPS；延迟 + RPS 双轴折线、延迟直方图、最近 50 条失败样本；**一键导出 HTML 报表**并在系统程序中打开 |
| **MCP Server**（P7） | 13 个 tool（snake_case 命名 + `readOnlyHint` 注解），stdio 与回环 HTTP 两种传输，只读模式，一键生成 WorkBuddy / Claude Code / Cursor 的 JSON 片段与 Hermes 的 YAML 片段 |
| **命令行 CLI**（P8-A） | 15 条命令，覆盖集合 / 请求 / 环境 / 发请求 / 压测 / Token / 导入；`--json` 输出与 MCP 返回形状一致，可直接 `\| jq` |
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
| 写操作保护 | 只读模式下 4 个只读 tool 免审批、写操作按 `readOnlyHint` 弹确认 | `--read-only` 让 9 个写操作返回 `E_READ_ONLY` |

### MCP

桌面端「设置 → MCP」面板直接给出可以粘进配置文件的片段（JSON 一家、YAML 一家），以及它指向的可执行文件绝对路径。传输支持 stdio 与回环 HTTP（只监听 `127.0.0.1`）。

### CLI

```powershell
# 桌面端「设置」页能看到库文件的绝对路径，也可以设 REQLY_DB 环境变量
reqly-cli --db "%APPDATA%\com.reqly.app\reqly.db" folders
reqly-cli --db "%APPDATA%\com.reqly.app\reqly.db" requests list --query 用户
reqly-cli --db "%APPDATA%\com.reqly.app\reqly.db" requests get 12
reqly-cli --db "%APPDATA%\com.reqly.app\reqly.db" send 12 --header "X-Debug: 1"
reqly-cli --db "%APPDATA%\com.reqly.app\reqly.db" send --method POST --url https://httpbin.org/post --body '{"a":1}'
reqly-cli --db "%APPDATA%\com.reqly.app\reqly.db" import curl --file request.txt --json | jq '.created'
```

命令树：`folders` · `requests {list,get,create,update,move,delete}` · `env {list,set}` · `send` · `loadtest run` · `token refresh` · `import curl`。

几条刻意的设计：

- **不进 argv 的密钥**：`env set 1 API_KEY`（值省略）会从 stdin 读一行 —— Windows 上命令行参数在进程列表里可见。
- **退出码两档**：`1` = 业务失败（带 `E_*` 错误码）、`2` = 用法错误。不给「网络错」单独一档，因为网络错本来就带 `E_DNS` / `E_CONN_REFUSED` / `E_TLS` 这类细分码，判 `code` 比判数字精确。
- **分隔符按位置定**：`--header "K: V"` 按 `:` 切、`--param "K=V"` 按 `=` 切（header 值里常有 `=`，param 值里常有 `:`）。

⚠️ 安装包会把 `reqly-cli.exe` 放进安装目录，但**不会加进 PATH**（`reqly-mcp.exe` 不受影响，MCP 客户端按配置里的绝对路径拉起它）。想在任意目录敲 `reqly-cli`，需要自己把安装目录加进 PATH。

---

## 技术栈

| 层 | 选型 | 版本 |
|---|---|---|
| 应用外壳 | Tauri | 2 |
| 前端 | React / TypeScript / Vite | 19.1 / 7.0 / 8.0 |
| 样式 | Tailwind CSS | 4.3 |
| 状态 | zustand | 5.0 |
| 图表 | uPlot | 1.6 |
| 校验 | zod | 4.6 |
| 后端 | Rust（edition 2021） | — |
| HTTP | reqwest | 0.12 |
| 存储 | SQLite（rusqlite + r2d2） | 0.40 |
| CLI | clap | 4.6 |
| 包管理 | pnpm | — |

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
│    importer/  curl · postman · reqly                      │
│    template/  变量 / Token 占位符解析                      │
│    service/   13 个操作的编排层（ops.rs）                  │
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

数据文件位置：`%APPDATA%\com.reqly.app\reqly.db`（macOS：`~/Library/Application Support/com.reqly.app/reqly.db`），桌面端「设置」页会显示绝对路径。

---

## 数据与隐私

- **无账号、无云同步、无遥测**。所有数据在本机一个 SQLite 文件里。
- 标记为 secret 的变量与 Token 凭据不下发前端、不进日志、不进 CLI 的 `--json` 输出。
- MCP 的 HTTP 通道只监听 `127.0.0.1`。
- 删掉的请求先进回收站，不立刻落盘消失。
- AI 通过 MCP / CLI 改库后，桌面端会刷新列表 —— 但**不会**重拉你正在编辑的那条请求，避免静默覆盖草稿。

---

## 快速开始

**前置条件**：Node.js ≥ 20、pnpm、Rust 工具链（stable），Windows 另需 MSVC 生成工具 + Windows SDK。

```powershell
git clone https://github.com/yanghuiwei/h_req.git
cd h_req
pnpm install
pnpm tauri dev
```

打包（`beforeBuildCommand` 会自动跑前端构建与 sidecar staging，无需手动准备）：

```powershell
pnpm tauri build                      # 按 tauri.conf.json 的 targets 全量打包
pnpm tauri build --bundles nsis       # 只要 Windows NSIS 安装器（不额外拉 WiX）
```

产物：`src-tauri/target/release/bundle/nsis/reqly_0.1.0_x64-setup.exe`（安装目录里是 `reqly.exe` + `reqly-mcp.exe` + `reqly-cli.exe` 三个可执行文件）。

> **sidecar 是怎么进包的**：`scripts/stage-sidecars.mjs` 会把两个兄弟二进制复制成
> `src-tauri/binaries/<name>-$TARGET_TRIPLE.exe`，再交给 `tauri.conf.json` 的 `bundle.externalBin`。
> 这一步由 `pnpm sidecar:stage` 单独跑，也挂在 `beforeBuildCommand` 上。
> 手工拷进去没用 —— 文件名少一个 target triple 后缀，构建期就会报 `resource path ... doesn't exist`。

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

**当前基线**

| 指标 | 值 |
|---|---|
| `cargo test --workspace` | **462 passed / 0 failed**（6 ignored = `http_live` 5 + `loadtest_ab` 1，按设计需公网 / 需 `ab`） |
| `clippy -D warnings` | 0 告警 |
| `tsc --noEmit` | 0 错误 |
| `pnpm build` | 2025 modules；CSS 37.40 kB / JS 625.78 kB（gzip 192.01 kB） |
| release 体积 | `reqly.exe` 9.5 MB · `reqly-mcp.exe` 7.4 MB · `reqly-cli.exe` 5.5 MB |

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
│   │   └── external_changes.rs 外部进程改库 → 通知前端刷新
│   ├── core/                   reqly-core：不含 tauri，三入口共用
│   │   ├── migrations/         001_init · 002_folder_auth · 003_body_by_type
│   │   └── src/                storage http importer loadtest template service mcp
│   ├── mcp/                    reqly-mcp：MCP Server（lib + bin）
│   ├── cli/                    reqly-cli：命令行（clap）
│   ├── binaries/               sidecar staging 中转（产物已 gitignore）
│   └── tests/                  startup.rs · http_live.rs · loadtest_ab.rs
├── scripts/                    stage-sidecars.mjs
└── docs/                       设计文档 · 实施计划 · 校准留档
```

---

## 文档索引

| 文件 | 内容 |
|---|---|
| [`docs/design.md`](docs/design.md) | 设计文档：定位、7 项已确认决策、数据模型、各模块设计、Rust 侧技术根因 |
| [`docs/plan.md`](docs/plan.md) | 实施计划 + **§7–§17 各阶段实测记录**（完成范围、验收命令与结果、坑、路线图欠账） |
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
| P5 | 导入器：curl ✅ · Postman v2.1 ✅ · **OpenAPI 3.x / HAR 1.2 ❌ 未实现** | ⚠️ |
| P6 | 并发压测与报表导出 | ✅ |
| P7 | MCP Server：13 个 tool、stdio + 回环 HTTP、配置片段生成、只读模式 | ✅ |
| P8-A | 拆 crate（core / mcp / cli）+ CLI 通道 + 外部改动自动刷新 + 两个 sidecar 进安装包 | ✅ |
| P8 | 打磨：命令面板 `Ctrl+K`、整包导入导出、自动更新、图标、关于页、体积优化 | ⏳ |
| P9 | AI Provider：任意 OpenAI 兼容端点（含自建网关）、`baseUrl` 原样使用、连通性测试回原始报文 | — |
| P10 | 内置 AI 助手：自然语言生成请求 / 解释响应 / 生成断言（可选，依赖 P9） | — |

---

## 非目标（本期明确不做）

- 云同步 / 团队协作 / 账号体系
- gRPC / WebSocket / MQTT / Kafka（架构预留，不实现）
- 抓包代理
- 专业级压测（ramp-up、RPS 曲线、分布式加压）
- 插件市场

---

## 界面语言

本期只做中文。所有文案走 `t('域.语义')` 查表，不写死在组件里，为后续多语言留出口。CLI 的错误文案同理，集中在 `cli/src/render.rs`。

---

## 许可证

仓库暂未指定开源许可证。
