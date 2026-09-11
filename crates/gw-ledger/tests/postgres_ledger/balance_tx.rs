//! Submodule testing transactional credit and debit operations (credit_tx and debit_tx)
//! within caller-managed Postgres transactions, partial unique index enforcement,
//! and idempotency of payment and service-surface credits.

use std::sync::Arc;

use gw_ledger::{BalanceChange, LedgerError};
use tokio::sync::Barrier;

use super::approx;
use super::common::Fixture;

/// credit_tx 和 debit_tx 在调用方事务中执行：调用方回滚时两者和伴随写一并撤销；
/// 调用方提交时两者和伴随写一并持久化。
#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn credit_tx_and_debit_tx_with_caller_rollback_and_companion_write() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(10.0).await;

    // 1. 回滚分支：credit_tx + 伴随写回滚后全部消失
    {
        let mut tx = fx.pool.begin().await.expect("begin tx");
        let change = fx
            .ledger
            .credit_tx(&mut tx, user, 5.0, "ordinary-credit-rollback")
            .await
            .expect("credit_tx");
        assert!(
            matches!(change, BalanceChange::Applied { balance_after, .. } if approx(balance_after, 15.0))
        );

        sqlx::query(
            "INSERT INTO balance_logs (user_id, amount, type, reference, created_at)              VALUES ($1, 0, 'credit', 'companion-rollback', NOW())",
        )
        .bind(user)
        .execute(&mut *tx)
        .await
        .expect("companion write");

        tx.rollback().await.expect("rollback tx");
    }

    assert!(
        approx(fx.balance(user).await, 10.0),
        "回滚后余额必须恢复为 10.0"
    );
    assert!(
        fx.logs_for(user, "ordinary-credit-rollback")
            .await
            .is_empty()
    );
    assert!(fx.logs_for(user, "companion-rollback").await.is_empty());

    // 2. 提交分支：credit_tx 与 debit_tx 以及伴随写一并提交
    {
        let mut tx = fx.pool.begin().await.expect("begin tx");

        let c1 = fx
            .ledger
            .credit_tx(&mut tx, user, 10.0, "ordinary-credit-commit")
            .await
            .expect("credit_tx");
        assert!(
            matches!(c1, BalanceChange::Applied { balance_after, .. } if approx(balance_after, 20.0))
        );

        let d1 = fx
            .ledger
            .debit_tx(&mut tx, user, 4.0, "ordinary-debit-commit")
            .await
            .expect("debit_tx");
        assert!(
            matches!(d1, BalanceChange::Applied { balance_after, .. } if approx(balance_after, 16.0))
        );

        sqlx::query(
            "INSERT INTO balance_logs (user_id, amount, type, reference, created_at)              VALUES ($1, 0, 'credit', 'companion-commit', NOW())",
        )
        .bind(user)
        .execute(&mut *tx)
        .await
        .expect("companion write");

        tx.commit().await.expect("commit tx");
    }

    assert!(approx(fx.balance(user).await, 16.0), "提交后余额应为 16.0");
    assert_eq!(fx.logs_for(user, "ordinary-credit-commit").await.len(), 1);
    assert_eq!(fx.logs_for(user, "ordinary-debit-commit").await.len(), 1);
    assert_eq!(fx.logs_for(user, "companion-commit").await.len(), 1);

    fx.cleanup().await;
}

/// debit_tx 遇到余额不足时报错 InsufficientBalance 并保��余额不变，
/// 调用方事务回滚不留任何残留。
#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn debit_tx_insufficient_balance_causes_caller_rollback() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(5.0).await;

    {
        let mut tx = fx.pool.begin().await.expect("begin tx");

        let d1 = fx
            .ledger
            .debit_tx(&mut tx, user, 2.0, "purchase-part-1")
            .await
            .expect("first debit_tx");
        assert!(
            matches!(d1, BalanceChange::Applied { balance_after, .. } if approx(balance_after, 3.0))
        );

        let err = fx
            .ledger
            .debit_tx(&mut tx, user, 3.01, "purchase-part-2")
            .await
            .unwrap_err();
        assert!(matches!(err, LedgerError::InsufficientBalance));

        tx.rollback().await.expect("rollback on failure");
    }

    assert!(
        approx(fx.balance(user).await, 5.0),
        "事务回滚后余额必须仍为 5.0"
    );
    assert!(fx.logs_for(user, "purchase-part-1").await.is_empty());
    assert!(fx.logs_for(user, "purchase-part-2").await.is_empty());

    fx.cleanup().await;
}

/// 相同 payment_order reference 在串行与并发调用下，第二次及后续调用均返回 AlreadyApplied，
/// 余额不被重复累加，流水仅保留唯一一条。
#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn same_payment_ref_returns_already_applied_sequentially_and_concurrently() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(10.0).await;
    let payment_ref = "payment_order:88801";

    // 1. 串行调用：第一次 Applied，第二次 AlreadyApplied
    {
        let mut tx1 = fx.pool.begin().await.expect("begin tx1");
        let res1 = fx
            .ledger
            .credit_tx(&mut tx1, user, 25.0, payment_ref)
            .await
            .expect("tx1 credit_tx");
        assert!(
            matches!(res1, BalanceChange::Applied { balance_after, .. } if approx(balance_after, 35.0))
        );
        tx1.commit().await.expect("commit tx1");
    }

    {
        let mut tx2 = fx.pool.begin().await.expect("begin tx2");
        let res2 = fx
            .ledger
            .credit_tx(&mut tx2, user, 25.0, payment_ref)
            .await
            .expect("tx2 credit_tx");
        assert!(matches!(res2, BalanceChange::AlreadyApplied));
        tx2.commit().await.expect("commit tx2");
    }

    assert!(
        approx(fx.balance(user).await, 35.0),
        "重复相同 payment_order reference 不得重复增加余额"
    );
    let logs = fx.logs_for(user, payment_ref).await;
    assert_eq!(
        logs.len(),
        1,
        "同一 payment_order reference 必须仅存一条流水"
    );
    assert!(approx(logs[0].1, 25.0));

    // 2. 并发调用：8 路任务通过 Barrier 对同一个 payment_order reference 发起 credit_tx
    let concurrent_ref = "payment_order:88802";
    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();

    for _ in 0..8 {
        let pool = fx.pool.clone();
        let ledger = fx.ledger.clone();
        let b = barrier.clone();
        let r = concurrent_ref.to_string();

        handles.push(tokio::spawn(async move {
            b.wait().await;
            let mut tx = pool.begin().await.expect("begin tx");
            let change = ledger
                .credit_tx(&mut tx, user, 50.0, &r)
                .await
                .expect("concurrent credit_tx");
            tx.commit().await.expect("commit concurrent tx");
            change
        }));
    }

    let mut applied_count = 0;
    let mut already_applied_count = 0;
    for h in handles {
        match h.await.expect("join") {
            BalanceChange::Applied { balance_after, .. } => {
                applied_count += 1;
                assert!(approx(balance_after, 85.0)); // 35.0 + 50.0
            }
            BalanceChange::AlreadyApplied => {
                already_applied_count += 1;
            }
        }
    }

    assert_eq!(applied_count, 1, "并发 8 路中必须有且仅有 1 路 Applied");
    assert_eq!(
        already_applied_count, 7,
        "并发 8 路中其余 7 路必须为 AlreadyApplied"
    );
    assert!(
        approx(fx.balance(user).await, 85.0),
        "并发结束后余额必须为 85.0"
    );
    assert_eq!(fx.logs_for(user, concurrent_ref).await.len(), 1);

    fx.cleanup().await;
}

/// 非 payment_order 前缀的普通 reference（如日常月度发放、注册充值、兑换等）
/// 保留原有的重复写入能力，跨用户和同用户多次写入均正常 Applied。
#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn ordinary_same_refs_preserve_legacy_repeated_credits_and_debits_including_across_users() {
    let mut fx = Fixture::postgres_only().await;
    let user1 = fx.seed_user(0.0).await;
    let user2 = fx.seed_user(0.0).await;

    let ordinary_ref = "system_monthly_grant_202501";

    // 跨不同用户使用相同的非 payment_order reference
    {
        let mut tx1 = fx.pool.begin().await.expect("begin tx1");
        let c1 = fx
            .ledger
            .credit_tx(&mut tx1, user1, 10.0, ordinary_ref)
            .await
            .expect("credit user1");
        assert!(
            matches!(c1, BalanceChange::Applied { balance_after, .. } if approx(balance_after, 10.0))
        );
        tx1.commit().await.expect("commit tx1");

        let mut tx2 = fx.pool.begin().await.expect("begin tx2");
        let c2 = fx
            .ledger
            .credit_tx(&mut tx2, user2, 10.0, ordinary_ref)
            .await
            .expect("credit user2");
        assert!(
            matches!(c2, BalanceChange::Applied { balance_after, .. } if approx(balance_after, 10.0))
        );
        tx2.commit().await.expect("commit tx2");
    }

    // 同一用户使用相同普通 reference 重复 credit：均正常 Applied
    {
        let mut tx3 = fx.pool.begin().await.expect("begin tx3");
        let c3 = fx
            .ledger
            .credit_tx(&mut tx3, user1, 10.0, ordinary_ref)
            .await
            .expect("repeat credit user1");
        assert!(
            matches!(c3, BalanceChange::Applied { balance_after, .. } if approx(balance_after, 20.0))
        );
        tx3.commit().await.expect("commit tx3");
    }

    assert!(approx(fx.balance(user1).await, 20.0));
    assert!(approx(fx.balance(user2).await, 10.0));
    assert_eq!(fx.logs_for(user1, ordinary_ref).await.len(), 2);
    assert_eq!(fx.logs_for(user2, ordinary_ref).await.len(), 1);

    // 同一用户使用相同普通 reference 重复 debit
    let debit_ref = "usage_bucket_ref_001";
    {
        let mut tx = fx.pool.begin().await.expect("begin tx");
        let d1 = fx
            .ledger
            .debit_tx(&mut tx, user1, 3.0, debit_ref)
            .await
            .expect("debit 1");
        assert!(
            matches!(d1, BalanceChange::Applied { balance_after, .. } if approx(balance_after, 17.0))
        );

        let d2 = fx
            .ledger
            .debit_tx(&mut tx, user1, 2.0, debit_ref)
            .await
            .expect("debit 2");
        assert!(
            matches!(d2, BalanceChange::Applied { balance_after, .. } if approx(balance_after, 15.0))
        );
        tx.commit().await.expect("commit tx");
    }

    assert!(approx(fx.balance(user1).await, 15.0));
    assert_eq!(fx.logs_for(user1, debit_ref).await.len(), 2);

    fx.cleanup().await;
}

/// 部分唯一索引 (0010_control_plane_credit_unique.sql) 在底层拒绝 raw duplicate payment credit (23505)，
/// 使用 ESCAPE '!' 避免 '_' 通配符匹配到其他命名空间（如 paymentXorder:），
/// 允许不同 type（如 settle）或非 payment_order 前缀的流水重复存在。
#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn partial_unique_index_rejects_raw_duplicate_payment_credit_but_allows_other_refs() {
    let mut fx = Fixture::postgres_only().await;
    let user1 = fx.seed_user(0.0).await;
    let user2 = fx.seed_user(0.0).await;

    // 确保部分唯一索引已就绪，使用 ESCAPE '!'
    sqlx::raw_sql(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS idx_balance_logs_payment_order_credit
            ON balance_logs (reference)
            WHERE type = 'credit' AND reference LIKE 'payment!_order:%' ESCAPE '!';
        "#,
    )
    .execute(&fx.pool)
    .await
    .expect("create partial unique index");

    let p_ref = "payment_order:99901";

    // 1. 第一条 payment_order credit 正常写入
    sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at)          VALUES ($1, 10.0, 'credit', $2, NOW())",
    )
    .bind(user1)
    .bind(p_ref)
    .execute(&fx.pool)
    .await
    .expect("insert first payment credit");

    // 2. 第二条相同 payment_order credit 必被唯一索引拒绝 (23505)
    let err = sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at)          VALUES ($1, 10.0, 'credit', $2, NOW())",
    )
    .bind(user2)
    .bind(p_ref)
    .execute(&fx.pool)
    .await
    .unwrap_err();

    let db_err = err.as_database_error().expect("must be a database error");
    assert_eq!(
        db_err.code().as_deref(),
        Some("23505"),
        "必须命中 PostgreSQL 唯一索引冲突 (23505): {err}"
    );

    // 3. 通配符回归验证：因使用 ESCAPE '!' 转义，'paymentXorder:123' 不被索引捕获，允许重复写入
    let wildcard_ref = "paymentXorder:123";
    sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at)          VALUES ($1, 5.0, 'credit', $2, NOW())",
    )
    .bind(user1)
    .bind(wildcard_ref)
    .execute(&fx.pool)
    .await
    .expect("first paymentXorder credit");

    sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at)          VALUES ($1, 5.0, 'credit', $2, NOW())",
    )
    .bind(user2)
    .bind(wildcard_ref)
    .execute(&fx.pool)
    .await
    .expect("second paymentXorder credit must be allowed because underscore is escaped");

    // 4. 相同 reference 但不同 type（如 settle）允许并存
    sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at)          VALUES ($1, -1.0, 'settle', $2, NOW())",
    )
    .bind(user1)
    .bind(p_ref)
    .execute(&fx.pool)
    .await
    .expect("settle row with same reference is allowed");

    // 5. 非 payment_order 前缀的 credit 允许相同 reference 多次写入
    let other_ref = "initial_register_credit";
    sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at)          VALUES ($1, 5.0, 'credit', $2, NOW())",
    )
    .bind(user1)
    .bind(other_ref)
    .execute(&fx.pool)
    .await
    .expect("insert non-payment credit 1");

    sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at)          VALUES ($1, 5.0, 'credit', $2, NOW())",
    )
    .bind(user2)
    .bind(other_ref)
    .execute(&fx.pool)
    .await
    .expect("insert non-payment credit 2 for another user");

    fx.cleanup().await;
}

/// 当存在历史重复的 payment credit 脏数据时，执行实际的生产迁移脚本必须失败 (fail-closed)，
/// 从而阻止上线迁移破坏或静默删除资金记录。
#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn dirty_historical_duplicate_credits_block_unique_index_creation() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(0.0).await;

    // 先删除已有的部分唯一索引以模拟迁移前历史环境
    sqlx::raw_sql("DROP INDEX IF EXISTS idx_balance_logs_payment_order_credit")
        .execute(&fx.pool)
        .await
        .expect("drop index for dirty data simulation");

    let dup_ref = "payment_order:dirty_history_01";

    // 插入两条历史重复 credit 流���
    sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at)          VALUES ($1, 20.0, 'credit', $2, NOW())",
    )
    .bind(user)
    .bind(dup_ref)
    .execute(&fx.pool)
    .await
    .expect("insert dirty row 1");

    sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at)          VALUES ($1, 20.0, 'credit', $2, NOW())",
    )
    .bind(user)
    .bind(dup_ref)
    .execute(&fx.pool)
    .await
    .expect("insert dirty row 2");

    // 执行真实的生产迁移脚本必须报错失败
    let err = sqlx::raw_sql(include_str!(
        "../../../../migrations/0010_control_plane_credit_unique.sql"
    ))
    .execute(&fx.pool)
    .await
    .unwrap_err();

    let db_err = err.as_database_error().expect("must be a database error");
    assert_eq!(
        db_err.code().as_deref(),
        Some("23505"),
        "脏历史数据存在时执行迁移脚本必须失败报错: {err}"
    );

    fx.cleanup().await;
}

/// P2 retry bug regression: 浮点精度舍入与 numeric 比较。
///
/// 0.1 + 0.2 在 IEEE-754 中为 0.30000000000000004。
/// 首次写入时 Postgres float8->numeric 将其持久化为 0.3。
/// 相同请求带着相同的 0.1 + 0.2 重试时，credit_tx 必须正确识别为 AlreadyApplied，
/// 余额与流水严格保持为持久化的规范值 0.3（不得因浮点不精确导致拒绝重试）。
/// 不同的金额 0.4 仍必须报错且不改变任何状态。
#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn payment_retry_uses_persisted_numeric_amount() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(0.0).await;
    let payment_ref = "payment_order:p2_numeric_precision_retry";

    let retry_amount = 0.1_f64 + 0.2_f64; // 0.30000000000000004
    let persisted_spec_amount = 0.3_f64;

    // 1. 首次 credit_tx 提交
    {
        let mut tx = fx.pool.begin().await.expect("begin tx1");
        let change = fx
            .ledger
            .credit_tx(&mut tx, user, retry_amount, payment_ref)
            .await
            .expect("first credit_tx");
        assert!(matches!(
            change,
            BalanceChange::Applied { balance_after, .. } if approx(balance_after, persisted_spec_amount)
        ));
        tx.commit().await.expect("commit tx1");
    }

    // 验证库内持久化的规范余额与流水
    assert!(
        approx(fx.balance(user).await, persisted_spec_amount),
        "首次入账后余额必须为规范值 0.3"
    );
    let logs1 = fx.logs_for(user, payment_ref).await;
    assert_eq!(logs1.len(), 1);
    assert!(approx(logs1[0].1, persisted_spec_amount));

    // 2. 相同入参 (0.1 + 0.2) 重试：必须返回 AlreadyApplied
    {
        let mut tx = fx.pool.begin().await.expect("begin tx2");
        let retry_change = fx
            .ledger
            .credit_tx(&mut tx, user, retry_amount, payment_ref)
            .await
            .expect("identical retry must return AlreadyApplied");
        assert!(
            matches!(retry_change, BalanceChange::AlreadyApplied),
            "相同浮点累加金额重试必须返回 AlreadyApplied"
        );
        tx.commit().await.expect("commit tx2");
    }

    // 重试后余额与流水保持为一条 0.3
    assert!(
        approx(fx.balance(user).await, persisted_spec_amount),
        "重试后余额仍为规范值 0.3"
    );
    let logs2 = fx.logs_for(user, payment_ref).await;
    assert_eq!(logs2.len(), 1, "流水数保持一条");
    assert!(approx(logs2[0].1, persisted_spec_amount));

    // 3. 不同的金额 0.4 仍必须报错 fail-closed 且不产生任何状态变更
    {
        let mut tx = fx.pool.begin().await.expect("begin tx3");
        let distinct_err = fx
            .ledger
            .credit_tx(&mut tx, user, 0.4_f64, payment_ref)
            .await
            .unwrap_err();
        assert!(
            matches!(distinct_err, LedgerError::InvalidArgument(_)),
            "金额不匹配必须 fail-closed 报错: {distinct_err:?}"
        );
        tx.rollback().await.expect("rollback tx3");
    }

    assert!(
        approx(fx.balance(user).await, persisted_spec_amount),
        "报错后余额保持 0.3 不变"
    );
    assert_eq!(fx.logs_for(user, payment_ref).await.len(), 1);

    fx.cleanup().await;
}

/// 服务面（ozon-pod 契约 §4）的入账幂等：同一 `service_credit:` reference 重放返回
/// AlreadyApplied 且只留一条流水；换金额或换 user 必须 fail-closed，绝不静默改账。
#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn service_credit_is_idempotent_and_conflicts_fail_closed() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(0.0).await;
    let other = fx.seed_user(0.0).await;
    let reference = "service_credit:ozon-order:PO-1";

    // 首次入账：真扣一次，返回计算后的余额。
    {
        let mut tx = fx.pool.begin().await.expect("begin tx1");
        let change = fx
            .ledger
            .credit_tx(&mut tx, user, 10.0, reference)
            .await
            .expect("first service credit");
        assert!(matches!(
            change,
            BalanceChange::Applied { balance_after, .. } if approx(balance_after, 10.0)
        ));
        tx.commit().await.expect("commit tx1");
    }

    // 超时重放：同 reference + 同 user + 同金额 → AlreadyApplied，余额不动。
    {
        let mut tx = fx.pool.begin().await.expect("begin tx2");
        let change = fx
            .ledger
            .credit_tx(&mut tx, user, 10.0, reference)
            .await
            .expect("replayed service credit");
        assert!(matches!(change, BalanceChange::AlreadyApplied));
        tx.commit().await.expect("commit tx2");
    }
    assert!(approx(fx.balance(user).await, 10.0));
    assert_eq!(fx.logs_for(user, reference).await.len(), 1);

    // 同 reference 不同金额：硬错误，且什么都没写。
    {
        let mut tx = fx.pool.begin().await.expect("begin tx3");
        let err = fx
            .ledger
            .credit_tx(&mut tx, user, 25.0, reference)
            .await
            .unwrap_err();
        assert!(matches!(err, LedgerError::InvalidArgument(_)), "{err:?}");
        tx.rollback().await.expect("rollback tx3");
    }

    // 同 reference 不同 user：一样硬错误，另一个人一分钱都不进账。
    {
        let mut tx = fx.pool.begin().await.expect("begin tx4");
        let err = fx
            .ledger
            .credit_tx(&mut tx, other, 10.0, reference)
            .await
            .unwrap_err();
        assert!(matches!(err, LedgerError::InvalidArgument(_)), "{err:?}");
        tx.rollback().await.expect("rollback tx4");
    }

    assert!(approx(fx.balance(user).await, 10.0));
    assert!(approx(fx.balance(other).await, 0.0));
    assert!(fx.logs_for(other, reference).await.is_empty());

    fx.cleanup().await;
}

/// 0015 的部分唯一索引把服务面入账钉在库里：同 reference 的第二条 credit 直接被
/// 23505 拒绝；`ESCAPE '!'` 让 `serviceXcredit:` 不被误伤；非 credit 的 settle
/// 行（超预留结算会对同一 reference 写两条）不受影响。
#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn service_credit_partial_unique_index_is_partitioned_and_escaped() {
    let mut fx = Fixture::postgres_only().await;
    let user1 = fx.seed_user(0.0).await;
    let user2 = fx.seed_user(0.0).await;

    sqlx::raw_sql(include_str!("../../../../migrations/0015_service_credit_unique.sql"))
        .execute(&fx.pool)
        .await
        .expect("create the service-credit partial unique index");

    let reference = "service_credit:ozon-order:99901";
    sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at) \
         VALUES ($1, 10.0, 'credit', $2, NOW())",
    )
    .bind(user1)
    .bind(reference)
    .execute(&fx.pool)
    .await
    .expect("first service credit");

    let err = sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at) \
         VALUES ($1, 10.0, 'credit', $2, NOW())",
    )
    .bind(user2)
    .bind(reference)
    .execute(&fx.pool)
    .await
    .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23505"),
        "重复的 service_credit 必须撞唯一索引: {err}"
    );

    // `_` 被 ESCAPE 钉成字面下划线：serviceXcredit 不是同一个命名空间。
    let wildcard = "serviceXcredit:99901";
    for user in [user1, user2] {
        sqlx::query(
            "INSERT INTO balance_logs (user_id, amount, type, reference, created_at) \
             VALUES ($1, 5.0, 'credit', $2, NOW())",
        )
        .bind(user)
        .bind(wildcard)
        .execute(&fx.pool)
        .await
        .expect("serviceXcredit is not the indexed namespace");
    }

    // 同 reference 的 settle 行不受该索引约束。
    for _ in 0..2 {
        sqlx::query(
            "INSERT INTO balance_logs (user_id, amount, type, reference, created_at) \
             VALUES ($1, -1.0, 'settle', $2, NOW())",
        )
        .bind(user1)
        .bind(reference)
        .execute(&fx.pool)
        .await
        .expect("settle rows with the same reference stay legal");
    }

    fx.cleanup().await;
}

/// 脏历史数据（同 reference 的多条 credit）必须让 0015 的建索引**失败**，
/// 而不是静默删除或留一笔糊涂账 —— 运维人工裁决后才放行。
#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn dirty_service_credit_history_blocks_the_unique_index() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(0.0).await;

    sqlx::raw_sql("DROP INDEX IF EXISTS idx_balance_logs_service_credit")
        .execute(&fx.pool)
        .await
        .expect("drop the index to simulate a pre-migration database");

    let reference = "service_credit:ozon-order:dirty";
    for _ in 0..2 {
        sqlx::query(
            "INSERT INTO balance_logs (user_id, amount, type, reference, created_at) \
             VALUES ($1, 10.0, 'credit', $2, NOW())",
        )
        .bind(user)
        .bind(reference)
        .execute(&fx.pool)
        .await
        .expect("dirty row");
    }

    let err = sqlx::raw_sql(include_str!("../../../../migrations/0015_service_credit_unique.sql"))
        .execute(&fx.pool)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23505"),
        "脏历史数据存在时迁移必须失败报错: {err}"
    );

    fx.cleanup().await;
}
