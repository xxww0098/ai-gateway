-- 0001_init.sql —— 建表部分：22 张表的 CREATE TABLE IF NOT EXISTS。
--
-- 每一条 CREATE TABLE 都与既有库的 schema 逐字节对齐：列名、类型、DEFAULT、
-- NOT NULL 逐个一致。校验方式见 migrations/README.md。
--
-- 全部用 IF NOT EXISTS：这份迁移必须能直接跑在已有表的现有库上而不报错，
-- 也不改动任何已有对象。缺列的补列在 0002。
--
-- 注意几个反直觉但必须保留的名字（既有 schema 的既成事实，改了就读不到老数据）：
--   OAuthSession        -> o_auth_sessions        （不是 oauth_sessions）
--   InputPricePer1M     -> input_price_per1_m     （不是 input_price_per_1m）
--   ApiKey              -> api_keys, ApiKeyID -> api_key_id
--   IPAddress           -> ip_address
-- 以及历史建库脚本里若干写成 `size=32`（等号）的列定义 —— 解析不到，
-- 于是这些列建出来是 text 而不是 varchar(n)。这里保持 text，不"修正"。


CREATE TABLE IF NOT EXISTS "users" (
    "id" bigserial,
    "email" text NOT NULL,
    "password_hash" text NOT NULL,
    "role" text DEFAULT 'user',
    "username" text,
    "balance" decimal DEFAULT 0,
    "status" text DEFAULT 'active',
    "concurrency" bigint DEFAULT 1,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "api_keys" (
    "id" bigserial,
    "user_id" bigint NOT NULL,
    "key_hash" text NOT NULL,
    "key_prefix" text NOT NULL,
    "name" text,
    "status" text DEFAULT 'active',
    "group_id" bigint,
    "last_used_at" timestamptz,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "groups" (
    "id" bigserial,
    "name" text NOT NULL,
    "rate_multiplier" decimal DEFAULT 1,
    "quota_limit" decimal DEFAULT 0,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "balance_logs" (
    "id" bigserial,
    "user_id" bigint NOT NULL,
    "amount" decimal NOT NULL,
    "type" text NOT NULL,
    "reference" text,
    "metadata" jsonb,
    "created_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "usage_logs" (
    "id" bigserial,
    "user_id" bigint NOT NULL,
    "api_key_id" bigint NOT NULL,
    "group_id" bigint,
    "request_id" text,
    "idempotency_key" text,
    "event_key" text,
    "model" text,
    "provider" text,
    "auth_id" text,
    "tokens_in" bigint DEFAULT 0,
    "tokens_out" bigint DEFAULT 0,
    "input_tokens" bigint DEFAULT 0,
    "output_tokens" bigint DEFAULT 0,
    "reasoning_tokens" bigint DEFAULT 0,
    "cached_tokens" bigint DEFAULT 0,
    "input_cost" decimal DEFAULT 0,
    "output_cost" decimal DEFAULT 0,
    "total_cost" decimal DEFAULT 0,
    "actual_cost" decimal DEFAULT 0,
    "cost" decimal DEFAULT 0,
    "rate_multiplier" decimal DEFAULT 1,
    "stream" boolean DEFAULT false,
    "duration_ms" bigint DEFAULT 0,
    "ip_address" text,
    "raw_metadata" jsonb,
    "failed" boolean DEFAULT false,
    "created_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "operation_logs" (
    "id" bigserial,
    "source" text NOT NULL,
    "actor_id" bigint,
    "actor_email" text,
    "actor_role" text,
    "action" text NOT NULL,
    "target" text,
    "method" text,
    "path" text,
    "status_code" bigint DEFAULT 0,
    "ip_address" text,
    "request_id" text,
    "metadata" jsonb,
    "created_at" timestamptz,
    "entry_hash" varchar(64),
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "subscription_packages" (
    "id" bigserial,
    "name" text NOT NULL,
    "description" text,
    "group_id" bigint NOT NULL,
    "rate_multiplier" decimal DEFAULT 1,
    "default_validity_days" bigint DEFAULT 30,
    "daily_limit_usd" decimal,
    "weekly_limit_usd" decimal,
    "monthly_limit_usd" decimal,
    "subscription_price_usd" decimal DEFAULT 0,
    "enabled" boolean DEFAULT true,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "subscriptions" (
    "id" bigserial,
    "user_id" bigint NOT NULL,
    "package_id" bigint NOT NULL,
    "group_id" bigint NOT NULL,
    "group_name" text,
    "status" text DEFAULT 'active',
    "starts_at" timestamptz NOT NULL,
    "expires_at" timestamptz NOT NULL,
    "daily_usage_usd" decimal DEFAULT 0,
    "daily_reset_at" timestamptz,
    "weekly_usage_usd" decimal DEFAULT 0,
    "weekly_reset_at" timestamptz,
    "monthly_usage_usd" decimal DEFAULT 0,
    "monthly_reset_at" timestamptz,
    "daily_limit_usd" decimal,
    "weekly_limit_usd" decimal,
    "monthly_limit_usd" decimal,
    "funding_source" text,
    "funding_reference" text,
    "price_paid_usd" decimal DEFAULT 0,
    "notes" text,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "tickets" (
    "id" bigserial,
    "user_id" bigint NOT NULL,
    "title" text NOT NULL,
    "category" text DEFAULT 'other',
    "priority" text DEFAULT 'medium',
    "status" text DEFAULT 'open',
    "assignee_id" bigint,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "ticket_replies" (
    "id" bigserial,
    "ticket_id" bigint NOT NULL,
    "user_id" bigint NOT NULL,
    "is_admin" boolean DEFAULT false,
    "content" text NOT NULL,
    "created_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "model_prices" (
    "id" bigserial,
    "model_id" text NOT NULL,
    "input_price_per1_m" decimal DEFAULT 0,
    "output_price_per1_m" decimal DEFAULT 0,
    "cached_input_price_per1_m" decimal DEFAULT 0,
    "reasoning_price_per1_m" decimal DEFAULT 0,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "model_catalog_entries" (
    "id" bigserial,
    "channel_key" varchar(128) NOT NULL,
    "model_id" varchar(128) NOT NULL,
    "visible" boolean DEFAULT true,
    "models_url" varchar(512),
    "created_at" timestamptz,
    "updated_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "redeem_codes" (
    "id" bigserial,
    "code" varchar(64) NOT NULL,
    "amount" decimal NOT NULL,
    "status" varchar(16) NOT NULL DEFAULT 'unused',
    "used_by_id" bigint,
    "used_by" varchar(255),
    "used_at" timestamptz,
    "created_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "refunds" (
    "id" bigserial,
    "user_id" bigint NOT NULL,
    "subscription_id" bigint,
    "amount" decimal NOT NULL DEFAULT 0,
    "reason" varchar(1024),
    "status" varchar(16) NOT NULL DEFAULT 'pending',
    "days_used" bigint,
    "total_days" bigint,
    "daily_rate" decimal,
    "processed_at" timestamptz,
    "processed_by" bigint,
    "created_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "user_token_versions" (
    "user_id" bigserial,
    "version" bigint NOT NULL DEFAULT 0,
    "updated_at" timestamptz,
    PRIMARY KEY ("user_id")
);

CREATE TABLE IF NOT EXISTS "announcements" (
    "id" bigserial,
    "title" varchar(255) NOT NULL,
    "content" text,
    "type" varchar(32) NOT NULL DEFAULT 'info',
    "is_active" boolean NOT NULL DEFAULT true,
    "created_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "payment_orders" (
    "id" bigserial,
    "user_id" bigint NOT NULL,
    "provider" varchar(32) NOT NULL,
    "amount_usd" decimal NOT NULL,
    "amount_local" decimal,
    "currency" varchar(8),
    "status" varchar(16) NOT NULL DEFAULT 'pending',
    "transaction_id" varchar(128),
    "metadata" text,
    "paid_at" timestamptz,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "ampcode_configs" (
    "id" bigserial,
    "config_data" jsonb,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "o_auth_sessions" (
    "id" bigserial,
    "provider" varchar(64) NOT NULL,
    "state" varchar(255) NOT NULL,
    "auth_url" varchar(1024),
    "status" varchar(32) DEFAULT 'pending',
    "auth_id" varchar(128),
    "config_data" jsonb,
    "created_at" timestamptz,
    "expires_at" timestamptz NOT NULL,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "provider_configs" (
    "id" bigserial,
    "provider" varchar(128) NOT NULL,
    "config_data" jsonb,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "auth_records" (
    "id" varchar(128),
    "provider" varchar(64) NOT NULL,
    "prefix" varchar(128),
    "label" varchar(255),
    "status" varchar(64),
    "status_message" varchar(512),
    "disabled" boolean NOT NULL DEFAULT false,
    "unavailable" boolean NOT NULL DEFAULT false,
    "proxy_url" varchar(1024),
    "attributes" jsonb,
    "metadata" jsonb,
    "quota" jsonb,
    "model_states" jsonb,
    "last_error" jsonb,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    "last_refreshed_at" timestamptz,
    "next_refresh_after" timestamptz,
    "next_retry_after" timestamptz,
    PRIMARY KEY ("id")
);

CREATE TABLE IF NOT EXISTS "channel_policies" (
    "id" bigserial,
    "auth_id" varchar(191) NOT NULL,
    "weight" bigint DEFAULT 1,
    "priority" bigint DEFAULT 0,
    "enabled" boolean DEFAULT true,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    PRIMARY KEY ("id")
);
