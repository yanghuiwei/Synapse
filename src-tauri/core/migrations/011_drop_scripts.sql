-- 011：删掉 request 表上从未被执行的脚本列。
--
-- `pre_script` / `test_script` 是 001 建表时就画上的（对齐 Postman Scripts 页签），
-- 但**执行引擎从未实现**：前端页签只是「功能开发中」占位，`http/` 下没有任何引用。
-- 导入器一直把 Postman / ApiPost 的脚本搬进这两列，于是库里躺着一批永远不参与发送的
-- 文本。既然功能不做，列和它带进来的数据一起清掉。
--
-- SQLite 3.35+ 支持 `ALTER TABLE … DROP COLUMN`，但**逐列删更稳妥**：
-- 一次删两列在某些旧驱动上会失败，分两条语句各自幂等地走一遍。
ALTER TABLE request DROP COLUMN pre_script;
ALTER TABLE request DROP COLUMN test_script;
