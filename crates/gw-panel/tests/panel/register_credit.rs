//! 注册金：同一用户只入一次，跨用户同字面量合法。

use crate::common::{balance_of, fresh_db, ledger_without_redis, seed_user};
use gw_ledger::LedgerError;

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn initial_register_credit_is_once_per_user() {
    let pool = fresh_db("register_credit_once").await;
    let ledger = ledger_without_redis(&pool);
    let user = seed_user(&pool, "new@example.com", 0.0).await;

    {
        let mut tx = pool.begin().await.expect("tx");
        ledger
            .credit_tx(&mut tx, user, 1.0, "initial_register_credit")
            .await
            .expect("first credit");
        tx.commit().await.expect("commit");
    }
    assert!((balance_of(&pool, user).await - 1.0).abs() < 1e-9);

    let mut tx = pool.begin().await.expect("tx2");
    let err = ledger
        .credit_tx(&mut tx, user, 1.0, "initial_register_credit")
        .await
        .expect_err("second credit");
    let _ = tx.rollback().await;
    assert!(
        matches!(err, LedgerError::Db(_)),
        "same-user second register credit must hit the unique index: {err:?}"
    );
    assert!((balance_of(&pool, user).await - 1.0).abs() < 1e-9);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn two_users_may_share_the_initial_register_credit_literal() {
    let pool = fresh_db("register_credit_two_users").await;
    let ledger = ledger_without_redis(&pool);
    let a = seed_user(&pool, "a@example.com", 0.0).await;
    let b = seed_user(&pool, "b@example.com", 0.0).await;
    for user in [a, b] {
        let mut tx = pool.begin().await.expect("tx");
        ledger
            .credit_tx(&mut tx, user, 1.0, "initial_register_credit")
            .await
            .expect("credit");
        tx.commit().await.expect("commit");
    }
    assert!((balance_of(&pool, a).await - 1.0).abs() < 1e-9);
    assert!((balance_of(&pool, b).await - 1.0).abs() < 1e-9);
}
