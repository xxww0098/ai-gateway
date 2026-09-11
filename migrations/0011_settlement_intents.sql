-- 同一网关 request_id 最多成功结算一次。
-- 01 在 settle 事务内 CAS 写入 settled；03 再在 hold 时写入 pending。
-- 不给 usage_logs.request_id 或 settle 流水加全局唯一：失败审计行与 shortfall
-- 双行必须继续合法。
CREATE TABLE IF NOT EXISTS "settlement_intents" (
    "request_id" text PRIMARY KEY,
    "user_id" bigint NOT NULL,
    "hold_amount" numeric NOT NULL DEFAULT 0,
    "subscription_id" bigint,
    "status" text NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "settled_at" timestamptz
);
