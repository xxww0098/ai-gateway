//! In-memory snapshot of the `model_prices` table.

use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use arc_swap::ArcSwap;
use gw_model::ModelPrice;
use sqlx::PgPool;
use tokio::task::JoinHandle;

/// Explicit column list for every cache load, matching what
/// [`ModelPrice`]'s `FromRow` expects.
///
/// Spelled out rather than `SELECT *` so the reverse-intuitive names stay
/// visible at the query site: `input_price_per1_m`, not `input_price_per_1m`
/// (see CONTRACT §3.5 — the historical column name breaks between the `1` and the `M`).
///
/// No casts and no `COALESCE`: the entity decodes its money columns through
/// `gw_model::compat::Money`, which reads `numeric` directly and maps a NULL to
/// `0`. Adding a `::float8` here would work but would put a
/// second, weaker copy of that rule in this file.
const SELECT_PRICES: &str = "SELECT id, \
     model_id, \
     input_price_per1_m, \
     output_price_per1_m, \
     cached_input_price_per1_m, \
     reasoning_price_per1_m, \
     created_at, \
     updated_at \
     FROM model_prices";

/// A shared, atomically-swapped snapshot of the pricing table.
///
/// This uses [`ArcSwap`] rather than a read-write lock: readers never block and
/// never see a torn map, and a writer publishes a whole new snapshot in one
/// store.
///
/// Share it as `Arc<ModelPriceCache>`. The panel's price editor and the
/// proxy's [`Calculator`](crate::Calculator) must hold the *same* instance —
/// that is what makes an admin price edit visible to billing without a
/// restart.
///
/// The rows are [`gw_model::ModelPrice`] — the one entity for that table.
/// The calculator reads only four of its columns, but keeping a narrower
/// local struct here would mean the same row shape declared twice and a
/// `From` conversion at every boundary that hands one over.
#[derive(Debug)]
pub struct ModelPriceCache {
    items: ArcSwap<HashMap<String, Arc<ModelPrice>>>,
}

impl Default for ModelPriceCache {
    fn default() -> Self {
        Self::empty()
    }
}

impl ModelPriceCache {
    /// An empty cache. Every [`get`](Self::get) misses, so a
    /// [`Calculator`](crate::Calculator) built on it falls back to its default
    /// price for every model — the same posture a calculator with no cache
    /// takes during bootstrap.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            items: ArcSwap::from_pointee(HashMap::new()),
        }
    }

    /// Builds a cache from rows already in hand, applying the same key
    /// normalization the database load path uses.
    ///
    /// This is the seam the DB-free tests use, and it is the *same* code
    /// [`invalidate`](Self::invalidate) runs after its query — there is no
    /// second normalization implementation to drift.
    ///
    /// Rows whose model id normalizes to the empty string are dropped. On
    /// duplicate ids the last row wins.
    #[must_use]
    pub fn from_rows<I: IntoIterator<Item = ModelPrice>>(rows: I) -> Self {
        let cache = Self::empty();
        cache.store_rows(rows);
        cache
    }

    /// Loads the full pricing table: construct + initial full load, with a
    /// load failure surfacing to the caller instead of yielding a half-built
    /// cache.
    ///
    /// # Errors
    /// Propagates any query error from the `model_prices` read.
    pub async fn load(pool: &PgPool) -> Result<Self, sqlx::Error> {
        let cache = Self::empty();
        cache.invalidate(pool).await?;
        Ok(cache)
    }

    /// Reloads the cache from the database in a single read and publishes the
    /// new snapshot atomically.
    ///
    /// Concurrent readers see either the whole old snapshot or the whole new
    /// one. A failed read leaves the previous snapshot untouched, so a
    /// transient database outage degrades to "prices are stale", never to
    /// "every model is unpriced".
    ///
    /// # Errors
    /// Propagates any query error from the `model_prices` read.
    pub async fn invalidate(&self, pool: &PgPool) -> Result<(), sqlx::Error> {
        let rows = sqlx::query_as::<_, ModelPrice>(SELECT_PRICES)
            .fetch_all(pool)
            .await?;
        self.store_rows(rows);
        Ok(())
    }

    /// Looks up a price. The key is trimmed and lower-cased, matching the
    /// normalization applied when the snapshot was built, so callers need not
    /// care about the casing an upstream used for the model id.
    #[must_use]
    pub fn get(&self, model_id: &str) -> Option<Arc<ModelPrice>> {
        let key = normalize_model_key(model_id);
        if key.is_empty() {
            return None;
        }
        self.items.load().get(&key).cloned()
    }

    /// A snapshot of every cached row, in unspecified order.
    #[must_use]
    pub fn list(&self) -> Vec<Arc<ModelPrice>> {
        self.items.load().values().cloned().collect()
    }

    /// Number of distinct model ids in the current snapshot.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.load().len()
    }

    /// Whether the current snapshot prices no models at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Periodically reloads the cache until the last [`Arc`] to it is dropped,
    /// so price edits made on another replica (or straight in the database)
    /// converge everywhere within `interval` instead of needing a restart.
    ///
    /// Two deliberate differences from the historical refresh task:
    ///
    /// * Cancellation is ownership-driven rather than context-driven: the task
    ///   holds a [`Weak`] handle and exits as soon as the cache itself is
    ///   dropped, so a forgotten `JoinHandle` cannot leak a task past the
    ///   cache's lifetime. Callers wanting explicit shutdown can still
    ///   [`abort`](JoinHandle::abort) the returned handle.
    /// * A zero `interval` returns `None` without spawning, mirroring the
    ///   no-op on a non-positive interval. (Rust `Duration`s cannot be
    ///   negative, so zero is the whole degenerate case.)
    ///
    /// A failed reload keeps the last good snapshot and logs a warning — it
    /// never clears prices.
    pub fn start_refresh(
        self: &Arc<Self>,
        pool: PgPool,
        interval: Duration,
    ) -> Option<JoinHandle<()>> {
        if interval.is_zero() {
            return None;
        }
        let weak: Weak<Self> = Arc::downgrade(self);
        Some(tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // The first tick of a tokio interval fires immediately; the cache
            // was just loaded, so burn it and keep a "reload every
            // `interval`, starting one interval from now" cadence.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let Some(cache) = weak.upgrade() else {
                    return;
                };
                if let Err(err) = cache.invalidate(&pool).await {
                    tracing::warn!(error = %err, "model price cache periodic refresh failed");
                }
            }
        }))
    }

    /// Normalizes and publishes a new snapshot. The single writer path shared
    /// by [`from_rows`](Self::from_rows) and [`invalidate`](Self::invalidate).
    pub(crate) fn store_rows<I: IntoIterator<Item = ModelPrice>>(&self, rows: I) {
        let mut next = HashMap::new();
        for row in rows {
            let key = normalize_model_key(&row.model_id);
            if key.is_empty() {
                continue;
            }
            next.insert(key, Arc::new(row));
        }
        self.items.store(Arc::new(next));
    }
}

/// Canonical cache-key form for a model id. Keeping it in one place is what
/// lets [`ModelPriceCache::get`] and the snapshot builder agree.
fn normalize_model_key(model_id: &str) -> String {
    model_id.trim().to_lowercase()
}

#[cfg(test)]
mod tests;
