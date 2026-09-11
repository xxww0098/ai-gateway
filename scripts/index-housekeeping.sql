-- 索引清理脚本 —— 手动运维执行，不属于 migrations/ 目录。
--
-- migrations/ 的硬规矩是每条语句必须可重放（禁 DROP/TRUNCATE），所以索引
-- 删除只能走运维脚本。对已部署的库执行一次即可：
--
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f scripts/index-housekeeping.sql
--
-- 每个被删的单列索引都被既有复合索引的最左前缀覆盖（或无查询使用），
-- 删除后语义不变；usage_logs / balance_logs 是每请求一行的热写表，
-- 少一个索引就是每次 insert 少一份写放大。

BEGIN;

-- usage_logs：idx_usage_logs_user_created (user_id, created_at DESC) 已覆盖
DROP INDEX IF EXISTS idx_usage_logs_user_id;
-- request_id 换成**部分**唯一索引：不变量是「一个 request_id 最多一条
-- 成功行」（settlement_intents 已把它当主键钉在扣费侧）。failed=true 的
-- 审计行可以合法地与后来的成功行共用一个 request_id —— gw-proxy 的
-- A0c 验收测试钉的就是这条 —— 所以谓词必须排除 failed 行，全列 UNIQUE 会
-- 把「先失败审计、后重试成功」这条正当路径拒在库层。
-- 若历史数据里存在重复的成功行，本语句会失败 —— 先人工核对再重跑。
DROP INDEX IF EXISTS idx_usage_logs_request_id;
CREATE UNIQUE INDEX idx_usage_logs_request_id ON usage_logs (request_id)
    WHERE failed = false;

-- balance_logs：idx_balance_logs_user_created (user_id, created_at DESC) 已覆盖
DROP INDEX IF EXISTS idx_balance_logs_user_id;
-- GIN 全列索引：唯一按 metadata 键查询的是 shortfall 预检，已由
-- idx_balance_logs_shortfall_open 部分索引服务；流水表上 GIN 是纯写税。
DROP INDEX IF EXISTS idx_balance_logs_metadata;

-- operation_logs：复合索引左前缀已覆盖单列等值查询
DROP INDEX IF EXISTS idx_operation_logs_actor_id;  -- ← actor_created (actor_id, created_at DESC)
DROP INDEX IF EXISTS idx_operation_logs_action;    -- ← action_created (action, created_at DESC)

-- subscriptions：idx_subscriptions_user_status_expires 已覆盖 user_id
DROP INDEX IF EXISTS idx_subscriptions_user_id;

-- api_keys：last_used_at 只做展示（touch 已按 TTL debounce），
-- 列表按 created_at 排序，没有查询按它过滤或排序。
DROP INDEX IF EXISTS idx_api_keys_last_used_at;

COMMIT;
