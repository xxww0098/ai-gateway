-- Per-upstream-account in-flight cap, and the index that account usage
-- aggregation needs. 0 means unlimited (missing row / missing column reads as 0).
ALTER TABLE IF EXISTS "channel_policies"
    ADD COLUMN IF NOT EXISTS "max_concurrent" bigint DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_usage_logs_auth_created
    ON usage_logs (auth_id, created_at DESC);
