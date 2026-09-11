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

fn ctx() -> SettleCtx {
    SettleCtx {
        request_id: "req-1".to_owned(),
        user_id: 7,
        rate_mult: 1.0,
        model: "gpt-4o".to_owned(),
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
    fixture
        .ledger
        .hold(7, 1.0, "req-1", std::time::Duration::from_secs(60))
        .await
        .expect("hold");

    fixture
        .settlement
        .settle(&ctx(), UsageOutcome::precise(usage(100, 200)))
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
        ["req-1"],
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
    fixture
        .ledger
        .hold(7, 2.5, "req-1", std::time::Duration::from_secs(60))
        .await
        .expect("hold");

    fixture
        .settlement
        .settle(&ctx(), UsageOutcome::default())
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
    fixture
        .ledger
        .hold(7, 2.5, "req-1", std::time::Duration::from_secs(60))
        .await
        .expect("hold");
    fixture.settlement.set_strict_usage_metadata(true);

    fixture
        .settlement
        .settle(&ctx(), UsageOutcome::default())
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
        fixture.ledger.held_amount("req-1"),
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
    *fixture.ledger.hold_lookup_errors.lock() = true;

    fixture
        .settlement
        .settle(&ctx(), UsageOutcome::default())
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
    fixture
        .ledger
        .hold(7, 1.0, "req-1", std::time::Duration::from_secs(60))
        .await
        .expect("hold");

    fixture
        .settlement
        .settle(&ctx(), UsageOutcome::failed())
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
    *fixture.store.commit_fails.lock() = true;
    fixture
        .ledger
        .hold(7, 1.0, "req-1", std::time::Duration::from_secs(60))
        .await
        .expect("hold");

    fixture
        .settlement
        .settle(&ctx(), UsageOutcome::precise(usage(10, 10)))
        .await;

    assert!(
        fixture.store.cleared_holds.lock().is_empty(),
        "a hold cleared after a rollback would charge nothing but lose the reservation",
    );
    assert_eq!(fixture.ledger.held_amount("req-1"), Some(1.0));

    let logs = fixture.store.logs.lock();
    let failed = &logs[0];
    assert!(failed.failed);
    // 回滚意味着**一分钱都没动**，所以这条审计行的金额列必须是 0：面板的总花费
    // 直接 SUM(usage_logs.cost)、不按 failed 过滤，留着全额就会把一次失败的尝试
    // 算成真花掉的钱，与 balance_logs 对不上账。
    assert_eq!(
        (failed.total_cost, failed.actual_cost, failed.cost),
        (0.0, 0.0, 0.0),
        "回滚的结算行不许带可汇总的金额",
    );
    // 尝试收多少不能丢 —— 它是运维判断这次回滚值不值得追的唯一线索。
    let attempted = failed
        .raw_metadata
        .as_ref()
        .and_then(|meta| meta.get("attempted_cost"))
        .and_then(serde_json::Value::as_f64);
    assert!(
        attempted.is_some_and(|cost| cost > 0.0),
        "失败行必须留下尝试扣款的金额：{:?}",
        failed.raw_metadata,
    );
}

#[tokio::test]
async fn a_partial_debit_records_the_shortfall_on_the_usage_row() {
    let fixture = fixture();
    *fixture.store.shortfall.lock() = 0.75;

    fixture
        .settlement
        .settle(&ctx(), UsageOutcome::precise(usage(10, 10)))
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
    *fixture.store.balance_before.lock() = 0.4;
    *fixture.store.balance_after.lock() = 0.0;

    fixture
        .settlement
        .settle(&ctx(), UsageOutcome::precise(usage(10, 10)))
        .await;

    let events = fixture.store.balance_events.lock();
    assert!(events.iter().any(|e| e.event_type == "balance_depleted"));
    assert_eq!(
        events[0].reference, "req-1",
        "the event must be traceable to its request"
    );
}

#[tokio::test]
async fn the_usage_row_carries_the_credential_that_served_the_request() {
    let fixture = fixture();
    fixture
        .settlement
        .settle(
            &ctx(),
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

// --------------------------------------- 四列互斥 + 真实费率解析的计价

/// 真价目表上的**真**计价器，不是替身。
///
/// 这一节测的是「某一块 token 有没有被白送或被卖两次」，而两件事都由真
/// `gw_pricing::Calculator` 的两条规则共同决定（四列相加 + 子费率留空时回落到
/// 基准费率）。把费率抄进一个替身里，测出来的就只是替身的行为；所以这里直接搭
/// 真 `Calculator`。行取 `ModelPrice` 实体的形状 —— 缓存读的就是这个实体。
fn priced_calculator(rows: &[(&str, f64, f64, f64, f64)]) -> Arc<gw_pricing::Calculator> {
    let rows = rows.iter().map(|&(model_id, input, output, cached, reasoning)| {
        gw_model::ModelPrice {
            id: 1,
            model_id: model_id.to_owned(),
            input_price_per_1m: input,
            output_price_per_1m: output,
            cached_input_price_per_1m: cached,
            reasoning_price_per_1m: reasoning,
            created_at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
            updated_at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        }
    });
    Arc::new(gw_pricing::Calculator::new(
        Some(Arc::new(gw_pricing::ModelPriceCache::from_rows(rows))),
        0.0,
    ))
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

// --------------------------------------------------- 四列互斥（计价视图）

/// `specs/billing-hardening/slices/06` 的金样。
///
/// OpenAI 官方 prompt-caching 指南给的正是这个减法：
/// `ordinaryInputTokens = inputTokens - cachedTokens - cacheWriteTokens`；
/// `cached_tokens` 与 `reasoning_tokens` 在 API 参考里都写作「Breakdown of
/// tokens used in ...」，即子集而不是并列项。
#[test]
fn openai_nested_columns_are_priced_once() {
    let raw = TokenUsage {
        input: 120,
        output: 80,
        cached: 40,
        reasoning: 30,
    };
    let billable = billable_tokens("openai", raw);
    assert_eq!(billable.input, 80, "cached ⊆ prompt_tokens，不减就是全价再收一次");
    assert_eq!(billable.output, 50, "reasoning ⊆ completion_tokens，同理");
    assert_eq!(billable.cached, 40, "缓存那一段本身仍要按缓存价收");
    assert_eq!(billable.reasoning, 30, "思考那一段本身仍要按推理价收");
    assert_eq!(
        billable_tokens("codex", raw),
        billable,
        "codex 走同一条 OpenAI 线格式，不能只在 openai 上修",
    );
}

/// Google 只在**输入**侧与 OpenAI 同形。
///
/// `promptTokenCount` 的原文是 "this includes the number of tokens in the cached
/// content"；而 `totalTokenCount = prompt + thoughts + candidates` ——
/// 思考与候选是**并列**项，所以输出侧要相加而不是相减。
#[test]
fn google_folds_thoughts_and_excludes_cached_from_the_prompt() {
    let raw = TokenUsage {
        input: 1_000,
        output: 100,
        cached: 800,
        reasoning: 400,
    };
    for google in ["gemini", "vertex"] {
        let billable = billable_tokens(google, raw);
        assert_eq!(
            billable.input, 200,
            "{google}: promptTokenCount 含 cachedContentTokenCount，不减就是收两次",
        );
        assert_eq!(
            billable.output, 500,
            "{google}: candidatesTokenCount 不含 thoughtsTokenCount，必须折进来",
        );
        assert_eq!(billable.reasoning, 0, "{google}: 折进来之后不能再计一次");
        assert_eq!(billable.cached, 800);
    }
}

/// Anthropic 的四列本来就是并列的。
///
/// 原文："Total input tokens in a request is the summation of `input_tokens`,
/// `cache_creation_input_tokens`, and `cache_read_input_tokens`"，且
/// `output_tokens` 已含 thinking token。照着 OpenAI 的样子减一次就是实打实少收。
#[test]
fn anthropic_columns_are_already_disjoint() {
    let raw = TokenUsage {
        input: 100,
        output: 200,
        cached: 910,
        reasoning: 0,
    };
    assert_eq!(billable_tokens("claude", raw), raw);
}

/// 判定标准是**线格式**，不是「不是 Google 就要折」。
///
/// 不认识的上游一律原样计价：多减会少收，少减只是按原价收，两个方向的风险
/// 不对称。
#[test]
fn an_unknown_provider_is_priced_as_reported() {
    let raw = TokenUsage {
        input: 10,
        output: 20,
        cached: 3,
        reasoning: 4,
    };
    assert_eq!(billable_tokens("some-new-vendor", raw), raw);
}

/// 端到端金样：一次缓存很重的 OpenAI 请求，落账金额必须等于厂商口径。
///
/// 这一条才是「静默多收费」的回归门禁 —— 上面几条只盯 token 视图，
/// 这里把真计价器接上，金额对不上就是账错了。
#[tokio::test]
async fn an_openai_cache_hit_is_not_sold_twice() {
    // gpt-4o 的官方档位（USD / 1M）：input 2.5 / cached input 1.25 / output 10，
    // reasoning 没有独立档位，所以留 0 由费率解析回落到 output 价。
    let calculator = priced_calculator(&[("gpt-4o", 2.5, 10.0, 1.25, 0.0)]);
    let usage = UsageRecord {
        model: "gpt-4o".to_owned(),
        provider: "openai".to_owned(),
        input_tokens: Some(100_000),
        output_tokens: Some(25_000),
        cached_tokens: Some(80_000),
        reasoning_tokens: Some(20_000),
    };

    let charged = settle_with(calculator, usage).await;

    // 厂商口径：普通输入 (100k−80k)×2.5 + 缓存 80k×1.25 + 输出 25k×10
    //         = 50_000 + 100_000 + 250_000 = 400_000 → $0.40/1M 单位
    // 修之前是 100k×2.5 + 25k×10 + 80k×1.25 = 600_000 → $0.60，缓存那 80k
    // 被「全价 input + 缓存价」卖了两次，贵 50%。
    let expected = 0.4;
    assert!(
        (charged - expected).abs() < 1e-9,
        "OpenAI 缓存命中被卖两次：落账 {charged}，厂商口径 {expected}",
    );
}

/// 跑一次完整结算，返回落账的金额。
async fn settle_with(calculator: Arc<gw_pricing::Calculator>, usage: UsageRecord) -> f64 {
    let ledger = FakeLedger::with_balance(1_000.0);
    let store = FakeUsageStore::shared();
    let settlement = Settlement::new(
        ledger,
        Arc::new(crate::adapters::pricing::SharedCalculator::new(calculator)),
        store.clone(),
    );
    settlement
        .settle(
            &ctx(),
            UsageOutcome {
                provider: usage.provider.clone(),
                ..UsageOutcome::precise(usage)
            },
        )
        .await;
    let costs = store.settled_costs();
    assert_eq!(costs.len(), 1, "一次请求恰好结算一次");
    costs[0]
}

/// 一次 Gemini 结算。价目表的 `reasoning` 列**留空** —— 建表默认值就是 0，
/// 而费率解析会把它读成「按 output 价走」，这正是 Google 自己的口径
/// （"Output price (including thinking tokens)"）。
async fn settle_google(usage: UsageRecord) -> f64 {
    settle_with(
        priced_calculator(&[("a-thinking-model", 1.0, 4.0, 0.25, 0.0)]),
        usage,
    )
    .await
}
