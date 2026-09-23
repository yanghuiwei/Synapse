# Synapse

> **Agent 原生的 API 工作台。** 单文件安装包 5.4 MB，本地优先，无账号 —— 让 AI 直接操作你的接口集合。

Postman 很重、要登录、要云同步。更重要的是：**它是给人点的，不是给 AI 调的。**

Synapse 把两件事同时做干净：

1. **管好日常 API 工作** —— 集合、环境变量、历史、压测，该有的都有。
2. **把同一份数据暴露给 AI** —— 通过 MCP Server 或 CLI，让 Claude Code / Cursor / WorkBuddy 直接找接口、改参数、发请求、看响应。不是把 curl 贴进对话框，而是 **AI 自己动手**。

应用内不内置聊天框、不配 provider —— AI 从外面进来，Synapse 只负责干净的数据和干净的通道。

---

## 亮点

| | |
|---|---|
| **5.4 MB 安装包** | Tauri 2 + Rust。不依赖 OpenSSL / SQLite / VC++ 可再发行包，双击即用 |
| **本地优先** | 全部数据在一个 SQLite 文件里。无云、无账号、无遥测 |
| **MCP Server** | 13 个 tool，stdio 与回环 HTTP 双通道，一键生成客户端配置 |
| **CLI** | 8 组子命令，`--json` 输出可直接 `\| jq`，适合脚本与管道 |
| **外部改动自动刷新** | AI 在另一个进程改了库，桌面端 1 秒内自动刷新，不用重启 |
| **并发压测** | 控制并发数，P50 / P90 / P95 / P99 / TPS 精确分位，导出 HTML 报表 |
| **体积即架构** | 不是"压出来的"：拆 crate 让 MCP / CLI 各带各的依赖，静态链接 CRT，LTO + strip |

## 快速开始

从 [Releases](https://github.com/yanghuiwei/Synapse/releases) 下载对应平台安装包，安装即用。Windows 安装器会自动把安装目录加进用户 PATH，装完新开终端就能敲 `reqly-cli`。

### 接上 AI 客户端

打开 **设置 → MCP**，面板会给出与当前安装路径匹配的片段，直接粘进客户端配置。形状如下（`<you>` 处是面板填好的真实路径）：

```jsonc
// Claude Code / Cursor / WorkBuddy —— mcpServers
{
  "mcpServers": {
    "reqly": {
      "args": ["--db", "C:/Users/<you>/AppData/Roaming/com.reqly.desktop/reqly.db"],
      "command": "C:/Users/<you>/AppData/Local/Synapse/reqly-mcp.exe",
      "env": {}
    }
  }
}
```

```yaml
# Hermes Agent —— mcp_servers（只粘 reqly: 这一段，不要连 mcp_servers: 头一起）
mcp_servers:
  reqly:
    command: "C:/Users/<you>/AppData/Local/Synapse/reqly-mcp.exe"
    args: ["--db", "C:/Users/<you>/AppData/Roaming/com.reqly.desktop/reqly.db"]
    enabled: true
    timeout: 120
    connect_timeout: 60
    trust: untrusted
```

路径一律输出**正斜杠**：YAML 双引号里 `\U` 不是合法转义，`\t`/`\n` 又恰好合法 —— 后者会让路径被静默改写成带制表符的字符串，配置文件看着没问题、进程却找不到文件。

装完客户端后，AI 就能 `list_requests` 找接口、`send_request` 直接发请求。

### CLI 速览

CLI 需要知道库在哪：用 `--db` 指定，或设一次环境变量 `REQLY_DB`（桌面端「设置」页能直接看到它的绝对路径）。

```bash
export REQLY_DB="$APPDATA/com.reqly.desktop/reqly.db"   # Windows，或每次传 --db

reqly-cli folders                                # 列出集合
reqly-cli requests list --json | jq '.requests[] | .name'
reqly-cli send --request-id 151                  # 发一条已保存的请求
reqly-cli env list                               # 看当前环境的变量（secret 恒为 ••••）
reqly-cli --help                                 # 全部 8 组子命令
```

AI 用 MCP 是因为它能**持续对话**；脚本用 CLI 是因为它**没有上下文成本**（`--help` 现查，不常驻 schema）。

> 输出约定：数据一律走 **stdout**，日志走 **stderr** —— 所以 `--json` 可以放心直接管道给 `jq`。

## 从源码构建

需要：**Node.js `^20.19.0 || >=22.12.0`**、**pnpm**、**Rust stable**、**MSVC 生成工具**（Windows 上由 Visual Studio 2022 / VS Build Tools 提供，勾选「使用 C++ 的桌面开发」）。

```bash
git clone https://github.com/yanghuiwei/Synapse.git
cd Synapse
pnpm install
pnpm tauri dev      # 开发
pnpm tauri build    # 出安装包
```

> Windows 目标三元组是 `x86_64-pc-windows-msvc`，**编译期需要 MSVC 链接器**（`link.exe`）。
> 但这只是构建依赖：产出物静态链接了 VC 运行时（`static_vcruntime`），
> **终端用户不需要装 VS，也不需要 VC++ 可再发行包**。

## 功能一览

| 模块 | 说明 |
|---|---|
| 集合管理 | 无限层级文件夹，拖拽排序 |
| 请求闭环 | 多方法、查询 / 头 / 表单 / 原始体，六类网络错误分开报 |
| 环境变量 | `{{var}}` 占位符，secret 不回显 |
| Token 源 | 绑定到文件夹，自动刷新，并发单飞 |
| 历史与回收站 | 软删除，误删可还原 |
| 压测 | 并发可控，分位数精确计算，HTML 报表 |
| 导入 | curl 命令、Postman v2.1、ApiPost、HAR、自有 JSON |
| MCP / CLI | AI 双通道，共用同一份数据 |

## AI 双通道：MCP vs CLI

| | MCP Server | CLI |
|---|---|---|
| 适合 | 边聊边查，AI 自己找接口 | 脚本、管道、批处理 |
| 传输 | JSON-RPC（stdio / 127.0.0.1 HTTP） | 进程 stdout |
| 响应体 | 硬截 64KB | 可不截，适合落文件 |
| 上下文成本 | 13 个 tool schema 常驻 | 零，`--help` 现查 |

## 技术栈

React 19 · TypeScript · Tailwind v4 · Tauri 2 · Rust · SQLite（bundled）· reqwest（rustls）· rmcp · clap · Sqlite 单文件存储

## 文档

- [设计文档](docs/design.md) —— 定位、数据模型、模块设计
- [实施计划与踩坑记录](docs/plan.md) —— 各阶段实测记录

## 许可证

仓库暂未指定开源许可证。
