//! 准入处落下的两样东西：**冻结的报价**与**配额的在途预留**。
//!
//! 两者的共同点是它们都必须在锁 / 冻结的那一刻定下来，之后谁也改不动 ——
//! 一个防的是「在途请求被改价追上」，另一个防的是「两个请求抢同一格额度都过」。

use std::sync::Arc;

use gw_ledger::BillingOperationId;

use super::*;
use crate::ports::QuotaAdmission;
use crate::testsupport::{FakeQuotaStore, FakeUsageStore};
use crate::usage::UsageOutcome;

/// 一个额度还剩一点点的订阅：`limit - used` 恰好容得下一笔 `slot`，容不下两笔。
fn last_slot_quota(id: crate::ports::Id, used: f64, limit: f64) -> SubscriptionQuota {
    SubscriptionQuota {
        id,
        daily_usage_usd: used,
        daily_limit_usd: Some(limit),
        ..SubscriptionQuota::default()
    }
}

// ------------------------------------------------------------ 配额预留

/// **两个并发预留抢最后一格额度，只有一个能过。**
///
/// 收敛前的实现是「锁行 → 轮转 → **提交** → 事务外面比限额」，两个并发请求
/// 提交之后读到同一个「已用」，于是两个都放行。这条测的就是那个比较有没有
/// 搬进锁里 —— [`FakeQuotaStore`] 的临界区里有一个真的 `yield_now().await`，
/// 所以「锁外面比」在这里一定会被观察到。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_reserves_for_the_last_slot_admit_exactly_one() {
    let store = FakeQuotaStore::shared();
    // 已用 5、限额 10：一笔 4 过得去，两笔 4 一起就顶穿。
    store.seed(last_slot_quota(55, 5.0, 10.0)).await;

    let mut tasks = Vec::new();
    for _ in 0..2 {
        let store = Arc::clone(&store);
        tasks.push(tokio::spawn(async move {
            store
                .reserve(55, &BillingOperationId::mint(), 4.0, Utc::now())
                .await
                .expect("reserve runs")
        }));
    }

    let mut reserved = 0;
    let mut exceeded = 0;
    for task in tasks {
        match task.await.expect("task joins") {
            QuotaAdmission::Reserved => reserved += 1,
            QuotaAdmission::Exceeded { .. } => exceeded += 1,
            QuotaAdmission::NoSubscription => panic!("the subscription exists"),
        }
    }
    assert_eq!(reserved, 1, "只有一个请求能拿到最后一格额度");
    assert_eq!(exceeded, 1);
    assert_eq!(
        store.reserved_total(55).await,
        4.0,
        "被拒的那一个不许留下预留行",
    );
}

/// 一次成功的请求：预扣时压住额度，结算时**转成实际用量**并把预留删掉。
///
/// 只删不加 = 白用一次额度；只加不删 = 在途与实际同时占着，一次请求算两遍。
#[tokio::test]
async fn a_settled_request_converts_its_reservation_into_actual_usage() {
    let harness = Harness::build();
    let quota = last_slot_quota(55, 0.0, 1_000.0);
    harness.quota.seed(quota.clone()).await;
    harness
        .directory
        .subscriptions
        .lock()
        .insert(TEST_USER_ID, quota);

    let (status, _) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let charged = harness.usage_store.settled_costs();
    assert_eq!(charged.len(), 1);
    assert!(charged[0] > 0.0);
    assert_eq!(
        harness.quota.reserved_total(55).await,
        0.0,
        "结算之后预留必须消失，否则在途负债永远压着额度",
    );
    let after = harness
        .quota
        .quota(55)
        .await
        .expect("the row is still there");
    for (period, used) in [
        ("daily", after.daily_usage_usd),
        ("weekly", after.weekly_usage_usd),
        ("monthly", after.monthly_usage_usd),
    ] {
        assert!(
            (used - charged[0]).abs() < 1e-12,
            "{period} 计数器应当正好加上实际扣款：{used} vs {}",
            charged[0],
        );
    }
}

/// 一次被上游 4xx 的请求：额度还回去，**一分钱也不累加**。
#[tokio::test]
async fn a_rejected_request_gives_its_quota_slot_back_without_accumulating() {
    let harness = Harness::build();
    let quota = last_slot_quota(55, 0.0, 1_000.0);
    harness.quota.seed(quota.clone()).await;
    harness
        .directory
        .subscriptions
        .lock()
        .insert(TEST_USER_ID, quota);

    let (status, _) = send(
        harness.stub_router(StatusCode::BAD_REQUEST),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    assert_eq!(
        harness.quota.reserved_total(55).await,
        0.0,
        "一个 4xx 不该永久占着订阅的额度",
    );
    let after = harness
        .quota
        .quota(55)
        .await
        .expect("the row is still there");
    assert_eq!(after.daily_usage_usd, 0.0, "释放不许把预留算成实际用量",);
}

/// 配额预留成功、余额准入却失败时，**预留必须被撤回**。
///
/// 否则一次 402 就永久吃掉一格额度，而且租户完全看不出来。
#[tokio::test]
async fn a_balance_refusal_after_a_quota_reserve_takes_the_slot_back() {
    let harness = Harness::build();
    let quota = last_slot_quota(55, 0.0, 1_000.0);
    harness.quota.seed(quota.clone()).await;
    harness
        .directory
        .subscriptions
        .lock()
        .insert(TEST_USER_ID, quota);
    // 余额不足以覆盖上界 —— 配额那一关先过，账本这一关才拒。
    *harness.ledger.balance.lock() = 0.0;

    let (status, body) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(body["error"].as_str(), Some("insufficient_balance"));
    assert_eq!(
        harness.quota.reserved_total(55).await,
        0.0,
        "被账本拒掉的请求不许在配额上留下在途负债",
    );
}

// ------------------------------------------------------------ 冻结报价

/// **Hold 与 Settle 用同一份报价，哪怕价目表在两者之间被改了。**
///
/// 这是这把刀的第一条：收敛前 Settle 会拿 `usage.model` 重新查一次价目表，
/// 于是管理员的在途改价、以及上游回的模型名，都能改变这次请求实际被扣的钱。
#[tokio::test]
async fn hold_and_settle_share_one_quote_across_a_mid_flight_price_edit() {
    let harness = Harness::build();
    let peek = billing_peek(chat_body("gpt-4o").to_string().as_bytes());
    let frozen = harness.calc.quote(&peek.price_key, 1.0);

    // 请求跑起来（stub handler 不出 usage 信封，走 fallback，
    // 兜底估算同样必须来自冻结的报价）。
    let (status, _) = send(
        harness.stub_router(StatusCode::OK),
        signed_request("/v1/chat/completions", chat_body("gpt-4o")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let charged = harness.usage_store.settled_costs();
    assert_eq!(charged.len(), 1);

    // 同样的请求，但价目表在**准入之后**涨了一千倍。
    let after = Harness::build();
    after.calc.set_price(1_000_000.0);
    let expensive = after.calc.quote(&peek.price_key, 1.0);
    assert!(
        expensive.estimate(true) > frozen.estimate(true),
        "fixture 需要一个真的涨价，否则这条测的是空气",
    );

    // 而在途的那一个（用旧报价冻的 ctx）结算出来必须还是旧价钱。
    let ledger = crate::testsupport::FakeLedger::with_balance(1_000_000.0);
    let store = FakeUsageStore::shared();
    let settlement = Settlement::new(ledger.clone(), store.clone());
    let ctx = SettleCtx {
        user_id: TEST_USER_ID,
        quote: frozen.clone(),
        model: peek.model.clone(),
        ..SettleCtx::default()
    };
    ledger
        .plant_hold(TEST_USER_ID, &ctx.operation, 0.0001)
        .await;
    settlement
        .settle(
            &ctx,
            UsageOutcome {
                provider: "openai".to_owned(),
                ..UsageOutcome::precise(gw_provider::types::UsageRecord {
                    // 上游还顺手回了一个别的模型名。它也不许改价格键。
                    model: "some-other-model".to_owned(),
                    provider: "openai".to_owned(),
                    input_tokens: Some(1_000),
                    output_tokens: Some(1_000),
                    cached_tokens: None,
                    reasoning_tokens: None,
                })
            },
        )
        .await;

    let billable = gw_pricing::ObservedUsage::new(1_000, 1_000, 0, 0)
        .expect("envelope")
        .normalize(gw_pricing::UsageDialect::OpenAi)
        .expect("consistent");
    let settled = store.settled_costs();
    assert_eq!(settled.len(), 1);
    assert!(
        (settled[0] - frozen.compute(billable).total_cost).abs() < 1e-12,
        "扣的不是准入时冻下来的价：{}",
        settled[0],
    );
    assert!(
        settled[0] < expensive.compute(billable).total_cost,
        "改价追上了在途请求：{} 不该达到新价的 {}",
        settled[0],
        expensive.compute(billable).total_cost,
    );
}

// ------------------------------------------------------------ 租约续期

/// **续租只推时间，不动金额。** 续第二次也一样。
///
/// 金额一旦能被续租改动，租户的在途负债就会随流的长度漂移，
/// 余额闸门看到的将是一个与准入时不同的数。
#[tokio::test]
async fn renewing_a_lease_never_changes_the_reserved_amount() {
    let ledger = crate::testsupport::FakeLedger::with_balance(100.0);
    let operation = BillingOperationId::mint();
    ledger.plant_hold(TEST_USER_ID, &operation, 7.5).await;
    let reserved = ledger.held_amount(&operation).expect("the hold is live");

    for round in 0..2 {
        let renewed = ledger
            .renew_lease(TEST_USER_ID, &operation)
            .await
            .expect("a live hold renews");
        assert_eq!(renewed, reserved, "第 {round} 次续租改了金额");
        assert_eq!(
            ledger.held_amount(&operation),
            Some(reserved),
            "第 {round} 次续租之后预留本身也必须原样",
        );
    }
    assert_eq!(ledger.renewals.lock().len(), 2);
    assert!(
        !ledger
            .calls()
            .iter()
            .any(|call| matches!(call, LedgerCall::Settle { .. } | LedgerCall::Release { .. })),
        "续租不是一次终态操作",
    );
}

/// 续租**绝不凭空造出一笔预留**：一个没有活预留的操作续租失败。
#[tokio::test]
async fn renewing_an_unknown_operation_does_not_conjure_a_hold() {
    let ledger = crate::testsupport::FakeLedger::with_balance(100.0);
    let stranger = BillingOperationId::mint();
    assert!(matches!(
        ledger.renew_lease(TEST_USER_ID, &stranger).await,
        Err(crate::ports::BillingError::HoldNotFound),
    ));
    assert!(ledger.held_amount(&stranger).is_none());
}
