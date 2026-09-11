//! [`TenantDirectory`] over Postgres, fronted by the two L1 caches.
//!
//! The tenant-lookup queries together with the `APIKeyCache` /
//! `UserStatusCache` reads that wrap them. The caching policy is here; the
//! authorization policy stays in
//! [`crate::access::AccessProvider`], which is why this type never decides
//! whether a status means "allowed".

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use gw_infra::{ApiKeyCache, CachedKey, Db, TtlCache, UserStatusCache};
use tracing::Instrument as _;

use crate::ports::{ApiKeyRow, Id, SubscriptionQuota, TenantDirectory};

/// L1 lifetime of a validated API key and of a `users.status` reading.
pub const CACHE_TTL: Duration = Duration::from_secs(5 * 60);

/// L1 lifetime of the subscription / group-multiplier / entitlement lookups.
///
/// Shorter than [`CACHE_TTL`] because these feed billing (the rate multiplier)
/// and quota-gate wiring (which subscription row hold locks): a panel-side
/// change — new subscription, cancellation, multiplier edit — is honored on
/// `/v1/*` within this window. The cached quota *usage* numbers are advisory
/// only: the hold middleware re-reads the row under `FOR UPDATE`
/// (`lock_and_rotate`) before enforcing, so staleness here never loosens the
/// quota itself.
pub const AUTH_LOOKUP_CACHE_TTL: Duration = Duration::from_secs(30);

/// Debounce window for `last_used_at` bumps: one write per key per window
/// instead of one write per request. The column is a display field; losing
/// sub-minute resolution costs nothing, the per-request UPDATE was measurably
/// the single largest write amplifier on the auth path.
pub const TOUCH_DEBOUNCE_TTL: Duration = Duration::from_secs(60);

/// Cached in place of a `users.status` when the row is absent entirely.
///
/// Caching the negative is deliberate: without it a credential-guessing burst
/// against a non-existent id re-hits the database once per request. It is a
/// distinct value rather than `""` so "missing" can never be confused with a
/// legitimate status.
pub const USER_STATUS_MISSING: &str = "missing";

/// Tenant lookups for the `/v1/*` access path.
#[derive(Debug, Clone)]
pub struct SqlTenantDirectory {
    db: Db,
    api_keys: ApiKeyCache,
    users: UserStatusCache,
    /// `user_id → active subscription`, negatives included — "no subscription"
    /// is the common case and exactly the one that must not re-hit Postgres
    /// per request.
    subscriptions: Arc<TtlCache<Id, Option<SubscriptionQuota>>>,
    /// `group_id → rate_multiplier`. Admin-edited config, near-static.
    group_mults: Arc<TtlCache<Id, Option<f64>>>,
    /// `(user_id, group_id) → still entitled?`.
    entitlements: Arc<TtlCache<(Id, Id), bool>>,
    /// Keys whose `last_used_at` was bumped within [`TOUCH_DEBOUNCE_TTL`].
    touched: Arc<TtlCache<Id, ()>>,
}

impl SqlTenantDirectory {
    /// `api_keys` and `users` must be the SAME instances the panel holds, or an
    /// admin suspending a user is invisible to `/v1/*` until the TTL lapses —
    /// the two caches must be shared instances.
    ///
    /// The subscription / multiplier / entitlement caches are private to this
    /// adapter: nothing panel-side reads them, and panel writes surface here
    /// within [`AUTH_LOOKUP_CACHE_TTL`] — the same TTL-only contract the
    /// status cache already lives with, at a sixth of the window.
    pub fn new(db: Db, api_keys: ApiKeyCache, users: UserStatusCache) -> Self {
        Self {
            db,
            api_keys,
            users,
            subscriptions: Arc::new(TtlCache::new(AUTH_LOOKUP_CACHE_TTL)),
            group_mults: Arc::new(TtlCache::new(AUTH_LOOKUP_CACHE_TTL)),
            entitlements: Arc::new(TtlCache::new(AUTH_LOOKUP_CACHE_TTL)),
            touched: Arc::new(TtlCache::new(TOUCH_DEBOUNCE_TTL)),
        }
    }

    /// Loads `(status, concurrency)` from the L1 cache or Postgres.
    ///
    /// Both columns share one cache entry so a panel JWT recheck cannot hide
    /// a freshly-written concurrency cap from `/v1` for the rest of the TTL.
    async fn load_user(&self, user_id: Id) -> anyhow::Result<Option<(String, i64)>> {
        if let Some(cached) = self.users.get(user_id) {
            if cached.status == USER_STATUS_MISSING {
                return Ok(None);
            }
            return Ok(Some((cached.status, cached.concurrency)));
        }

        let row: Option<(String, i64)> = sqlx::query_as(
            "SELECT COALESCE(status, ''), COALESCE(concurrency, 0) FROM users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.db)
        .await?;

        match row {
            Some((status, concurrency)) => {
                self.users
                    .set_row(user_id, status.clone(), concurrency, CACHE_TTL);
                Ok(Some((status, concurrency)))
            }
            None => {
                self.users.set(user_id, USER_STATUS_MISSING, CACHE_TTL);
                Ok(None)
            }
        }
    }
}

/// Projects a cache entry onto the row shape the access path consumes.
///
/// `CachedKey::status` holds the **api key's** status, which is what the caller
/// gates on. The owning user's status is a separate lookup on purpose: one
/// cache entry must not be able to vouch for two independently mutable rows.
fn row_from_cache(entry: &CachedKey) -> ApiKeyRow {
    ApiKeyRow {
        id: entry.api_key_id,
        user_id: entry.user_id,
        group_id: entry.group_id,
        status: entry.status.clone(),
    }
}

#[async_trait]
impl TenantDirectory for SqlTenantDirectory {
    async fn api_key_by_hash(&self, key_hash: &str) -> anyhow::Result<Option<ApiKeyRow>> {
        if let Some(entry) = self.api_keys.get(key_hash) {
            return Ok(Some(row_from_cache(&entry)));
        }

        let row: Option<(Id, Id, Option<Id>, String)> = sqlx::query_as(
            "SELECT id, user_id, group_id, COALESCE(status, '') \
             FROM api_keys WHERE key_hash = $1",
        )
        .bind(key_hash)
        .fetch_optional(&self.db)
        .await?;

        let Some((id, user_id, group_id, status)) = row else {
            // A miss is NOT cached: an unknown hash is either a typo or an
            // attack, and caching it would let a key created a moment later
            // read as absent for the whole TTL.
            return Ok(None);
        };

        // Only an active key is worth caching — an inactive one is rejected on
        // every path anyway, and keeping it out of the cache means reactivation
        // takes effect immediately.
        if status == "active" {
            self.api_keys.set(
                key_hash,
                CachedKey {
                    user_id,
                    api_key_id: id,
                    group_id,
                    // The multiplier is resolved per request from `groups`, and
                    // re-checked against the entitlement, so nothing reads this
                    // back; storing the neutral value keeps a stale discount
                    // from ever surviving here.
                    rate_mult: 1.0,
                    status: status.clone(),
                    expires_at: Utc::now() + CACHE_TTL,
                },
            );
        }

        Ok(Some(ApiKeyRow {
            id,
            user_id,
            group_id,
            status,
        }))
    }

    async fn group_rate_multiplier(&self, group_id: Id) -> anyhow::Result<Option<f64>> {
        if let Some(cached) = self.group_mults.get(&group_id) {
            return Ok(cached);
        }
        // `numeric` column: it needs `compat::Money` to decode, and that also
        // reproduces the historical NULL -> 0.0 reading.
        let row: Option<(gw_model::compat::Money,)> =
            sqlx::query_as("SELECT rate_multiplier FROM groups WHERE id = $1")
                .bind(group_id)
                .fetch_optional(&self.db)
                .await?;
        let mult = row.map(|(mult,)| mult.0);
        self.group_mults.insert(group_id, mult);
        Ok(mult)
    }

    async fn user_status(&self, user_id: Id) -> anyhow::Result<Option<String>> {
        Ok(self.load_user(user_id).await?.map(|(status, _)| status))
    }

    async fn user_concurrency(&self, user_id: Id) -> anyhow::Result<Option<i64>> {
        Ok(self
            .load_user(user_id)
            .await?
            .map(|(_, concurrency)| concurrency))
    }

    async fn active_subscription(&self, user_id: Id) -> anyhow::Result<Option<SubscriptionQuota>> {
        if let Some(cached) = self.subscriptions.get(&user_id) {
            return Ok(cached);
        }
        let quota = sqlx::query_as::<_, SubscriptionRow>(
            "SELECT id, group_id, daily_usage_usd, weekly_usage_usd, monthly_usage_usd, \
                    daily_limit_usd, weekly_limit_usd, monthly_limit_usd, \
                    daily_reset_at, weekly_reset_at, monthly_reset_at \
             FROM subscriptions \
             WHERE user_id = $1 AND status = 'active' AND expires_at > NOW() \
             ORDER BY expires_at DESC LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&self.db)
        .await?;
        let quota = quota.map(SubscriptionRow::into_quota);
        self.subscriptions.insert(user_id, quota.clone());
        Ok(quota)
    }

    async fn holds_group_entitlement(&self, user_id: Id, group_id: Id) -> anyhow::Result<bool> {
        if let Some(cached) = self.entitlements.get(&(user_id, group_id)) {
            return Ok(cached);
        }
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM subscriptions \
             WHERE user_id = $1 AND group_id = $2 AND status = 'active' AND expires_at > NOW() \
             LIMIT 1",
        )
        .bind(user_id)
        .bind(group_id)
        .fetch_optional(&self.db)
        .await?;
        let entitled = row.is_some();
        self.entitlements.insert((user_id, group_id), entitled);
        Ok(entitled)
    }

    async fn token_version(&self, user_id: Id) -> anyhow::Result<i64> {
        // No L1 cache: a panel "log out everywhere" bump must take effect on
        // the next `/v1` JWT, the same contract `gw-panel` already keeps.
        // JWT is the secondary `/v1` credential; one extra indexed lookup is
        // the cost of not letting a revoked session keep spending.
        gw_authcore::TokenVersionStore::new(self.db.clone())
            .current(user_id)
            .await
            .map_err(|err| anyhow::anyhow!(err))
    }

    async fn touch_api_key(&self, api_key_id: Id) {
        if api_key_id == 0 {
            return;
        }
        // At most one bump per key per debounce window. Two concurrent misses
        // can both spawn the write; that costs one redundant UPDATE of a
        // display column, not correctness.
        if self.touched.get(&api_key_id).is_some() {
            return;
        }
        self.touched.insert(api_key_id, ());
        // Detached task — `last_used_at` is a display
        // field, and blocking the auth hot path on a write for it is not worth
        // the latency. It is deliberately NOT on the settlement drain: losing
        // one at shutdown costs a timestamp, not money.
        let db = self.db.clone();
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::spawn(
                async move {
                    let write =
                        sqlx::query("UPDATE api_keys SET last_used_at = NOW() WHERE id = $1")
                            .bind(api_key_id)
                            .execute(&db);
                    match tokio::time::timeout(Duration::from_secs(2), write).await {
                        Ok(Err(err)) => {
                            tracing::debug!(%err, api_key_id, "last_used_at bump failed");
                        }
                        Err(_) => tracing::debug!(api_key_id, "last_used_at bump timed out"),
                        Ok(Ok(_)) => {}
                    }
                }
                .in_current_span(),
            );
        }
    }
}

/// The subscription columns both the access path and the quota gate read.
///
/// `pub(crate)` so [`super::quota`] binds the identical projection instead of
/// keeping a field-for-field copy of the Money / reset-normalisation rules.
#[derive(sqlx::FromRow)]
pub(crate) struct SubscriptionRow {
    id: Id,
    group_id: Id,
    #[sqlx(try_from = "gw_model::compat::Money")]
    daily_usage_usd: f64,
    #[sqlx(try_from = "gw_model::compat::Money")]
    weekly_usage_usd: f64,
    #[sqlx(try_from = "gw_model::compat::Money")]
    monthly_usage_usd: f64,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    daily_limit_usd: Option<f64>,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    weekly_limit_usd: Option<f64>,
    #[sqlx(try_from = "gw_model::compat::MoneyOpt")]
    monthly_limit_usd: Option<f64>,
    daily_reset_at: Option<chrono::DateTime<Utc>>,
    weekly_reset_at: Option<chrono::DateTime<Utc>>,
    monthly_reset_at: Option<chrono::DateTime<Utc>>,
}

impl SubscriptionRow {
    pub(crate) fn into_quota(self) -> SubscriptionQuota {
        SubscriptionQuota {
            id: self.id,
            group_id: self.group_id,
            daily_usage_usd: self.daily_usage_usd,
            weekly_usage_usd: self.weekly_usage_usd,
            monthly_usage_usd: self.monthly_usage_usd,
            daily_limit_usd: self.daily_limit_usd,
            weekly_limit_usd: self.weekly_limit_usd,
            monthly_limit_usd: self.monthly_limit_usd,
            // The schema writes the zero timestamp for "this period never
            // rotates"; it reads back as the zero timestamp, which
            // `normalise_reset_at` maps to `None` so the rotation rule can
            // skip it.
            daily_reset_at: normalise_reset_at(self.daily_reset_at),
            weekly_reset_at: normalise_reset_at(self.weekly_reset_at),
            monthly_reset_at: normalise_reset_at(self.monthly_reset_at),
        }
    }
}

/// The zero-timestamp guard, expressed against what the column holds.
///
/// A NULL or a timestamp at/below the Unix epoch means "no rotation configured
/// for this period", which [`crate::hold::rotate_counters`] must leave alone —
/// treating it as an elapsed boundary would zero the counter on every request.
///
/// Shared with [`super::quota`], which reads the same three columns.
pub fn normalise_reset_at(value: Option<chrono::DateTime<Utc>>) -> Option<chrono::DateTime<Utc>> {
    value.filter(|ts| *ts > chrono::DateTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests;
