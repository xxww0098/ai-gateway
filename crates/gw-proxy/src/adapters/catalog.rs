//! [`ChannelPolicyStore`] and [`ModelCatalog`] over Postgres.
//!
//! The channel-policy refresh query and the model listing served on
//! `GET /v1/models`.

use async_trait::async_trait;
use gw_infra::Db;

use crate::ports::{ChannelPolicy, ChannelPolicyStore, ModelCatalog, ModelEntry};

/// Source of the per-account routing policy snapshot.
#[derive(Debug, Clone)]
pub struct SqlChannelPolicyStore {
    db: Db,
}

impl SqlChannelPolicyStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl ChannelPolicyStore for SqlChannelPolicyStore {
    async fn list_channel_policies(&self) -> anyhow::Result<Vec<ChannelPolicy>> {
        // Whole-table read: the table has one row per configured upstream
        // account, and the cache that consumes this replaces its snapshot
        // wholesale so a deleted row reverts to the default instead of
        // lingering. Accounts WITHOUT a row are the common case and are not
        // represented here at all — `ChannelPolicyCache::lookup` defaults them.
        let rows: Vec<(String, i64, i64, bool)> = sqlx::query_as(
            "SELECT auth_id, COALESCE(weight, 1), COALESCE(priority, 0), \
                    COALESCE(enabled, TRUE) \
             FROM channel_policies",
        )
        .fetch_all(&self.db)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(auth_id, weight, priority, enabled)| ChannelPolicy {
                auth_id,
                weight,
                priority,
                enabled,
            })
            .collect())
    }
}

/// Source of the `GET /v1/models` catalogue.
#[derive(Debug, Clone)]
pub struct SqlModelCatalog {
    db: Db,
}

impl SqlModelCatalog {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl ModelCatalog for SqlModelCatalog {
    async fn list_models(&self) -> anyhow::Result<Vec<ModelEntry>> {
        // `visible` is the admin's switch for hiding a model from tenants, so
        // it filters here rather than in the handler — an invisible model must
        // not appear on any surface that lists models.
        //
        // One row per (channel_key, model_id) pair upstream, but the OpenAI
        // payload is keyed by model id alone; `DISTINCT ON` keeps the first
        // channel that offers each model instead of emitting duplicate ids.
        let rows: Vec<(String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            "SELECT DISTINCT ON (model_id) model_id, channel_key, created_at \
             FROM model_catalog_entries \
             WHERE visible = TRUE \
             ORDER BY model_id, channel_key",
        )
        .fetch_all(&self.db)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(model_id, channel_key, created_at)| ModelEntry {
                id: model_id,
                created: created_at.timestamp(),
                owned_by: channel_key,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;
