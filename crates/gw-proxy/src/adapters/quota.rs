//! [`SubscriptionQuotaStore`] over Postgres.
//!
//! The explicit quota transaction: `SELECT ... FOR UPDATE` to serialise
//! concurrent requests against one
//! subscription, rotate any period whose boundary has passed, persist the
//! rotation, commit. Only the read/reset lives in the transaction — the limit
//! comparison happens after it, so the row lock is held for as short a time as
//! possible.
//!
//! **The rotation rule is not reimplemented here.** It comes from
//! [`crate::hold::rotate_counters`], the same function the hold middleware's
//! tests pin. Expressing "next UTC midnight / next Monday / the first of next
//! month, strictly after now" in SQL would be a second implementation of
//! boundary arithmetic that is easy to get subtly wrong (the Monday-at-
//! midnight case is the classic trap) and impossible to unit-test.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use gw_infra::Db;

use crate::adapters::directory::normalise_reset_at;
use crate::hold::rotate_counters;
use crate::ports::{Id, SubscriptionQuota, SubscriptionQuotaStore};

/// Row-locking quota reads for the hold pre-flight.
#[derive(Debug, Clone)]
pub struct SqlSubscriptionQuotaStore {
    db: Db,
}

impl SqlSubscriptionQuotaStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SubscriptionQuotaStore for SqlSubscriptionQuotaStore {
    async fn lock_and_rotate(
        &self,
        subscription_id: Id,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<SubscriptionQuota>> {
        let mut tx = self.db.begin().await?;

        let row = sqlx::query_as::<_, LockedRow>(
            "SELECT id, group_id, daily_usage_usd, weekly_usage_usd, monthly_usage_usd, \
                    daily_limit_usd, weekly_limit_usd, monthly_limit_usd, \
                    daily_reset_at, weekly_reset_at, monthly_reset_at \
             FROM subscriptions WHERE id = $1 FOR UPDATE",
        )
        .bind(subscription_id)
        .fetch_optional(&mut *tx)
        .await?;

        // A missing subscription is permissive, not an error: the quota system
        // is opt-in, and a user without one is billed purely from their balance.
        let Some(row) = row else {
            tx.rollback().await?;
            return Ok(None);
        };

        let mut quota = row.into_quota();
        if rotate_counters(&mut quota, now) {
            // The reset and the evaluation must be atomic, or two concurrent
            // requests can both observe the pre-rotation counter and both pass
            // a quota that only had room for one.
            sqlx::query(
                "UPDATE subscriptions SET \
                    daily_usage_usd = $2, daily_reset_at = $3, \
                    weekly_usage_usd = $4, weekly_reset_at = $5, \
                    monthly_usage_usd = $6, monthly_reset_at = $7, \
                    updated_at = NOW() \
                 WHERE id = $1",
            )
            .bind(subscription_id)
            .bind(quota.daily_usage_usd)
            .bind(quota.daily_reset_at)
            .bind(quota.weekly_usage_usd)
            .bind(quota.weekly_reset_at)
            .bind(quota.monthly_usage_usd)
            .bind(quota.monthly_reset_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(Some(quota))
    }
}

#[derive(sqlx::FromRow)]
struct LockedRow {
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
    daily_reset_at: Option<DateTime<Utc>>,
    weekly_reset_at: Option<DateTime<Utc>>,
    monthly_reset_at: Option<DateTime<Utc>>,
}

impl LockedRow {
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
            daily_reset_at: normalise_reset_at(self.daily_reset_at),
            weekly_reset_at: normalise_reset_at(self.weekly_reset_at),
            monthly_reset_at: normalise_reset_at(self.monthly_reset_at),
        }
    }
}

#[cfg(test)]
mod tests;
