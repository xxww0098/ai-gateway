use std::time::Duration;

use super::common::Fixture;
use gw_ledger::LedgerError;

#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn a_fresh_pending_hold_is_invisible_to_a_hold_ttl_scan() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(10.0).await;
    fx.ledger
        .record_pending_hold(user, "req-young", 1.5)
        .await
        .expect("pending");
    let found = fx
        .ledger
        .list_pending_intents(Duration::from_secs(300))
        .await
        .expect("list");
    assert!(found.iter().all(|row| row.request_id != "req-young"));
    fx.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn an_expired_pending_hold_is_charged_once_then_is_a_no_op() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(10.0).await;
    fx.ledger
        .record_pending_hold(user, "req-old", 2.5)
        .await
        .expect("pending");
    sqlx::query(
        "UPDATE settlement_intents SET created_at = NOW() - INTERVAL '10 minutes' WHERE request_id = $1",
    )
    .bind("req-old")
    .execute(&fx.pool)
    .await
    .expect("backdate");

    let found = fx
        .ledger
        .list_pending_intents(Duration::from_secs(300))
        .await
        .expect("list");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].request_id, "req-old");
    assert!((found[0].hold_amount - 2.5).abs() < 1e-9);

    {
        let mut tx = fx.pool.begin().await.expect("tx");
        fx.ledger
            .settle_tx(&mut tx, user, "req-old", 2.5)
            .await
            .expect("settle");
        tx.commit().await.expect("commit");
    }
    assert!((fx.balance(user).await - 7.5).abs() < 1e-9);

    let found = fx
        .ledger
        .list_pending_intents(Duration::from_secs(1))
        .await
        .expect("list after");
    assert!(found.is_empty());

    let mut tx = fx.pool.begin().await.expect("tx2");
    let err = fx
        .ledger
        .settle_tx(&mut tx, user, "req-old", 2.5)
        .await
        .unwrap_err();
    assert!(matches!(err, LedgerError::AlreadySettled));
    tx.rollback().await.expect("rollback");
    assert!((fx.balance(user).await - 7.5).abs() < 1e-9);
    fx.cleanup().await;
}

#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn releasing_a_pending_hold_drops_it_from_recovery() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(10.0).await;
    fx.ledger
        .record_pending_hold(user, "req-rel", 1.0)
        .await
        .expect("pending");
    fx.ledger.release(user, "req-rel").await.expect("release");
    sqlx::query(
        "UPDATE settlement_intents SET created_at = NOW() - INTERVAL '10 minutes' WHERE request_id = $1",
    )
    .bind("req-rel")
    .execute(&fx.pool)
    .await
    .expect("backdate");
    let found = fx
        .ledger
        .list_pending_intents(Duration::from_secs(1))
        .await
        .expect("list");
    assert!(found.is_empty());
    assert!((fx.balance(user).await - 10.0).abs() < 1e-9);
    fx.cleanup().await;
}
