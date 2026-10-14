-- ============================================================
-- Reqly 008_request_scoped_token_source.sql —— Token 刷新下沉到「每个接口一份」
-- ------------------------------------------------------------
-- 把 Token 源的归属从 `folder_id`（挂在系统分类「token刷新」上，全工作区共用
-- 一份）换成 `request_id`（每个请求自己一份）。
--
-- 为什么必须换归属：实际用法是**每个接口单独配**自己的取 token 请求 ——
-- 同一个工作区里可能同时躺着多条拿 token 请求（生产 / 测试 / 各类服务，
-- 各自的 client_id 与 scope 都不同）。挂在分类上时，一个工作区只能有一份配置，
-- 那些差异根本表达不出来。
--
-- ⚠️ **整段不可重放**：它丢掉了所有 `request_id IS NULL` 的旧行（见下）。
-- 幂等由 `schema_migration` 版本表把关（migrate.rs 每个迁移一个事务）。
--
-- ⚠️ 关于播种：008 之后的那一步铺源，其白名单**由调用方传入**且默认为空 ——
-- 本仓库不预设任何域名清单（见 migrate.rs 的 run_with_seed_hosts）。
-- 全新安装的库里没有任何请求，铺源天然是空操作。
-- ============================================================


-- ============================================================
-- ① 为什么是重建表，而不是 ALTER
-- ------------------------------------------------------------
-- `folder_id` 在 001 里是 `NOT NULL REFERENCES folder(id) ON DELETE CASCADE`，
-- 并且是 `idx_token_source_folder_name(folder_id, name)` 的组成部分。
-- SQLite 的 `ALTER TABLE … DROP COLUMN` 对这两种情况都直接拒绝
-- （不能删外键子句里的列，也不能删索引里的列）。
--
-- 于是按 SQLite 官方的重建流程走一遍。但**顺序不能照抄文档**：
--
-- 🔴 `token_cache.token_source_id` 上有 `REFERENCES token_source(id)
--    ON DELETE CASCADE`。`foreign_keys=ON` 时 `DROP TABLE token_source`
--    会连带把 token_cache 整个清空 —— 静默、无报错、事后只能靠"token 怎么
--    没了"来发现。
--
--    但**光把子表排在前面删掉并不够**：`token_cache_new` 自己也写着
--    `REFERENCES token_source(id)`，而那句 FK 在 `DROP TABLE token_source`
--    那一刻解析到的是**旧**表（新表这时还叫 `token_source_new`）。级联一样
--    会打到新子表上。所以这里不靠顺序躲，而是**先把缓存显式删干净**
--    （见下面那句 `DELETE FROM token_cache`）—— 把「意外级联」变成
--    「有意为之」，代价是那里面本来就活不下来的东西。
--
-- 顺序：建两张新表 → 删旧缓存 → 删子表 → 删父表 → 两张依次改名 → 重建索引。
-- ============================================================

CREATE TABLE token_source_new (
  id                 INTEGER PRIMARY KEY,
  workspace_id       INTEGER NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
  -- 这条 Token 配置属于哪个接口。NOT NULL —— 「每个接口一份」是硬约束，
  -- 不是"尽量"：可空的话就会重新长出"无归属的全局源"这个概念。
  request_id         INTEGER NOT NULL REFERENCES request(id) ON DELETE CASCADE,
  name               TEXT NOT NULL,
  -- 取 token 用的那个请求；请求被彻底删除时该 Token 源也无意义
  fetch_request_id   INTEGER NOT NULL REFERENCES request(id) ON DELETE CASCADE,
  access_path        TEXT NOT NULL DEFAULT '$.access_token',
  refresh_path       TEXT NOT NULL DEFAULT '$.refresh_token',
  expires_in_path    TEXT NOT NULL DEFAULT '$.expires_in',
  expires_in_unit    TEXT NOT NULL DEFAULT 'seconds',
  -- 可空（NULL = 复用 fetch_request_id）
  refresh_request_id INTEGER REFERENCES request(id) ON DELETE SET NULL,
  auto_refresh       INTEGER NOT NULL DEFAULT 1,
  skew_seconds       INTEGER NOT NULL DEFAULT 30,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL
);

CREATE TABLE token_cache_new (
  id               INTEGER PRIMARY KEY,
  token_source_id  INTEGER NOT NULL REFERENCES token_source(id) ON DELETE CASCADE,
  access_token     TEXT NOT NULL,
  refresh_token    TEXT,
  expires_at       TEXT,
  raw_response     TEXT,
  updated_at       TEXT NOT NULL
);

-- 旧行**一条都不搬**：它们的归属是「分类」，在新模型里没有落点。
-- 硬塞一个 request_id 出去只会凭空给某个接口配上一份它没要过的 Token 源。
-- 迁移前那份旧行的字段值与新模型默认值逐字段相同，
-- 而它服务的那些接口会由播种重新配 —— 丢失的信息量为零。
--
-- 🔴 缓存也**一条都不留**，而且是显式删的，不是「顺手不搬」。
--    为什么不留：新表里 source id 从 1 重新长起（播种铺出的行就是 1~N）。
--    旧缓存行的 `token_source_id` 跟着搬过去，就会指向**另一个接口**的新源 ——
--    「凭据串了，而且不报错」。一条几秒就能重取的 token，不值这个风险。
--
--    （这里原先写的是「把旧缓存搬进新表，让它在重建中活下来」。那件事做不到：
--    下面 `DROP TABLE token_source` 删的是**旧**父表，而 `token_cache_new` 的
--    FK 此刻正指向它，级联照样把新表清空 —— 静默。最小复现：造 1 行旧缓存，
--    搬运后新表 1 行，DROP 旧父表后新表 0 行。一个「必须活下来」的断言，
--    测的其实是「绝不可能活下来」。）
DELETE FROM token_cache;

DROP TABLE token_cache;
DROP TABLE token_source;

ALTER TABLE token_source_new RENAME TO token_source;
ALTER TABLE token_cache_new  RENAME TO token_cache;

-- 一个接口至多一份 Token 配置。取代原来的 UNIQUE(folder_id, name)：
-- 在请求级模型里，同名的两个接口是两件互不相干的事，不该互相挤掉。
CREATE UNIQUE INDEX idx_token_source_request ON token_source(request_id);
-- fetch 请求被彻底删除时要有路径能反查
CREATE INDEX idx_token_source_fetch ON token_source(fetch_request_id);
-- 一个 Token 源在缓存表里只留一行（刷新即 upsert）
CREATE UNIQUE INDEX idx_token_cache_source ON token_cache(token_source_id);
