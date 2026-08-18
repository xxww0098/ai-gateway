-- Per-request billing lookups that existing 0003 indexes cannot serve:
--   subscriptions: user_id + status + expires_at (single-column indexes force Filter)
--   model_catalog_entries: model_id alone (unique key is channel_key, model_id)
CREATE INDEX IF NOT EXISTS idx_subscriptions_user_status_expires
    ON subscriptions (user_id, status, expires_at DESC);

CREATE INDEX IF NOT EXISTS idx_model_catalog_entries_model_id
    ON model_catalog_entries (model_id);
