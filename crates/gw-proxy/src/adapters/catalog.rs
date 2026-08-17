//! [`ChannelPolicyStore`] and [`ModelCatalog`] over Postgres, plus the
//! [`ChannelResolver`] the four-level upstream chain reads.
//!
//! The channel-policy refresh query, the model listing served on
//! `GET /v1/models`, and — new in wave 3 — the **routing** view of
//! `model_catalog_entries`: a table the panel has been maintaining all along
//! was being used as a display field (`owned_by`) while routing guessed from
//! model-name prefixes. See [`CatalogChannelResolver`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use async_trait::async_trait;
use gw_infra::Db;
use gw_relay::endpoint::matrix::Provider;
use gw_relay::endpoint::upstream::ChannelResolver;

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
             WHERE visible = TRUE AND model_id <> $1 \
             ORDER BY model_id, channel_key",
        )
        .bind(MODELS_URL_SENTINEL)
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

    async fn resolve_channels(&self, model_id: &str) -> anyhow::Result<Vec<String>> {
        // 注意**没有** `WHERE visible = TRUE` —— 见 `ModelCatalog::resolve_channels`
        // 的 doc：`visible` 是展示开关，不是调用开关。
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT channel_key FROM model_catalog_entries \
             WHERE model_id = $1 AND model_id <> $2 \
             ORDER BY channel_key",
        )
        .bind(model_id)
        .bind(MODELS_URL_SENTINEL)
        .fetch_all(&self.db)
        .await?;
        Ok(rows.into_iter().map(|(key,)| key).collect())
    }

    async fn model_routes(&self) -> anyhow::Result<Vec<(String, Vec<String>)>> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT DISTINCT model_id, channel_key FROM model_catalog_entries \
             WHERE model_id <> $1 \
             ORDER BY model_id, channel_key",
        )
        .bind(MODELS_URL_SENTINEL)
        .fetch_all(&self.db)
        .await?;

        let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
        for (model_id, channel_key) in rows {
            match grouped.last_mut() {
                Some((last, keys)) if *last == model_id => keys.push(channel_key),
                _ => grouped.push((model_id, vec![channel_key])),
            }
        }
        Ok(grouped)
    }
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

/// `channel_key` 没有显式映射时落到哪个 executor 的默认词表。
///
/// 词表来自前端已经写死的渠道下拉框
/// （`frontend/src/features/pricing/model_prices.ts` 的 `providerToChannelMap`），
/// 所以默认配置**天生与面板对齐**，前端一行不动。
///
/// 不在表里的 `channel_key` 落 [`gw_relay::endpoint::upstream::WILDCARD_PROVIDER`]
/// （`openai`）—— 它就是那个通配的 OpenAI 兼容 executor，DeepSeek / Qwen / Kimi /
/// 自建 vLLM 都从它出网。
const DEFAULT_CHANNEL_PROVIDERS: [(&str, Provider); 10] = [
    ("openai", Provider::OpenAi),
    ("openai_compatible", Provider::OpenAi),
    ("claude", Provider::Claude),
    ("gemini", Provider::Gemini),
    ("gemini-cli", Provider::Gemini),
    ("aistudio", Provider::Gemini),
    ("vertex", Provider::Vertex),
    ("codex", Provider::Codex),
    // xAI Grok is OpenAI-compatible; L1 `xai/<model>` / `grok/<model>` uses
    // the OpenAI executor. Auth records stay stored as `xai`.
    ("xai", Provider::OpenAi),
    ("grok", Provider::OpenAi),
];

/// 四级链 L2/L3 的真实数据源：`model_catalog_entries` 的**路由视图** + 快照缓存。
///
/// # 为什么是快照
///
/// [`ChannelResolver`] 的两个方法都是**同步**的，因为它们在推理热路径上，
/// 合同明确写着「不许阻塞，缓存未命中回落 L4，**不要等 DB**」。
/// 所以这里持有一个 [`ArcSwap`] 快照，由 [`Self::refresh`] 整体替换 ——
/// 与 [`crate::channel::ChannelPolicyCache`] 是同一种生命周期。
///
/// **从未刷新过的快照是空的**，此时四级链的 L2 全部落空、直接落 L4 兜底，
/// 行为与收敛前的 `provider_candidates()` **逐字节相同**。这不是退化，
/// 这是安全灰度的默认态。
///
/// L1（显式渠道前缀 `<channel_key>/<model_id>`）**不依赖快照**：
/// 它只查 [`Self::provider_for_channel`]，而那张表是静态的，所以
/// `codex/gpt-5` 这类写法从装上这个 resolver 的第一秒就生效。
pub struct CatalogChannelResolver {
    catalog: Arc<dyn ModelCatalog>,
    routes: ArcSwap<HashMap<String, Vec<String>>>,
    channels: HashMap<String, Provider>,
    refreshed_at: parking_lot::Mutex<Option<Instant>>,
}

impl std::fmt::Debug for CatalogChannelResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogChannelResolver")
            .field("models", &self.routes.load().len())
            .field("channels", &self.channels.len())
            .finish_non_exhaustive()
    }
}

impl CatalogChannelResolver {
    /// 建在一个 [`ModelCatalog`] 之上，带默认的 `channel_key → provider` 词表。
    #[must_use]
    pub fn new(catalog: Arc<dyn ModelCatalog>) -> Self {
        Self {
            catalog,
            routes: ArcSwap::from_pointee(HashMap::new()),
            channels: DEFAULT_CHANNEL_PROVIDERS
                .into_iter()
                .map(|(key, provider)| (key.to_owned(), provider))
                .collect(),
            refreshed_at: parking_lot::Mutex::new(None),
        }
    }

    /// 覆盖或补充一条 `channel_key → provider` 映射。
    ///
    /// 部署方在面板里自填的渠道名不在默认词表里时用它接上，
    /// 不必改这份源码里的常量。
    #[must_use]
    pub fn with_channel(mut self, channel_key: impl Into<String>, provider: Provider) -> Self {
        self.channels.insert(channel_key.into(), provider);
        self
    }

    /// 整体替换路由快照。组合根挂到既有的刷新 ticker 上即可
    /// （与 `ChannelPolicyCache::spawn_refresh` 同一个位置）。
    ///
    /// # Errors
    ///
    /// 目录查询失败时上抛。调用方**只记日志、不要清空快照** ——
    /// 一次查库失败不该让全部路由凭空退回猜测。
    pub async fn refresh(&self) -> anyhow::Result<()> {
        let routes = self.catalog.model_routes().await?;
        self.routes
            .store(Arc::new(routes.into_iter().collect::<HashMap<_, _>>()));
        *self.refreshed_at.lock() = Some(Instant::now());
        Ok(())
    }

    /// 快照有多旧。`None` = 从未成功刷新过（此时四级链全落 L4）。
    #[must_use]
    pub fn snapshot_age(&self) -> Option<Duration> {
        self.refreshed_at.lock().map(|at| at.elapsed())
    }

    /// 定时刷新，直到任务被 drop。照抄 [`crate::channel::ChannelPolicyCache::spawn_refresh`]。
    pub fn spawn_refresh(self: Arc<Self>, interval: Duration) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                if let Err(err) = self.refresh().await {
                    tracing::warn!(%err, "模型路由快照刷新失败，继续用旧快照");
                }
            }
        })
    }
}

impl ChannelResolver for CatalogChannelResolver {
    fn channels_for_model(&self, model_id: &str) -> Vec<String> {
        self.routes
            .load()
            .get(model_id)
            .cloned()
            .unwrap_or_default()
    }

    fn provider_for_channel(&self, channel_key: &str) -> Option<Provider> {
        self.channels.get(channel_key).copied()
    }
}

#[cfg(test)]
mod tests;
