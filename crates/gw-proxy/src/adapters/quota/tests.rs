//! Locking, rotation and persistence of the quota counters.

use super::*;
use crate::testsupport::fresh_db;

/// Seeds one subscription whose three reset boundaries are set individually,
/// so a test can make exactly one period elapse.
async fn seed_subscription(
    pool: &sqlx::PgPool,
    id: Id,
    daily_reset: &str,
    weekly_reset: &str,
    monthly_reset: &str,
) {
    sqlx::query(&format!(
        "INSERT INTO subscriptions (id, user_id, package_id, group_id, group_name, status, \
                starts_at, expires_at, \
                daily_usage_usd, daily_reset_at, weekly_usage_usd, weekly_reset_at, \
                monthly_usage_usd, monthly_reset_at, daily_limit_usd, \
                funding_source, funding_reference, price_paid_usd, notes, created_at, updated_at) \
         VALUES ($1, 7, 1, 3, '', 'active', NOW(), NOW() + INTERVAL '30 days', \
                 5, {daily_reset}, 5, {weekly_reset}, 5, {monthly_reset}, 100, \
                 '', '', 0, '', NOW(), NOW())"
    ))
    .bind(id)
    .execute(pool)
    .await
    .expect("seeding a subscription");
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_missing_subscription_is_permissive_rather_than_an_error() {
    // The quota system is opt-in; a user without a row bills purely from their
    // balance, so this must not read as "denied".
    let pool = fresh_db("quota_missing").await;
    let rotated = SqlSubscriptionQuotaStore::new(pool.clone())
        .lock_and_rotate(404, Utc::now())
        .await
        .expect("a missing row is not a failure");
    assert_eq!(rotated, None);
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_live_period_is_returned_untouched() {
    let pool = fresh_db("quota_live").await;
    seed_subscription(
        &pool,
        1,
        "NOW() + INTERVAL '1 hour'",
        "NOW() + INTERVAL '2 days'",
        "NOW() + INTERVAL '20 days'",
    )
    .await;

    let quota = SqlSubscriptionQuotaStore::new(pool.clone())
        .lock_and_rotate(1, Utc::now())
        .await
        .expect("locks")
        .expect("the row exists");
    assert_eq!(
        quota.daily_usage_usd, 5.0,
        "a running period keeps its counter"
    );
    assert_eq!(quota.daily_limit_usd, Some(100.0));
    assert_eq!(quota.group_id, 3);
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn an_elapsed_period_is_zeroed_and_the_reset_is_persisted() {
    let pool = fresh_db("quota_rotates").await;
    seed_subscription(
        &pool,
        1,
        "NOW() - INTERVAL '1 hour'", // elapsed
        "NOW() + INTERVAL '2 days'",
        "NOW() + INTERVAL '20 days'",
    )
    .await;
    let store = SqlSubscriptionQuotaStore::new(pool.clone());

    let quota = store
        .lock_and_rotate(1, Utc::now())
        .await
        .expect("locks")
        .expect("the row exists");
    assert_eq!(quota.daily_usage_usd, 0.0, "the elapsed period is zeroed");
    assert_eq!(
        quota.weekly_usage_usd, 5.0,
        "a running period is left alone"
    );

    // The rotation has to survive the transaction, or the next request rotates
    // again and the counter never accumulates.
    let persisted: (gw_model::compat::Money, gw_model::compat::Money) =
        sqlx::query_as("SELECT daily_usage_usd, weekly_usage_usd FROM subscriptions WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("reading back");
    assert_eq!((persisted.0.0, persisted.1.0), (0.0, 5.0));

    let again = store
        .lock_and_rotate(1, Utc::now())
        .await
        .expect("locks")
        .expect("the row exists");
    assert_eq!(
        again.daily_usage_usd, 0.0,
        "a second pass within the same period must be a no-op, not another reset",
    );
    assert!(
        again.daily_reset_at.expect("advanced") > Utc::now(),
        "the new boundary must be in the future",
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_zero_boundary_means_the_period_never_rotates() {
    // The schema stores the zero timestamp for an unconfigured period; treating
    // it as elapsed would zero that counter on every single request.
    let pool = fresh_db("quota_zero_boundary").await;
    seed_subscription(
        &pool,
        1,
        "TIMESTAMPTZ 'epoch'",
        "TIMESTAMPTZ 'epoch'",
        "TIMESTAMPTZ 'epoch'",
    )
    .await;

    let quota = SqlSubscriptionQuotaStore::new(pool.clone())
        .lock_and_rotate(1, Utc::now())
        .await
        .expect("locks")
        .expect("the row exists");
    assert_eq!(quota.daily_usage_usd, 5.0);
    assert_eq!(quota.daily_reset_at, None);
    assert_eq!(
        sqlx::query_as::<_, (gw_model::compat::Money,)>(
            "SELECT daily_usage_usd FROM subscriptions WHERE id = 1"
        )
        .fetch_one(&pool)
        .await
        .expect("reading back")
        .0
        .0,
        5.0,
    );
}
