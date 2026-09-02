//! Settlement. The three modes AGENTS.md names — precise, fallback, strict —
//! plus the two failure postures that fall out of them.

use std::sync::Arc;

use gw_pricing::PricingQuote;
use gw_provider::types::UsageRecord;

use super::*;
use crate::testsupport::{FakeLedger, FakeUsageStore, LedgerCall};

// ---------------------------------------------------------------- the plan

fn inputs() -> SettlementInputs {
    SettlementInputs {
        computed_cost: 0.5,
        usage_present: true,
        upstream_failed: false,
        strict_mode: false,
        active_hold: Some(0.2),
        fallback_estimate: 0.3,
    }
}

#[test]
fn a_failed_upstream_releases_whatever_else_is_true() {
    for strict in [false, true] {
        for present in [false, true] {
            let plan = plan_settlement(&SettlementInputs {
                upstream_failed: true,
                strict_mode: strict,
                usage_present: present,
                ..inputs()
            });
            assert!(
                matches!(plan, SettlementPlan::Release { .. }),
                "strict={strict} present={present} produced {plan:?}",
            );
        }
    }
}

#[test]
fn a_reported_usage_envelope_is_billed_exactly_and_never_tagged() {
    let plan = plan_settlement(&inputs());
    assert_eq!(
        plan,
        SettlementPlan::Settle {
            cost: 0.5,
            fallback: None
        },
    );
}

#[test]
fn strict_mode_refuses_to_guess_when_the_envelope_is_missing() {
    let plan = plan_settlement(&SettlementInputs {
        usage_present: false,
        strict_mode: true,
        ..inputs()
    });
    assert_eq!(plan, SettlementPlan::StrictSkip);
}

#[test]
fn a_present_envelope_is_billed_precisely_even_in_strict_mode() {
    let plan = plan_settlement(&SettlementInputs {
        strict_mode: true,
        ..inputs()
    });
    assert!(matches!(
        plan,
        SettlementPlan::Settle { fallback: None, .. }
    ));
}

#[test]
fn the_fallback_never_bills_below_the_hold_or_the_fallback_estimate() {
    // "no free upstream output" is the whole point of this branch.
    for hold in [0.0, 0.1, 5.0] {
        for estimate in [0.0, 0.2, 3.0] {
            let plan = plan_settlement(&SettlementInputs {
                usage_present: false,
                active_hold: Some(hold),
                fallback_estimate: estimate,
                ..inputs()
            });
            let SettlementPlan::Settle { cost, fallback } = plan else {
                panic!("expected a settle for hold={hold} estimate={estimate}");
            };
            assert!(cost >= hold);
            assert!(cost >= estimate);
            assert_eq!(fallback, Some(REASON_MISSING_USAGE));
        }
    }
}

#[test]
fn an_unresolvable_hold_is_treated_like_strict_mode_not_like_zero() {
    // Settling at zero would make the request free; the hold is left to expire.
    let plan = plan_settlement(&SettlementInputs {
        usage_present: false,
        active_hold: None,
        ..inputs()
    });
    assert_eq!(plan, SettlementPlan::HoldLookupFailed);
}

// ---------------------------------------------------------------- annotations

#[test]
fn a_clean_settlement_leaves_the_metadata_column_null() {
    assert!(settle_annotations(None, 0.0).is_none());
    assert!(settle_annotations(None, -1.0).is_none());
}

#[test]
fn a_fallback_and_a_shortfall_are_both_recorded_and_compose() {
    let fallback_only = settle_annotations(Some(REASON_MISSING_USAGE), 0.0).expect("annotated");
    assert_eq!(
        fallback_only["billing_fallback"]["reason"].as_str(),
        Some(REASON_MISSING_USAGE)
    );
    assert!(fallback_only.get("shortfall_usd").is_none());

    let both = settle_annotations(Some(REASON_MISSING_USAGE), 1.5).expect("annotated");
    assert_eq!(both["shortfall_usd"].as_f64(), Some(1.5));
    assert!(both.get("billing_fallback").is_some());
}

#[test]
fn merging_a_shortfall_preserves_annotations_already_present() {
    let base = settle_annotations(Some(REASON_MISSING_USAGE), 0.0);
    let merged = merge_shortfall(base, 2.0).expect("merged");
    assert_eq!(merged["shortfall_usd"].as_f64(), Some(2.0));
    assert_eq!(
        merged["billing_fallback"]["reason"].as_str(),
        Some(REASON_MISSING_USAGE),
        "a partially-paid fallback must stay distinguishable from a clean one",
    );

    assert!(
        merge_shortfall(None, 0.0).is_none(),
        "a fully-paid request keeps a NULL metadata column",
    );
}

// ---------------------------------------------------------------- balance events

#[test]
fn crossing_the_threshold_warns_once_and_hitting_zero_reports_depletion() {
    assert_eq!(balance_events(5.0, 0.5, 1.0), vec!["low_balance_warning"]);
    assert_eq!(balance_events(5.0, 0.0, 1.0), vec!["balance_depleted"]);
    assert!(
        balance_events(0.5, 0.4, 1.0).is_empty(),
        "already below the threshold is not a new crossing",
    );
    assert!(balance_events(5.0, 4.0, 1.0).is_empty());
    assert_eq!(
        balance_events(5.0, 0.5, 0.0),
        vec!["low_balance_warning"],
        "a non-positive configured threshold falls back to the default",
    );
}

// ---------------------------------------------------------------- end to end

struct Fixture {
    settlement: Settlement,
    ledger: Arc<FakeLedger>,
    store: Arc<FakeUsageStore>,
}

fn fixture() -> Fixture {
    let ledger = FakeLedger::with_balance(100.0);
    let store = FakeUsageStore::shared();
    Fixture {
        settlement: Settlement::new(ledger.clone(), store.clone()),
        ledger,
        store,
    }
}

/// A context with a freshly-minted operation. Tests that need the reservation
/// to line up bind it once and pass `&ctx` — the operation id *is* the key.
///
/// 报价在这里就冻好了，和生产一样：结算路径拿不到别的价钱来源。
fn ctx() -> SettleCtx {
    ctx_priced(PricingQuote::flat("gpt-4o", 1_000.0, 1.0, 0))
}

fn ctx_priced(quote: PricingQuote) -> SettleCtx {
    SettleCtx {
        user_id: 7,
        quote,
        model: "gpt-4o".to_owned(),
        client_trace: gw_ledger::ClientTraceId::new("trace-the-client-saw"),
        ..SettleCtx::default()
    }
}

fn usage(input: i64, output: i64) -> UsageRecord {
    UsageRecord {
        model: "gpt-4o".to_owned(),
        provider: "openai".to_owned(),
        input_tokens: Some(input),
        output_tokens: Some(output),
        cached_tokens: None,
        reasoning_tokens: None,
    }
}

#[tokio::test]
async fn a_precise_settlement_debits_then_clears_the_reservation() {
    let fixture = fixture();
    let ctx = ctx();
    fixture.ledger.plant_hold(7, &ctx.operation, 1.0).await;

    fixture
        .settlement
        .settle(&ctx, UsageOutcome::precise(usage(100, 200)))
        .await;

    let commits = fixture.store.commits.lock();
    assert_eq!(commits.len(), 1);
    assert!(commits[0].actual_cost > 0.0);
    assert!(
        commits[0].entry.raw_metadata.is_none(),
        "a precise settlement carries no fallback tag",
    );
    assert_eq!(
        fixture.store.cleared_holds.lock().as_slice(),
        [ctx.operation.to_string()],
        "the reservation is cleared only after the transaction commits",
    );
    assert!(
        !fixture
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
        "a settled request must never also be released",
    );
}

#[tokio::test]
async fn a_missing_envelope_falls_back_to_the_reservation_and_says_so() {
    let fixture = fixture();
    let ctx = ctx();
    fixture.ledger.plant_hold(7, &ctx.operation, 2.5).await;

    fixture
        .settlement
        .settle(&ctx, UsageOutcome::default())
        .await;

    let commits = fixture.store.commits.lock();
    assert_eq!(commits.len(), 1);
    assert!(
        commits[0].actual_cost >= 2.5,
        "the tenant must be billed at least what was reserved",
    );
    let metadata = commits[0]
        .entry
        .raw_metadata
        .clone()
        .expect("fallback annotated");
    assert_eq!(
        metadata["billing_fallback"]["reason"].as_str(),
        Some(REASON_MISSING_USAGE),
        "ops alerts on the volume of this tag",
    );
}

#[tokio::test]
async fn strict_mode_neither_charges_nor_releases_and_records_the_event() {
    let fixture = fixture();
    let ctx = ctx();
    fixture.ledger.plant_hold(7, &ctx.operation, 2.5).await;
    fixture.settlement.set_strict_usage_metadata(true);

    fixture
        .settlement
        .settle(&ctx, UsageOutcome::default())
        .await;

    assert!(
        fixture.store.commits.lock().is_empty(),
        "strict mode must not settle",
    );
    assert!(
        !fixture
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
        "strict mode must not release either — the hold expires on its TTL",
    );
    assert_eq!(
        fixture.ledger.held_amount(&ctx.operation),
        Some(2.5),
        "the reservation stays put so reconciliation can match it",
    );

    let logs = fixture.store.logs.lock();
    assert_eq!(logs.len(), 1);
    assert!(logs[0].failed);
    assert_eq!(logs[0].actual_cost, 0.0);
    assert_eq!(
        logs[0].raw_metadata.as_ref().expect("reason")["reason"].as_str(),
        Some(REASON_MISSING_USAGE_STRICT),
    );
}

#[tokio::test]
async fn an_unreadable_hold_leaves_the_reservation_alone_rather_than_zero_billing() {
    let fixture = fixture();
    let ctx = ctx();
    *fixture.ledger.hold_lookup_errors.lock() = true;

    fixture
        .settlement
        .settle(&ctx, UsageOutcome::default())
        .await;

    assert!(fixture.store.commits.lock().is_empty());
    let logs = fixture.store.logs.lock();
    assert!(logs[0].failed);
    assert_eq!(
        logs[0].raw_metadata.as_ref().expect("event")["event"].as_str(),
        Some(EVENT_HOLD_LOOKUP_FAILED),
    );
}

#[tokio::test]
async fn a_failed_upstream_gives_the_reservation_back() {
    let fixture = fixture();
    let ctx = ctx();
    fixture.ledger.plant_hold(7, &ctx.operation, 1.0).await;

    fixture
        .settlement
        .settle(&ctx, UsageOutcome::failed())
        .await;

    assert!(
        fixture
            .ledger
            .calls()
            .iter()
            .any(|c| matches!(c, LedgerCall::Release { .. })),
    );
    assert!(fixture.store.commits.lock().is_empty());
    assert!(fixture.store.logs.lock()[0].failed);
}

#[tokio::test]
async fn a_rolled_back_transaction_leaves_the_reservation_for_reconciliation() {
    // Balance and usage stay consistent, and the hold is still there, so the
    // request can be reconciled instead of silently charged.
    let fixture = fixture();
    let ctx = ctx();
    *fixture.store.commit_fails.lock() = true;
    fixture.ledger.plant_hold(7, &ctx.operation, 1.0).await;

    fixture
        .settlement
        .settle(&ctx, UsageOutcome::precise(usage(10, 10)))
        .await;

    assert!(
        fixture.store.cleared_holds.lock().is_empty(),
        "a hold cleared after a rollback would charge nothing but lose the reservation",
    );
    assert_eq!(fixture.ledger.held_amount(&ctx.operation), Some(1.0));
    assert!(fixture.store.logs.lock()[0].failed);
}

#[tokio::test]
async fn a_partial_debit_records_the_shortfall_on_the_usage_row() {
    let fixture = fixture();
    let ctx = ctx();
    *fixture.store.shortfall.lock() = 0.75;

    fixture
        .settlement
        .settle(&ctx, UsageOutcome::precise(usage(10, 10)))
        .await;

    let logs = fixture.store.logs.lock();
    assert_eq!(
        logs[0].raw_metadata.as_ref().expect("shortfall")["shortfall_usd"].as_f64(),
        Some(0.75),
        "reporting must distinguish a free request from a partially-paid one",
    );
}

#[tokio::test]
async fn a_subscription_accumulates_only_on_a_real_settlement() {
    let fixture = fixture();
    let mut ctx = ctx();
    ctx.subscription_id = Some(55);

    fixture
        .settlement
        .settle(&ctx, UsageOutcome::precise(usage(10, 10)))
        .await;
    assert_eq!(fixture.store.commits.lock()[0].subscription_id, Some(55));

    fixture
        .settlement
        .settle(&ctx, UsageOutcome::failed())
        .await;
    assert_eq!(
        fixture.store.commits.lock().len(),
        1,
        "a failed request must not touch quota counters",
    );
}

#[tokio::test]
async fn crossing_zero_writes_a_depletion_event() {
    let fixture = fixture();
    let ctx = ctx();
    *fixture.store.balance_before.lock() = 0.4;
    *fixture.store.balance_after.lock() = 0.0;

    fixture
        .settlement
        .settle(&ctx, UsageOutcome::precise(usage(10, 10)))
        .await;

    let events = fixture.store.balance_events.lock();
    assert!(events.iter().any(|e| e.event_type == "balance_depleted"));
    assert_eq!(
        events[0].reference,
        ctx.operation.to_string(),
        "the event must be traceable to its request"
    );
}

#[tokio::test]
async fn the_usage_row_carries_the_credential_that_served_the_request() {
    let fixture = fixture();
    let ctx = ctx();
    fixture
        .settlement
        .settle(
            &ctx,
            UsageOutcome {
                usage: Some(usage(1, 2)),
                auth_id: "acct-9".to_owned(),
                provider: "openai".to_owned(),
                duration_ms: 1234,
                failed: false,
            },
        )
        .await;

    let logs = fixture.store.logs.lock();
    assert_eq!(logs[0].auth_id, "acct-9");
    assert_eq!(logs[0].provider, "openai");
    assert_eq!(logs[0].duration_ms, 1234);
}

#[test]
fn strict_mode_can_be_toggled_at_runtime() {
    let fixture = fixture();
    assert!(!fixture.settlement.strict_usage_metadata());
    fixture.settlement.set_strict_usage_metadata(true);
    assert!(fixture.settlement.strict_usage_metadata());
    fixture.settlement.set_strict_usage_metadata(false);
    assert!(!fixture.settlement.strict_usage_metadata());
}

// ------------------------------------------------- 上游语义与冻结报价

/// 上游 provider 名 → 信封语义。三家分完，其余按 OpenAI 线形读。
#[test]
fn each_upstream_family_maps_onto_its_envelope_semantics() {
    assert_eq!(usage_dialect("claude"), gw_pricing::UsageDialect::Anthropic);
    for google in ["gemini", "vertex"] {
        assert_eq!(
            usage_dialect(google),
            gw_pricing::UsageDialect::Google,
            "{google} 的 candidatesTokenCount 不含思考",
        );
    }
    for openai_shaped in ["openai", "codex", "xai", "some-new-compatible-upstream"] {
        assert_eq!(
            usage_dialect(openai_shaped),
            gw_pricing::UsageDialect::OpenAi,
            "{openai_shaped} 的 completion_tokens 含思考，按并列读会收两遍",
        );
    }
}

fn google_usage(candidates: i64, thoughts: i64) -> UsageRecord {
    UsageRecord {
        model: "a-thinking-model".to_owned(),
        provider: "gemini".to_owned(),
        input_tokens: Some(10),
        output_tokens: Some(candidates),
        cached_tokens: None,
        reasoning_tokens: Some(thoughts),
    }
}

/// Google 的 `candidatesTokenCount` **不含** `thoughtsTokenCount`，
/// 而 `model_prices.reasoning_price_per1_m` 的建表默认值是 **0**。
/// 两件事叠起来，思考型模型的每一个思考 token 都会是免费的 ——
/// 而思考 token 在推理模型上经常是可见输出的数倍。
#[tokio::test]
async fn google_thinking_tokens_are_not_free() {
    // 只给 output 列定价，reasoning 列为 0 —— 就是建表默认值下的真实部署。
    let quote = PricingQuote::new(
        "a-thinking-model".to_owned(),
        gw_pricing::UnitPrice::ZERO,
        gw_pricing::UnitPrice::new(10.0).expect("output price"),
        gw_pricing::UnitPrice::ZERO,
        gw_pricing::UnitPrice::ZERO,
        gw_pricing::RateMultiplier::ONE,
        0,
    );
    let quiet = settle_google(&quote, google_usage(100, 0)).await;
    let thinking = settle_google(&quote, google_usage(100, 400)).await;

    assert!(
        thinking > quiet,
        "同样的输出、多了 400 个思考 token，收的钱却没变：{thinking} vs {quiet}",
    );
}

/// 跑一次完整结算，返回落账的金额。
async fn settle_google(quote: &PricingQuote, usage: UsageRecord) -> f64 {
    let ledger = FakeLedger::with_balance(1_000.0);
    let store = FakeUsageStore::shared();
    let settlement = Settlement::new(ledger, store.clone());
    settlement
        .settle(
            &ctx_priced(quote.clone()),
            UsageOutcome {
                provider: "gemini".to_owned(),
                ..UsageOutcome::precise(usage)
            },
        )
        .await;
    let costs = store.settled_costs();
    assert_eq!(costs.len(), 1, "一次请求恰好结算一次");
    costs[0]
}

/// **结算按 Hold 处冻下来的那份报价算，不按上游回的模型名重新查价。**
///
/// 这一条同时挡住两个洞：管理员在途改价（报价里的单价已经定了），
/// 以及上游回一个别的模型名（价格键在报价里，改不了）。
#[tokio::test]
async fn settlement_uses_the_frozen_quote_not_the_upstream_model_name() {
    let fixture = fixture();
    // 请求的是 gpt-4o，冻的是 gpt-4o 的价。
    let ctx = ctx_priced(PricingQuote::flat("gpt-4o", 1_000.0, 1.0, 0));
    let expensive = PricingQuote::flat("something-else", 999_000.0, 1.0, 1);

    fixture
        .settlement
        .settle(
            &ctx,
            UsageOutcome::precise(UsageRecord {
                // 上游回了一个完全不同的模型名。它只能上日志。
                model: "something-else".to_owned(),
                provider: "openai".to_owned(),
                input_tokens: Some(1_000),
                output_tokens: Some(1_000),
                cached_tokens: None,
                reasoning_tokens: None,
            }),
        )
        .await;

    let charged = fixture.store.settled_costs();
    assert_eq!(charged.len(), 1);
    let billable = gw_pricing::ObservedUsage::new(1_000, 1_000, 0, 0)
        .expect("envelope")
        .normalize(gw_pricing::UsageDialect::OpenAi)
        .expect("consistent");
    assert!(
        (charged[0] - ctx.quote.compute(billable).total_cost).abs() < 1e-12,
        "扣的不是冻结报价算出来的数：{}",
        charged[0],
    );
    assert!(
        charged[0] < expensive.compute(billable).total_cost,
        "上游回的模型名换掉了价格键 —— 上游因此能决定按什么价收租户的钱",
    );
    assert_eq!(
        fixture.store.logs.lock()[0].model,
        "something-else",
        "上游那个名字仍然要上日志，审计才对得上",
    );
}

/// 负数信封既不是零消耗，也不是一笔退款：它按「上游没报 usage」处理，
/// 走既有的 fallback，**绝不产生一笔负数扣款**。
#[tokio::test]
async fn a_negative_usage_column_never_becomes_a_credit() {
    let fixture = fixture();
    let ctx = ctx();
    fixture.ledger.plant_hold(7, &ctx.operation, 2.5).await;

    fixture
        .settlement
        .settle(
            &ctx,
            UsageOutcome::precise(UsageRecord {
                model: "gpt-4o".to_owned(),
                provider: "openai".to_owned(),
                input_tokens: Some(100),
                output_tokens: Some(-5_000),
                cached_tokens: None,
                reasoning_tokens: None,
            }),
        )
        .await;

    let commits = fixture.store.commits.lock();
    assert_eq!(commits.len(), 1);
    assert!(
        commits[0].actual_cost >= 2.5,
        "无效信封必须落到 fallback（不低于预留），得到 {}",
        commits[0].actual_cost,
    );
    assert_eq!(
        commits[0].entry.raw_metadata.as_ref().expect("annotated")["billing_fallback"]["reason"]
            .as_str(),
        Some(REASON_MISSING_USAGE),
        "它和「上游根本没报 usage」是同一条路",
    );
    assert_eq!(
        commits[0].entry.output_tokens, -5_000,
        "日志写的仍然是上游原话，否则审计对不上上游账单",
    );
}

/// OpenAI 的思考 token 走完整条结算链之后也不能被收两遍。
///
/// `gw-pricing` 那边已经按性质卡住了归一化；这一条卡的是**结算真的用了它**。
#[tokio::test]
async fn openai_reasoning_is_not_double_charged_end_to_end() {
    // output 与 reasoning 两列都有价且不同，否则「按哪一列收」观察不到。
    let quote = PricingQuote::new(
        "o3".to_owned(),
        gw_pricing::UnitPrice::ZERO,
        gw_pricing::UnitPrice::new(40.0).expect("output price"),
        gw_pricing::UnitPrice::ZERO,
        gw_pricing::UnitPrice::new(7.0).expect("reasoning price"),
        gw_pricing::RateMultiplier::ONE,
        0,
    );
    let fixture = fixture();
    fixture
        .settlement
        .settle(
            &ctx_priced(quote.clone()),
            UsageOutcome {
                provider: "openai".to_owned(),
                ..UsageOutcome::precise(UsageRecord {
                    model: "o3".to_owned(),
                    provider: "openai".to_owned(),
                    input_tokens: Some(0),
                    output_tokens: Some(50),
                    cached_tokens: None,
                    reasoning_tokens: Some(20),
                })
            },
        )
        .await;

    let charged = fixture.store.settled_costs();
    assert_eq!(charged.len(), 1);
    let per_unit = |tokens: i64, price: f64| price * tokens as f64 / gw_pricing::TOKENS_PER_UNIT;
    let want =
        per_unit(30, quote.output_price().get()) + per_unit(20, quote.reasoning_price().get());
    let double_counted =
        per_unit(50, quote.output_price().get()) + per_unit(20, quote.reasoning_price().get());
    assert!(
        (charged[0] - want).abs() < 1e-12,
        "{} != {want}（重复计价会是 {double_counted}）",
        charged[0],
    );
}
