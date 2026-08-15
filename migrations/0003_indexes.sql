-- 0003_indexes.sql —— 既有 schema 的索引全集：自动索引 + 7 个手写索引。
--
-- 前半段是建库脚本从 index / uniqueIndex 定义推出来的索引（名字生成规则：
-- idx_<表>_<列>，或定义里显式指定的名字，如 idx_model_catalog_channel_model）。
-- 后半段是那 7 条手写 SQL（GIN + 复合索引），
-- 逐字保留，包括 DESC 排序 —— 它们是 "filter by owner + ORDER BY created_at DESC"
-- 这类列表查询能只扫一个索引的前提。
-- ── 自动索引 ──────────────────────────────────────────────────────
CREATE UNIQUE INDEX IF NOT EXISTS "idx_users_email" ON "users" ("email");
CREATE INDEX IF NOT EXISTS "idx_api_keys_last_used_at" ON "api_keys" ("last_used_at");
CREATE INDEX IF NOT EXISTS "idx_api_keys_group_id" ON "api_keys" ("group_id");
CREATE UNIQUE INDEX IF NOT EXISTS "idx_api_keys_key_hash" ON "api_keys" ("key_hash");
CREATE INDEX IF NOT EXISTS "idx_api_keys_user_id" ON "api_keys" ("user_id");
CREATE UNIQUE INDEX IF NOT EXISTS "idx_groups_name" ON "groups" ("name");
CREATE INDEX IF NOT EXISTS "idx_balance_logs_user_id" ON "balance_logs" ("user_id");
CREATE INDEX IF NOT EXISTS "idx_usage_logs_created_at" ON "usage_logs" ("created_at");
CREATE INDEX IF NOT EXISTS "idx_usage_logs_failed" ON "usage_logs" ("failed");
CREATE INDEX IF NOT EXISTS "idx_usage_logs_provider" ON "usage_logs" ("provider");
CREATE INDEX IF NOT EXISTS "idx_usage_logs_model" ON "usage_logs" ("model");
CREATE INDEX IF NOT EXISTS "idx_usage_logs_event_key" ON "usage_logs" ("event_key");
CREATE INDEX IF NOT EXISTS "idx_usage_logs_idempotency_key" ON "usage_logs" ("idempotency_key");
CREATE INDEX IF NOT EXISTS "idx_usage_logs_request_id" ON "usage_logs" ("request_id");
CREATE INDEX IF NOT EXISTS "idx_usage_logs_group_id" ON "usage_logs" ("group_id");
CREATE INDEX IF NOT EXISTS "idx_usage_logs_api_key_id" ON "usage_logs" ("api_key_id");
CREATE INDEX IF NOT EXISTS "idx_usage_logs_user_id" ON "usage_logs" ("user_id");
CREATE INDEX IF NOT EXISTS "idx_operation_logs_entry_hash" ON "operation_logs" ("entry_hash");
CREATE INDEX IF NOT EXISTS "idx_operation_logs_created_at" ON "operation_logs" ("created_at");
CREATE INDEX IF NOT EXISTS "idx_operation_logs_request_id" ON "operation_logs" ("request_id");
CREATE INDEX IF NOT EXISTS "idx_operation_logs_status_code" ON "operation_logs" ("status_code");
CREATE INDEX IF NOT EXISTS "idx_operation_logs_target" ON "operation_logs" ("target");
CREATE INDEX IF NOT EXISTS "idx_operation_logs_action" ON "operation_logs" ("action");
CREATE INDEX IF NOT EXISTS "idx_operation_logs_actor_email" ON "operation_logs" ("actor_email");
CREATE INDEX IF NOT EXISTS "idx_operation_logs_actor_id" ON "operation_logs" ("actor_id");
CREATE INDEX IF NOT EXISTS "idx_operation_logs_source" ON "operation_logs" ("source");
CREATE INDEX IF NOT EXISTS "idx_subscription_packages_group_id" ON "subscription_packages" ("group_id");
CREATE INDEX IF NOT EXISTS "idx_subscriptions_monthly_reset_at" ON "subscriptions" ("monthly_reset_at");
CREATE INDEX IF NOT EXISTS "idx_subscriptions_weekly_reset_at" ON "subscriptions" ("weekly_reset_at");
CREATE INDEX IF NOT EXISTS "idx_subscriptions_daily_reset_at" ON "subscriptions" ("daily_reset_at");
CREATE INDEX IF NOT EXISTS "idx_subscriptions_expires_at" ON "subscriptions" ("expires_at");
CREATE INDEX IF NOT EXISTS "idx_subscriptions_status" ON "subscriptions" ("status");
CREATE INDEX IF NOT EXISTS "idx_subscriptions_group_id" ON "subscriptions" ("group_id");
CREATE INDEX IF NOT EXISTS "idx_subscriptions_package_id" ON "subscriptions" ("package_id");
CREATE INDEX IF NOT EXISTS "idx_subscriptions_user_id" ON "subscriptions" ("user_id");
CREATE INDEX IF NOT EXISTS "idx_tickets_assignee_id" ON "tickets" ("assignee_id");
CREATE INDEX IF NOT EXISTS "idx_tickets_status" ON "tickets" ("status");
CREATE INDEX IF NOT EXISTS "idx_tickets_user_id" ON "tickets" ("user_id");
CREATE INDEX IF NOT EXISTS "idx_ticket_replies_user_id" ON "ticket_replies" ("user_id");
CREATE INDEX IF NOT EXISTS "idx_ticket_replies_ticket_id" ON "ticket_replies" ("ticket_id");
CREATE UNIQUE INDEX IF NOT EXISTS "idx_model_prices_model_id" ON "model_prices" ("model_id");
CREATE UNIQUE INDEX IF NOT EXISTS "idx_model_catalog_channel_model" ON "model_catalog_entries" ("channel_key","model_id");
CREATE INDEX IF NOT EXISTS "idx_redeem_codes_used_by_id" ON "redeem_codes" ("used_by_id");
CREATE INDEX IF NOT EXISTS "idx_redeem_codes_status" ON "redeem_codes" ("status");
CREATE UNIQUE INDEX IF NOT EXISTS "idx_redeem_codes_code" ON "redeem_codes" ("code");
CREATE INDEX IF NOT EXISTS "idx_refunds_status" ON "refunds" ("status");
CREATE INDEX IF NOT EXISTS "idx_refunds_subscription_id" ON "refunds" ("subscription_id");
CREATE INDEX IF NOT EXISTS "idx_refunds_user_id" ON "refunds" ("user_id");
CREATE INDEX IF NOT EXISTS "idx_announcements_is_active" ON "announcements" ("is_active");
CREATE INDEX IF NOT EXISTS "idx_payment_orders_status" ON "payment_orders" ("status");
CREATE INDEX IF NOT EXISTS "idx_payment_orders_provider" ON "payment_orders" ("provider");
CREATE INDEX IF NOT EXISTS "idx_payment_orders_user_id" ON "payment_orders" ("user_id");
CREATE INDEX IF NOT EXISTS "idx_o_auth_sessions_expires_at" ON "o_auth_sessions" ("expires_at");
CREATE UNIQUE INDEX IF NOT EXISTS "idx_o_auth_sessions_state" ON "o_auth_sessions" ("state");
CREATE UNIQUE INDEX IF NOT EXISTS "idx_provider_configs_provider" ON "provider_configs" ("provider");
CREATE INDEX IF NOT EXISTS "idx_auth_records_status" ON "auth_records" ("status");
CREATE INDEX IF NOT EXISTS "idx_auth_records_prefix" ON "auth_records" ("prefix");
CREATE INDEX IF NOT EXISTS "idx_auth_records_provider" ON "auth_records" ("provider");
CREATE UNIQUE INDEX IF NOT EXISTS "idx_channel_policies_auth_id" ON "channel_policies" ("auth_id");
-- ── 手写索引 ──────────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idx_balance_logs_metadata ON balance_logs USING GIN (metadata);
CREATE INDEX IF NOT EXISTS idx_balance_logs_reference ON balance_logs (reference);
CREATE INDEX IF NOT EXISTS idx_usage_logs_user_created ON usage_logs (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_usage_logs_user_model_created ON usage_logs (user_id, model, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_balance_logs_user_created ON balance_logs (user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_operation_logs_action_created ON operation_logs (action, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_operation_logs_actor_created ON operation_logs (actor_id, created_at DESC);
