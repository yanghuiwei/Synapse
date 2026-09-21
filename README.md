# Synapse

> **Agent 原生的 API 工作台。** 安装包 5MB，本地优先，无账号 —— 让 AI 直接操作你的接口集合。

## 为什么

Postman 很重、要登录、要云同步。更重要的是：它是给人点的，不是给 AI 调的。

Synapse 做了两件事：

1. **替你管好日常 API 工作** —— 集合、环境变量、历史、压测，该有的都有。
2. **把同一套数据干净地暴露给 AI** —— 通过 MCP Server 或 CLI，让 Claude Code / Cursor / WorkBuddy 直接去找接口、改参数、发请求、看响应。不是把 curl 贴进对话框，而是 AI 自己动手。

应用内不内置聊天框、不配 provider —— AI 从外面进来，Synapse 只负责干净的数据和干净的通道。

## 核心亮点

- **5.2 MB 安装包** —— Tauri + Rust 写的，不依赖 OpenSSL / SQLite / VC++ 运行时，解压即跑
- **本地优先** —— 所有数据在你自己电脑的一个 SQLite 文件里，无云、无账号、无遥测
- **MCP Server** —— 13 个 tool，stdio / 回环 HTTP 双通道，一键生成 Claude Code / Cursor / WorkBuddy 的配置
- **CLI** —— 15 条命令，`--json` 输出可直接 `| jq`，适合脚本和管道
- **外部改动自动刷新** —— AI 在另一个进程里改了库，桌面端 1 秒内自动刷新，不用重启
- **并发压测** —— P50 / P90 / P95 / P99 / TPS，一键导出 HTML 报表

## 快速开始

```bash
git clone https://github.com/yanghuiwei/Synapse.git
cd Synapse
pnpm install
pnpm tauri dev
```

从源码构建需要 Node.js 20+、Rust stable、VS 2022 生成工具。详见 [docs/design.md](docs/design.md)。

## 功能一览

| 模块 | 说明 |
|---|---|
| 集合管理 | 无限层级文件夹，拖拽排序 |
| 请求闭环 | 多方法、查询/头/表单/原始体，六类网络错误分开报 |
| 环境变量 | `{{var}}` 占位符，secret 不回显 |
| Token 源 | 绑定到文件夹，自动刷新，并发单飞 |
| 历史与回收站 | 软删除，误删可还原 |
| 压测 | 并发可控，分位数精确计算，HTML 报表 |
| 导入 | curl 命令、Postman v2.1、HAR 1.2 |
| MCP / CLI | AI 双通道，共用同一份数据 |

## AI 双通道：MCP vs CLI

| | MCP Server | CLI |
|---|---|---|
| 适合 | 边聊边查，AI 自己找接口 | 脚本、管道、批处理 |
| 传输 | JSON-RPC（stdio / 127.0.0.1 HTTP） | 进程 stdout |
| 响应体 | 硬截 64KB | 可不截，适合落文件 |
| 上下文成本 | 13 个 tool schema 常驻 | 零，`--help` 现查 |

桌面端「设置 → MCP」面板直接给出可粘贴的配置片段。

## 技术栈

React 19 · TypeScript · Tailwind v4 · Tauri 2 · Rust · SQLite（bundled）· reqwest · rmcp · clap

## 文档

- [设计文档](docs/design.md) —— 定位、数据模型、模块设计
- [实施计划与踩坑记录](docs/plan.md) —— 各阶段实测记录

## 路线图

P1–P8 已完成（集合、请求、历史、环境、导入、压测、MCP、CLI）。正在打磨：命令面板、自动更新、应用图标。

## 许可证

仓库暂未指定开源许可证。