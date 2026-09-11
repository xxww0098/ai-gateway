-- Monotonic per-user cache epoch. Redis publishes compare this integer and
-- refuse to SETEX a remaining older than the last committed write.
ALTER TABLE IF EXISTS "users" ADD COLUMN IF NOT EXISTS "balance_version" bigint NOT NULL DEFAULT 0;
