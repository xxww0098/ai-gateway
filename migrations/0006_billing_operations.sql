-- 0006_billing_operations.sql —— 计费操作的持久状态机。
--
-- 在这张表之前，一次预扣的「身份」是 Redis 里的一个 hold 成员，键是客户端可以
-- 自己指定的 `X-Trace-ID`。于是两件事同时成立：客户端能撞键，而对账只能拿
-- Redis TTL 当真相 —— 进程崩在 settle 之前，那笔钱到底结没结算，库里没有任何一行
-- 能回答。
--
-- 这张表就是那一行。**它是非终态操作的唯一真相**，Redis 退回成预留缓存：
--
--   held     预扣已入账，尚未终结（对账扫的就是这些行）
--   settled  已按真实用量扣款，终态
--   released 已释放未扣款，终态
--
-- `billing_operation_id` 由**服务端**生成，唯一约束在这里 —— settle_once /
-- release_once 的「恰好一次」就是靠 `WHERE state = 'held'` 的条件更新拿到行锁
-- 实现的，并发的第二次更新看到 0 行，返回 already-terminal，不再扣第二次钱。
--
-- 钱是 decimal（`CONTRACT.md` §3.5），整数是 bigint，与既有列口径一致。

CREATE TABLE IF NOT EXISTS "billing_operations" (
    "id" bigserial,
    -- 服务端生成的计费操作 id。**这是钱的键**，不是 X-Trace-ID。
    "billing_operation_id" text NOT NULL,
    "user_id" bigint NOT NULL,
    -- held | settled | released
    "state" text NOT NULL,
    -- 实际预留住的上限（预付模式下等于 admitted_liability，不是更小的下限）。
    "reserved_amount" decimal DEFAULT 0,
    -- 准入时认可的责任上限，即 preflight 拿去和余额比的那个数。
    "admitted_liability" decimal DEFAULT 0,
    -- 请求指纹。同一个 operation id 换了金额或指纹再来预扣 = 冲突，不是覆盖。
    "request_fingerprint" text,
    -- 观测用的客户端链路 id（入站 X-Trace-ID）。**只读，不参与任何判定。**
    "client_trace_id" text,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    -- 进入终态的时刻。非终态行为 NULL —— 对账扫的正是 terminal_at IS NULL。
    "terminal_at" timestamptz,
    PRIMARY KEY ("id")
);

-- 唯一约束就是「同一个操作只有一行」这条不变量本身。
CREATE UNIQUE INDEX IF NOT EXISTS idx_billing_operations_operation_id
    ON billing_operations (billing_operation_id);

-- 对账扫描：非终态 + 够老。部分索引让扫描代价与**未结清**的数量成正比，
-- 而不是与历史总量成正比。
CREATE INDEX IF NOT EXISTS idx_billing_operations_non_terminal
    ON billing_operations (created_at)
    WHERE terminal_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_billing_operations_user_state
    ON billing_operations (user_id, state);
