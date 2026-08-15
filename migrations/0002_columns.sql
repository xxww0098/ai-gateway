-- 0002_columns.sql —— 老库补列。
--
-- 0001 的 CREATE TABLE IF NOT EXISTS 在「表已存在」时整条跳过，因此旧版本建库
-- 脚本建出来的库可能缺列（旧脚本会 ALTER TABLE ADD COLUMN，
-- CREATE TABLE IF NOT EXISTS 不会）。这里对每张表的每个非主键列补一次
-- ADD COLUMN IF NOT EXISTS，语句与 0001 中的列定义完全一致。
--
-- 对已经是最新 schema 的库，这个文件的每一条都是 no-op。
-- 带 DEFAULT 的列走 PG 11+ 的快速路径，不重写表。

-- users
ALTER TABLE IF EXISTS "users" ADD COLUMN IF NOT EXISTS "email" text NOT NULL;
ALTER TABLE IF EXISTS "users" ADD COLUMN IF NOT EXISTS "password_hash" text NOT NULL;
ALTER TABLE IF EXISTS "users" ADD COLUMN IF NOT EXISTS "role" text DEFAULT 'user';
ALTER TABLE IF EXISTS "users" ADD COLUMN IF NOT EXISTS "username" text;
ALTER TABLE IF EXISTS "users" ADD COLUMN IF NOT EXISTS "balance" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "users" ADD COLUMN IF NOT EXISTS "status" text DEFAULT 'active';
ALTER TABLE IF EXISTS "users" ADD COLUMN IF NOT EXISTS "concurrency" bigint DEFAULT 1;
ALTER TABLE IF EXISTS "users" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "users" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;

-- api_keys
ALTER TABLE IF EXISTS "api_keys" ADD COLUMN IF NOT EXISTS "user_id" bigint NOT NULL;
ALTER TABLE IF EXISTS "api_keys" ADD COLUMN IF NOT EXISTS "key_hash" text NOT NULL;
ALTER TABLE IF EXISTS "api_keys" ADD COLUMN IF NOT EXISTS "key_prefix" text NOT NULL;
ALTER TABLE IF EXISTS "api_keys" ADD COLUMN IF NOT EXISTS "name" text;
ALTER TABLE IF EXISTS "api_keys" ADD COLUMN IF NOT EXISTS "status" text DEFAULT 'active';
ALTER TABLE IF EXISTS "api_keys" ADD COLUMN IF NOT EXISTS "group_id" bigint;
ALTER TABLE IF EXISTS "api_keys" ADD COLUMN IF NOT EXISTS "last_used_at" timestamptz;
ALTER TABLE IF EXISTS "api_keys" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "api_keys" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;

-- groups
ALTER TABLE IF EXISTS "groups" ADD COLUMN IF NOT EXISTS "name" text NOT NULL;
ALTER TABLE IF EXISTS "groups" ADD COLUMN IF NOT EXISTS "rate_multiplier" decimal DEFAULT 1;
ALTER TABLE IF EXISTS "groups" ADD COLUMN IF NOT EXISTS "quota_limit" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "groups" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "groups" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;

-- balance_logs
ALTER TABLE IF EXISTS "balance_logs" ADD COLUMN IF NOT EXISTS "user_id" bigint NOT NULL;
ALTER TABLE IF EXISTS "balance_logs" ADD COLUMN IF NOT EXISTS "amount" decimal NOT NULL;
ALTER TABLE IF EXISTS "balance_logs" ADD COLUMN IF NOT EXISTS "type" text NOT NULL;
ALTER TABLE IF EXISTS "balance_logs" ADD COLUMN IF NOT EXISTS "reference" text;
ALTER TABLE IF EXISTS "balance_logs" ADD COLUMN IF NOT EXISTS "metadata" jsonb;
ALTER TABLE IF EXISTS "balance_logs" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;

-- usage_logs
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "user_id" bigint NOT NULL;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "api_key_id" bigint NOT NULL;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "group_id" bigint;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "request_id" text;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "idempotency_key" text;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "event_key" text;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "model" text;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "provider" text;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "auth_id" text;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "tokens_in" bigint DEFAULT 0;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "tokens_out" bigint DEFAULT 0;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "input_tokens" bigint DEFAULT 0;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "output_tokens" bigint DEFAULT 0;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "reasoning_tokens" bigint DEFAULT 0;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "cached_tokens" bigint DEFAULT 0;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "input_cost" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "output_cost" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "total_cost" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "actual_cost" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "cost" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "rate_multiplier" decimal DEFAULT 1;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "stream" boolean DEFAULT false;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "duration_ms" bigint DEFAULT 0;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "ip_address" text;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "raw_metadata" jsonb;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "failed" boolean DEFAULT false;
ALTER TABLE IF EXISTS "usage_logs" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;

-- operation_logs
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "source" text NOT NULL;
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "actor_id" bigint;
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "actor_email" text;
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "actor_role" text;
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "action" text NOT NULL;
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "target" text;
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "method" text;
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "path" text;
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "status_code" bigint DEFAULT 0;
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "ip_address" text;
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "request_id" text;
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "metadata" jsonb;
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "operation_logs" ADD COLUMN IF NOT EXISTS "entry_hash" varchar(64);

-- subscription_packages
ALTER TABLE IF EXISTS "subscription_packages" ADD COLUMN IF NOT EXISTS "name" text NOT NULL;
ALTER TABLE IF EXISTS "subscription_packages" ADD COLUMN IF NOT EXISTS "description" text;
ALTER TABLE IF EXISTS "subscription_packages" ADD COLUMN IF NOT EXISTS "group_id" bigint NOT NULL;
ALTER TABLE IF EXISTS "subscription_packages" ADD COLUMN IF NOT EXISTS "rate_multiplier" decimal DEFAULT 1;
ALTER TABLE IF EXISTS "subscription_packages" ADD COLUMN IF NOT EXISTS "default_validity_days" bigint DEFAULT 30;
ALTER TABLE IF EXISTS "subscription_packages" ADD COLUMN IF NOT EXISTS "daily_limit_usd" decimal;
ALTER TABLE IF EXISTS "subscription_packages" ADD COLUMN IF NOT EXISTS "weekly_limit_usd" decimal;
ALTER TABLE IF EXISTS "subscription_packages" ADD COLUMN IF NOT EXISTS "monthly_limit_usd" decimal;
ALTER TABLE IF EXISTS "subscription_packages" ADD COLUMN IF NOT EXISTS "subscription_price_usd" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "subscription_packages" ADD COLUMN IF NOT EXISTS "enabled" boolean DEFAULT true;
ALTER TABLE IF EXISTS "subscription_packages" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "subscription_packages" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;

-- subscriptions
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "user_id" bigint NOT NULL;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "package_id" bigint NOT NULL;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "group_id" bigint NOT NULL;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "group_name" text;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "status" text DEFAULT 'active';
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "starts_at" timestamptz NOT NULL;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "expires_at" timestamptz NOT NULL;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "daily_usage_usd" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "daily_reset_at" timestamptz;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "weekly_usage_usd" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "weekly_reset_at" timestamptz;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "monthly_usage_usd" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "monthly_reset_at" timestamptz;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "daily_limit_usd" decimal;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "weekly_limit_usd" decimal;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "monthly_limit_usd" decimal;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "funding_source" text;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "funding_reference" text;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "price_paid_usd" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "notes" text;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "subscriptions" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;

-- tickets
ALTER TABLE IF EXISTS "tickets" ADD COLUMN IF NOT EXISTS "user_id" bigint NOT NULL;
ALTER TABLE IF EXISTS "tickets" ADD COLUMN IF NOT EXISTS "title" text NOT NULL;
ALTER TABLE IF EXISTS "tickets" ADD COLUMN IF NOT EXISTS "category" text DEFAULT 'other';
ALTER TABLE IF EXISTS "tickets" ADD COLUMN IF NOT EXISTS "priority" text DEFAULT 'medium';
ALTER TABLE IF EXISTS "tickets" ADD COLUMN IF NOT EXISTS "status" text DEFAULT 'open';
ALTER TABLE IF EXISTS "tickets" ADD COLUMN IF NOT EXISTS "assignee_id" bigint;
ALTER TABLE IF EXISTS "tickets" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "tickets" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;

-- ticket_replies
ALTER TABLE IF EXISTS "ticket_replies" ADD COLUMN IF NOT EXISTS "ticket_id" bigint NOT NULL;
ALTER TABLE IF EXISTS "ticket_replies" ADD COLUMN IF NOT EXISTS "user_id" bigint NOT NULL;
ALTER TABLE IF EXISTS "ticket_replies" ADD COLUMN IF NOT EXISTS "is_admin" boolean DEFAULT false;
ALTER TABLE IF EXISTS "ticket_replies" ADD COLUMN IF NOT EXISTS "content" text NOT NULL;
ALTER TABLE IF EXISTS "ticket_replies" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;

-- model_prices
ALTER TABLE IF EXISTS "model_prices" ADD COLUMN IF NOT EXISTS "model_id" text NOT NULL;
ALTER TABLE IF EXISTS "model_prices" ADD COLUMN IF NOT EXISTS "input_price_per1_m" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "model_prices" ADD COLUMN IF NOT EXISTS "output_price_per1_m" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "model_prices" ADD COLUMN IF NOT EXISTS "cached_input_price_per1_m" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "model_prices" ADD COLUMN IF NOT EXISTS "reasoning_price_per1_m" decimal DEFAULT 0;
ALTER TABLE IF EXISTS "model_prices" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "model_prices" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;

-- model_catalog_entries
ALTER TABLE IF EXISTS "model_catalog_entries" ADD COLUMN IF NOT EXISTS "channel_key" varchar(128) NOT NULL;
ALTER TABLE IF EXISTS "model_catalog_entries" ADD COLUMN IF NOT EXISTS "model_id" varchar(128) NOT NULL;
ALTER TABLE IF EXISTS "model_catalog_entries" ADD COLUMN IF NOT EXISTS "visible" boolean DEFAULT true;
ALTER TABLE IF EXISTS "model_catalog_entries" ADD COLUMN IF NOT EXISTS "models_url" varchar(512);
ALTER TABLE IF EXISTS "model_catalog_entries" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "model_catalog_entries" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;

-- redeem_codes
ALTER TABLE IF EXISTS "redeem_codes" ADD COLUMN IF NOT EXISTS "code" varchar(64) NOT NULL;
ALTER TABLE IF EXISTS "redeem_codes" ADD COLUMN IF NOT EXISTS "amount" decimal NOT NULL;
ALTER TABLE IF EXISTS "redeem_codes" ADD COLUMN IF NOT EXISTS "status" varchar(16) NOT NULL DEFAULT 'unused';
ALTER TABLE IF EXISTS "redeem_codes" ADD COLUMN IF NOT EXISTS "used_by_id" bigint;
ALTER TABLE IF EXISTS "redeem_codes" ADD COLUMN IF NOT EXISTS "used_by" varchar(255);
ALTER TABLE IF EXISTS "redeem_codes" ADD COLUMN IF NOT EXISTS "used_at" timestamptz;
ALTER TABLE IF EXISTS "redeem_codes" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;

-- refunds
ALTER TABLE IF EXISTS "refunds" ADD COLUMN IF NOT EXISTS "user_id" bigint NOT NULL;
ALTER TABLE IF EXISTS "refunds" ADD COLUMN IF NOT EXISTS "subscription_id" bigint;
ALTER TABLE IF EXISTS "refunds" ADD COLUMN IF NOT EXISTS "amount" decimal NOT NULL DEFAULT 0;
ALTER TABLE IF EXISTS "refunds" ADD COLUMN IF NOT EXISTS "reason" varchar(1024);
ALTER TABLE IF EXISTS "refunds" ADD COLUMN IF NOT EXISTS "status" varchar(16) NOT NULL DEFAULT 'pending';
ALTER TABLE IF EXISTS "refunds" ADD COLUMN IF NOT EXISTS "days_used" bigint;
ALTER TABLE IF EXISTS "refunds" ADD COLUMN IF NOT EXISTS "total_days" bigint;
ALTER TABLE IF EXISTS "refunds" ADD COLUMN IF NOT EXISTS "daily_rate" decimal;
ALTER TABLE IF EXISTS "refunds" ADD COLUMN IF NOT EXISTS "processed_at" timestamptz;
ALTER TABLE IF EXISTS "refunds" ADD COLUMN IF NOT EXISTS "processed_by" bigint;
ALTER TABLE IF EXISTS "refunds" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;

-- user_token_versions
ALTER TABLE IF EXISTS "user_token_versions" ADD COLUMN IF NOT EXISTS "version" bigint NOT NULL DEFAULT 0;
ALTER TABLE IF EXISTS "user_token_versions" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;

-- announcements
ALTER TABLE IF EXISTS "announcements" ADD COLUMN IF NOT EXISTS "title" varchar(255) NOT NULL;
ALTER TABLE IF EXISTS "announcements" ADD COLUMN IF NOT EXISTS "content" text;
ALTER TABLE IF EXISTS "announcements" ADD COLUMN IF NOT EXISTS "type" varchar(32) NOT NULL DEFAULT 'info';
ALTER TABLE IF EXISTS "announcements" ADD COLUMN IF NOT EXISTS "is_active" boolean NOT NULL DEFAULT true;
ALTER TABLE IF EXISTS "announcements" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;

-- payment_orders
ALTER TABLE IF EXISTS "payment_orders" ADD COLUMN IF NOT EXISTS "user_id" bigint NOT NULL;
ALTER TABLE IF EXISTS "payment_orders" ADD COLUMN IF NOT EXISTS "provider" varchar(32) NOT NULL;
ALTER TABLE IF EXISTS "payment_orders" ADD COLUMN IF NOT EXISTS "amount_usd" decimal NOT NULL;
ALTER TABLE IF EXISTS "payment_orders" ADD COLUMN IF NOT EXISTS "amount_local" decimal;
ALTER TABLE IF EXISTS "payment_orders" ADD COLUMN IF NOT EXISTS "currency" varchar(8);
ALTER TABLE IF EXISTS "payment_orders" ADD COLUMN IF NOT EXISTS "status" varchar(16) NOT NULL DEFAULT 'pending';
ALTER TABLE IF EXISTS "payment_orders" ADD COLUMN IF NOT EXISTS "transaction_id" varchar(128);
ALTER TABLE IF EXISTS "payment_orders" ADD COLUMN IF NOT EXISTS "metadata" text;
ALTER TABLE IF EXISTS "payment_orders" ADD COLUMN IF NOT EXISTS "paid_at" timestamptz;
ALTER TABLE IF EXISTS "payment_orders" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "payment_orders" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;

-- ampcode_configs
ALTER TABLE IF EXISTS "ampcode_configs" ADD COLUMN IF NOT EXISTS "config_data" jsonb;
ALTER TABLE IF EXISTS "ampcode_configs" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "ampcode_configs" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;

-- o_auth_sessions
ALTER TABLE IF EXISTS "o_auth_sessions" ADD COLUMN IF NOT EXISTS "provider" varchar(64) NOT NULL;
ALTER TABLE IF EXISTS "o_auth_sessions" ADD COLUMN IF NOT EXISTS "state" varchar(255) NOT NULL;
ALTER TABLE IF EXISTS "o_auth_sessions" ADD COLUMN IF NOT EXISTS "auth_url" varchar(1024);
ALTER TABLE IF EXISTS "o_auth_sessions" ADD COLUMN IF NOT EXISTS "status" varchar(32) DEFAULT 'pending';
ALTER TABLE IF EXISTS "o_auth_sessions" ADD COLUMN IF NOT EXISTS "auth_id" varchar(128);
ALTER TABLE IF EXISTS "o_auth_sessions" ADD COLUMN IF NOT EXISTS "config_data" jsonb;
ALTER TABLE IF EXISTS "o_auth_sessions" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "o_auth_sessions" ADD COLUMN IF NOT EXISTS "expires_at" timestamptz NOT NULL;

-- provider_configs
ALTER TABLE IF EXISTS "provider_configs" ADD COLUMN IF NOT EXISTS "provider" varchar(128) NOT NULL;
ALTER TABLE IF EXISTS "provider_configs" ADD COLUMN IF NOT EXISTS "config_data" jsonb;
ALTER TABLE IF EXISTS "provider_configs" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "provider_configs" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;

-- auth_records
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "provider" varchar(64) NOT NULL;
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "prefix" varchar(128);
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "label" varchar(255);
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "status" varchar(64);
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "status_message" varchar(512);
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "disabled" boolean NOT NULL DEFAULT false;
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "unavailable" boolean NOT NULL DEFAULT false;
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "proxy_url" varchar(1024);
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "attributes" jsonb;
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "metadata" jsonb;
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "quota" jsonb;
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "model_states" jsonb;
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "last_error" jsonb;
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "last_refreshed_at" timestamptz;
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "next_refresh_after" timestamptz;
ALTER TABLE IF EXISTS "auth_records" ADD COLUMN IF NOT EXISTS "next_retry_after" timestamptz;

-- channel_policies
ALTER TABLE IF EXISTS "channel_policies" ADD COLUMN IF NOT EXISTS "auth_id" varchar(191) NOT NULL;
ALTER TABLE IF EXISTS "channel_policies" ADD COLUMN IF NOT EXISTS "weight" bigint DEFAULT 1;
ALTER TABLE IF EXISTS "channel_policies" ADD COLUMN IF NOT EXISTS "priority" bigint DEFAULT 0;
ALTER TABLE IF EXISTS "channel_policies" ADD COLUMN IF NOT EXISTS "enabled" boolean DEFAULT true;
ALTER TABLE IF EXISTS "channel_policies" ADD COLUMN IF NOT EXISTS "created_at" timestamptz;
ALTER TABLE IF EXISTS "channel_policies" ADD COLUMN IF NOT EXISTS "updated_at" timestamptz;
