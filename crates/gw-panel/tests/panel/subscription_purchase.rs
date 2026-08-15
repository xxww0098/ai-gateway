//! 购买订阅：扣款与订阅同生共死。
//!
//! 对应原实现的 purchase_atomicity / purchase_insufficient 测试。
//!
//! 三条不变量，每条都直接对应真金白银：
//!
//! 1. **守恒**：成功时余额恰好减少标价，且订阅存在；
//! 2. **余额不足零写入**：拒绝时既没有 `balance_logs` 也没有订阅；
//! 3. **补偿**：扣款成功但建订阅失败时，同额退回，reference 里嵌着原始扣款串。
//!
//! 第 3 条能被测到，是因为 `purchase_subscription` 把"建订阅"做成了闭包参数 ——
//! 传一个必然失败的闭包比人为制造数据库故障可靠得多。

use crate::common::{
    balance_log_count, balance_log_entries, balance_of, fresh_db, ledger_without_redis, seed_user,
};
use gw_panel::commerce::subscription::{PurchaseError, purchase_subscription};

const PRICE: f64 = 29.9;
const PACKAGE_ID: i64 = 7;

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_successful_purchase_debits_exactly_the_price() {
    let pool = fresh_db("purchase_conservation").await;
    let ledger = ledger_without_redis(&pool);
    let user = seed_user(&pool, "buyer@example.com", 100.0).await;

    let id = purchase_subscription(&ledger, user, PACKAGE_ID, PRICE, || async { Ok(4242) })
        .await
        .expect("purchase");

    assert_eq!(id, 4242);
    let after = balance_of(&pool, user).await;
    assert!(
        (after - (100.0 - PRICE)).abs() < 1e-9,
        "余额应当恰好减少标价：{after}"
    );

    let entries = balance_log_entries(&pool, user).await;
    assert_eq!(entries.len(), 1, "成功路径只该留一条流水：{entries:?}");
    assert!(
        (entries[0].0 + PRICE).abs() < 1e-9,
        "流水金额应当是负的标价"
    );
    assert!(
        entries[0]
            .1
            .starts_with(&format!("subscription_purchase:{PACKAGE_ID}:")),
        "reference 前缀不对：{}",
        entries[0].1
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_unaffordable_purchase_writes_nothing_at_all() {
    // Requirement 5.5：余额不足时既不扣款也不建订阅，连流水都没有。
    let pool = fresh_db("purchase_insufficient").await;
    let ledger = ledger_without_redis(&pool);
    let user = seed_user(&pool, "broke@example.com", 1.0).await;

    let outcome = purchase_subscription(&ledger, user, PACKAGE_ID, PRICE, || async { Ok(1) }).await;

    assert!(matches!(outcome, Err(PurchaseError::InsufficientBalance)));
    assert!(
        (balance_of(&pool, user).await - 1.0).abs() < 1e-9,
        "余额不该动"
    );
    assert_eq!(balance_log_count(&pool, user).await, 0, "不该留下任何流水");
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn a_failed_create_is_compensated_back_to_the_original_balance() {
    let pool = fresh_db("purchase_compensated").await;
    let ledger = ledger_without_redis(&pool);
    let user = seed_user(&pool, "unlucky@example.com", 100.0).await;

    let outcome = purchase_subscription(&ledger, user, PACKAGE_ID, PRICE, || async {
        Err(sqlx::Error::RowNotFound)
    })
    .await;

    match outcome {
        Err(PurchaseError::CreateFailed {
            compensated,
            debit_reference,
        }) => {
            assert!(compensated, "补偿必须成功（账本是健康的）");
            let after = balance_of(&pool, user).await;
            assert!((after - 100.0).abs() < 1e-9, "余额必须回到原值：{after}");

            let entries = balance_log_entries(&pool, user).await;
            assert_eq!(entries.len(), 2, "一扣一退，两条流水：{entries:?}");
            assert!(
                (entries[0].0 + entries[1].0).abs() < 1e-9,
                "两条流水必须相抵"
            );
            assert!(
                entries[1].1.contains(&debit_reference),
                "补偿串里必须嵌着原始扣款串，否则运维无法配对：{}",
                entries[1].1
            );
            assert!(entries[1].1.contains(":compensate:"));
        }
        other => panic!("期望 CreateFailed，得到 {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn two_purchases_by_the_same_user_get_distinguishable_references() {
    // nonce 存在的理由：否则第二次的补偿会配对到第一次的扣款上。
    let pool = fresh_db("purchase_distinct_refs").await;
    let ledger = ledger_without_redis(&pool);
    let user = seed_user(&pool, "repeat@example.com", 100.0).await;

    for _ in 0..2 {
        purchase_subscription(&ledger, user, PACKAGE_ID, PRICE, || async { Ok(1) })
            .await
            .expect("purchase");
    }
    let entries = balance_log_entries(&pool, user).await;
    assert_eq!(entries.len(), 2);
    assert_ne!(
        entries[0].1, entries[1].1,
        "两次扣款的 reference 必须可区分"
    );
}
