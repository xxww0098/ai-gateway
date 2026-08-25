//! Delegation to `gw_pricing::Calculator`.
//!
//! The calculator is a pure in-memory value (no cache, no database), so the
//! adapter is testable end to end without any infrastructure.

use gw_pricing::{ModelPriceCache, ObservedUsage, UsageDialect};

use super::*;

/// A calculator with no price table: every model falls back to the default
/// per-1M rate, which is enough to observe the shape of each estimate.
fn calculator(default_price_per_1m: f64) -> SharedCalculator {
    SharedCalculator::new(Arc::new(gw_pricing::Calculator::new(
        None,
        default_price_per_1m,
    )))
}

/// 一份非零的可计价用量。
fn usage(input: i64, output: i64) -> gw_pricing::BillableUsage {
    ObservedUsage::new(input, output, 0, 0)
        .expect("a non-negative envelope")
        .normalize(UsageDialect::OpenAi)
        .expect("a consistent envelope")
}

#[test]
fn the_streaming_estimate_is_never_below_the_unary_one() {
    // The reservation biases upward for a stream because its output length is
    // unknown until it ends.
    let quote = calculator(2.0).quote("gpt-4o", 1.0);
    assert!(quote.estimate(true) >= quote.estimate(false));
}

#[test]
fn an_output_cap_tightens_the_estimate_and_its_absence_falls_back() {
    let quote = calculator(2.0).quote("gpt-4o", 1.0);
    let capped = quote.estimate_with_max_tokens(10, false);
    let uncapped = quote.estimate_with_max_tokens(0, false);
    assert!(capped < uncapped, "a cap of 10 tokens should cost less");
    assert_eq!(
        uncapped,
        quote.estimate(false),
        "a non-positive cap must fall back to the plain estimate, not to zero",
    );
}

#[test]
fn the_reservation_grows_with_the_prompt() {
    let quote = calculator(2.0).quote("gpt-4o", 1.0);
    let small = quote.estimate_with_tokens(10, 100, false);
    let large = quote.estimate_with_tokens(100_000, 100, false);
    assert!(large > small, "{large} should exceed {small}");
}

#[test]
fn the_rate_multiplier_scales_every_estimate_linearly() {
    let calc = calculator(2.0);
    let full = calc.quote("gpt-4o", 1.0).estimate(false);
    let half = calc.quote("gpt-4o", 0.5).estimate(false);
    assert!(
        (full * 0.5 - half).abs() < 1e-9,
        "{half} should be half of {full}"
    );
}

#[test]
fn a_zero_token_settlement_costs_nothing() {
    let quote = calculator(2.0).quote("gpt-4o", 1.0);
    assert_eq!(
        quote
            .compute(gw_pricing::BillableUsage::default())
            .total_cost,
        0.0,
    );
}

/// 适配器必须**读到共享的那份价目表**，否则面板改的价和计费读的价是两份。
///
/// 可观察后果：表里有的模型按表里的价报，表里没有的才落兜底价。
/// （「改价对下一次报价可见、对已冻结的报价不可见」由 `gw-pricing` 自己按
/// 性质卡住，不在这一层重复。）
#[test]
fn the_adapter_quotes_through_the_shared_price_table() {
    let cache = Arc::new(ModelPriceCache::from_rows([gw_model::ModelPrice {
        id: 1,
        model_id: "gpt-4o".to_owned(),
        input_price_per_1m: 1.0,
        output_price_per_1m: 1.0,
        cached_input_price_per_1m: 0.0,
        reasoning_price_per_1m: 0.0,
        created_at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        updated_at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
    }]));
    let calc = SharedCalculator::new(Arc::new(gw_pricing::Calculator::new(
        Some(Arc::clone(&cache)),
        50.0,
    )));

    let listed = calc
        .quote("gpt-4o", 1.0)
        .compute(usage(1_000_000, 0))
        .total_cost;
    let unlisted = calc
        .quote("never-priced", 1.0)
        .compute(usage(1_000_000, 0))
        .total_cost;
    assert!(listed > 0.0, "表里那一行的价必须被读到");
    assert!(
        unlisted > listed,
        "表里没有的模型该落兜底价（这里配得更贵），得到 {unlisted} vs {listed}",
    );
}
