-- ============================================================
-- Reqly 001_init.sql —— 初始 schema
-- ------------------------------------------------------------
-- 对应 design.md §3.2。相对设计稿有 3 处必要补充，均在此标注：
--   [补1] `setting` 键值表 —— design.md §3.2 漏了，但 P1-6 的
--         get_setting/set_setting（theme 等）需要它
--   [补2] token_source.fetch_request_id 补 ON DELETE CASCADE
--         —— 设计稿只写了 REFERENCES，回收站「彻底删除」该 request
--         时会因外键约束直接失败
--   [补3] token_source.refresh_request_id 补 ON DELETE SET NULL
--
-- 全库约定：
--   * 主键 INTEGER PRIMARY KEY（= rowid，无需 AUTOINCREMENT）
--   * 时间戳一律 TEXT，ISO8601 带时区（如 2026-09-11T06:00:00.000Z），
--     由 Rust 侧生成而非 SQLite 的 datetime()，保证时区语义一致
--   * JSON 列存 TEXT，字段名 camelCase，与前端 types.ts 对齐
--   * 布尔用 INTEGER 0/1
--   * 软删除统一 `deleted_at`：NULL = 存活，非 NULL = 在回收站
--   * PRAGMA 不写在这里（journal_mode=WAL 不能在事务内执行），
--     见 storage/pool.rs
-- ============================================================

-- ---------------- 工作区 ----------------
CREATE TABLE workspace (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL,
  sort_order  INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL,
  deleted_at  TEXT
);

-- ---------------- 文件夹 / 分类（自引用树） ----------------
CREATE TABLE folder (
  id           INTEGER PRIMARY KEY,
  workspace_id INTEGER NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
  parent_id    INTEGER REFERENCES folder(id) ON DELETE CASCADE,  -- NULL = 根
  name         TEXT NOT NULL,
  sort_order   INTEGER NOT NULL DEFAULT 0,
  -- Token 源分组标记（见 design.md §4.3）
  is_token_source INTEGER NOT NULL DEFAULT 0,
  created_at   TEXT NOT NULL,
  updated_at   TEXT NOT NULL,
  deleted_at   TEXT
);
CREATE INDEX idx_folder_parent ON folder(workspace_id, parent_id, sort_order);

-- ---------------- 接口请求 ----------------
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

-- ---------------- 环境 ----------------
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
CREATE INDEX idx_env_var_env ON env_var(environment_id, sort_order);

-- ---------------- Token 提供者（design.md §4.3） ----------------
CREATE TABLE token_source (
  id                INTEGER PRIMARY KEY,
  workspace_id      INTEGER NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
  folder_id         INTEGER NOT NULL REFERENCES folder(id) ON DELETE CASCADE,
  name              TEXT NOT NULL,               -- 如 'iPSA-OAuth'
  -- [补2] 取 token 用的那个请求；请求被彻底删除时该 Token 源也无意义
  fetch_request_id  INTEGER NOT NULL REFERENCES request(id) ON DELETE CASCADE,
  access_path       TEXT NOT NULL DEFAULT '$.access_token',
  refresh_path      TEXT NOT NULL DEFAULT '$.refresh_token',
  expires_in_path   TEXT NOT NULL DEFAULT '$.expires_in',     -- 取秒数或绝对时间
  expires_in_unit   TEXT NOT NULL DEFAULT 'seconds',          -- seconds|millis|absolute
  -- [补3] 刷新用哪个请求；可空（NULL = 复用 fetch_request_id）
  refresh_request_id INTEGER REFERENCES request(id) ON DELETE SET NULL,
  auto_refresh      INTEGER NOT NULL DEFAULT 1,
  skew_seconds      INTEGER NOT NULL DEFAULT 30,              -- 提前 30s 视为过期
  created_at        TEXT NOT NULL,
  updated_at        TEXT NOT NULL
);
-- 一个文件夹下同名 Token 源只允许一个
CREATE UNIQUE INDEX idx_token_source_folder_name ON token_source(folder_id, name);
-- fetch 请求被彻底删除时要有路径能反查
CREATE INDEX idx_token_source_fetch ON token_source(fetch_request_id);

-- ---------------- Token 运行时缓存 ----------------
CREATE TABLE token_cache (
  id               INTEGER PRIMARY KEY,
  token_source_id  INTEGER NOT NULL REFERENCES token_source(id) ON DELETE CASCADE,
  access_token     TEXT NOT NULL,
  refresh_token    TEXT,
  expires_at       TEXT,           -- ISO8601，NULL = 永不过期
  raw_response     TEXT,           -- 原始响应快照，便于调试
  updated_at       TEXT NOT NULL
);
-- 一个 Token 源在缓存表里只留一行（刷新即 upsert）
CREATE UNIQUE INDEX idx_token_cache_source ON token_cache(token_source_id);

-- ---------------- 历史记录（软删除 = 回收站） ----------------
CREATE TABLE history (
  -- workspace_id 刻意不加外键：历史要能在工作区被删后仍可读
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

-- ---------------- 压测 ----------------
CREATE TABLE loadtest_run (
  id           INTEGER PRIMARY KEY,
  request_id   INTEGER REFERENCES request(id) ON DELETE SET NULL,
  request_name TEXT NOT NULL,
  config       TEXT NOT NULL,     -- JSON: {concurrency,total,timeoutMs,...}
  summary      TEXT NOT NULL,     -- JSON: 统计汇总
  started_at   TEXT NOT NULL,
  finished_at  TEXT
);
CREATE INDEX idx_loadtest_run_time ON loadtest_run(started_at DESC);

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

-- ---------------- 应用设置（键值）----------------
-- [补1] design.md §3.2 未定义；theme / locale / mcp 开关等都落这里。
-- 值统一存 TEXT，布尔与数字由读取方解析（避免 schema 里堆一堆类型字段）。
CREATE TABLE setting (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

-- 注意：`schema_migration` 表**刻意不在这里建**。
-- 它是迁移机制自己的元数据表，由 storage/migrate.rs 的 ensure_version_table()
-- 在本脚本执行**之前**就建好。写在这里会撞「table already exists」。
