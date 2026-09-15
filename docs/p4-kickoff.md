# Reqly P4 开工提示词（新对话直接整段复制）

> 这份文件是给「新开一个对话继续做 P4」用的交接说明。
> 用法：把下面 `=== 复制开始 ===` 到 `=== 复制结束 ===` 之间的全部内容粘贴进新对话的第一条消息。

---

=== 复制开始 ===

继续开发 Reqly 项目，这次做 **P4（变量与 Token）**。

## 你要先读的东西（按顺序，别跳过）

1. `E:\H_Req\reqly\docs\plan.md` 的 **§4 P4 任务清单**（第 195~220 行左右）和 **§11 P3 实测记录**（尤其 §11.3 的坑清单，全是会静默出错的那种）
2. `E:\H_Req\reqly\docs\design.md` 的 **§4 请求引擎**（§4.2 占位符命名空间总表、§4.3 Token 自动刷新机制 ★、§4.4 Auth 类型）
3. `E:\H_Req\.workbuddy\memory\MEMORY.md` —— 项目长期约定与所有踩过的坑。**这里面的规约是硬性的**，动手前通读一遍
4. 项目根目录是 `E:\H_Req\reqly`（不是 `E:\H_Req`，后者只是工作区根）

## 项目是什么 & 现状

**Reqly**：轻量级、AI 原生、数据本地保存的类 Postman 接口请求工具。
技术栈（已定，勿改）：**Tauri 2 + React 19 + Vite 8 + TypeScript 7 + Tailwind v4 + zustand + SQLite(rusqlite)**，包管理 **pnpm**，全部界面中文。

**进度**：P0 脚手架 / P1 数据层与主题 / P2 核心请求闭环 / P3 集合、历史与回收站 —— **全部完成并验收通过**。
当前基线：`cargo test` = 115 lib + 5 startup，`cargo clippy --all-targets -- -D warnings` 零告警，`pnpm exec tsc --noEmit` 零错误，`pnpm build` 通过。

## P4 要做的事（这是项目最核心的差异化功能）

用户的原始需求原话：**「支持接口请求多种模式……支持刷新 token（来自一个特殊分组的接口），支持通过占位符的方式匹配 token 进行请求」**。

### 已经有的地基（别重复造）

| 已有 | 位置 | 状态 |
|---|---|---|
| 建表 | `src-tauri/src/storage/migrate.rs` | `environment` / `env_var` / `token_source` / `token_cache` 四张表**已建好**，含 `idx_token_cache_source` 与级联规则 |
| 环境仓储 | `src-tauri/src/storage/repo/env.rs`（413 行） | **已实现**（`list_by_workspace` / `get_active` / `create` / `rename` / `set_active` / `soft_delete` / `restore` / `purge` / `list_vars` / `replace_vars`），带单测 |
| Token 仓储 | `src-tauri/src/storage/repo/token.rs`（356 行） | **已实现**（`TokenSource` CRUD + `get_cache` / `upsert_cache` / `clear_cache`），带单测 |
| 数据模型 | `src-tauri/src/storage/models.rs` + `src/lib/types.ts` | `Environment` / `EnvVar` / `TokenSource` / `TokenCache` 两端都定义好了 |
| zod schema | `src/lib/schemas.ts` | 同上四个实体的 schema 也有了 |

### 缺的（P4 的主体工作）

**Rust 侧**
1. `src-tauri/src/template/`（**新模块**）：`mod.rs` + `resolver.rs` + `functions.rs`。
   **TDD 先行**——先把解析器的测试写出来再实现。
   解析顺序固定为：**动态函数 → env → token → resp**。
   未命中的占位符**原样保留**并收集进「警告列表」返回（**绝不静默变空串**，这是验收硬项）。
   动态函数：`$uuid` `$timestamp` `$timestampMs` `$randomInt` `$datetime '<fmt>'`。
2. `src-tauri/src/http/token.rs`（**新文件**）：**单飞锁（single-flight）**。
   `tokio::sync::Mutex<HashMap<TokenSourceId, Arc<Shared<...>>>>` 或 `OnceCell` + `Notify`。
   过期判定要含 `skew_seconds`（提前刷新秒数）。
   缓存有效 → 直接注入、零额外请求；过期/不存在 → 打一次 `fetch_request_id`、按 JSONPath 提取、写 `token_cache`、再注入；用户点「立即刷新」→ 强制忽略缓存。
3. `src-tauri/src/commands/env.rs` + `src-tauri/src/commands/token.rs`（**新文件**）：命令层。
   命令层规矩：**只做「校验入参 → 调仓储/引擎 → 映射错误」**，不写业务逻辑、不产生面向用户的文案（契约见 `src-tauri/src/error.rs`）。
   记得在 `commands/mod.rs` 加 `pub mod`，并在 `lib.rs` 的 `generate_handler!` 里注册。
4. 把模板引擎**接进** `src-tauri/src/http/execute.rs` 的流水线（`design.md` §4.1 的第 2 步）。

**前端**
5. `src/features/env/EnvManager.tsx`：环境切换 + 变量增删改 + `isSecret` 打码/显示（现在 `src/features/env/` 是空目录，侧栏「环境」板块还是 `EmptyState` 占位）
6. `src/features/token/TokenSourceForm.tsx`：`design.md` §4.3 那套配置界面（Token 源分组下拉 → 该分组下的请求 → 三个 JSONPath + 过期单位 → 刷新接口 → 提前秒数 → 自动刷新开关 → 立即刷新并预览）
7. `src/features/token/TokenStatusBadge.tsx`：剩余有效期倒计时 + 手动刷新按钮
8. `src/features/request/PlaceholderInput.tsx`：占位符彩色高亮 + **未解析的黄色警告** + hover 显示解析来源。接到 `UrlBar` / `KeyValueTable` / `BodyEditor` 上
9. 接上 `Auth` 标签页的真实内容（`src/features/request/RequestPane.tsx` 里 auth 现在还是 `EmptyState` 占位），按 `design.md` §4.4 做 None / Bearer / Basic / API Key / 继承父级
10. 状态栏的 Token 计数（`src/features/layout/StatusBar.tsx`，i18n 里 `status.tokens` / `status.tokenValid` / `status.tokenExpired` 已经写好了）

### ⚠️ 一个待你决策的点

`environment.is_active`（列）与 `settings.rs` 里的 `ACTIVE_ENV_ID: "activeEnvironmentId"`（setting 键）**两处都在表达「当前环境」**。
动手前先定一个为唯一真源，把另一处改成派生/删除，并在代码注释里写清楚为什么。别让两处都能写。

## 验收标准（plan.md §4，逐条都要能演示）

1. `{{env.baseUrl}}` 正确替换
2. 用 mock server 做 token 源，过期后**自动刷新一次**并成功注入
3. **并发打 5 个请求且 token 过期时，mock server 侧只收到 1 次刷新请求** ← 这是硬指标，必须有并发单测钉住
4. 未定义的 `{{env.nope}}` 在 UI 上**黄色高亮**，而不是静默变空
5. 刷新失败时中止请求，并弹出指向配置的错误卡片

> mock server 不要引 wiremock —— 项目既有做法是「进程内起一个极简 TCP server」（`src-tauri/src/http/` 里的测试就是这么干的，见 `MEMORY.md` 的「Rust / HTTP 的坑」）。真实联网的端到端验收放 `src-tauri/tests/*_live.rs`，用 `#[ignore]` 标上「依赖公网」。

## 必须遵守的工程约定（违反会静默出错，逐条血泪）

**样式**
- `src/styles/theme.css` 是**唯一**颜色来源，业务组件里**禁止硬编码色值**，一律 `bg-canvas / text-ink / border-line / text-brand / text-m-get`
- **无分层的 CSS 永远优先于任何 `@layer` 里的样式，与选择器特异性无关**。所以 `global.css` 必须整体包在 `@layer base {}` 里
- `@theme inline` 的左值属于 Tailwind 主题命名空间，右值引用的原始令牌**绝不能同名**（自引用 → guaranteed-invalid → 静默失效且构建不报错）

**i18n**
- 组件里**禁止写中文字面量**，全部 `useI18n().t("域.语义")`，文案加进 `src/lib/i18n/zh-CN.ts`
- 契约：**后端只返回错误码，不返回文案**

**TypeScript**（`strict` + `noUnusedLocals` + `noUnusedParameters` + `noUncheckedIndexedAccess` + `verbatimModuleSyntax`，**别关**）
- 类型导入必须 `import type`；没用的 import/参数会直接编译失败

**Rust**
- 异步→阻塞的**唯一**边界是 `storage::with_conn()`；仓储函数全同步收 `&Connection`，这样 SQL 单测不需要 tokio 运行时
- `changes()` **不含**外键级联删掉的行，要精确计数就显式取子集大小
- **Tauri 异步命令必须返回 `Result`**（`async fn -> bool` 会报 `E0277`）
- 单测里**永远不要**裸用 `reqwest::Client::new()`（会读代理、会默认跟 10 跳重定向），一律走 `client_for_spec(&spec)`

**工具与沙箱**
- `cargo` 不在 PATH 里：每条命令前加 `export PATH="$HOME/.cargo/bin:$PATH"`
- 跑 cargo **必须脱离沙箱**（`dangerouslyDisableSandbox: true`）——tauri 的 build script 要写 `src-tauri/target/`
- 每次 bash 调用是**独立网络命名空间**：「起本地服务 + 访问它」必须写在**同一条**命令里
- 往 Windows 程序传路径必须用 `E:/...` / `C:/...` 形式；`mktemp -d` 的 `/tmp/x` 会静默失败
- **同一轮消息里并行编辑同一个文件会互相覆盖**（都报 success，后写的基于旧内容）→ 同一文件的多次 Edit 必须**串行**发
- 临时物（截图、脚本、浏览器 profile）一律放 `%LOCALAPPDATA%\Temp\reqly-*`，**不要**留在项目根；大目录自己删不掉（safe-delete 护栏）就列清单交给用户 + 给 PowerShell 版清理命令

## 收尾动作（每个阶段都要做，别省）

1. `cd src-tauri && export PATH="$HOME/.cargo/bin:$PATH" && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` 全绿
2. `pnpm exec tsc --noEmit && pnpm build` 通过
3. **实际运行验证**，不要只声明「已完成」——P4 的核心指标（单飞锁只打一次刷新）必须有能跑的单测
4. 无头 UI 复验并归档截图到 `docs/screenshots/p4-*.png`。手法见 `MEMORY.md` 的「UI 验证手法」，要点：
   - 无头 Edge 截图（一张图一个 `--user-data-dir`，轮询等 PNG 落盘，脚本别用 `set -e` 包整段）
   - 没有后端时要在 `src/lib/ipc.ts` 的 `call()` 里**临时**挂一层假后端（`?__dev` 开关），**别按 store 注入**
   - 切主题用同源探针页写 `localStorage.reqly.theme`；截非默认板块用 `?__section=`；截弹窗用 `?__dialog=` + `useState` 惰性初值
   - **回滚要逐文件 `md5sum` 核对**，并 `grep` 确认无残留
   - **别靠肉眼比色，也别靠肉眼判断「哪一行被选中」**——缩放图里 `#e4ecfd` 与 `#ffffff` 几乎一样。用区域采样脚本 `%LOCALAPPDATA%\Temp\reqly_region_px.py`
5. 在 `docs/plan.md` 追加 **§12 P4 实测记录**（格式照抄 §11：完成范围 / 验收命令与结果 / 坑 / 逐条验收对照 / 截图与像素核对 / 下一步）
6. 把本轮成果与坑追加到 `E:\H_Req\.workbuddy\memory\2026-MM-DD.md`；如果是**长期约定**级别的（新依赖的坑、新铁律）则更新 `MEMORY.md`

## 沟通与交付偏好

- **中文**回答；**结论先行**，再列证据链与验证步骤
- 改动之后**明确列出改了哪些文件、各自改了什么**，避免和原有未提交改动混淆
- 接受改动前**显式确认语义等价性**；提出优化方案后要说明**代价与权衡**（比如索引设计的影响）
- 诊断与执行分开：先给结论和证据，再动手
- 删除操作要**明确说明将删除哪些文件**；删除前先备份
- 进度汇报用**紧凑表格**（步骤 / 状态 / 缺项）
- 动手类任务（下载、安装、构建、排查）直接执行，别只给方案；遇阻自己换路子推进，别停下来问「要不要我继续」

## 工作方式

用 `TaskCreate` 把 P4 拆成任务清单（P4-1 ~ P4-10 天然就是任务），边做边更新状态，别批量报完成。
**TDD 先行**：`template/resolver.rs` 与单飞锁这两块先写测试。
卡住或需要我拍板时，用 `AskUserQuestion` 列出选项和各自代价，别自己猜。

代码里**不要留 TODO 也不要想当然**——项目既有代码的注释密度很高，每处「为什么这么做」都写清楚了，请保持同样的标准：注释解释**为什么**，不解释**是什么**。

现在开始：先读上面指定的四份文档，然后给出 P4 的任务拆解与关键技术决策（占位符解析器接口设计、单飞锁的具体数据结构、活跃环境单一真源的选择），我确认后再动手写代码。

=== 复制结束 ===
