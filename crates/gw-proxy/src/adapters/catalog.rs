//! [`ChannelPolicyStore`] and [`ModelCatalog`] over Postgres.
//!
//! The channel-policy refresh query and the model listing served on
//! `GET /v1/models`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use gw_infra::Db;

use crate::ports::{
    ChannelPolicy, ChannelPolicyStore, ModelCatalog, ModelEntry, ModelReasoning,
    ModelReasoningEffort,
};

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
        let rows: Vec<(String, i64, i64, bool, i64)> = sqlx::query_as(
            "SELECT auth_id, COALESCE(weight, 1), COALESCE(priority, 0), \
                    COALESCE(enabled, TRUE), COALESCE(max_concurrent, 0) \
             FROM channel_policies",
        )
        .fetch_all(&self.db)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(auth_id, weight, priority, enabled, max_concurrent)| ChannelPolicy {
                    auth_id,
                    weight,
                    priority,
                    enabled,
                    max_concurrent,
                },
            )
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

type CatalogRow = (
    String,
    String,
    chrono::DateTime<chrono::Utc>,
    Option<serde_json::Value>,
);

fn entry_from_row((model_id, channel_key, created_at, capabilities): CatalogRow) -> ModelEntry {
    let mut entry = ModelEntry {
        id: model_id,
        created: created_at.timestamp(),
        owned_by: channel_key,
        ..ModelEntry::default()
    };
    apply_capabilities(&mut entry, capabilities.as_ref());
    entry
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
        let rows: Vec<CatalogRow> = sqlx::query_as(
            "SELECT DISTINCT ON (model_id) model_id, channel_key, created_at, capabilities \
             FROM model_catalog_entries \
             WHERE visible = TRUE AND model_id <> $1 \
             ORDER BY model_id, channel_key",
        )
        .bind(MODELS_URL_SENTINEL)
        .fetch_all(&self.db)
        .await?;

        Ok(rows.into_iter().map(entry_from_row).collect())
    }
}

/// In-memory snapshot of the **listing** view (`visible = TRUE`).
///
/// `GET /v1/models` is the first hop every OpenAI client makes. Serving it
/// from a snapshot keeps that hop off the full-table listing query.
/// [`Self::get_model`] reads the same map, so list and detail share one cache.
pub struct CachedModelCatalog {
    inner: Arc<dyn ModelCatalog>,
    snapshot: ArcSwap<CatalogSnapshot>,
    refreshing: tokio::sync::Mutex<()>,
}

struct CatalogSnapshot {
    list: Vec<ModelEntry>,
    by_id: HashMap<String, ModelEntry>,
    ready: bool,
}

impl CatalogSnapshot {
    fn empty() -> Self {
        Self {
            list: Vec::new(),
            by_id: HashMap::new(),
            ready: false,
        }
    }

    fn from_models(models: Vec<ModelEntry>) -> Self {
        let by_id = models
            .iter()
            .cloned()
            .map(|entry| (entry.id.clone(), entry))
            .collect();
        Self {
            list: models,
            by_id,
            ready: true,
        }
    }
}

impl std::fmt::Debug for CachedModelCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let snap = self.snapshot.load();
        f.debug_struct("CachedModelCatalog")
            .field("ready", &snap.ready)
            .field("models", &snap.list.len())
            .finish_non_exhaustive()
    }
}

impl CachedModelCatalog {
    /// Empty until [`Self::refresh`] or the first listing/detail request.
    #[must_use]
    pub fn new(inner: Arc<dyn ModelCatalog>) -> Self {
        Self {
            inner,
            snapshot: ArcSwap::from_pointee(CatalogSnapshot::empty()),
            refreshing: tokio::sync::Mutex::new(()),
        }
    }

    /// Replace the snapshot from the inner catalogue.
    ///
    /// # Errors
    ///
    /// Inner listing failure is returned and the previous snapshot is kept.
    pub async fn refresh(&self) -> anyhow::Result<()> {
        let _guard = self.refreshing.lock().await;
        self.reload().await
    }

    async fn reload(&self) -> anyhow::Result<()> {
        let models = self.inner.list_models().await?;
        self.snapshot
            .store(Arc::new(CatalogSnapshot::from_models(models)));
        Ok(())
    }

    async fn ensure_loaded(&self) -> anyhow::Result<arc_swap::Guard<Arc<CatalogSnapshot>>> {
        {
            let snap = self.snapshot.load();
            if snap.ready {
                return Ok(snap);
            }
        }
        let _guard = self.refreshing.lock().await;
        {
            let snap = self.snapshot.load();
            if snap.ready {
                return Ok(snap);
            }
        }
        self.reload().await?;
        Ok(self.snapshot.load())
    }

    /// Periodic reload until the task is dropped. Same lifecycle as
    /// [`crate::channel::ChannelPolicyCache::spawn_refresh`].
    pub fn spawn_refresh(self: Arc<Self>, interval: Duration) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if let Err(err) = self.refresh().await {
                    tracing::warn!(%err, "模型目录快照刷新失败，继续用旧快照");
                }
            }
        })
    }
}

#[async_trait]
impl ModelCatalog for CachedModelCatalog {
    async fn list_models(&self) -> anyhow::Result<Vec<ModelEntry>> {
        Ok(self.ensure_loaded().await?.list.clone())
    }

    async fn get_model(&self, id: &str) -> anyhow::Result<Option<ModelEntry>> {
        Ok(self.ensure_loaded().await?.by_id.get(id).cloned())
    }
}

/// Copy catalog-owned capability fields onto a listing entry.
///
/// Unknown modality names are dropped. An empty reasoning list is treated as
/// "this model has no thinking" rather than an empty selector.
pub(crate) fn apply_capabilities(entry: &mut ModelEntry, raw: Option<&serde_json::Value>) {
    let Some(value) = raw else {
        return;
    };
    if let Some(n) = value
        .get("context_length")
        .and_then(serde_json::Value::as_i64)
        && n > 0
    {
        entry.context_length = Some(n);
    }
    if let Some(n) = value
        .get("max_output_tokens")
        .and_then(serde_json::Value::as_i64)
        && n > 0
    {
        entry.max_output_tokens = Some(n);
    }
    if let Some(arr) = value
        .get("input_modalities")
        .and_then(serde_json::Value::as_array)
    {
        entry.input_modalities = arr
            .iter()
            .filter_map(serde_json::Value::as_str)
            .filter(|m| *m == "text" || *m == "image")
            .map(ToOwned::to_owned)
            .collect();
    }
    let Some(reasoning) = value.get("reasoning") else {
        return;
    };
    let Some(efforts) = reasoning
        .get("efforts")
        .and_then(serde_json::Value::as_array)
    else {
        return;
    };
    let efforts: Vec<ModelReasoningEffort> = efforts
        .iter()
        .filter_map(|item| {
            let id = item.get("id").and_then(serde_json::Value::as_str)?.trim();
            if id.is_empty() {
                return None;
            }
            let name = item
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(id);
            Some(ModelReasoningEffort {
                id: id.to_owned(),
                name: name.to_owned(),
            })
        })
        .collect();
    if efforts.is_empty() {
        return;
    }
    let default_effort = reasoning
        .get("default_effort")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    entry.reasoning = Some(ModelReasoning {
        efforts,
        default_effort,
    });
}

/// 哨兵行：面板把「上游模型列表 URL」也存进 `model_catalog_entries`，
/// 用这个保留的 `model_id` 占位（`gw-panel/src/ops/catalog.rs`）。
///
/// 面板自己的查询（`gw-panel/src/billing/prices.rs`）显式排除了它，
/// 而 [`SqlModelCatalog::list_models`] 此前**没有**，只靠「它是以
/// `visible = false` 写入的」挡住。只要有人通过面板把那行翻成 visible，
/// `__models_url__` 就会作为一个模型出现在 `GET /v1/models` 里。
/// 这里补齐，与面板对齐。
const MODELS_URL_SENTINEL: &str = "__models_url__";

#[cfg(test)]
mod tests;
