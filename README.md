# Reqly

> 轻量级 **AI 原生** API 工作台 —— 一个 3–10MB、本地优先、无账号、内置 MCP Server 的桌面 API 客户端。

替代 Postman 的日常使用场景（集合管理 / 发请求 / 环境变量 / 历史 / 压测），同时让 AI（WorkBuddy / Hermes / Claude Code / Cursor）能**直接操作你的接口集合** —— 不是把请求贴进对话框，而是通过 MCP 让 AI 去找接口、改参数、发请求、看响应。

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

---

## 已实现的能力（P1 – P6）

| 模块 | 说明 |
|---|---|
| **集合树** | Workspace → Folder（无限层级自引用）→ Request；拖拽移动、右键移动对话框；未归类请求单独成组 |
| **请求闭环** | 多方法；查询参数 / 请求头 / 表单 / 原始体编辑器；DNS、连接、超时、TLS、HTTP 状态**六类错误分开报**，后端只回错误码不回文案 |
| **响应面板** | 状态码 / 耗时 / 大小；格式化 JSON；响应头；原始报文 |
| **历史与回收站** | 每次发送落 `history`；全局软删除（`deleted_at`），回收站可还原 / 彻底删除 |
| **变量与 Token** | 环境（Environment）→ 变量；`{{var}}` 占位符；标记 secret 的变量不回显；Token 源绑定到 Folder，命名空间占位符 `{{token.access}}`，同源并发刷新走单飞锁 |
| **导入器** | curl、apipost 必做；**白送** Postman v2.1、OpenAPI 3.x、HAR —— 全部在 Rust 侧解析 |
| **压测** | 常驻 worker + 原子领号控并发；成功率 / P50 / P90 / P95 / P99 / TPS / 有效 RPS；延迟 + RPS 双轴折线、延迟直方图、最近 50 条失败样本；**一键导出 HTML 报表**并在系统程序中打开 |
| **主题** | 浅色 / 深色 / 跟随系统，纯 CSS 变量双主题 |

### 压测口径（三件必须先说清的事）

1. **成功** = 拿到响应且状态码 `< 400`（3xx 算成功）；**拿不到响应一律失败**，`status` 记为 NULL。
2. **`tps`**（实际吞吐，含失败）与 **`throughput.avg_rps`**（有效吞吐，只数成功）**不是一个数**。
3. **峰值 RPS** 只统计完整的 1 秒窗口；不足 1 秒时按比例折算，并置 `peak_window_is_partial` 标志。

分位数用**最近秩法**直接算精确值（满足「累计计数 ≥ ⌈q × n⌉」的最小耗时），按毫秒索引，内存有界、零依赖 —— 不是近似直方图。

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
| 包管理 | pnpm | — |

**没有** eslint / prettier：格式化交给 `cargo fmt` 与 TS 编译器本身。

---

## 架构

```
┌────────────────────────────────────────────────────────┐
│  WebView  (React 19 + TS + Vite + Tailwind v4)         │
│                                                        │
│   ┌──────────┬──────────────────────┬──────────────┐   │
│   │ 侧栏     │ 请求面板             │ 响应面板     │   │
│   │ 集合树   │ 方法/URL/参数/头/体  │ 状态/头/正文 │   │
│   │ 历史     │                      │              │   │
│   │ 回收站   │                      │              │   │
│   └──────────┴──────────────────────┴──────────────┘   │
│   ┌────────────────────────────────────────────────┐   │
│   │  底部抽屉：环境变量 / Token / 压测 / 导入 / 设置│   │
│   └────────────────────────────────────────────────┘   │
└───────────────────────┬────────────────────────────────┘
                        │  Tauri IPC（只传错误码，不传文案）
┌───────────────────────▼────────────────────────────────┐
│  Rust Core                                             │
│                                                        │
│   commands/   workspace folder request collection      │
│               env token history import loadtest ...    │
│   http/       连接池 · 模板渲染 · 六类错误分流          │
│   loadtest/   并发引擎 · 统计 · 分位 · 报表             │
│   importer/   curl · apipost · postman · openapi · har │
│   storage/    SQLite（WAL + 外键 + 软删除）             │
│   template/   变量 / Token 占位符解析                   │
│   mcp/        MCP Server（P7 落地）                     │
│   ai/         AI Provider（P9 落地）                    │
└───────────────────────┬────────────────────────────────┘
                        │
             %APPDATA%/reqly/reqly.db
           （macOS: ~/Library/Application Support/reqly/）
```

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

---

## 数据与隐私

- **无账号、无云同步、无遥测**。所有数据在本机一个 SQLite 文件里。
- 标记为 secret 的变量与 Token 凭据不下发前端、不进日志。
- MCP 的 HTTP 通道（P7）只监听 `127.0.0.1`。
- 删掉的请求先进回收站，不立刻落盘消失。

---

## 快速开始

**前置条件**：Node.js ≥ 20、pnpm、Rust 工具链（stable），Windows 另需 MSVC 生成工具 + Windows SDK。

```powershell
git clone https://github.com/yanghuiwei/h_req.git
cd h_req
pnpm install
pnpm tauri dev
```

打包：

```powershell
pnpm tauri build
```

---

## 开发与验证

改完代码按这张表跑一遍，全绿才算完：

| 检查项 | 命令 |
|---|---|
| 后端测试 | `cargo test`（在 `src-tauri/` 下） |
| 后端 lint | `cargo clippy --all-targets -- -D warnings` |
| 后端格式 | `cargo fmt --all -- --check` |
| 前端类型 | `pnpm exec tsc --noEmit` |
| 前端构建 | `pnpm build` |

**当前基线**

| 指标 | 值 |
|---|---|
| `cargo test` | 405 lib + 5 startup 通过 |
| `clippy -D warnings` | 0 告警 |
| `tsc --noEmit` | 0 错误 |
| `pnpm build` | 2020 modules；CSS 36.94 kB / JS 615.34 kB |

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
│   │                           layout loadtest request response token trash ui
│   ├── lib/                    ipc 封装 · i18n
│   ├── store/                  zustand
│   └── styles/                 theme.css（唯一颜色来源）· global.css
├── src-tauri/                  后端
│   ├── migrations/             001_init.sql · 002_folder_auth.sql
│   ├── src/                    commands http importer loadtest storage template mcp ai
│   └── tests/                  startup.rs · http_live.rs · loadtest_ab.rs
└── docs/                       设计文档 · 实施计划 · 校准留档 · 截图
```

---

## 文档索引

| 文件 | 内容 |
|---|---|
| [`docs/design.md`](docs/design.md) | 设计文档：定位、7 项已确认决策、数据模型、各模块设计、Rust 侧技术根因 |
| [`docs/plan.md`](docs/plan.md) | 实施计划 + **§7–§14 各阶段实测记录**（完成范围、验收命令与结果、坑、截图核对） |
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
| P5 | 导入器（curl / apipost / Postman / OpenAPI / HAR） | ✅ |
| P6 | 并发压测与报表导出 | ✅ |
| **P7** | **MCP Server**：13 个 tool（snake_case + 注解）、stdio + 回环 HTTP、配置片段生成、只读模式 | ⏳ |
| P8 | 打磨：命令面板 `Ctrl+K`、整包导入导出、自动更新、打包体积优化、图标、关于页 | — |
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

本期只做中文。所有文案走 `t('域.语义')` 查表，不写死在组件里，为后续多语言留出口。

---

## 许可证

仓库暂未指定开源许可证。
