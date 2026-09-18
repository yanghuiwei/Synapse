-- ============================================================
-- Reqly 010_history_response.sql —— 历史里留一份「当时收到了什么」
-- ------------------------------------------------------------
-- 006 存的是「当时发出去的是什么」（curl）。这一版补另一半：**响应**。
-- 在此之前 history 只有 status / duration_ms / resp_size 三个标量 ——
-- 能看出「200，用了 143ms，1.2KB」，但看不到那 1.2KB 到底是什么。
-- 隔天回来复查一个接口的返回结构，唯一的办法是再发一次，
-- 而那已经是另一次执行（环境变了、Token 换了、服务端发版了）。
--
-- 与 006 同一个理由：这个信息只在执行的那一瞬间存在，事后读不出来。
--
-- 三列：
--   resp_headers        响应头，JSON 数组 [{name, value}]，**原样存**。
--                       打码是「回给 AI 的 DTO」那一侧的事（`service::ops`
--                       的 `response_dto`），不在这里做 —— 历史是用户回看
--                       「服务端当时到底回了什么」用的，存成 `••••` 会让
--                       界面把打码值显示得像真实回答。
--   resp_body           响应体。**有上限**（`storage::models::HISTORY_RESPONSE_BODY_LIMIT`），
--                       超了截断 —— 历史是「回看」不是「归档」，一次 8MB 的
--                       导出响应存进去，发几百次就能把库撑到 GB 级。
--   resp_body_truncated 1 = 上面那份不是全部。真实字节数在 resp_size
--                       （006 之前就有这列），界面据此提示「已截断」。
--
-- 为什么截断标记单独一列而不是拿 length(resp_body) 与 resp_size 比：
-- 二进制响应（body 为空串、内容在临时文件里）那种情况下两者都对不上，
-- 靠长度反推会把「本来就空」误判成「被截断了」。
--
-- 存量为 NULL = 那条历史发生在本版之前，或那条路径没给响应
-- （网络层失败的记录没有响应，这是正常的，不是缺数据）。
-- ============================================================

ALTER TABLE history ADD COLUMN resp_headers        TEXT;
ALTER TABLE history ADD COLUMN resp_body           TEXT;
ALTER TABLE history ADD COLUMN resp_body_truncated INTEGER;
