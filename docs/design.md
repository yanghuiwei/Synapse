# Synapse — 轻量级 Agent 原生 API 工作台 · 设计文档

> 版本 v0.2 · 2026-09-11 · 阶段：设计评审通过（第二轮决议已完成）· 下一步 `plan.md`
> 项目代号：`reqly`（名称可一行替换）

---

## 0. 一句话定位

一个 **3–10MB、本地优先、无账号、内置 MCP Server** 的桌面 API 客户端。它替代 Postman 的日常使用场景，同时让 AI（WorkBuddy / Hermes / Claude Code / Cursor）能直接操作你的接口集合。

### 0.1 已确认的决策（7 项）

| 决策项   | 结论                | 核心理由                                                |
| ----- | ----------------- | --------------------------------------------------- |
| 应用形态  | **Tauri 2 桌面应用**  | 安装包 3–10MB / 空闲内存 ~50MB / 冷启动 <0.5s；绕过 CORS、超时、并发上限 |
| AI 连接 | **内置 MCP Server** | AI 客户端连上后直接读写集合、发请求、管环境变量                           |
| 压测能力  | **基础并发统计**        | 并发数 + 总请求数 → 成功率 / P50 / P90 / P95 / P99 / TPS + 报表 |
| 数据存储  | **SQLite 单文件**    | 单 `.db` 文件、历史量大不卡、天然支持软删除回收站、复制即备份                  |

### 0.1.1 补充确认项（2026-09-11 第二轮）

| 决策项     | 结论                          | 说明                                                     |
| ------- | --------------------------- | ------------------------------------------------------ |
| 界面语言    | **本期只做中文**                  | 架构预留 `t('key')` 包裹与 `zh-CN` 资源文件，后续加语言不改业务代码（§9.1）      |
| AI 接口配置 | ~~支持任意 OpenAI 兼容第三方端点~~ → **已裁剪（2026-09-15）** | 原计划给应用内置 provider 配置层（§8.5）。现决定：**AI 能力只从外部 Agent 进来**（MCP / CLI），应用内不做 —— 见 §0.2 与 `plan.md` §22 |
| MCP 客户端 | **WorkBuddy / Hermes / Claude Code 全支持** | 协议一致，仅配置文件格式不同（JSON vs YAML）；配置生成器同时产出两种（§8.1） |

### 0.2 非目标（本期明确不做）

- ❌ 云同步 / 团队协作 / 账号体系
- ❌ gRPC / WebSocket / MQTT / Kafka（架构预留，不做实现）
- ❌ 抓包代理（Proxy / Interceptor）
- ❌ 专业级压测（ramp-up、RPS 曲线、分布式加压）
- ❌ 插件市场
- ❌ **应用内置 AI**（原 P9 AI Provider 配置层 + P10 内置助手，2026-09-15 裁剪）——
  AI 能力只从外部 Agent 经 MCP / CLI 进入，应用内不设 provider 配置与对话面板
- ❌ **OpenAPI 3.x / 2.0 导入**（2026-09-15 裁剪，按需再迭代）

---

## 1. 需求 → 设计映射

| # | 你的需求                             | 设计落点                                                         | 章节          |
| - | -------------------------------- | ------------------------------------------------------------ | ----------- |
| 1 | 导入接口数据（apipost / curl）           | 统一 Importer 层，Rust 侧解析；额外覆盖 Postman v2.1 / HAR 1.2（OpenAPI 见 §0.2） | §5          |
| 2 | 多方法请求 + 授权刷新 token + 占位符匹配 token | Token Provider 机制 + 命名空间占位符 `{{token.access}}`               | §4.3 / §4.4 |
| 3 | 请求历史记录 + 删除回收站                   | `history` 表 + 全局软删除 `deleted_at`                             | §3.3        |
| 4 | 暗色 / 浅色主题切换                      | CSS 变量双主题 + 跟随系统                                             | §6.1        |
| 5 | 移动接口到其他分类（文件夹）                   | 树形 `folder` 自引用 + 拖拽移动 / 右键移动对话框                             | §3.2 / §6.3 |
| 6 | 并发压测 + 结果报表                      | Rust 侧 tokio Semaphore 并发引擎 + hdrhistogram 统计 + HTML 报表导出    | §7          |
| + | 轻量 / 本地 / AI 连接 / 美观             | Tauri 2 / SQLite / MCP Server / 自研设计系统                       | 全文          |

---


## 2. 总体架构

```
┌──────────────────────────────────────────────────────────────┐
│  前端 WebView  (React 19 + TS + Vite + Tailwind v4)           │
│                                                              │
│  ┌────────────┬─────────────────────────┬─────────────────┐  │
│  │ 左栏        │ 中栏                     │ 右栏             │  │
│  │ 集合/历史/  │ 请求编辑器               │ 响应查看器        │  │
│  │ 回收站      │ 方法·URL·Params·Headers │ Pretty/Raw/      │  │
│  │ (树形+拖拽) │ Body·Auth·压测配置       │ Headers/Timeline │  │
│  └────────────┴─────────────────────────┴─────────────────┘  │
│  底部抽屉：压测实时面板 / 控制台日志                             │
└───────────────────────┬──────────────────────────────────────┘
                        │  Tauri IPC (invoke / event 双向)
┌───────────────────────▼──────────────────────────────────────┐
│  Rust Core  (src-tauri)                                       │
│                                                              │
│  ┌──────────────┐ ┌──────────────┐ ┌───────────────────────┐ │
│  │ http 引擎     │ │ 压测引擎      │ │ storage (SQLite)      │ │
│  │ reqwest 池    │ │ Semaphore    │ │ rusqlite + r2d2       │ │
│  │ HTTP/2·gzip   │ │ hdrhistogram │ │ 迁移 + 仓储            │ │
│  └──────────────┘ └──────────────┘ └───────────────────────┘ │
│  ┌──────────────┐ ┌──────────────┐ ┌───────────────────────┐ │
│  │ template 引擎 │ │ importer     │ │ MCP Server (rmcp)     │ │
│  │ {{}} 占位符   │ │ curl/apipost │ │ stdio + HTTP 双通道    │ │
│  │ token 注入    │ │ postman/oas  │ │ 13 个 tools            │ │
│  └──────────────┘ └──────────────┘ └───────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
                        │
                        ▼
            %APPDATA%/com.reqly.desktop/reqly.db   ← 全部数据本地单文件
```

### 2.1 为什么请求执行必须放在 Rust 侧

| 维度     | 浏览器 fetch         | Rust reqwest                     |
| ------ | ----------------- | -------------------------------- |
| CORS   | 被目标站策略拦截          | 无限制                              |
| 超时     | 通常 30s 强制中断       | 自定义，可关                           |
| 并发上限   | HTTP/1.1 单域名 6 连接 | 连接池可控到数百                         |
| HTTP/2 | 视浏览器              | 原生支持                             |
| 响应体积   | 内存墙 + 卡 UI        | 流式落盘 + 分块渲染                      |
| 压测     | 基本不可行             | Semaphore 精确控并发                  |
| 证书     | 无法配置自签名/关闭校验      | 可配 `danger_accept_invalid_certs` |

**结论**：这是"必须做桌面应用"而非"想做桌面应用"的技术根因。

---

## 3. 数据模型

单文件 SQLite，路径 `%APPDATA%/com.reqly.desktop/reqly.db`（macOS `~/Library/Application Support/com.reqly.desktop/`）。  
启用 `PRAGMA journal_mode=WAL` `foreign_keys=ON` `synchronous=NORMAL`。

### 3.1 核心概念层级

```
Workspace（工作区）
  └── Folder（文件夹 / 分类，支持无限层级自引用）
        ├── Folder ...
        └── Request（接口请求 = Postman 的 request item）
Environment（环境：dev / test / prod）
  └── EnvVar（变量键值对，可标记 secret）
TokenSource（Token 提供者，绑定到某个 Folder）
History（请求执行历史，软删除 → 回收站）
LoadTestRun / LoadTestSample（压测运行与采样）
```


### 3.2 表结构

```sql
-- 工作区
CREATE TABLE workspace (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL,
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  deleted_at  TEXT
);

-- 文件夹 / 分类（自引用树）
CREATE TABLE folder (
  id           INTEGER PRIMARY KEY,
  workspace_id INTEGER NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
  parent_id    INTEGER REFERENCES folder(id) ON DELETE CASCADE,  -- NULL = 根
  name         TEXT NOT NULL,
  sort_order   INTEGER NOT NULL DEFAULT 0,
  -- Token 源分组标记（见 §4.3）。改造后已经没有界面入口，
  -- 只有 ensure_system_folder() 会把它置 1；CLI 仍在渲染这一列（欠账，见 plan.md §26.7）
  is_token_source INTEGER NOT NULL DEFAULT 0,
  -- 系统分类标记（见 §4.3）：每个工作区有且只有一个「token刷新」分类
  is_system    INTEGER NOT NULL DEFAULT 0,
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL,
  deleted_at   TEXT
);
CREATE INDEX idx_folder_parent ON folder(workspace_id, parent_id, sort_order);
-- 部分唯一索引：只约束系统分类。普通分类的 is_system = 0 不参与唯一性，
-- 否则第二个普通分类就插不进来了。迁移见 004_system_folder.sql。
CREATE UNIQUE INDEX idx_folder_system ON folder(workspace_id) WHERE is_system = 1;

-- 接口请求
CREATE TABLE request (
  id           INTEGER PRIMARY KEY,
  folder_id    INTEGER REFERENCES folder(id) ON DELETE SET NULL,
  workspace_id INTEGER NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
  name         TEXT NOT NULL,
  method       TEXT NOT NULL DEFAULT 'GET',       -- GET/POST/PUT/PATCH/DELETE/HEAD/OPTIONS
  url          TEXT NOT NULL DEFAULT '',
  params       TEXT NOT NULL DEFAULT '[]',        -- JSON: [{key,value,enabled,desc}]
  headers      TEXT NOT NULL DEFAULT '[]',        -- JSON: 同上
  body_type    TEXT NOT NULL DEFAULT 'none',      -- none|json|form|urlencoded|raw|binary|graphql
  body         TEXT NOT NULL DEFAULT '',
  auth         TEXT NOT NULL DEFAULT '{}',        -- JSON: AuthConfig
  settings     TEXT NOT NULL DEFAULT '{}',        -- JSON: 超时/重定向/证书/代理
  pre_script   TEXT NOT NULL DEFAULT '',
  test_script  TEXT NOT NULL DEFAULT '',
  sort_order   INTEGER NOT NULL DEFAULT 0,
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL,
  deleted_at   TEXT
);
CREATE INDEX idx_request_folder ON request(folder_id, sort_order);
CREATE INDEX idx_request_name   ON request(workspace_id, name);

-- 环境
CREATE TABLE environment (
  id           INTEGER PRIMARY KEY,
  workspace_id INTEGER NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
  name         TEXT NOT NULL,
  is_active    INTEGER NOT NULL DEFAULT 0,
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL,
  deleted_at   TEXT
);
CREATE TABLE env_var (
  id             INTEGER PRIMARY KEY,
  environment_id INTEGER NOT NULL REFERENCES environment(id) ON DELETE CASCADE,
  key            TEXT NOT NULL,
  value          TEXT NOT NULL DEFAULT '',
  is_secret      INTEGER NOT NULL DEFAULT 0,   -- 1 → 系统密钥串加密存储
  enabled        INTEGER NOT NULL DEFAULT 1,
  sort_order     INTEGER NOT NULL DEFAULT 0
);

-- Token 提供者（§4.3）
CREATE TABLE token_source (
  id                INTEGER PRIMARY KEY,
  workspace_id      INTEGER NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
  folder_id         INTEGER NOT NULL REFERENCES folder(id) ON DELETE CASCADE,
  name              TEXT NOT NULL,               -- 如 'iPSA-OAuth'
  fetch_request_id  INTEGER NOT NULL REFERENCES request(id),  -- 用哪个请求取 token
  access_path       TEXT NOT NULL DEFAULT '$.access_token',
  refresh_path      TEXT NOT NULL DEFAULT '$.refresh_token',
  expires_in_path   TEXT NOT NULL DEFAULT '$.expires_in',     -- 取秒数或绝对时间
  expires_in_unit   TEXT NOT NULL DEFAULT 'seconds',          -- seconds|millis|absolute
  refresh_request_id INTEGER REFERENCES request(id),          -- 刷新用哪个请求（可空=复用 fetch）
  auto_refresh      INTEGER NOT NULL DEFAULT 1,
  skew_seconds      INTEGER NOT NULL DEFAULT 30,              -- 提前 30s 视为过期
  created_at        TEXT NOT NULL,
  updated_at        TEXT NOT NULL
);

-- Token 运行时缓存
CREATE TABLE token_cache (
  id               INTEGER PRIMARY KEY,
  token_source_id  INTEGER NOT NULL REFERENCES token_source(id) ON DELETE CASCADE,
  access_token     TEXT NOT NULL,
  refresh_token    TEXT,
  expires_at       TEXT,           -- ISO8601，NULL = 永不过期
  raw_response     TEXT,           -- 原始响应快照，便于调试
  updated_at       TEXT NOT NULL
);

-- 历史记录（软删除 = 回收站）
CREATE TABLE history (
  id            INTEGER PRIMARY KEY,
  workspace_id  INTEGER NOT NULL,
  request_id    INTEGER,                    -- 可为 NULL（请求已被删）
  request_name  TEXT NOT NULL,              -- 冗余快照，请求删除后仍可读
  snapshot      TEXT NOT NULL,              -- JSON：完整请求快照
  status        INTEGER,                    -- HTTP 状态码，NULL = 网络错误
  duration_ms   INTEGER,
  resp_size     INTEGER,
  error         TEXT,
  environment   TEXT,                       -- 当时使用的环境名
  executed_at   TEXT NOT NULL,
  deleted_at    TEXT                        -- 非空 → 在回收站
);
CREATE INDEX idx_history_time ON history(workspace_id, executed_at DESC);
CREATE INDEX idx_history_req  ON history(request_id);

-- 压测
CREATE TABLE loadtest_run (
  id           INTEGER PRIMARY KEY,
  request_id   INTEGER REFERENCES request(id) ON DELETE SET NULL,
  request_name TEXT NOT NULL,
  config       TEXT NOT NULL,     -- JSON: {concurrency,total,timeoutMs,...}
  summary      TEXT NOT NULL,     -- JSON: 统计汇总
  started_at   TEXT NOT NULL,
  finished_at  TEXT
);
CREATE TABLE loadtest_sample (
  id          INTEGER PRIMARY KEY,
  run_id      INTEGER NOT NULL REFERENCES loadtest_run(id) ON DELETE CASCADE,
  seq         INTEGER NOT NULL,
  status      INTEGER,
  duration_ms INTEGER NOT NULL,
  bytes       INTEGER,
  error       TEXT,
  started_offset_ms INTEGER NOT NULL   -- 相对压测开始的偏移，画时间线用
);
CREATE INDEX idx_sample_run ON loadtest_sample(run_id, seq);

-- 迁移版本
CREATE TABLE schema_migration (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
```

### 3.3 回收站策略（统一软删除）

- 所有可删对象（`folder` / `request` / `environment` / `history`）都有 `deleted_at`。
- "删除" = 写 `deleted_at = now()`；"回收站" = `WHERE deleted_at IS NOT NULL`。
- **级联软删除**：删文件夹时，递归标记其所有子孙文件夹与请求。
- 回收站支持：还原（清空 `deleted_at`，若父级已删则还原到根）、彻底删除、清空回收站。
- 回收站默认保留 30 天，启动时清理超期项（可配置）。

---

## 4. 请求引擎

### 4.1 请求执行流水线

```
用户点 Send
  │
  ├─1 取 Request 定义 + 当前 Environment
  ├─2 template 引擎解析所有字符串字段（url/params/headers/body/auth）
  │     ├── {{env.KEY}}         → 环境变量
  │     ├── {{token.access}}    → 见 §4.3，可能触发同步刷新
  │     ├── {{$uuid}} {{$timestamp}} {{$randomInt}}  → 动态函数
  │     ├── {{folder.xxx.yyy}}  → 上一级文件夹级变量（可选）
  │     └── {{resp.$.data.id}}  → 上一次响应提取值（会话内）
  ├─3 reqwest 组装 RequestBuilder（超时/重定向/证书/代理/HTTP版本）
  ├─4 发送 → 记录 (status, headers, body, duration, size)
  ├─5 写入 history（异步，不阻塞 UI）
  └─6 推送事件 reqly://response 给前端；前端渲染
```

### 4.2 占位符命名空间总表

| 语法                           | 来源                     | 示例                        |
| ---------------------------- | ---------------------- | ------------------------- |
| `{{env.KEY}}`                | 激活的环境变量                | `{{env.baseUrl}}`         |
| `{{token.access}}`           | Token Provider 自动获取/刷新 | `Bearer {{token.access}}` |
| `{{token.refresh}}`          | 同上，refresh token       | —                         |
| `{{token.expiresAt}}`        | 过期时间 ISO 字符串           | —                         |
| `{{$uuid}}`                  | 运行时生成 UUIDv4           | —                         |
| `{{$timestamp}}`             | Unix 秒                 | —                         |
| `{{$timestampMs}}`           | Unix 毫秒                | —                         |
| `{{$randomInt}}`             | 0–10000 随机整数           | —                         |
| `{{$datetime 'YYYY-MM-DD'}}` | 格式化当前时间                | —                         |
| `{{resp.$.path}}`            | 最近一次响应 JSONPath        | `{{resp.$.data.token}}`   |
| `{{prev.$.path}}`            | 上一条历史响应                | —                         |

> 解析顺序：动态函数 → 环境变量 → token → 响应提取。未匹配的占位符**原样保留**并在 UI 上高亮为警告黄，不静默吞掉。


### 4.3 Token 自动刷新机制 ★核心需求

**概念**：Token 来源**固定配在一个系统分类下** —— 每个工作区初始化时会自带一个名为「token刷新」的分类（`folder.is_system = 1`，见 §3.2）。项目里任意请求用到 `{{token.*}}`，系统在发请求前检查缓存，过期就自动打一次 Provider 接口刷新。

**为什么不再「标记某个分类为 Token 源分组」**：旧设计把 Token 来源挂在用户手选的分类上，于是「请求没归档到那个分类」就永远配不了 Token —— 解析链为空直接返回 `None`。现在 `token_source.folder_id` 恒指向系统分类，解析走两级：

| 顺序 | 来源                 | 说明                                 |
| -- | ------------------ | ---------------------------------- |
| 1  | `first_in_chain`   | 请求所在分类沿父链向上，最近的一个有 Token 来源就赢      |
| 2  | `system_source`    | 全局兜底：系统分类下的第一条 Token 来源            |

两级都空才算「没配 Token」。所以请求挂在哪、有没有归档，`{{token.access}}` 都能解析。

**获取 Token 与刷新 Token 是同一件事**：只有一个 `fetch_request_id`，缓存失效就打它，拿回来的 access 写进 `token_cache`。不单独配「刷新接口」，也**不问用户要 refresh_token 的路径** —— 没有「拿 refresh_token 去换新 Token」这条路径（表里的 `refresh_path` 列还在，老配置照原样读出来，界面上没有它的位置）。

**配置界面**（Auth 面板 → Token 源 选项卡，**页内直配，不弹窗**）。字段按亲疏排成三行，整页在请求面板的默认高度里**不用下拉**：

```
名称 [ 内网 OAuth ]      获取 Token 的接口（重新调它换新） [ POST /oauth/token ▾ ]
┌ 从响应里取什么 ──────────────────────────────────────────────────────┐
│ access_token 路径 [ $.data.accessToken ]  过期时间路径 [ 留空 = 永不过期 ]  单位 [ 秒 ▾ ] │
└──────────────────────────────────────────────────────────────────┘
────────────────────────────────────────────────────────────────────
[✓] 自动重新获取   提前量（秒） [ 30 ]        关掉后过期直接报错，不会拿旧 Token 硬发

当前状态：● 有效  剩余 47 分 12 秒      [ 手动刷新 ]          [ 取消 ] [ 保存 ]
```

**系统分类的约束**（三层防护，缺一层都会漏）：

| 层   | 位置                    | 内容                                                         |
| --- | --------------------- | ---------------------------------------------------------- |
| 数据库 | `idx_folder_system`   | 部分唯一索引，同工作区只能有一个                                           |
| 仓储  | `ensure_not_system()` | `rename` / `move_to` / `soft_delete_tree` / `purge_tree` 各一处守卫 |
| 界面  | `CollectionTree`      | 系统分类右键只有「在此新建请求 / 新建子分类」，不可拖拽，F2 / Delete 不绑                |

系统分类的名字走常量 `SYSTEM_FOLDER_NAME` 而不是 i18n —— 它会出现在 MCP / CLI 的输出里，那两个入口没有前端 i18n 表。

**运行时行为**：

| 场景             | 行为                                                 |
| -------------- | -------------------------------------------------- |
| 缓存有效           | 直接注入，零额外请求                                         |
| 缓存过期 / 不存在     | 用 `fetch_request_id` 打一次，按路径提取，写 `token_cache`，再注入 |
| 刷新失败（401/网络错）  | 中止当前请求，UI 弹出错误卡片并指向 Token 源配置                      |
| 并发 5 个请求同时发现过期 | **单飞锁（single-flight）**：只打一次刷新请求，其余 await 同一 Future |
| 用户点"立即刷新"      | 强制刷新，忽略缓存                                          |

**单飞锁实现**：`tokio::sync::Mutex<HashMap<TokenSourceId, Arc<Shared<...>>>>`，或 `OnceCell` + `tokio::sync::Notify`。保证压测场景下 token 不会被打爆。

**存储安全**：`token_cache.raw_response` 默认不落盘（内存态），仅调试模式开启；access/refresh token 用系统密钥串（Windows Credential Manager）加密后存储。

### 4.4 Auth 类型支持

| 类型        | 字段                                       | 说明                  |
| --------- | ---------------------------------------- | ------------------- |
| None      | —                                        |                     |
| Bearer    | token（支持 `{{token.access}}`）             |                     |
| Basic     | username / password                      |                     |
| API Key   | key / value / 位置(header|query)           |                     |
| OAuth 2.0 | 走 §4.3 Token Provider                    |                     |
| Digest    | username / password                      | reqwest 支持          |
| AWS SigV4 | accessKey / secretKey / region / service | 可选，Phase 后期         |
| 继承父级      | —                                        | 沿文件夹树向上找最近的 auth 配置 |

---

## 5. 导入器

统一 trait：`trait Importer { fn detect(&self, input: &ImportInput) -> Confidence; fn parse(&self, input) -> Result<ImportBundle>; }`  
`ImportBundle { folders: Vec<FolderNode>, requests: Vec<RequestDraft>, environments: Vec<EnvDraft> }`

### 5.1 curl 导入（粘贴即用）

支持语法：

| 参数                                                            | 支持                                         |
| ------------------------------------------------------------- | ------------------------------------------ |
| `-X / --request`                                              | ✅ 含大小写任意方法                                 |
| `-H / --header`                                               | ✅ 重复出现，`Key: Value` 拆解                     |
| `-d / --data / --data-raw / --data-binary / --data-urlencode` | ✅ 自动推断 `application/x-www-form-urlencoded` |
| `-F / --form`                                                 | ✅ → multipart                              |
| `-u / --user`                                                 | ✅ → Basic Auth                             |
| `-b / --cookie`                                               | ✅ → Cookie header                          |
| `-A / --user-agent`                                           | ✅                                          |
| `--compressed` / `-k` / `--insecure`                          | ✅ → settings 标记                            |
| `-L / --location`                                             | ✅ → 跟随重定向                                  |
| `--max-time`                                                  | ✅ → 超时                                     |
| `--url`                                                       | ✅                                          |

**引号与换行处理**（最容易踩坑的地方）：

- 支持单引号、双引号、双引号内 `\"` 转义、`$'...'` ANSI-C 转义
- 支持 `\` + 换行续行、Windows `^` 续行、CRLF
- 支持从浏览器 DevTools「Copy as cURL (bash / cmd / powershell)」三种格式
- 自动剥掉 `curl`/`curl.exe` 前缀与 `-s`/`-v`/`--silent` 等无关键

**推断逻辑**：无 `-X` 时有 body → `POST`，无 body → `GET`；有 `-d` 且未显式设 Content-Type → 补 `application/x-www-form-urlencoded`。

### 5.2 apipost 导入

Apipost 导出两条路径，都支持：

| 导出格式                                                  | 解析方式                                                                                     |
| ----------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| **Postman 格式**（Apipost 支持导出为 Postman Collection v2.1） | 主解析器：`postman.rs`，按 v2.1 schema 递归 item/folder，`{{var}}` 保留                              |
| **Apipost 原生 JSON**（含 `apipost` 标志 / `target_type`）   | 副解析器：按 apipost 自有字段映射（`method`/`url`/`headers`/`body`/`auth`/`pre_script`/`test_script`） |

自动识别：读 JSON 根节点的 `info.schema` / `_postman_id` / `apipost` 等指纹，选择解析器，用户无需手动选。

**「Apipost 原生 JSON」实际有两种形状，指纹得看两层**：

| 形状 | 顶层键 | 走哪条路 |
| --- | --- | --- |
| 桌面端「导出项目」 | `project` / `apis` / `envs` / `models`（`target_type` 藏 `apis[]` 元素里） | 严格路径：按其真实层次读（`request.url` / `header.parameter[]` / `body.mode` / `event`） |
| 扁平结构 | 顶层 `apipost` / `target_type` | 宽松路径：多候选字段名，认不出记警告 |

只查顶层 `apipost` / `target_type` 会把第一种整个漏掉，而漏掉的后果不是报错，
是掉进 reqly 兜底 →「导入成功、0 条接口」。成因与修法见 `plan.md` §25。

**映射表（Postman v2.1 → reqly）**：

| Postman                                            | reqly                        |
| -------------------------------------------------- | ---------------------------- |
| `item[].item[]`                                    | `folder` 树                   |
| `item[].request.method/url/header/body`            | `request.*`                  |
| `url.query[]`                                      | `params[]`                   |
| `body.mode` = raw/urlencoded/formdata/graphql      | `body_type`                  |
| `variable[]` (collection/folder 级)                 | 提升为环境变量                      |
| `event[].listen=prerequest/test` + `script.exec[]` | `pre_script` / `test_script` |
| `auth.type` = bearer/basic/apikey                  | `auth`                       |

### 5.3 附赠的导入器（低成本高价值）

| 格式                            | 工作量                | 价值              |
| ----------------------------- | ------------------ | --------------- |
| **Postman Collection v2.1**   | 复用 apipost 主解析器，≈0 | 极高（生态最大）        |
| ~~**OpenAPI 3.x / Swagger 2.0**~~ | — | **已裁剪（2026-09-15，按需再迭代）** —— 见 §0.2 |
| **HAR 1.2**（浏览器 F12 导出）       | 低                  | 高（比 curl 批量强得多） |
| **reqly 自有 JSON**             | 极低                 | 必做（导入导出一体）      |

### 5.4 冲突处理

导入对话框提供三种模式：**合并（同名更新）** / **新建副本（重命名加后缀）** / **覆盖**。  
导入前显示预览树：`+12 新增 / ~3 更新 / =5 跳过`。

---

## 6. 界面设计


### 6.1 设计系统

来自 `ui-ux-pro-max` 的 `Developer Tool / IDE` 配色 + `Dark Mode (OLED)` 风格。

**色彩（CSS 变量驱动双主题）**

| Token             | Light     | Dark      | 用途           |
| ----------------- | --------- | --------- | ------------ |
| `--bg`            | `#FFFFFF` | `#0F172A` | 主背景          |
| `--bg-subtle`     | `#F8FAFC` | `#0B1220` | 页面底色 / 侧栏    |
| `--surface`       | `#FFFFFF` | `#161B2B` | 卡片 / 面板      |
| `--surface-hover` | `#F1F5F9` | `#1E2637` | 悬停态          |
| `--border`        | `#E2E8F0` | `#273349` | 分隔线          |
| `--border-strong` | `#CBD5E1` | `#334155` | 强调边框         |
| `--text`          | `#0F172A` | `#F1F5F9` | 主文本          |
| `--text-muted`    | `#475569` | `#94A3B8` | 次要文本         |
| `--text-faint`    | `#94A3B8` | `#64748B` | 提示文本         |
| `--primary`       | `#2563EB` | `#3B82F6` | 主色 / 主按钮     |
| `--primary-hover` | `#1D4ED8` | `#2563EB` | 主色悬停         |
| `--success`       | `#16A34A` | `#4ADE80` | 2xx / 成功     |
| `--warning`       | `#D97706` | `#FBBF24` | 3xx / 警告     |
| `--danger`        | `#DC2626` | `#F87171` | 4xx-5xx / 危险 |
| `--info`          | `#0891B2` | `#22D3EE` | 提示 / 1xx     |

**方法色（业界约定）**：`GET #2563EB` · `POST #16A34A` · `PUT #D97706` · `PATCH #7C3AED` · `DELETE #DC2626` · `HEAD/OPTIONS #64748B`

**字体**

```css
--font-sans: "IBM Plex Sans", "PingFang SC", "Microsoft YaHei", system-ui, sans-serif;
--font-mono: "JetBrains Mono", "Cascadia Code", "Consolas", monospace;
```

数字（耗时/大小/百分位）统一 `font-variant-numeric: tabular-nums`。

**其他规范**

- 图标：**Lucide**（`lucide-react`），统一 24×24 viewBox，线宽 1.5，**零 emoji**
- 圆角：`--radius-sm 6px` / `--radius-md 8px` / `--radius-lg 12px`
- 阴影：仅用 `0 1px 2px rgba(0,0,0,.04)` 级别的极浅阴影 + 边框，**无渐变、无毛玻璃**
- 过渡：`transition: background-color 150ms, color 150ms, border-color 150ms`；**不用 scale 避免布局抖动**
- 暗色模式下滚动条、CodeMirror、图表都需同步换色（通过 CSS 变量统一下发）


### 6.2 主窗口布局

```
┌─ TopBar ─────────────────────────────────────────────────────────────┐
│ ● Reqly │ [工作区 ▾] │ 🔍 搜索接口 (Ctrl+K) │ 环境:[prod ▾] │ ☾ │ ⚙ │
├──────────────┬────────────────────────────────┬──────────────────────┤
│ Sidebar      │ Request Editor                 │ Response             │
│ 280px 可折叠  │ flex                           │ 50% 可拖拽           │
│              │                                │                      │
│ [集合|历史|   │ GET ▾ │ https://... │ [Send ▾]│ 200 OK · 132ms · 4.2KB│
│  回收站]      │ ───────────────────────────────│ ──────────────────── │
│              │ Params Headers Body Auth 压测   │ Pretty Raw Headers   │
│ 🔍 过滤       │ ───────────────────────────────│ Cookies Timeline     │
│              │                                │                      │
│ ▾ 📁 系统     │  ☑ key      value       desc   │ {                    │
│   ▾ 📁 认证   │  ────────────────────────────  │   "code": 0,         │
│     • POST   │  ☑ page     1                  │   "data": { ... }    │
│       /token  │  ☐ size     20                 │ }                    │
│   • GET /user │                                │                      │
│ ▾ 📁 工时     │                                │                      │
│   • GET /ts   │                                │                      │
├──────────────┴────────────────────────────────┴──────────────────────┤
│ ▼ 压测面板 (可折叠抽屉)                                                │
└──────────────────────────────────────────────────────────────────────┘
```

交互细节：

- **左栏**：三种模式切换（集合 / 历史 / 回收站）。树节点右键菜单：新建文件夹 / 新建请求 / 重命名 / 复制 / 移动到… / 导出 / 删除。
- **拖拽移动**：HTML5 DnD，拖到文件夹上高亮，拖到文件夹之间显示插入线；拖动时禁用到自身子孙（防成环）。
- **中栏**：URL 栏占位符彩色高亮（`{{env.*}}` 蓝、`{{token.*}}` 紫、未解析 黄底）。
- **右栏**：响应体用 CodeMirror 6（JSON 折叠 / 语法高亮 / 大文件虚拟滚动）；`>2MB` 自动切纯文本模式并提示。
- **底部抽屉**：默认收起；点 Send 下拉的"压测"或切到压测 Tab 时展开。
- **状态栏**（最底 22px）：当前工作区 · 数据库路径 · 请求计数 · 版本号。

### 6.3 关键交互清单

| 快捷键                       | 动作                         |
| ------------------------- | -------------------------- |
| `Ctrl+Enter`              | 发送当前请求                     |
| `Ctrl+K`                  | 打开命令面板（跳转任意请求 / 执行任意命令）    |
| `Ctrl+N` / `Ctrl+Shift+N` | 新建请求 / 新建文件夹               |
| `Ctrl+S`                  | 保存（本工具全自动保存，此键用于强制落盘 + 提示） |
| `Ctrl+Shift+T`            | 切换主题                       |
| `Ctrl+1/2/3`              | 左栏切到 集合 / 历史 / 回收站         |
| `Alt+↑/↓`                 | 上一条 / 下一条历史                |
| `Esc`                     | 取消正在进行的请求 / 压测             |

**命令面板（Ctrl+K）**&#x662F;目前同类工具体验分水岭——必须先做，成本不高（一个模糊搜索 + 动作注册表）。

### 6.4 请求生命周期状态机

```
idle ──Send──▶ sending ──┬─▶ success (2xx/3xx)
                         ├─▶ error   (4xx/5xx)
                         ├─▶ network_error (DNS/TLS/超时)
                         └─▶ cancelled (Esc / 用户取消)
```

每个状态在 UI 上有明确视觉：URL 栏右侧转圈 / 绿色时长 / 红色错误卡片（含原始错误信息 + 建议）。  
压测进行中时，"取消"按钮切换为红色 `Cancel`，通过 `CancellationToken` 广播给所有 worker。

---

## 7. 压测引擎

### 7.1 配置模型

```ts
interface LoadTestConfig {
  concurrency: number;      // 并发数 C
  total: number;            // 总请求数 N
  timeoutMs: number;        // 单请求超时
  followRedirects: boolean;
  keepAlive: boolean;       // 复用连接（默认 true）
  thinkTimeMs: number;      // 每个 worker 两轮之间的间隔，默认 0
  // 高级（折叠区）
  httpVersion: 'auto' | 'http1' | 'http2';
  warmupRequests: number;   // 预热请求数，不计入统计
}
```


### 7.2 执行模型

```
                ┌──────────────────────────────┐
                │  reqwest::Client (共享)       │
                │  pool_max_idle_per_host = C   │
                │  连接复用 + HTTP/2 多路复用    │
                └──────────────┬───────────────┘
                               │
   Arc<Semaphore(C)>  ─────────┤
                               │
   for i in 0..N  spawn task ──┤──▶ 每个 task:
       acquire permit          │      t0 = Instant::now()
       send request            │      resp = client.execute()
       record sample           │      dur = t0.elapsed()
       release permit          │      tx.send(Sample{ seq, status, dur, bytes })
                               │
   CancellationToken ──────────┘      每 200ms 或每 500 个样本
                                      → emit('reqly://loadtest-progress')
                                      → 前端实时更新
```

关键点：

- **共享 Client**：`pool_max_idle_per_host(C)` 让连接池预热后基本零握手开销。
- **Semaphore 控并发**：比 `buffer_unordered` 更精确（后者是"在飞数"而非"并发数"）。
- **Warmup**：先跑 `warmupRequests` 个不计入统计，避免首轮 TLS 握手污染 P99。
- **样本不落内存爆掉**：样本流式写入 `loadtest_sample`（批量 500 条一次事务），只在内存保留直方图。
- **取消**：`CancellationToken` 广播，已发出的请求不强制中断（自然结束），未发出的直接丢弃。

### 7.3 统计指标

用 **`hdrhistogram`** 计算延迟分布（比 `Vec<u64>` 排序省内存且高精度）。

| 分组 | 指标                                                                 |
| -- | ------------------------------------------------------------------ |
| 概要 | 总请求数、成功数、失败数、成功率、总耗时、**实际 TPS**                                    |
| 延迟 | min、mean、**P50**、**P90**、**P95**、**P99**、max                       |
| 吞吐 | 平均 RPS、峰值 RPS（1s 窗口）、总传输字节、平均响应大小                                  |
| 错误 | 按状态码分组计数、按错误类型分组（Timeout / DNS / TLS / ConnRefused / …），各取 TopN 样本 |

### 7.4 报表（自动生成）

**实时面板**（压测中）：

- 4 张指标卡：已发送 / 成功率 / 当前 RPS / P95
- 进度条：`3120 / 5000`
- 折线图：延迟时间线（**uPlot**，轻量 45KB）+ RPS 时间线
- 直方图：延迟分布（自绘 SVG 柱状）
- 实时日志：最近 50 条失败样本

**结果报表**（压测结束）：

- 完整指标表 + 图表快照
- **导出 HTML**：单文件自包含（内联 CSS + 图表 SVG），可直接发人
- **导出 CSV**：原始样本 `seq,status,duration_ms,bytes,error,offset_ms`
- **导出 JSON**：`summary` 结构化数据，便于 CI 比对
- 历史压测记录列表（`loadtest_run`），可对比两次运行

### 7.5 诚实的边界说明

基础版压测引擎的能力与局限，**必须在 UI 上明说**，避免误导：

| 能做                | 不能做（专业工具才行）       |
| ----------------- | ----------------- |
| 固定并发下的延迟分布与成功率    | 恒定 RPS 加压（要开环模型）  |
| 单机 TCP 连接池压测      | 分布式多机加压           |
| 定位"并发 N 时 P99 恶化" | 精确的 QPS 阶梯爬坡与拐点分析 |
| 快速验证接口稳定性         | 长稳测试（需数小时 + 资源监控） |

现实提醒：单机压测受限于本机 CPU / 网卡 / 连接数（Windows 默认动态端口约 16k）。到量级时先看客户端 CPU 是否打满。

---

## 8. MCP Server（Agent 原生连接）

### 8.1 双通道暴露

| 通道                        | 用途                                               | 说明                           |
| ------------------------- | ------------------------------------------------ | ---------------------------- |
| **stdio**                 | 被 AI 客户端 spawn（Claude Code / Cursor / WorkBuddy） | 标准 MCP stdio 传输，最稳           |
| **HTTP (SSE/Streamable)** | 独立进程被远程连接 / 同机多客户端                               | `127.0.0.1:7777/mcp`，仅绑定本地回环 |

**配置生成器**：应用内提供「连接 AI 客户端」面板，一键生成并复制对应格式的配置片段。

Claude Code / WorkBuddy / Cursor（JSON，`mcpServers`）：

```json
{
  "mcpServers": {
    "reqly": {
      "command": "%LOCALAPPDATA%/reqly/reqly-mcp.exe",
      "args": ["--db", "%APPDATA%/com.reqly.desktop/reqly.db"],
      "env": {}
    }
  }
}
```

Hermes Agent（YAML，写在 Hermes 数据目录下的 `config.yaml` 的 `mcp_servers:` 下。
Windows 上实测是 `%LOCALAPPDATA%\hermes\config.yaml` —— **不是** `~/.hermes/config.yaml`，
那句早期写法在这台机器上根本不存在）。注意该文件里 `mcp_servers:` **通常已经存在**
（Hermes 自己会写别的 server），所以要粘的是 `reqly:` 那一段、**不能连 `mcp_servers:` 头一起粘**，
否则 YAML 出现重复键：

```yaml
mcp_servers:
  reqly:
    command: "%LOCALAPPDATA%/reqly/reqly-mcp.exe"
    args: ["--db", "%APPDATA%/com.reqly.desktop/reqly.db"]
    enabled: true
    timeout: 120
    connect_timeout: 60
    trust: untrusted
```

**各客户端差异（已在本设计中消化，不是待办）**：

| 客户端                            | 配置格式              | 关键差异                                                        | 我们的应对                                                       |
| ------------------------------ | ----------------- | ----------------------------------------------------------- | ----------------------------------------------------------- |
| WorkBuddy / Claude Code / Cursor | JSON `mcpServers` | 无工具名改写                                                      | 直接照抄                                                        |
| Hermes Agent (Nous Research)   | YAML `mcp_servers` | 工具名会加前缀 `mcp_reqly_<tool>`，且**点号与连字符被替换为下划线**                  | **tool 名只用 `snake_case`，绝不含 `.` 和 `-`**                       |
| Hermes Agent                   | —                 | 按 `readOnlyHint` 注解决定是否弹审批；`trust: untrusted` 时所有写操作需用户确认 | **每个 tool 必须声明注解**（`readOnlyHint` / `idempotentHint` / `destructiveHint`） |
| Hermes Agent                   | —                 | 支持 `stateless`（2026-07-28 的 `server/discover`）与 `legacy` 两种协议时代 | rmcp 侧开启 `auto` 协商，两条路都接                                     |

> 关键设计：MCP 进程**直接读写同一个 SQLite 文件**（WAL 模式支持多进程并发读），不依赖桌面 App 是否在运行。桌面 App 打开时通过文件监听刷新 UI。

### 8.2 对外 tools 清单（13 个）

| Tool                | 参数                                                    | 作用                                            |
| ------------------- | ----------------------------------------------------- | --------------------------------------------- |
| `list_folders`      | `workspaceId?`                                        | 列出文件夹树                                        |
| `list_requests`     | `folderId?` `query?`                                  | 列出请求（可按名模糊搜）                                  |
| `get_request`       | `requestId`                                           | 取完整请求定义                                       |
| `create_request`    | `folderId` `name` `method` `url` `headers?` `body?` … | 创建请求                                          |
| `update_request`    | `requestId` + 部分字段                                    | 局部更新                                          |
| `delete_request`    | `requestId`                                           | 软删除（进回收站）                                     |
| `move_request`      | `requestId` `targetFolderId`                          | **移动到其他分类**                                   |
| `send_request`      | `requestId` 或内联定义 + `environment?`                    | 发请求，返回 status/headers/body（body 超 64KB 截断并标记） |
| `list_environments` | `workspaceId?`                                        | 列出环境与变量（secret 值打码）                           |
| `set_env_var`       | `environmentId` `key` `value`                         | 写环境变量                                         |
| `refresh_token`     | `tokenSourceId`                                       | 强制刷新 token 并返回剩余有效期                           |
| `run_load_test`     | `requestId` `concurrency` `total`                     | 跑压测，返回 summary（阻塞直到完成或超时）                     |
| `import_curl`       | `curlText` `targetFolderId?`                          | 粘贴 curl 即导入，返回创建的请求                           |

**Tool 命名与注解要求**（为兼容 Hermes 的审批模型，见 §8.1）：

- 名称一律 `snake_case`，**不含点号与连字符**——Hermes 会把 `mcp_reqly_send.request` 这类名字规整成下划线，容易与其他 server 撞名。
- 每个 tool 必须声明 MCP 注解，供客户端决定审批策略：

| Tool                                                                  | readOnlyHint | idempotentHint | destructiveHint |
| --------------------------------------------------------------------- | ------------ | -------------- | --------------- |
| `list_folders` / `list_requests` / `get_request` / `list_environments`  | ✅ true       | —              | —               |
| `create_request` / `update_request` / `set_env_var`                     | false        | false          | false           |
| `send_request` / `run_load_test` / `refresh_token` / `import_curl`      | false        | false          | false           |
| `move_request`                                                          | false        | false          | false           |
| `delete_request`                                                        | false        | false          | **true**        |

> `send_request` 虽然语义上是"只读查询"，但它会真实打到外部服务，因此**刻意不标 `readOnlyHint`**——否则 Hermes 在 `untrusted` 模式下会静默替用户发请求。

### 8.3 安全边界

- 监听地址**只绑 `127.0.0.1`**，不对外网暴露。
- MCP 层**不提供**读取原始 secret 的能力（`list_environments` 对 `is_secret=1` 的值返回 `••••`）。
- `send_request` 返回的响应体做 64KB 截断 + 敏感 header（`set-cookie`/`authorization`）默认打码。
- 提供"AI 只读模式"开关：开启后所有写类 tool 直接返回 `permission_denied`。

### 8.4 典型 AI 工作流（这就是"原生 AI 连接"的价值）

> 用户对 Claude Code 说："帮我把工时接口的 pageSize 改成 100，试一下有没有报错，如果 200 就顺便压测一下 50 并发。"

AI 的执行链：

```
list_requests(query="工时")        → 找到 requestId=42
get_request(42)                    → 看清当前 params
update_request(42, params)         → 改 pageSize=100
send_request(42, env="test")       → 200 OK，返回结构正常
run_load_test(42, C=50, N=500)     → 成功率 100%，P95=87ms
```

全程用户不切窗口。这是 Postman 在 2026 年仍未打通的一环。

---

### 8.5 ~~AI Provider 配置层~~（2026-09-15 裁剪，本节原有设计作废）

原计划给应用内置一个 OpenAI 兼容的 provider 配置层（`ai_provider` 表 + `src-tauri/src/ai/`
+ 连通性测试按钮），为 P10 内置助手铺路。**现已裁剪**：本工具的 AI 能力**只从外部 Agent 进来**
（MCP 13 个 tool + CLI 15 条命令），应用内不做 provider 配置与对话面板 —— 见 §0.2 与 `plan.md` §22。

裁剪时留下的两点判断（都是从原设计里抠出来的）：

- 内网网关形态不统一（`http://10.x.x.x:8080/v1`、自签证书、`X-Tenant-Id` 这类自定义头），
  真要支持就得自带 `authStyle` / `extraHeaders` / `allowInvalidCert` / `proxyUrl` 一整套 ——
  这是**长期维护面**，不是一次性功能点。
- 「原始报文排障」确实有用，但那本来就是**请求引擎**已有的能力：用本工具直接发一条
  `POST /chat/completions` 就能看到状态码与错误体，不必为它单建一层。
- 剩下真正不可替代的那部分（让 AI 自己找接口、改参数、发请求）**外部 Agent 已经做得更好**，
  所以工具只需要提供干净的数据与通道 —— 这正是 MCP / CLI 在做的事。

真要恢复：完整设计在本次改动之前的 `design.md` 版本里（git 历史可查）。

---

## 9. 技术栈定稿

### 9.1 前端

| 层     | 选型                          | 版本                      | 理由                         |
| ----- | --------------------------- | ----------------------- | -------------------------- |
| 框架    | **React**                   | 19.x                    | 生态最大，你已熟悉                  |
| 语言    | **TypeScript**              | 5.7+                    | strict 模式                  |
| 构建    | **Vite**                    | 6.x                     | Tauri 官方推荐，HMR 极快          |
| 样式    | **Tailwind CSS**            | v4（`@tailwindcss/vite`） | v4 的 CSS-first 配置正好承载双主题变量 |
| 状态    | **zustand**                 | 5.x                     | 轻量，与你 DSH 项目一致             |
| 面板    | **react-resizable-panels**  | 3.x                     | 极轻，shadcn 同作者              |
| 代码编辑器 | **CodeMirror 6**            | 6.x                     | ~200KB vs Monaco 5MB+，够用且快 |
| 图表    | **uPlot**                   | 1.6.x                   | 45KB，万级数据点流畅；直方图自绘 SVG     |
| 图标    | **lucide-react**            | latest                  | 统一线性风格，零 emoji             |
| 虚拟列表  | **@tanstack/react-virtual** | 3.x                     | 历史记录 / 大响应                 |
| 命令面板  | 自研                          | —                       | 模糊匹配 + 动作注册表，约 200 行       |
| 校验    | **zod**                     | 3.x                     | IPC 边界类型校验                 |

**刻意不引入**：shadcn/ui（组件数太多、体积不可控）、Ant Design（重且风格不合）、Redux（过重）、TanStack Query（本场景 IPC 调用不需缓存层）。

**i18n 策略（本期只出中文）**：

- 不引入 `i18next` / `react-intl`（体积与心智负担不划算），自建约 60 行的 `I18nProvider`。
- 所有面向用户的文案**从第一天就写成 `t('request.send')` 形式**，而不是直接写字符串——这是"留口子"的全部含义。
- `src/lib/i18n/zh-CN.ts` 是唯一资源文件；`en-US.ts` 后续补，加语言只改一个 `availableLocales` 数组。
- 后端返回的错误信息用**错误码**（如 `E_TOKEN_REFRESH_FAILED`）而非中文句子，由前端 `t()` 映射——否则加语言要改 Rust。
- 例外：日志、崩溃堆栈、MCP tool 的 description 保持英文（面向开发者与 LLM，翻译反而降低匹配质量）。


### 9.2 后端（Rust）

| 层        | 选型                                                                                                | 理由                                                    |
| -------- | ------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| 运行时      | **Tauri 2.x**                                                                                     | 3–10MB 包体，能力权限系统                                      |
| 异步       | **tokio 1.x**（full）                                                                               | 压测并发基石                                                |
| HTTP     | **reqwest 0.12**（`rustls-tls` + `gzip` + `brotli` + `http2` + `cookies` + `multipart` + `stream`） | 不依赖系统 OpenSSL，跨平台零配置                                  |
| 存储       | **rusqlite 0.32**（`bundled`）+ **r2d2** + **r2d2_sqlite**                                          | `bundled` 内置 SQLite，无外部依赖；同步 API 用 `spawn_blocking` 包 |
| 统计       | **hdrhistogram**                                                                                  | P99 高精度低成本                                            |
| JSONPath | **serde_json_path**                                                                               | token 提取 / `{{resp.*}}`                               |
| MCP      | **rmcp**                                                                                          | MCP 官方 Rust SDK                                       |
| 时间       | **chrono** / **time**                                                                             | ISO8601 处理                                            |
| 密钥       | **keyring**                                                                                       | 系统密钥串（Win Credential Manager / macOS Keychain）        |
| 日志       | **tracing** + **tracing-subscriber**                                                              | 结构化日志 + 前端控制台面板                                       |
| 错误       | **thiserror**（库内）+ **anyhow**（边界）                                                                 |                                                       |

**为什么不用 sqlx**：`sqlx` 的编译期检查需配 offline 模式与 `cargo sqlx prepare`，构建复杂度高；本工具查询都是简单 CRUD，`rusqlite` 编译更快、依赖更少，更符合"轻量"。

**Tauri 插件**：`tauri-plugin-dialog`（文件选择）、`tauri-plugin-clipboard-manager`（复制响应）、`tauri-plugin-opener`（导出后用系统程序打开）、`tauri-plugin-window-state`（记住窗口大小）、`tauri-plugin-updater`（自动更新，可选）。

### 9.3 工程与质量

| 项           | 选型                                                                       |
| ----------- | ------------------------------------------------------------------------ |
| 包管理         | pnpm（前端）+ cargo（后端）                                                      |
| 前端测试        | **Vitest** + Testing Library                                             |
| Rust 测试     | 内置 `#[test]` + `tokio::test`；解析器用快照测试（`insta`）                           |
| E2E         | `tauri-driver` + WebdriverIO（Phase 后期）                                   |
| Lint/Format | ESLint 9 flat config + Prettier；`cargo fmt` + `cargo clippy -D warnings` |
| CI          | GitHub Actions 三平台构建（可选，本地为主）                                            |

---


## 10. 项目结构

```
E:\H_Req\reqly\
├─ docs/
│  ├─ design.md                 ← 本文档
│  ├─ plan.md                   ← Phase 3 产出：细粒度任务清单
│  └─ review.md / final_report.md
├─ package.json
├─ vite.config.ts
├─ tsconfig.json
├─ index.html
├─ src/                                   # 前端
│  ├─ main.tsx
│  ├─ App.tsx
│  ├─ styles/
│  │  ├─ theme.css                        # ★ 双主题 CSS 变量
│  │  ├─ tailwind.css                     # @import "tailwindcss"
│  │  └─ global.css
│  ├─ lib/
│  │  ├─ ipc.ts                           # invoke 封装（唯一 IPC 出口）
│  │  ├─ types.ts                         # 与 Rust 侧对齐的类型
│  │  ├─ theme.ts                         # 主题切换 + 持久化
│  │  ├─ format.ts                        # 字节/耗时/数字格式化
│  │  └─ i18n/                            # 本期只出 zh-CN，但全部文案走 t()
│  │     ├─ index.tsx                     # I18nProvider + useT()
│  │     └─ zh-CN.ts
│  ├─ store/
│  │  ├─ workspace.ts  collection.ts  tabs.ts
│  │  ├─ history.ts    loadtest.ts    ui.ts
│  ├─ features/
│  │  ├─ layout/       TopBar Sidebar SplitPane StatusBar CommandPalette
│  │  ├─ collection/   CollectionTree TreeNode ContextMenu MoveDialog
│  │  ├─ request/      RequestEditor MethodSelect UrlBar KeyValueTable
│  │  │                BodyEditor AuthPanel PlaceholderInput
│  │  ├─ response/     ResponseViewer PrettyView HeadersTable TimelineView
│  │  ├─ history/      HistoryList TrashBin
│  │  ├─ env/          EnvManager
│  │  ├─ token/        TokenSourceForm TokenStatusBadge
│  │  ├─ loadtest/     LoadTestPanel StatCard LatencyChart Histogram
│  │  │                SampleLog ReportExporter
│  │  ├─ import/       ImportDialog ImportPreview
│  │  └─ ui/           Button Input Select Tabs Dialog Toast Tooltip
│  └─ assets/
└─ src-tauri/
   ├─ Cargo.toml
   ├─ tauri.conf.json
   ├─ capabilities/default.json
   ├─ core/migrations/001..004_*.sql       # 001 初始 / 002 分类 auth / 003 body 按类型 / 004 系统分类
   ├─ icons/
   └─ src/
      ├─ main.rs
      ├─ lib.rs                            # Builder 装配 + 命令注册
      ├─ error.rs
      ├─ commands/                         # ★ IPC 契约层
      │  ├─ mod.rs request.rs collection.rs history.rs
      │  ├─ env.rs token.rs loadtest.rs import_.rs settings.rs
      ├─ http/
      │  ├─ mod.rs client.rs execute.rs
      ├─ template/
      │  ├─ mod.rs resolver.rs functions.rs
      ├─ storage/
      │  ├─ mod.rs pool.rs migrate.rs models.rs
      │  └─ repo/ folder.rs request.rs history.rs env.rs token.rs loadtest.rs
      ├─ importer/
      │  ├─ mod.rs curl.rs tokenizer.rs apipost.rs postman.rs har.rs
      ├─ loadtest/
      │  ├─ mod.rs engine.rs stats.rs report.rs
      ├─ mcp/
      │  ├─ mod.rs server.rs tools.rs transport.rs
      │  └─ config_gen.rs                  # 生成 JSON / YAML 两种客户端配置（§8.1）
      │  ├─ mod.rs provider.rs client.rs test_connection.rs
      └─ bin/
         └─ reqly-mcp.rs                   # 独立 MCP 可执行文件
```

---

## 11. 分阶段交付计划

> Phase 4（TDD）与 Phase 5（Subagent）会为每个 Phase 生成 2–5 分钟粒度的任务清单（`plan.md`）。

| Phase  | 名称         | 交付物                                          | 验收标准（可验证）                                                  |
| ------ | ---------- | -------------------------------------------- | ---------------------------------------------------------- |
| **P0** | 脚手架        | Tauri 2 + React 19 + TS + Tailwind v4 跑通空白窗口 | `pnpm tauri dev` 打开窗口；三平台构建通过                              |
| **P1** | 数据与主题      | SQLite 迁移 + 仓储 + 双主题系统                       | 迁移可重复执行；`Ctrl+Shift+T` 切换主题且重启后保持                          |
| **P2** | 核心请求闭环     | HTTP 引擎 + 请求编辑器 + 响应查看器                      | 能发 GET/POST 到 `httpbin.org`，正确显示状态/耗时/JSON；`Ctrl+Enter` 可用 |
| **P3** | 集合与历史      | 文件夹树 CRUD + 拖拽移动 + 历史 + 回收站                  | 建 3 层文件夹；拖拽移动后层级正确；删除进回收站可还原                               |
| **P4** | 变量与 Token  | 环境变量 + 占位符 + **Token 自动刷新**                  | `{{env.x}}` 生效；token 过期自动刷新；并发 5 请求只打 1 次刷新                |
| **P5** | 导入器        | curl / apipost / postman / HAR（OpenAPI 已裁剪） | 三种浏览器 Copy as cURL 均正确解析；apipost 导出文件 100% 条目导入            |
| **P6** | 压测         | 并发引擎 + 实时面板 + HTML/CSV 报表                    | 500 请求 / 50 并发跑完；P95 与 `ab` 结果偏差 <10%；报表可导出打开              |
| **P7** | MCP Server | stdio + HTTP 双通道 + 13 tools                  | Claude Code 连上后能完成 §8.4 的完整工作流                             |
| **P8** | 打磨         | 命令面板、快捷键、导出、自动更新、打包                          | 生成 <10MB 安装包；`Ctrl+K` 可跳转任意请求                              |

### 11.1 追加阶段（第二轮确认后新增）

| Phase   | 名称             | 交付物                                            | 验收标准（可验证）                                                        |
| ------- | -------------- | ---------------------------------------------- | ---------------------------------------------------------------- |
| ~~**P9** / **P10**~~ | ~~AI Provider + 内置助手~~ | **已裁剪（2026-09-15）** —— AI 能力只从外部 Agent 进入（MCP / CLI） | 见 §0.2 |

**关键路径**：P1 → P2 → P4 → P6 → P7 → P8。P3 / P5 可与 P6 并行；**P9 / P10 已裁剪（见 §0.2）**。

**P7 验收补充**：WorkBuddy 与 Hermes **各连一次**，13 个 tool 全部被发现，且 `readOnlyHint` / `destructiveHint` 注解被客户端正确识别（Hermes 侧表现为只读 tool 免审批、`delete_request` 弹确认）。

---

## 12. 风险与对策

| 风险                       | 等级 | 对策                                                        |
| ------------------------ | -- | --------------------------------------------------------- |
| apipost 导出格式版本漂移         | 中  | 优先走 Postman v2.1 兼容路径；原生格式解析器做成独立模块 + 快照测试，格式变了只改一处       |
| curl 解析边界情况多（引号/转义/多行）   | 中  | 独立 `tokenizer.rs` + 20+ 快照测试用例覆盖 bash/cmd/powershell 三种形态 |
| rusqlite 同步 API 阻塞 tokio | 中  | 全部 DB 操作 `spawn_blocking` 包裹；连接池 `r2d2` 大小 = CPU×2        |
| 压测把本机打满导致 UI 卡死          | 中  | 压测在独立 tokio 运行时线程；样本批量落盘；UI 只收 event 不参与计算                |
| MCP 进程与桌面 App 同时写 SQLite | 低  | WAL 模式 + `busy_timeout=5000`；写操作加短事务；桌面端文件监听刷新            |
| Tauri 各平台 WebView 渲染差异   | 低  | 只用 WebView2/WKWebView 共同支持的 CSS；锁 `--font-mono` 回退链       |
| 拖拽移动成环（拖到自己的子孙）          | 低  | 拖拽起始即计算子孙集合，非法目标不显示投放区                                    |
| 大响应体（>50MB）卡顿            | 低  | Rust 侧流式写临时文件；前端 >2MB 切纯文本 + 虚拟滚动                         |

---

## 13. 开放问题与决议

| # | 问题        | 决议                                                                    | 状态     |
| - | --------- | --------------------------------------------------------------------- | ------ |
| 1 | 项目名       | 暂定 `Reqly`，未提出异议即沿用                                                   | 待一句话确认 |
| 2 | 双语        | **本期只做中文**，但全文案走 `t()` + `zh-CN.ts`，后端错误用错误码（§9.1）                     | ✅ 已定   |
| 3 | 工作区数量     | 数据层支持多工作区，UI 默认只显示一个，暂不做切换器                                          | 按默认推进  |
| 4 | ~~AI Provider~~ | **已裁剪（2026-09-15）**：应用内不做 AI，能力只从外部 Agent（MCP / CLI）进来 —— 原「必须支持任意 OpenAI 兼容端点」的要求作废 | 🔄 改为不做 |
| 5 | MCP 客户端   | **WorkBuddy / Hermes / Claude Code 全支持**，JSON + YAML 双格式配置生成器（§8.1）     | ✅ 已定   |

**残余决策（不阻塞开工，采用默认值即可，随时可推翻）**：

- **项目名 `Reqly`**：改名成本 = 重命名目录 + `package.json` / `Cargo.toml` / `tauri.conf.json` 三处 `name` 字段，随时可改。
- **工作区切换器**：UI 上先不出现，等真有第二个工作区再加。

---

## 附：主要文档演进路径

```
design.md（本文，Brainstorming 产出）
   └─ 你评审通过后 → plan.md（细粒度任务清单，每任务含文件路径+代码+验证步骤）
         └─ TDD 循环实现 → review.md（分级问题报告）
               └─ final_report.md（交付总结）
```
