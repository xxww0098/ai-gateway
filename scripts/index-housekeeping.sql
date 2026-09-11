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

-- event_key 补一个**部分**唯一索引：不变量是「一个 BillingOperationId 最多
-- 一条成功行」（billing_operations 的终态条件更新已经钉在扣费侧）。failed 行
-- 是审计行，可以合法地与成功行共用同一 event_key —— 所以谓词必须排除
-- failed；全列 UNIQUE 会把「先失败审计、后重试成功」这条正当路径拒在库层。
-- 若历史数据里存在重复的成功行，本语句会失败 —— 先人工核对再重跑。
CREATE UNIQUE INDEX idx_usage_logs_event_key_settled
    ON usage_logs (event_key) WHERE failed = false;

-- balance_logs：idx_balance_logs_user_created (user_id, created_at DESC) 已覆盖
DROP INDEX IF EXISTS idx_balance_logs_user_id;

-- operation_logs：复合索引左前缀已覆盖单列等值查询
DROP INDEX IF EXISTS idx_operation_logs_actor_id;  -- ← actor_created (actor_id, created_at DESC)
DROP INDEX IF EXISTS idx_operation_logs_action;    -- ← action_created (action, created_at DESC)

-- subscriptions：idx_subscriptions_user_status_expires 已覆盖 user_id
DROP INDEX IF EXISTS idx_subscriptions_user_id;

-- api_keys：last_used_at 只做展示（touch 已按 TTL debounce），
-- 列表按 created_at / id 排序，没有查询按它过滤或排序。
DROP INDEX IF EXISTS idx_api_keys_last_used_at;

-- 注意：balance_logs 的 metadata GIN 索引**保留** —— 它是 shortfall 欠款
-- 预检查询目前唯一可用的索引；这条线上没有等效的部分索引。

COMMIT;
