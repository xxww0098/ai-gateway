-- usage_logs 的 legacy 列物理下线 —— 手动运维执行，不属于 migrations/
-- （迁移的硬规矩禁 DROP；且旧行里这些列还有真数据，得先回填）。
--
-- 代码侧已经只写 canonical 列（input_tokens / output_tokens / cost），
-- 读侧用 canonical-first 的 coalesce 兜旧行。本脚本是**可选**的收尾：
-- 不跑也完全正确 —— 列留着只是每行多占几十字节、
-- 把「到底哪列是真」的歧义留给下一个读 schema 的人。
--
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f scripts/usage-log-columns.sql
--
-- 顺序约束：
--   1. 确认线上部署的是不再写 legacy 列的版本（本仓库里 gw-proxy
--      的 insert_usage_log 只写 canonical 列之后的任一版本）。
--   2. 跑下面的回填，再跑校验查询确认新旧行 canonical 列都齐了。
--   3. DROP COLUMN。
--   4. 下一轮代码清理时，再把读侧 SELECT 里的 coalesce 兜底删掉、
--      让查询只引用 canonical 列 —— 在那之前**不要**跑第 3 步。

BEGIN;

-- 回填：旧行的 legacy 值并进 canonical 列。新部署的行这些列恒为 0，
-- UPDATE 只碰还残留真数据的旧行。表大时分批跑（本脚本默认全量，
-- 生产大表请按 created_at 分批执行同样的语句）。
UPDATE usage_logs SET input_tokens = tokens_in
    WHERE COALESCE(input_tokens, 0) = 0 AND COALESCE(tokens_in, 0) > 0;
UPDATE usage_logs SET output_tokens = tokens_out
    WHERE COALESCE(output_tokens, 0) = 0 AND COALESCE(tokens_out, 0) > 0;
UPDATE usage_logs SET cost = GREATEST(
        COALESCE(total_cost, 0), COALESCE(actual_cost, 0), COALESCE(cost, 0))
    WHERE COALESCE(cost, 0) = 0
      AND (COALESCE(total_cost, 0) > 0 OR COALESCE(actual_cost, 0) > 0);

COMMIT;

-- 校验：三条都必须返回 0 再进入删除段。
--   1) 回填后不再有「legacy 有值而 canonical 是 0」的行：
SELECT count(*) AS unbackfilled
    FROM usage_logs
    WHERE (COALESCE(input_tokens, 0) = 0 AND COALESCE(tokens_in, 0) > 0)
       OR (COALESCE(output_tokens, 0) = 0 AND COALESCE(tokens_out, 0) > 0)
       OR (COALESCE(cost, 0) = 0
           AND (COALESCE(total_cost, 0) > 0 OR COALESCE(actual_cost, 0) > 0));

--   2) 新部署确实不再写 legacy 列（近 24h 的新行这些列应恒为 0）：
SELECT count(*) AS still_writing_legacy
    FROM usage_logs
    WHERE created_at > NOW() - INTERVAL '1 day'
      AND (COALESCE(tokens_in, 0) > 0 OR COALESCE(tokens_out, 0) > 0
           OR COALESCE(total_cost, 0) > 0 OR COALESCE(actual_cost, 0) > 0
           OR COALESCE(input_cost, 0) <> 0 OR COALESCE(output_cost, 0) <> 0);

--   3) 删除段（先确认读侧不再引用这些列，再放开）：
-- ALTER TABLE usage_logs
--     DROP COLUMN tokens_in,
--     DROP COLUMN tokens_out,
--     DROP COLUMN input_cost,
--     DROP COLUMN output_cost,
--     DROP COLUMN total_cost,
--     DROP COLUMN actual_cost;
