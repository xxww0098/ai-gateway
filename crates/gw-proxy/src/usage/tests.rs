//! Settlement. The three modes AGENTS.md names — precise, fallback, strict —
//! plus the two failure postures that fall out of them.

use std::sync::Arc;

use gw_provider::types::UsageRecord;

use super::*;
use crate::testsupport::{FakeCalculator, FakeLedger, FakeUsageStore, LedgerCall};

// ---------------------------------------------------------------- the plan

fn inputs() -> SettlementInputs {
    SettlementInputs {
        computed_cost: 0.5,
        usage_present: true,
        upstream_failed: false,
        strict_mode: false,
        active_hold: Some(0.2),
        streaming_estimate: 0.3,
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
fn the_fallback_never_bills_below_the_hold_or_the_streaming_estimate() {
    // "no free upstream output" is the whole point of this branch.
    for hold in [0.0, 0.1, 5.0] {
        for estimate in [0.0, 0.2, 3.0] {
            let plan = plan_settlement(&SettlementInputs {
                usage_present: false,
                active_hold: Some(hold),
                streaming_estimate: estimate,
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
        settlement: Settlement::new(ledger.clone(), FakeCalculator::shared(), store.clone()),
        ledger,
        store,
    }
}

/// A context with a freshly-minted operation. Tests that need the reservation
/// to line up bind it once and pass `&ctx` — the operation id *is* the key.
fn ctx() -> SettleCtx {
    SettleCtx {
        user_id: 7,
        rate_mult: 1.0,
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

// ------------------------------------------------- Google 的思考 token 计费

/// 一个只给 `output` 列定价、`reasoning` 列定价为 **0** 的计价器。
///
/// 这不是随手编的：`model_prices.reasoning_price_per1_m` 的**建表默认值就是 0**
/// （`migrations/0001_init.sql`），绝大多数部署从没填过这一列。
struct OutputOnlyCalculator;

impl PricingCalculator for OutputOnlyCalculator {
    fn estimate(&self, _model: &str, _stream: bool, _rate_mult: f64) -> f64 {
        0.0
    }
    fn estimate_with_max_tokens(
        &self,
        _model: &str,
        _max_output_tokens: i64,
        _stream: bool,
        _rate_mult: f64,
    ) -> f64 {
        0.0
    }
    fn estimate_with_tokens(
        &self,
        _model: &str,
        _input_tokens: i64,
        _max_output_tokens: i64,
        _stream: bool,
        _rate_mult: f64,
    ) -> f64 {
        0.0
    }
    fn compute(&self, _model: &str, tokens: TokenUsage, rate_mult: f64) -> f64 {
        // reasoning 列不计价 —— 这正是建表默认值下的真实行为。
        (tokens.input + tokens.output + tokens.cached) as f64 * rate_mult
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

#[tokio::test]
async fn google_thinking_tokens_are_not_free() {
    // Google 的 `candidatesTokenCount` **不含** `thoughtsTokenCount`
    // （OpenAI 的 `completion_tokens` 是含的），而 reasoning 列默认不计价。
    // 两件事叠起来，思考型模型的每一个思考 token 都是免费的 ——
    // 而思考 token 在推理模型上经常是输出的数倍。
    let quiet = settle_google(google_usage(100, 0)).await;
    let thinking = settle_google(google_usage(100, 400)).await;

    assert!(
        thinking > quiet,
        "同样的输出、多了 400 个思考 token，收的钱却没变：{thinking} vs {quiet}",
    );
}

#[test]
fn only_googles_output_field_needs_the_fold() {
    let raw = TokenUsage {
        input: 10,
        output: 100,
        cached: 0,
        reasoning: 400,
    };
    for google in ["gemini", "vertex"] {
        let folded = billable_tokens(google, raw);
        assert_eq!(
            folded.output,
            raw.output + raw.reasoning,
            "{google} 的输出字段不含思考 token，必须折进来",
        );
        assert_eq!(folded.reasoning, 0, "折进来之后不能再按 reasoning 计一次");
        assert_eq!(folded.input, raw.input);
    }
    for other in ["openai", "codex", "claude"] {
        assert_eq!(
            billable_tokens(other, raw),
            raw,
            "{other} 的输出字段本来就含思考 token，再折一次就是重复计费",
        );
    }
}

/// 跑一次完整结算，返回落账的金额。
async fn settle_google(usage: UsageRecord) -> f64 {
    let ledger = FakeLedger::with_balance(1_000.0);
    let store = FakeUsageStore::shared();
    let settlement = Settlement::new(ledger, Arc::new(OutputOnlyCalculator), store.clone());
    settlement
        .settle(
            &ctx(),
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
