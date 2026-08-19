//! The atomic settle. These need Postgres because atomicity is precisely what
//! cannot be observed against a fake.

use super::*;
use crate::ports::{UsageLogEntry, UsageStore};
use crate::testsupport::{fresh_db, seed_user};

fn entry(user_id: Id, request_id: &str, cost: f64) -> UsageLogEntry {
    UsageLogEntry {
        user_id,
        api_key_id: 1,
        request_id: request_id.to_owned(),
        model: "gpt-4o".to_owned(),
        provider: "openai".to_owned(),
        input_tokens: 100,
        output_tokens: 200,
        total_cost: cost,
        actual_cost: cost,
        cost,
        rate_multiplier: 1.0,
        ..UsageLogEntry::default()
    }
}

fn commit(user_id: Id, request_id: &str, cost: f64) -> SettlementCommit {
    SettlementCommit {
        user_id,
        request_id: request_id.to_owned(),
        actual_cost: cost,
        entry: entry(user_id, request_id, cost),
        subscription_id: None,
        skip_if_already_logged: false,
    }
}

/// A store over a ledger with no Redis: `settle_tx` is pure Postgres, and the
/// reservation side is `gw-ledger`'s own business.
fn store(pool: &sqlx::PgPool) -> SqlUsageStore {
    SqlUsageStore::new(pool.clone(), Arc::new(Ledger::new(pool.clone(), None)))
}

/// Money columns are `numeric`, so they decode through `compat::Money`.
async fn balance_of(pool: &sqlx::PgPool, user_id: Id) -> f64 {
    sqlx::query_as::<_, (gw_model::compat::Money,)>("SELECT balance FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .expect("reading the balance")
        .0
        .0
}

async fn usage_rows(
    pool: &sqlx::PgPool,
    request_id: &str,
) -> Vec<(f64, bool, Option<serde_json::Value>)> {
    sqlx::query_as::<_, (gw_model::compat::Money, bool, Option<serde_json::Value>)>(
        "SELECT actual_cost, failed, raw_metadata FROM usage_logs WHERE request_id = $1",
    )
    .bind(request_id)
    .fetch_all(pool)
    .await
    .expect("reading usage_logs")
    .into_iter()
    .map(|(cost, failed, metadata)| (cost.0, failed, metadata))
    .collect()
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_settlement_debits_and_logs_in_one_go() {
    let pool = fresh_db("settle_commits").await;
    seed_user(&pool, 7, 10.0).await;

    let receipt = store(&pool)
        .commit_settlement(&commit(7, "req-1", 2.5))
        .await
        .expect("the settlement commits");

    let SettleReceipt::Committed {
        shortfall,
        balance_before,
        balance_after,
    } = receipt
    else {
        panic!("expected a commit, got {receipt:?}");
    };
    assert_eq!(shortfall, 0.0, "a covered cost leaves no debt");
    assert_eq!(balance_before, 10.0);
    assert_eq!(balance_after, 7.5);
    assert_eq!(balance_of(&pool, 7).await, 7.5);
    assert_eq!(usage_rows(&pool, "req-1").await.len(), 1);
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_cost_the_balance_cannot_cover_is_debited_partially_and_recorded_as_debt() {
    // Partial debit, not refusal: the upstream work already happened, so the
    // uncovered part becomes a shortfall the tenant has to clear.
    let pool = fresh_db("settle_shortfall").await;
    seed_user(&pool, 7, 1.0).await;

    let receipt = store(&pool)
        .commit_settlement(&commit(7, "req-1", 3.0))
        .await
        .expect("the settlement commits");

    let SettleReceipt::Committed { shortfall, .. } = receipt else {
        panic!("expected a commit");
    };
    assert!((shortfall - 2.0).abs() < 1e-9, "got {shortfall}");
    assert_eq!(
        balance_of(&pool, 7).await,
        0.0,
        "the balance is drained, not negative"
    );

    let rows = usage_rows(&pool, "req-1").await;
    let metadata = rows[0].2.clone().expect("the shortfall must be annotated");
    assert_eq!(
        metadata["shortfall_usd"].as_f64(),
        Some(shortfall),
        "reporting has to tell a partially-paid request from a free one",
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_fallback_tag_survives_alongside_the_shortfall() {
    let pool = fresh_db("settle_fallback_tag").await;
    seed_user(&pool, 7, 1.0).await;

    let mut commit = commit(7, "req-1", 3.0);
    commit.entry.raw_metadata =
        crate::usage::settle_annotations(Some(crate::usage::REASON_MISSING_USAGE), 0.0);
    store(&pool)
        .commit_settlement(&commit)
        .await
        .expect("commits");

    let rows = usage_rows(&pool, "req-1").await;
    let metadata = rows[0].2.clone().expect("annotated");
    assert_eq!(
        metadata["billing_fallback"]["reason"].as_str(),
        Some(crate::usage::REASON_MISSING_USAGE),
        "merging the shortfall must not clobber the fallback tag",
    );
    assert!(metadata["shortfall_usd"].as_f64().is_some());
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_settlement_for_an_unknown_user_writes_nothing_at_all() {
    // The debit fails, so the usage row must roll back with it — a row without
    // a matching debit is the divergence this transaction exists to prevent.
    let pool = fresh_db("settle_rollback").await;

    let failed = store(&pool)
        .commit_settlement(&commit(404, "req-1", 1.0))
        .await;

    assert!(failed.is_err(), "settling against a missing user must fail");
    assert!(
        usage_rows(&pool, "req-1").await.is_empty(),
        "the usage row must not survive a rolled-back debit",
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn the_reconcile_guard_makes_a_second_run_a_no_op() {
    let pool = fresh_db("settle_idempotent").await;
    seed_user(&pool, 7, 10.0).await;
    let store = store(&pool);

    let mut first = commit(7, "req-1", 1.0);
    first.skip_if_already_logged = true;
    assert!(matches!(
        store.commit_settlement(&first).await.expect("first run"),
        SettleReceipt::Committed { .. }
    ));

    let second = store.commit_settlement(&first).await.expect("second run");
    assert_eq!(second, SettleReceipt::AlreadySettled);
    assert_eq!(
        balance_of(&pool, 7).await,
        9.0,
        "an orphaned hold must never be charged twice",
    );
    assert_eq!(usage_rows(&pool, "req-1").await.len(), 1);
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_live_subscription_accumulates_and_a_lapsed_one_does_not() {
    let pool = fresh_db("settle_subscription").await;
    seed_user(&pool, 7, 10.0).await;

    for (id, expires) in [
        (1_i64, "NOW() + INTERVAL '1 day'"),
        (2, "NOW() - INTERVAL '1 day'"),
    ] {
        sqlx::query(&format!(
            "INSERT INTO subscriptions (id, user_id, package_id, group_id, group_name, status, \
                    starts_at, expires_at, daily_usage_usd, daily_reset_at, weekly_usage_usd, \
                    weekly_reset_at, monthly_usage_usd, monthly_reset_at, funding_source, \
                    funding_reference, price_paid_usd, notes, created_at, updated_at) \
             VALUES ($1, 7, 1, 1, '', 'active', NOW(), {expires}, 0, NOW() + INTERVAL '1 day', \
                     0, NOW() + INTERVAL '7 days', 0, NOW() + INTERVAL '30 days', '', '', 0, '', \
                     NOW(), NOW())"
        ))
        .bind(id)
        .execute(&pool)
        .await
        .expect("seeding a subscription");
    }

    let store = store(&pool);
    for (subscription_id, request_id) in [(1_i64, "req-live"), (2, "req-lapsed")] {
        let mut commit = commit(7, request_id, 1.0);
        commit.subscription_id = Some(subscription_id);
        store.commit_settlement(&commit).await.expect("commits");
    }

    let used: Vec<(i64, gw_model::compat::Money)> =
        sqlx::query_as("SELECT id, daily_usage_usd FROM subscriptions ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("reading counters");
    assert_eq!(used[0].1.0, 1.0, "the live subscription accumulates");
    assert_eq!(
        used[1].1.0, 0.0,
        "an expired subscription must not accumulate — the predicate, not a prior read, is what excludes it",
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_standalone_usage_row_lands_outside_any_transaction() {
    // The failure paths write through this one, after their transaction died.
    let pool = fresh_db("settle_standalone_log").await;
    seed_user(&pool, 7, 10.0).await;

    let mut failed = entry(7, "req-1", 0.0);
    failed.failed = true;
    store(&pool)
        .insert_usage_log(&failed)
        .await
        .expect("inserting a failure row");

    let rows = usage_rows(&pool, "req-1").await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].1, "the row must record that it failed");
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_balance_event_lands_with_its_reference() {
    let pool = fresh_db("settle_balance_event").await;
    seed_user(&pool, 7, 10.0).await;

    store(&pool)
        .insert_balance_event(&BalanceEvent {
            user_id: 7,
            amount: 0.0,
            event_type: "balance_depleted".to_owned(),
            reference: "req-1".to_owned(),
            metadata: serde_json::json!({"current_balance": 0.0}),
        })
        .await
        .expect("inserting the event");

    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT type, reference FROM balance_logs WHERE user_id = 7")
            .fetch_all(&pool)
            .await
            .expect("reading balance_logs");
    assert_eq!(rows, [("balance_depleted".to_owned(), "req-1".to_owned())]);
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn model_usage_since_sums_today_and_ignores_other_users_and_older_rows() {
    let pool = fresh_db("usage_model_tokens").await;
    seed_user(&pool, 7, 10.0).await;
    seed_user(&pool, 8, 10.0).await;
    let store = store(&pool);

    let mut alpha = entry(7, "req-a", 0.0);
    alpha.model = "alpha".to_owned();
    alpha.input_tokens = 10;
    alpha.output_tokens = 4;
    let mut alpha_again = entry(7, "req-b", 0.0);
    alpha_again.model = "alpha".to_owned();
    alpha_again.input_tokens = 3;
    alpha_again.output_tokens = 1;
    let mut yesterday = entry(7, "req-c", 0.0);
    yesterday.model = "beta".to_owned();
    yesterday.input_tokens = 8;
    yesterday.output_tokens = 2;
    let mut stranger = entry(8, "req-d", 0.0);
    stranger.model = "other".to_owned();
    stranger.input_tokens = 99;
    stranger.output_tokens = 99;

    for row in [&alpha, &alpha_again, &yesterday, &stranger] {
        store.insert_usage_log(row).await.expect("seed usage_logs");
    }
    sqlx::query(
        "UPDATE usage_logs SET created_at = NOW() - INTERVAL '2 days' WHERE request_id = $1",
    )
    .bind("req-c")
    .execute(&pool)
    .await
    .expect("backdate yesterday");

    let since = chrono::Utc::now() - chrono::Duration::hours(1);
    let rows = store
        .model_usage_since(7, since)
        .await
        .expect("aggregate today");
    assert_eq!(rows.len(), 1, "昨天的 beta 和别人的 other 都必须排除");
    assert_eq!(rows[0].model, alpha.model);
    assert_eq!(
        rows[0].tokens_in,
        alpha.input_tokens + alpha_again.input_tokens
    );
    assert_eq!(
        rows[0].tokens_out,
        alpha.output_tokens + alpha_again.output_tokens
    );
    assert_eq!(rows[0].requests, 2);
}
