//! 充值单结算：重复确认只入账一次。
//!
//! 对应原实现的 payment_order 测试。人工确认和 Stripe 回调走的是同一个
//! `settle_payment_order`，所以这里验的其实是**两条外部触发路径共用的那把锁**。

use std::sync::Arc;

use crate::common::{
    balance_log_count, balance_log_entries, balance_of, fresh_db, ledger_without_redis,
    order_status, seed_payment_order, seed_user,
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

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn paid_without_credit_repaired_once() {
    // E0a: 订单已 paid 但缺流水的历史行，重试会补入恰好一次。
    // 在 8 路并发修复（通过 TokioBarrier 同步起跑）与后续串行重试下，也只能补入一次且最终状态为 paid。
    let pool = fresh_db("settle_e0a_repair").await;
    let ledger = ledger_without_redis(&pool);
    let user = seed_user(&pool, "e0a_repair@example.com", 0.0).await;
    let order = seed_payment_order(&pool, user, TOP_UP).await;

    // 制造历史 paid-无-credit 残留：直接把订单置为 paid，但不写 balance_logs 流水
    sqlx::query("UPDATE payment_orders SET status = 'paid' WHERE id = $1")
        .bind(order)
        .execute(&pool)
        .await
        .expect("simulate paid without credit");

    assert_eq!(order_status(&pool, order).await, "paid");
    assert_eq!(balance_of(&pool, user).await, 0.0);
    assert_eq!(balance_log_count(&pool, user).await, 0);

    // 8 路并发通过 TokioBarrier 同步释放发起结算修复
    let barrier = Arc::new(tokio::sync::Barrier::new(8));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        let ledger = ledger.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            settle_payment_order(&pool, &ledger, order).await
        }));
    }

    let mut concurrent_credited = 0;
    for handle in handles {
        if handle.await.expect("join").expect("settle") {
            concurrent_credited += 1;
        }
    }

    // 后续串行重试
    let mut serial_credited = 0;
    for _ in 0..3 {
        if settle_payment_order(&pool, &ledger, order)
            .await
            .expect("serial settle")
        {
            serial_credited += 1;
        }
    }

    // 验证补入恰好一次
    assert_eq!(
        concurrent_credited + serial_credited,
        1,
        "paid 缺流水行在并发与串行重试中必须恰好补入一次"
    );
    assert_eq!(order_status(&pool, order).await, "paid");
    assert!(
        (balance_of(&pool, user).await - TOP_UP).abs() < 1e-9,
        "用户余额必须恰好补入一次 TOP_UP"
    );

    let payment_ref = format!("payment_order:{order}");
    let logs = balance_log_entries(&pool, user).await;
    let scoped_logs: Vec<_> = logs
        .into_iter()
        .filter(|(_, r)| r == &payment_ref)
        .collect();
    assert_eq!(scoped_logs.len(), 1, "必须恰好有一条对应的充值流水");
    assert!((scoped_logs[0].0 - TOP_UP).abs() < 1e-9);
    assert_eq!(scoped_logs[0].1, payment_ref);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn successful_credit_followed_by_legacy_reset_to_pending_settle_must_not_add_again() {
    // E0b: 正常入账成功后，若遇旧逻辑制造的 paid->pending 回滚窗口，再次 settle 不得重复入账。
    // 第二笔 credit 视为 AlreadyApplied，余额保持不变，最终状态仍为 paid。
    let pool = fresh_db("settle_e0b_reset").await;
    let ledger = ledger_without_redis(&pool);
    let user = seed_user(&pool, "e0b_reset@example.com", 0.0).await;
    let order = seed_payment_order(&pool, user, TOP_UP).await;

    // 首次正常结算：入账成功并置为 paid
    let first = settle_payment_order(&pool, &ledger, order)
        .await
        .expect("first settle");
    assert!(first, "首次结算必须入账");
    assert_eq!(order_status(&pool, order).await, "paid");
    assert!((balance_of(&pool, user).await - TOP_UP).abs() < 1e-9);
    assert_eq!(balance_log_count(&pool, user).await, 1);

    // 模拟旧异常窗口：订单状态被重置回 pending
    sqlx::query("UPDATE payment_orders SET status = 'pending' WHERE id = $1")
        .bind(order)
        .execute(&pool)
        .await
        .expect("simulate legacy reset to pending");
    assert_eq!(order_status(&pool, order).await, "pending");

    // 再次调用 settle：因唯一 reference 流水已存在，必须视为 AlreadyApplied，绝不可二次入账
    let second = settle_payment_order(&pool, &ledger, order)
        .await
        .expect("second settle");
    assert!(!second, "已有流水时重试结算不得再次入账 (AlreadyApplied)");
    assert_eq!(
        order_status(&pool, order).await,
        "paid",
        "结算后订单状态必须收敛为 paid"
    );
    assert!(
        (balance_of(&pool, user).await - TOP_UP).abs() < 1e-9,
        "用户余额绝不能被重复增加"
    );

    let payment_ref = format!("payment_order:{order}");
    let logs = balance_log_entries(&pool, user).await;
    let scoped_logs: Vec<_> = logs
        .into_iter()
        .filter(|(_, r)| r == &payment_ref)
        .collect();
    assert_eq!(scoped_logs.len(), 1, "对应的充值流水必须仍仅有一条");
    assert!((scoped_logs[0].0 - TOP_UP).abs() < 1e-9);
    assert_eq!(scoped_logs[0].1, payment_ref);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn fault_between_credit_and_status_update_rolls_back_atomically() {
    let pool = fresh_db("settle_fault_rollback").await;
    let ledger = ledger_without_redis(&pool);
    let user = seed_user(&pool, "fault_user@example.com", 0.0).await;
    let order = seed_payment_order(&pool, user, TOP_UP).await;

    // 安装确定性触发器：在 UPDATE payment_orders 试图置 status='paid' 时，
    // 检查 balance_logs 中是否已经插入了该订单的 credit 流水；
    // 只有在已插入时才抛出异常，证明故障发生在 credit 之后、事务提交之前。
    sqlx::raw_sql(
        r#"
        CREATE OR REPLACE FUNCTION fail_after_credit() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN
          IF NEW.status = 'paid' AND EXISTS (
            SELECT 1 FROM balance_logs WHERE reference = ('payment_order:' || NEW.id)
          ) THEN
            RAISE EXCEPTION 'simulated fault after credit before commit';
          END IF;
          RETURN NEW;
        END;
        $$;
        CREATE TRIGGER trg_fail_after_credit BEFORE UPDATE ON payment_orders
        FOR EACH ROW EXECUTE FUNCTION fail_after_credit();
        "#,
    )
    .execute(&pool)
    .await
    .expect("install deterministic fault trigger");

    // 调用结算，预期触发异常失败
    let err = settle_payment_order(&pool, &ledger, order).await;
    let err_msg = err
        .expect_err("在 credit 后抛异常必须导致 settle 报错")
        .to_string();
    assert!(
        err_msg.contains("simulated fault after credit before commit"),
        "必须为确定性故障触发的异常: {err_msg}"
    );

    // 断言原子回滚：状态仍为 pending，余额仍为 0，没有残留流水
    assert_eq!(order_status(&pool, order).await, "pending");
    assert_eq!(balance_of(&pool, user).await, 0.0);
    assert_eq!(balance_log_count(&pool, user).await, 0);

    // 移除故障触发器
    sqlx::raw_sql(
        r#"
        DROP TRIGGER IF EXISTS trg_fail_after_credit ON payment_orders;
        DROP FUNCTION IF EXISTS fail_after_credit();
        "#,
    )
    .execute(&pool)
    .await
    .expect("drop fault trigger");

    // 故障恢复后重试结算：必须能够成功入账恰好一次
    let retry_res = settle_payment_order(&pool, &ledger, order)
        .await
        .expect("retry settle after fault clearance");
    assert!(retry_res, "故障恢复后的重试必须成功入账");
    assert_eq!(order_status(&pool, order).await, "paid");
    assert!((balance_of(&pool, user).await - TOP_UP).abs() < 1e-9);
    assert_eq!(balance_log_count(&pool, user).await, 1);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn paid_repair_credit_insert_fault_rolls_back_and_preserves_paid_at() {
    let pool = fresh_db("settle_paid_repair_fault").await;
    let ledger = ledger_without_redis(&pool);
    let user = seed_user(&pool, "repair_fault@example.com", 0.0).await;
    let order = seed_payment_order(&pool, user, TOP_UP).await;

    // 订单处于 paid 状态且拥有明确的 paid_at，但缺 credit 流水
    let initial_paid_at: chrono::DateTime<chrono::Utc> =
        chrono::DateTime::parse_from_rfc3339("2024-01-01T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

    sqlx::query("UPDATE payment_orders SET status = 'paid', paid_at = $2 WHERE id = $1")
        .bind(order)
        .bind(initial_paid_at)
        .execute(&pool)
        .await
        .expect("set order to paid with specific paid_at");

    // 安装流水插入故障触发器：谓词匹配 type='credit' 与带转义的 payment_order 前缀
    sqlx::raw_sql(
        r#"
        CREATE OR REPLACE FUNCTION fail_credit_insert() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN
          IF NEW.type = 'credit' AND NEW.reference LIKE 'payment!_order:%' ESCAPE '!' THEN
            RAISE EXCEPTION 'simulated credit insert failure';
          END IF;
          RETURN NEW;
        END;
        $$;
        CREATE TRIGGER trg_fail_credit_insert BEFORE INSERT ON balance_logs
        FOR EACH ROW EXECUTE FUNCTION fail_credit_insert();
        "#,
    )
    .execute(&pool)
    .await
    .expect("install credit insert fault trigger");

    // 尝试结算补入，因 credit 插入失败而报错
    let err = settle_payment_order(&pool, &ledger, order).await;
    assert!(err.is_err(), "流水插入失败必须导致 settle 报错");

    // 验证状态回滚：status 仍为 paid，paid_at 时间戳保持不变，余额仍为 0，流水数为 0
    assert_eq!(order_status(&pool, order).await, "paid");
    assert_eq!(balance_of(&pool, user).await, 0.0);
    assert_eq!(balance_log_count(&pool, user).await, 0);

    let current_paid_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT paid_at FROM payment_orders WHERE id = $1")
            .bind(order)
            .fetch_one(&pool)
            .await
            .expect("fetch paid_at");
    assert_eq!(
        current_paid_at,
        Some(initial_paid_at),
        "paid_at 绝不得在失败中被清空或篡改"
    );

    // 移除触发器后重试补入
    sqlx::raw_sql(
        r#"
        DROP TRIGGER IF EXISTS trg_fail_credit_insert ON balance_logs;
        DROP FUNCTION IF EXISTS fail_credit_insert();
        "#,
    )
    .execute(&pool)
    .await
    .expect("drop credit insert fault trigger");

    let repaired = settle_payment_order(&pool, &ledger, order)
        .await
        .expect("repair settle");
    assert!(repaired, "恢复后补入必须成功");
    assert!((balance_of(&pool, user).await - TOP_UP).abs() < 1e-9);

    // 补入成功后，历史订单原有的 paid_at 时间戳仍需被保留（paid repair preserves existing timestamps）
    let post_repair_paid_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT paid_at FROM payment_orders WHERE id = $1")
            .bind(order)
            .fetch_one(&pool)
            .await
            .expect("fetch paid_at after repair");
    assert_eq!(
        post_repair_paid_at,
        Some(initial_paid_at),
        "补入成功后必须保留历史订单的原始 paid_at 时间戳"
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn paid_credit_mismatched_owner_or_amount_fails_closed() {
    let pool = fresh_db("settle_mismatch_fail_closed").await;
    let ledger = ledger_without_redis(&pool);
    let user1 = seed_user(&pool, "owner1@example.com", 10.0).await;
    let user2 = seed_user(&pool, "owner2@example.com", 20.0).await;
    let order = seed_payment_order(&pool, user1, TOP_UP).await;

    // 预先插入一条 reference 重名但归属为 user2 的流水（伪造或串号冲突）
    let payment_ref = format!("payment_order:{order}");
    sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at)          VALUES ($1, $2, 'credit', $3, NOW())",
    )
    .bind(user2)
    .bind(TOP_UP)
    .bind(&payment_ref)
    .execute(&pool)
    .await
    .expect("seed mismatched user credit log");

    // 试图对 user1 的 order 发起结算，由于 reference 已被 user2 占用，必须 fail closed（返回 Err）
    let err = settle_payment_order(&pool, &ledger, order).await;
    assert!(
        err.is_err(),
        "流水归属用户不匹配时必须报错拒付，绝不可静默返回 Ok(false)"
    );

    // 验证双方余额和订单状态均未被破坏，订单状态严格保持 pending（绝不置为 paid）
    assert_eq!(balance_of(&pool, user1).await, 10.0);
    assert_eq!(balance_of(&pool, user2).await, 20.0);
    assert_eq!(
        order_status(&pool, order).await,
        "pending",
        "用户不匹配时订单绝不得被置为 paid"
    );

    // 同样，测试金额不匹配场景
    let order_amount_mismatch = seed_payment_order(&pool, user1, TOP_UP).await;
    let mismatch_ref = format!("payment_order:{order_amount_mismatch}");
    sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at)          VALUES ($1, $2, 'credit', $3, NOW())",
    )
    .bind(user1)
    .bind(TOP_UP + 10.0) // 金额不一致
    .bind(&mismatch_ref)
    .execute(&pool)
    .await
    .expect("seed mismatched amount credit log");

    let err2 = settle_payment_order(&pool, &ledger, order_amount_mismatch).await;
    assert!(
        err2.is_err(),
        "流水金额不匹配时必须报错拒付，绝不可静默返回 Ok(false)"
    );
    assert_eq!(balance_of(&pool, user1).await, 10.0);
    assert_eq!(
        order_status(&pool, order_amount_mismatch).await,
        "pending",
        "金额不匹配时订单绝不得被置为 paid"
    );
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn unserviceable_order_status_remains_uncredited_and_returns_false() {
    let pool = fresh_db("settle_unserviceable").await;
    let ledger = ledger_without_redis(&pool);
    let user = seed_user(&pool, "unserviceable@example.com", 0.0).await;

    let failed_order = seed_payment_order(&pool, user, TOP_UP).await;
    sqlx::query("UPDATE payment_orders SET status = 'failed' WHERE id = $1")
        .bind(failed_order)
        .execute(&pool)
        .await
        .expect("mark order failed");

    let canceled_order = seed_payment_order(&pool, user, TOP_UP).await;
    sqlx::query("UPDATE payment_orders SET status = 'canceled' WHERE id = $1")
        .bind(canceled_order)
        .execute(&pool)
        .await
        .expect("mark order canceled");

    let res_failed = settle_payment_order(&pool, &ledger, failed_order)
        .await
        .expect("settle failed order");
    assert!(!res_failed, "failed 状态订单不得入账，返回 false");
    assert_eq!(order_status(&pool, failed_order).await, "failed");

    let res_canceled = settle_payment_order(&pool, &ledger, canceled_order)
        .await
        .expect("settle canceled order");
    assert!(!res_canceled, "canceled 状态订单不得入账，返回 false");
    assert_eq!(order_status(&pool, canceled_order).await, "canceled");

    assert_eq!(balance_of(&pool, user).await, 0.0);
    assert_eq!(balance_log_count(&pool, user).await, 0);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn admin_confirm_order_http_contract() {
    let pool = fresh_db("settle_admin_http").await;
    let state = crate::common::http::panel_state(&pool);
    let admin =
        crate::common::seed_user_with(&pool, "admin@example.com", 0.0, "admin", "active").await;
    let token = crate::common::http::token_for(admin, "admin@example.com");

    let user = seed_user(&pool, "buyer@example.com", 0.0).await;
    let order = seed_payment_order(&pool, user, TOP_UP).await;

    // 1. 首次确认：HTTP 200 OK
    let status1 = crate::common::http::call(
        &state,
        axum::http::Method::PUT,
        &format!("/admin/orders/{order}/confirm"),
        &token,
    )
    .await;
    assert_eq!(
        status1,
        axum::http::StatusCode::OK,
        "首次管理员确认应返回 200"
    );
    assert_eq!(order_status(&pool, order).await, "paid");
    assert!((balance_of(&pool, user).await - TOP_UP).abs() < 1e-9);

    // 2. 重复确认：HTTP 409 Conflict
    let status2 = crate::common::http::call(
        &state,
        axum::http::Method::PUT,
        &format!("/admin/orders/{order}/confirm"),
        &token,
    )
    .await;
    assert_eq!(
        status2,
        axum::http::StatusCode::CONFLICT,
        "重复确认应返回 409 Conflict"
    );
    assert!((balance_of(&pool, user).await - TOP_UP).abs() < 1e-9);

    // 3. paid-无-credit 补入确认：HTTP 200 OK
    let repair_order = seed_payment_order(&pool, user, TOP_UP).await;
    sqlx::query("UPDATE payment_orders SET status = 'paid' WHERE id = $1")
        .bind(repair_order)
        .execute(&pool)
        .await
        .expect("simulate paid without credit");

    let status_repair = crate::common::http::call(
        &state,
        axum::http::Method::PUT,
        &format!("/admin/orders/{repair_order}/confirm"),
        &token,
    )
    .await;
    assert_eq!(
        status_repair,
        axum::http::StatusCode::OK,
        "补入修复确认应返回 200"
    );
    assert!((balance_of(&pool, user).await - 2.0 * TOP_UP).abs() < 1e-9);
}

#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn stripe_webhook_lifecycle_duplicate_repair_and_failure_contract() {
    use axum::extract::State;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    let pool = fresh_db("settle_stripe_webhook").await;
    let mut state = crate::common::http::panel_state(&pool);
    let secret = "whsec_test_stripe_secret_key_12345";
    state.stripe_webhook_secret = Some(Arc::new(secret.to_owned()));

    let user = seed_user(&pool, "stripe_buyer@example.com", 0.0).await;
    let order = seed_payment_order(&pool, user, TOP_UP).await;

    // 辅助闭包：构造带合法 Stripe 签名的 Webhook 请求头
    let make_headers = |payload: &[u8]| -> axum::http::HeaderMap {
        let now = chrono::Utc::now().timestamp();
        let mut mac = <HmacSha256>::new_from_slice(secret.as_bytes()).expect("hmac init");
        mac.update(now.to_string().as_bytes());
        mac.update(b".");
        mac.update(payload);
        let sig = hex::encode(mac.finalize().into_bytes());

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "Stripe-Signature",
            axum::http::HeaderValue::from_str(&format!("t={now},v1={sig}")).unwrap(),
        );
        headers
    };

    let make_body = |order_id: i64| -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "type": "payment_intent.succeeded",
            "data": {
                "object": {
                    "metadata": {
                        "order_id": order_id.to_string()
                    }
                }
            }
        }))
        .unwrap()
    };

    // 1. 首次触发：未支付订单正常结算入账，返回 200
    let body1 = make_body(order);
    let headers1 = make_headers(&body1);
    let resp1 = gw_panel::commerce::stripe::webhook(
        State(state.clone()),
        headers1,
        axum::body::Bytes::from(body1),
    )
    .await;
    assert_eq!(
        resp1.status(),
        axum::http::StatusCode::OK,
        "首次 webhook 应返回 200"
    );
    assert_eq!(order_status(&pool, order).await, "paid");
    assert!((balance_of(&pool, user).await - TOP_UP).abs() < 1e-9);

    // 2. 重复回调：已付订单幂等处理，返回 200 且不重复入账
    let body2 = make_body(order);
    let headers2 = make_headers(&body2);
    let resp2 = gw_panel::commerce::stripe::webhook(
        State(state.clone()),
        headers2,
        axum::body::Bytes::from(body2),
    )
    .await;
    assert_eq!(
        resp2.status(),
        axum::http::StatusCode::OK,
        "重复 webhook 应返回 200"
    );
    assert!((balance_of(&pool, user).await - TOP_UP).abs() < 1e-9);
    assert_eq!(balance_log_count(&pool, user).await, 1);

    // 3. paid-无-credit 历史补入：通过 webhook 补入成功，返回 200
    let repair_order = seed_payment_order(&pool, user, TOP_UP).await;
    sqlx::query("UPDATE payment_orders SET status = 'paid' WHERE id = $1")
        .bind(repair_order)
        .execute(&pool)
        .await
        .expect("mark repair order paid");

    let body_repair = make_body(repair_order);
    let headers_repair = make_headers(&body_repair);
    let resp_repair = gw_panel::commerce::stripe::webhook(
        State(state.clone()),
        headers_repair,
        axum::body::Bytes::from(body_repair),
    )
    .await;
    assert_eq!(
        resp_repair.status(),
        axum::http::StatusCode::OK,
        "补入 webhook 应返回 200"
    );
    assert!((balance_of(&pool, user).await - 2.0 * TOP_UP).abs() < 1e-9);

    // 4. 账本故障：安装失败触发器，webhook 必须返回 500（以便 Stripe 重试），且订单不被置为 paid
    let fault_order = seed_payment_order(&pool, user, TOP_UP).await;
    sqlx::raw_sql(
        r#"
        CREATE OR REPLACE FUNCTION fail_stripe_credit() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN
          IF NEW.type = 'credit' AND NEW.reference LIKE 'payment!_order:%' ESCAPE '!' THEN
            RAISE EXCEPTION 'injected ledger failure for stripe webhook';
          END IF;
          RETURN NEW;
        END;
        $$;
        CREATE TRIGGER trg_fail_stripe_credit BEFORE INSERT ON balance_logs
        FOR EACH ROW EXECUTE FUNCTION fail_stripe_credit();
        "#,
    )
    .execute(&pool)
    .await
    .expect("install stripe fault trigger");

    let body_fault = make_body(fault_order);
    let headers_fault = make_headers(&body_fault);
    let resp_fault = gw_panel::commerce::stripe::webhook(
        State(state.clone()),
        headers_fault,
        axum::body::Bytes::from(body_fault),
    )
    .await;
    assert_eq!(
        resp_fault.status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "结算失败时 webhook 必须返回 500 以让 Stripe 保持重试"
    );
    assert_eq!(
        order_status(&pool, fault_order).await,
        "pending",
        "故障时订单绝不得被置为 paid"
    );
    assert_eq!(
        balance_of(&pool, user).await,
        2.0 * TOP_UP,
        "故障时余额绝不得被增加"
    );

    // 清理触发器
    sqlx::raw_sql(
        r#"
        DROP TRIGGER IF EXISTS trg_fail_stripe_credit ON balance_logs;
        DROP FUNCTION IF EXISTS fail_stripe_credit();
        "#,
    )
    .execute(&pool)
    .await
    .expect("drop stripe fault trigger");
}
