//! The atomic settle. These need Postgres because atomicity is precisely what
//! cannot be observed against a fake.

use gw_ledger::NewOperation;

use super::*;
use crate::ports::{UsageLogEntry, UsageStore};
use crate::testsupport::{fresh_db, seed_user};

fn entry(user_id: Id, operation: &BillingOperationId, cost: f64) -> UsageLogEntry {
    UsageLogEntry {
        user_id,
        api_key_id: 1,
        request_id: "trace-the-client-saw".to_owned(),
        event_key: operation.to_string(),
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

fn commit(user_id: Id, operation: &BillingOperationId, cost: f64) -> SettlementCommit {
    SettlementCommit {
        user_id,
        operation: operation.clone(),
        actual_cost: cost,
        entry: entry(user_id, operation, cost),
        subscription_id: None,
    }
}

/// Admits an operation so there is a `held` row for the settle to terminate.
/// Every commit needs one — that row *is* the once-guard.
async fn hold_operation(pool: &sqlx::PgPool, user_id: Id, amount: f64) -> BillingOperationId {
    let ledger = Ledger::new(pool.clone(), None);
    let operation = NewOperation {
        operation_id: BillingOperationId::mint(),
        user_id,
        reserved_amount: amount,
        admitted_liability: amount,
        request_fingerprint: "fingerprint".to_owned(),
        client_trace_id: "trace-the-client-saw".to_owned(),
    };
    ledger
        .begin_operation(&operation)
        .await
        .expect("begin operation");
    operation.operation_id
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
    operation: &BillingOperationId,
) -> Vec<(f64, bool, Option<serde_json::Value>)> {
    sqlx::query_as::<_, (gw_model::compat::Money, bool, Option<serde_json::Value>)>(
        "SELECT actual_cost, failed, raw_metadata FROM usage_logs WHERE event_key = $1",
    )
    .bind(operation.as_str())
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
    let operation = hold_operation(&pool, 7, 5.0).await;

    let receipt = store(&pool)
        .commit_settlement(&commit(7, &operation, 2.5))
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
    assert_eq!(usage_rows(&pool, &operation).await.len(), 1);
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_cost_the_balance_cannot_cover_is_debited_partially_and_recorded_as_debt() {
    // Partial debit, not refusal: the upstream work already happened, so the
    // uncovered part becomes a shortfall the tenant has to clear.
    let pool = fresh_db("settle_shortfall").await;
    seed_user(&pool, 7, 1.0).await;
    let operation = hold_operation(&pool, 7, 5.0).await;

    let receipt = store(&pool)
        .commit_settlement(&commit(7, &operation, 3.0))
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

    let rows = usage_rows(&pool, &operation).await;
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
    let operation = hold_operation(&pool, 7, 5.0).await;

    let mut commit = commit(7, &operation, 3.0);
    commit.entry.raw_metadata =
        crate::usage::settle_annotations(Some(crate::usage::REASON_MISSING_USAGE), 0.0);
    store(&pool)
        .commit_settlement(&commit)
        .await
        .expect("commits");

    let rows = usage_rows(&pool, &operation).await;
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
    // The operation exists; the *user* does not, so the debit is what fails.
    let operation = hold_operation(&pool, 404, 5.0).await;

    let failed = store(&pool)
        .commit_settlement(&commit(404, &operation, 1.0))
        .await;

    assert!(failed.is_err(), "settling against a missing user must fail");
    assert!(
        usage_rows(&pool, &operation).await.is_empty(),
        "the usage row must not survive a rolled-back debit",
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_second_commit_for_one_operation_is_a_no_op() {
    // No caller flag arms this. The operation's terminal state does.
    let pool = fresh_db("settle_idempotent").await;
    seed_user(&pool, 7, 10.0).await;
    let store = store(&pool);
    let operation = hold_operation(&pool, 7, 5.0).await;

    let first = commit(7, &operation, 1.0);
    assert!(matches!(
        store.commit_settlement(&first).await.expect("first run"),
        SettleReceipt::Committed { .. }
    ));

    for _ in 0..5 {
        let again = store.commit_settlement(&first).await.expect("re-run");
        assert_eq!(again, SettleReceipt::AlreadyTerminal);
    }
    assert_eq!(
        balance_of(&pool, 7).await,
        9.0,
        "an orphaned operation must never be charged twice",
    );
    assert_eq!(usage_rows(&pool, &operation).await.len(), 1);
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
    for subscription_id in [1_i64, 2] {
        let operation = hold_operation(&pool, 7, 5.0).await;
        let mut commit = commit(7, &operation, 1.0);
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

    let operation = BillingOperationId::mint();
    let mut failed = entry(7, &operation, 0.0);
    failed.failed = true;
    store(&pool)
        .insert_usage_log(&failed)
        .await
        .expect("inserting a failure row");

    let rows = usage_rows(&pool, &operation).await;
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

    let mut alpha = entry(7, &BillingOperationId::mint(), 0.0);
    alpha.model = "alpha".to_owned();
    alpha.input_tokens = 10;
    alpha.output_tokens = 4;
    let mut alpha_again = entry(7, &BillingOperationId::mint(), 0.0);
    alpha_again.model = "alpha".to_owned();
    alpha_again.input_tokens = 3;
    alpha_again.output_tokens = 1;
    let mut yesterday = entry(7, &BillingOperationId::mint(), 0.0);
    yesterday.model = "beta".to_owned();
    yesterday.input_tokens = 8;
    yesterday.output_tokens = 2;
    // 回拨这一行的时间要挑得中它：`entry()` 给每一行的 `request_id` 都是同一个
    // 常量，所以 `WHERE request_id = 'req-c'` 一行都改不到 —— 那样「昨天」
    // 其实还在今天，断言测的就不是它写的那件事了。
    yesterday.request_id = "req-c".to_owned();
    let mut stranger = entry(8, &BillingOperationId::mint(), 0.0);
    stranger.model = "other".to_owned();
    stranger.input_tokens = 99;
    stranger.output_tokens = 99;

    for row in [&alpha, &alpha_again, &yesterday, &stranger] {
        store.insert_usage_log(row).await.expect("seed usage_logs");
    }
    sqlx::query(
        "UPDATE usage_logs SET created_at = NOW() - INTERVAL '2 days' WHERE request_id = $1",
    )
    .bind(yesterday.request_id.clone())
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

/// **结算把在途预留转成实际用量，两件事在同一个事务里。**
///
/// 只删不加 = 白用一次额度；只加不删 = 在途与实际同时占着额度，
/// 一次请求被算两遍。这条同时钉住两半。
///
/// 订阅 id 取自**预留行自己**（`commit.subscription_id` 这里刻意留空），
/// 因为对账在结算一笔崩溃遗留的操作时并不知道它属于哪个订阅 —— 而那一行知道。
#[tokio::test]
#[ignore = "needs a local Postgres: see testsupport::PG_HOWTO"]
async fn a_settlement_converts_the_quota_reservation_into_actual_usage() {
    let pool = fresh_db("settle_quota_reservation").await;
    seed_user(&pool, 7, 10.0).await;
    sqlx::query(
        "INSERT INTO subscriptions (id, user_id, package_id, group_id, group_name, status, \
                starts_at, expires_at, daily_usage_usd, daily_reset_at, weekly_usage_usd, \
                weekly_reset_at, monthly_usage_usd, monthly_reset_at, funding_source, \
                funding_reference, price_paid_usd, notes, created_at, updated_at) \
         VALUES (1, 7, 1, 1, '', 'active', NOW(), NOW() + INTERVAL '1 day', 0, \
                 NOW() + INTERVAL '1 day', 0, NOW() + INTERVAL '7 days', 0, \
                 NOW() + INTERVAL '30 days', '', '', 0, '', NOW(), NOW())",
    )
    .execute(&pool)
    .await
    .expect("seeding a subscription");

    let operation = hold_operation(&pool, 7, 5.0).await;
    sqlx::query(
        "INSERT INTO quota_reservations \
            (billing_operation_id, subscription_id, reserved_amount, created_at) \
         VALUES ($1, 1, CAST(5 AS numeric), NOW())",
    )
    .bind(operation.as_str())
    .execute(&pool)
    .await
    .expect("seeding a reservation");

    // `subscription_id` 留空：转账要靠预留行自己认领订阅。
    store(&pool)
        .commit_settlement(&commit(7, &operation, 1.25))
        .await
        .expect("commits");

    let left: (i64,) = sqlx::query_as("SELECT COUNT(*)::bigint FROM quota_reservations")
        .fetch_one(&pool)
        .await
        .expect("counting reservations");
    assert_eq!(left.0, 0, "结算之后预留必须消失，否则额度被算两遍");

    let used: (
        gw_model::compat::Money,
        gw_model::compat::Money,
        gw_model::compat::Money,
    ) = sqlx::query_as(
        "SELECT daily_usage_usd, weekly_usage_usd, monthly_usage_usd \
         FROM subscriptions WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("reading counters");
    assert_eq!(
        (used.0.0, used.1.0, used.2.0),
        (1.25, 1.25, 1.25),
        "三个周期计数器都要加上**实际**扣款，不是预留的那个上限",
    );
}
