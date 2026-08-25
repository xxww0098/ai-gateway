-- 0007_quota_reservations.sql —— 订阅配额的**在途预留**。
--
-- 在这张表之前，配额是这么判的：`SELECT ... FOR UPDATE` 锁住订阅行、轮转过期的
-- 计数器、提交事务，**然后**在事务外面拿 `daily_usage_usd` 和这次的估算比限额。
-- 两个并发请求抢最后一格额度时，两个都在提交后读到同一个「已用」，两个都放行。
-- 行锁白拿了 —— 它保护的是轮转，不是那个比较。
--
-- 还有第二个洞：预扣**从不预留配额**。结算时才 `daily_usage_usd += actual`。
-- 于是一千个在途请求对限额而言是隐形的，配额只在它们全部结算之后才追上来。
--
-- 这张表把两个洞一起补上，办法和余额那边是同一个：**把比较搬进事务**。
--
--   锁订阅行 → 轮转 → 比 (已用 + 在途预留合计 + 这一笔) 与限额 → 落这一行
--
-- 超限就整个回滚，一行预留都不留下。并发的第二个请求在行锁上排队，
-- 拿到锁时已经看得见第一个请求落下的预留行。
--
-- 生命周期与 `billing_operations` 完全对齐，键也是同一个（钱的键）：
--
--   预留   hold 处随操作 id 一起落行
--   结算   在**扣款那同一个事务里**删掉这一行、把 actual 加进三个计数器
--   释放   删掉这一行，不加任何计数器
--
-- 「删预留」和「加 actual」必须同一个事务，否则在途与实际会同时计一遍。
--
-- 钱是 decimal（`CONTRACT.md` §3.5），整数是 bigint，与既有列口径一致。

CREATE TABLE IF NOT EXISTS "quota_reservations" (
    -- 钱的键，与 `billing_operations.billing_operation_id` 同一个值。
    -- 一次操作只有一笔配额预留，所以它就是主键 —— 重复预留在这里是撞主键，
    -- 不是多出来的一行。
    "billing_operation_id" text NOT NULL,
    "subscription_id" bigint NOT NULL,
    -- 预留住的上限。预付模式下等于 `billing_operations.reserved_amount`：
    -- 拿去和限额比的那个数就是留住的数。
    "reserved_amount" decimal NOT NULL DEFAULT 0,
    "created_at" timestamptz NOT NULL DEFAULT NOW(),
    PRIMARY KEY ("billing_operation_id")
);

-- 准入时要对一个订阅的在途预留求和，这条索引让那次求和与**未结清**的数量
-- 成正比，而不是与历史总量成正比。
CREATE INDEX IF NOT EXISTS idx_quota_reservations_subscription
    ON quota_reservations (subscription_id);

-- 一笔卡住的预留（进程崩在结算之前）会一直压着额度。对账扫
-- `billing_operations.terminal_at IS NULL` 时会把它一并结清，这条索引供
-- 运维按年龄排查。
CREATE INDEX IF NOT EXISTS idx_quota_reservations_created_at
    ON quota_reservations (created_at);
