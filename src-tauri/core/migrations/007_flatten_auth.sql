-- ============================================================
-- Reqly 007_flatten_auth.sql —— 授权回归请求头
-- ------------------------------------------------------------
-- 「授权设计 v2」§5.1。取消 `request.auth` 与 `folder.auth` 两个列，
-- 把「这条请求带什么凭据」收敛成 `request.headers` 里的一行。
--
-- 为什么敢删掉运行时继承（旧 `inherited_auth()`）：实测本库 25 个分类
-- **全是根分类、零子分类**，继承链深度恒为 1；而它的代价是「4 条请求
-- 因为自己写了显式授权，反而切断了继承、一条凭据都不带」
-- （id=12/22/39 是空 Bearer，id=14 是全空 API Key）。
--
-- ⚠️ 与设计稿 §5.1 有一处**刻意不同**：那里把「展开 request.auth」排在
-- 「展开 folder.auth」前面，于是同一张展开对照表要写两遍。这里改成
-- **先物化、再展开** —— ① 先把分类链上的继承结果写回 `request.auth`
-- （只写「自己 auth 是 inherit」的请求），② 再统一展开一次。
-- 末态逐条相同，而去重规则只存在一份。
--
-- 三步：⓿ 归一非法 JSON → ① 物化文件夹继承 → ② 展开 ③ 删列。
-- ⚠️ ❸ 让**整段 SQL 不可重放**，这与 001~006 不同；幂等由 `schema_migration`
-- 版本表把关（migrate.rs 每个迁移一个事务）。❶❷ 本身可重复。
--
-- 库里的真实形状（2026-09-17 只读实测 + 真库副本上跑通）：
--   request.auth  256 条 `{"type":"inherit"}` + 1 条 `{}` + 1 条 `{"type":"none"}`
--                 + 8 条 bearer + 1 条 apiKey
--   folder.auth   只有 id=3「TS相关接口」非空：`{"type":"bearer","token":"{{token.access}}"}`
--   末态          36 条请求各多出恰好一行（28 走继承 + 8 展开），
--                 其余 231 条逐字节未变，去重撞车 0 条
-- ============================================================


-- ============================================================
-- ⓿ 先把四个 JSON 列里的非法值归一，后面才敢放心用 json_extract
-- ------------------------------------------------------------
-- 实测（SQLite 3.50）：`json_extract('不是 JSON', '$.type')` **抛错**
-- （`malformed JSON`），而不是返回 NULL。更要命的是
-- `json_valid(x) AND json_extract(x, …)` **并不短路**，照样抛。
--
-- 与其在每一处 json_extract 外面套一层 CASE，不如在这里一次归一。
-- 归一不丢信息：
--   * `auth` 非法 ⇒ 旧代码 `AuthConfig::from_json` 早就把它退化成 `Inherit`
--     （见 models.rs 的类型级注释），而 `{}` 的语义正是 `Inherit` —— 等价。
--   * `headers` / `params` 非法 ⇒ 应用侧走 serde，本来就读不出这条请求，
--     归一成空数组不丢任何**能用的**数据。
--   （本库 267 行四列全部合法，这一段是给别人的库留的护栏。）
--
-- `json_valid()` 对 NULL 返回 NULL，所以判据写 `IS NOT 1` 而不是 `= 0`。
-- 两列都是 NOT NULL，这一段实际上只可能命中非法字符串。
-- ============================================================

UPDATE request SET auth    = '{}' WHERE json_valid(auth)    IS NOT 1;
UPDATE folder  SET auth    = '{}' WHERE json_valid(auth)    IS NOT 1;
UPDATE request SET headers = '[]' WHERE json_valid(headers) IS NOT 1;
UPDATE request SET params  = '[]' WHERE json_valid(params)  IS NOT 1;


-- ============================================================
-- ① 把分类链上的继承结果物化回 request.auth
-- ------------------------------------------------------------
-- 与旧 `inherited_auth()` 的语义逐字对齐：只有**自己 auth 是 inherit**
-- 的请求才沿链向上找，取**最近**的一个「非 inherit」分类配置；
-- 显式配置（包括 `{"type":"none"}`）**中止**继承。
--
-- 判据只认 `type` 字段是 NULL 或 'inherit' 两种。
-- 这里有一个已知的、刻意接受的偏差：serde 在**载荷缺字段**时
-- （如 `{"type":"bearer"}` 少了 `token`）整条解析失败、退化成 `Inherit`，
-- 而本判据把它当显式 bearer、不去继承。本库不存在这种行，且没有任何
-- 代码路径会写出它们（`AuthConfig` 的序列化永远带全字段）。
--
-- `UNION`（不是 `UNION ALL`）是刻意的：万一分类树里有环，重复行会被去掉，
-- 递归自己就停了。（同 rust.md 里那条约定。）
--
-- 每个请求**至多**产出一行非 NULL：递归项一旦拿到非 NULL 的 inherited，
-- `WHERE c.inherited IS NULL` 就把这条分支掐掉，所以那行必然是最近的那个。
-- ============================================================

WITH RECURSIVE inherit_chain(req_id, folder_id, inherited) AS (
    SELECT r.id, r.folder_id, NULL
      FROM request AS r
     WHERE json_extract(r.auth, '$.type') IS NULL
        OR json_extract(r.auth, '$.type') = 'inherit'

    UNION

    SELECT c.req_id,
           f.parent_id,
           CASE WHEN json_extract(f.auth, '$.type') IS NOT NULL
                 AND json_extract(f.auth, '$.type') <> 'inherit'
                THEN f.auth
                ELSE NULL END
      FROM inherit_chain AS c
      JOIN folder AS f ON f.id = c.folder_id
     WHERE c.inherited IS NULL
)
UPDATE request
   SET auth = (SELECT ic.inherited
                 FROM inherit_chain AS ic
                WHERE ic.req_id = request.id
                  AND ic.inherited IS NOT NULL)
 WHERE EXISTS (SELECT 1
                 FROM inherit_chain AS ic
                WHERE ic.req_id = request.id
                  AND ic.inherited IS NOT NULL);


-- ============================================================
-- ② 展开 request.auth → 请求头 / 查询参数
-- ------------------------------------------------------------
-- 对照表（设计 §5.1）：
--   {type:"bearer", token:X}                         → Authorization: Bearer X
--   {type:"basic",  username:U, password:P}          → Authorization: {{$basic "U" "P"}}
--   {type:"apiKey", key:K, value:V, location:header} → K: V
--   {type:"apiKey", key:K, value:V, location:query}  → params 里 K: V
--   其余（none / inherit / 认不出的）                 → 什么都不做
--
-- 每条 UPDATE 都带两类判据，缺一不可：
--
--   * **载荷字段必须存在**（`… IS NOT NULL`）。serde 对缺字段是整条失败，
--     失败会退化成 `Inherit`；少了这个判据，`{"type":"bearer"}` 会被展开成
--     一条空 Bearer，而旧代码会老老实实去继承。
--   * **目标头名已存在（不分大小写）就跳过** —— 用户手写的头优先于自动展开的。
--     这条写反的代价是「用户的 Authorization 在升级瞬间被静默换掉」（风险 2）。
--
-- apiKey 额外要求**键名非空**：这与旧 `header_auth_pair()` / `query_auth_pair()`
-- 的 `!key.trim().is_empty()` 判据一致。id=14 就撞在这一条上 —— 它的键名是空串，
-- 所以迁移后**两边都不写**（它本来就一条凭据都不带，末态不变）。
--
-- `json_object(…, 'enabled', json('true'))` 里的 `json()` **不能省**：
-- 直接写 `1` 会产出 `"enabled":1`，而 `KeyValueItem.enabled` 是 `bool`，
-- serde 会因此拒绝整条请求。同理不写 `desc` —— 它是
-- `skip_serializing_if = "Option::is_none"`，库里存量也一条都没有。
-- ============================================================

-- bearer → Authorization: Bearer <token>
-- 空 token 也要写：`Bearer ` 作为一个**看得见的空值**留在表里，
-- 比它现在「界面上什么都不显示、请求裸着发出去」强（设计 §8 验收项）。
UPDATE request
   SET headers = json_insert(
           headers,
           '$[#]',
           json_object('key',     'Authorization',
                       'value',   'Bearer ' || json_extract(auth, '$.token'),
                       'enabled', json('true'))
       )
 WHERE json_extract(auth, '$.type') = 'bearer'
   AND json_extract(auth, '$.token') IS NOT NULL
   AND NOT EXISTS (
           SELECT 1 FROM json_each(request.headers) AS h
            WHERE lower(COALESCE(json_extract(h.value, '$.key'), '')) = 'authorization'
       );

-- basic → Authorization: {{$basic "U" "P"}}
-- 只在**真有** basic 配置（用户名或密码至少一个非空）时写，
-- 不给一条空配置凭空造出一条带密码的头（风险 3）。
-- ⚠️ 用户名 / 密码里若含双引号，写出来的模板会被 `$basic` 解析坏。
-- 本库不存在 basic 配置（实测 0 条），故不加转义 —— 转义规则属于模板解析器，
-- 不该由一条迁移自己发明。
UPDATE request
   SET headers = json_insert(
           headers,
           '$[#]',
           json_object('key',     'Authorization',
                       'value',   '{{$basic "' || json_extract(auth, '$.username')
                               || '" "'     || json_extract(auth, '$.password')
                               || '"}}',
                       'enabled', json('true'))
       )
 WHERE json_extract(auth, '$.type') = 'basic'
   AND json_extract(auth, '$.username') IS NOT NULL
   AND json_extract(auth, '$.password') IS NOT NULL
   AND (length(json_extract(auth, '$.username')) > 0
     OR length(json_extract(auth, '$.password')) > 0)
   AND NOT EXISTS (
           SELECT 1 FROM json_each(request.headers) AS h
            WHERE lower(COALESCE(json_extract(h.value, '$.key'), '')) = 'authorization'
       );

-- apiKey（location = header，也是缺 location 时的默认值）→ K: V
UPDATE request
   SET headers = json_insert(
           headers,
           '$[#]',
           json_object('key',     json_extract(auth, '$.key'),
                       'value',   json_extract(auth, '$.value'),
                       'enabled', json('true'))
       )
 WHERE json_extract(auth, '$.type') = 'apiKey'
   AND json_extract(auth, '$.value') IS NOT NULL
   AND COALESCE(json_extract(auth, '$.location'), 'header') = 'header'
   AND length(trim(COALESCE(json_extract(auth, '$.key'), ''))) > 0
   AND NOT EXISTS (
           SELECT 1 FROM json_each(request.headers) AS h
            WHERE lower(COALESCE(json_extract(h.value, '$.key'), ''))
                = lower(json_extract(auth, '$.key'))
       );

-- apiKey（location = query）→ params 里 K: V
-- 明确只认 `'query'`：缺 location 走上面的 header 分支，两者不会同时命中。
UPDATE request
   SET params = json_insert(
           params,
           '$[#]',
           json_object('key',     json_extract(auth, '$.key'),
                       'value',   json_extract(auth, '$.value'),
                       'enabled', json('true'))
       )
 WHERE json_extract(auth, '$.type') = 'apiKey'
   AND json_extract(auth, '$.value') IS NOT NULL
   AND json_extract(auth, '$.location') = 'query'
   AND length(trim(COALESCE(json_extract(auth, '$.key'), ''))) > 0
   AND NOT EXISTS (
           SELECT 1 FROM json_each(request.params) AS h
            WHERE lower(COALESCE(json_extract(h.value, '$.key'), ''))
                = lower(json_extract(auth, '$.key'))
       );


-- ============================================================
-- ③ 删掉两个 auth 列
-- ------------------------------------------------------------
-- 留一个不用的列，就是留一个下次会被误读的拐点。
-- 实测两列上都没有索引 / 触发器 / 视图引用（`idx_folder_parent` 走 parent_id、
-- `idx_folder_system` 走 is_system，`request` 上两个索引走 folder_id 与 name），
-- 所以 DROP COLUMN 不会牵连别的对象。
--
-- 🔴 **这一步让整段 SQL 不可重放** —— 别指望直接重放这个文件。
-- 也正因为删列会让 `repo/folder.rs` / `repo/request.rs` 的 SELECT 列表当场失效，
-- 它必须与「摘掉 auth 的全部代码」（模型 / 执行链 / 导入器 / 前端）同批落地。
-- ============================================================

ALTER TABLE request DROP COLUMN auth;
ALTER TABLE folder  DROP COLUMN auth;
