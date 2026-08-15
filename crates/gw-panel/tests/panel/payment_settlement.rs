//! 充值单结算：重复确认只入账一次。
//!
//! 对应原实现的 payment_order 测试。人工确认和 Stripe 回调走的是同一个
//! `settle_payment_order`，所以这里验的其实是**两条外部触发路径共用的那把锁**。

use crate::common::{
    balance_log_count, balance_of, fresh_db, ledger_without_redis, order_status,
    seed_payment_order, seed_user,
};
use gw_panel::commerce::payment::settle_payment_order;

const TOP_UP: f64 = 25.0;

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn the_first_settlement_credits_and_marks_the_order_paid() {
    let pool = fresh_db("settle_first").await;
    let ledger = ledger_without_redis(&pool);
    let user = seed_user(&pool, "topup@example.com", 0.0).await;
    let order = seed_payment_order(&pool, user, TOP_UP).await;

    let credited = settle_payment_order(&pool, &ledger, order)
        .await
        .expect("settle");

    assert!(credited, "第一次结算必须真的入账");
    assert_eq!(order_status(&pool, order).await, "paid");
    assert!((balance_of(&pool, user).await - TOP_UP).abs() < 1e-9);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn repeated_settlement_never_double_credits() {
    // 这是整条路径存在的理由：Stripe 会重投，管理员会重复点确认。
    let pool = fresh_db("settle_idempotent").await;
    let ledger = ledger_without_redis(&pool);
    let user = seed_user(&pool, "topup@example.com", 0.0).await;
    let order = seed_payment_order(&pool, user, TOP_UP).await;

    let mut credited_times = 0;
    for _ in 0..5 {
        if settle_payment_order(&pool, &ledger, order)
            .await
            .expect("settle")
        {
            credited_times += 1;
        }
    }

    assert_eq!(credited_times, 1, "五次调用里只有一次能入账");
    assert!((balance_of(&pool, user).await - TOP_UP).abs() < 1e-9);
    assert_eq!(balance_log_count(&pool, user).await, 1, "只该有一条流水");
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn concurrent_settlement_of_one_order_credits_exactly_once() {
    // 条件 UPDATE 的意义在并发下才显出来：先读后写在这里会双倍入账。
    let pool = fresh_db("settle_concurrent").await;
    let ledger = ledger_without_redis(&pool);
    let user = seed_user(&pool, "topup@example.com", 0.0).await;
    let order = seed_payment_order(&pool, user, TOP_UP).await;

    let mut handles = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        let ledger = ledger.clone();
        handles.push(tokio::spawn(async move {
            settle_payment_order(&pool, &ledger, order).await
        }));
    }
    let mut winners = 0;
    for handle in handles {
        if handle.await.expect("join").expect("settle") {
            winners += 1;
        }
    }

    assert_eq!(winners, 1, "并发下也只能有一个赢家");
    assert!((balance_of(&pool, user).await - TOP_UP).abs() < 1e-9);
    assert_eq!(balance_log_count(&pool, user).await, 1);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn settling_an_unknown_order_is_a_no_op_rather_than_an_error() {
    // 回调可能引用一张已经被清理掉的单；那不该变成 500 让 Stripe 无限重投。
    let pool = fresh_db("settle_unknown").await;
    let ledger = ledger_without_redis(&pool);
    let credited = settle_payment_order(&pool, &ledger, 987_654)
        .await
        .expect("settle");
    assert!(!credited);
}
