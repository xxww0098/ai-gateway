//! [`TenantDirectory`] over Postgres, fronted by the two L1 caches.
//!
//! The tenant-lookup queries together with the `APIKeyCache` /
//! `UserStatusCache` reads that wrap them. The caching policy is here; the
//! authorization policy stays in
//! [`crate::access::AccessProvider`], which is why this type never decides
//! whether a status means "allowed".

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use gw_infra::{ApiKeyCache, CachedKey, Db, UserStatusCache};
use tracing::Instrument as _;

use crate::ports::{ApiKeyRow, Id, SubscriptionQuota, TenantDirectory};

/// L1 lifetime of a validated API key and of a `users.status` reading.
pub const CACHE_TTL: Duration = Duration::from_secs(5 * 60);

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
}

impl SqlTenantDirectory {
    /// `api_keys` and `users` must be the SAME instances the panel holds, or an
    /// admin suspending a user is invisible to `/v1/*` until the TTL lapses —
    /// the two caches must be shared instances.
    pub fn new(db: Db, api_keys: ApiKeyCache, users: UserStatusCache) -> Self {
        Self {
            db,
            api_keys,
            users,
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
        // `numeric` column: it needs `compat::Money` to decode, and that also
        // reproduces the historical NULL -> 0.0 reading.
        let row: Option<(gw_model::compat::Money,)> =
            sqlx::query_as("SELECT rate_multiplier FROM groups WHERE id = $1")
                .bind(group_id)
                .fetch_optional(&self.db)
                .await?;
        Ok(row.map(|(mult,)| mult.0))
    }

    async fn user_status(&self, user_id: Id) -> anyhow::Result<Option<String>> {
        if let Some(cached) = self.users.get(user_id) {
            return Ok((cached.status != USER_STATUS_MISSING).then_some(cached.status));
        }

        let row: Option<(String,)> =
            sqlx::query_as("SELECT COALESCE(status, '') FROM users WHERE id = $1")
                .bind(user_id)
                .fetch_optional(&self.db)
                .await?;

        // A transient failure propagates instead of being cached, so a database
        // blip cannot pin a live user into "inactive" for the whole TTL. The
        // caller fails closed on the error itself.
        let status = row.map(|(status,)| status);
        self.users.set(
            user_id,
            status
                .clone()
                .unwrap_or_else(|| USER_STATUS_MISSING.to_owned()),
            CACHE_TTL,
        );
        Ok(status)
    }

    async fn active_subscription(&self, user_id: Id) -> anyhow::Result<Option<SubscriptionQuota>> {
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
        Ok(quota.map(SubscriptionRow::into_quota))
    }

    async fn holds_group_entitlement(&self, user_id: Id, group_id: Id) -> anyhow::Result<bool> {
        let row: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM subscriptions \
             WHERE user_id = $1 AND group_id = $2 AND status = 'active' AND expires_at > NOW() \
             LIMIT 1",
        )
        .bind(user_id)
        .bind(group_id)
        .fetch_optional(&self.db)
        .await?;
        Ok(row.is_some())
    }

    async fn touch_api_key(&self, api_key_id: Id) {
        if api_key_id == 0 {
            return;
        }
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
#[derive(sqlx::FromRow)]
struct SubscriptionRow {
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
    fn into_quota(self) -> SubscriptionQuota {
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
