//! Delegation to `gw_pricing::Calculator`.
//!
//! The calculator is a pure in-memory value (no cache, no database), so the
//! adapter is testable end to end without any infrastructure.

use super::*;

/// A calculator with no price table: every model falls back to the default
/// per-1M rate, which is enough to observe the shape of each estimate.
fn calculator(default_price_per_1m: f64) -> SharedCalculator {
    SharedCalculator::new(Arc::new(Calculator::new(None, default_price_per_1m)))
}

#[test]
fn the_streaming_estimate_is_never_below_the_unary_one() {
    // The reservation biases upward for a stream because its output length is
    // unknown until it ends.
    let calc = calculator(2.0);
    let unary = calc.estimate("gpt-4o", false, 1.0);
    let streamed = calc.estimate("gpt-4o", true, 1.0);
    assert!(streamed >= unary, "{streamed} should be at least {unary}");
}

#[test]
fn an_output_cap_tightens_the_estimate_and_its_absence_falls_back() {
    let calc = calculator(2.0);
    let capped = calc.estimate_with_max_tokens("gpt-4o", 10, false, 1.0);
    let uncapped = calc.estimate_with_max_tokens("gpt-4o", 0, false, 1.0);
    assert!(capped < uncapped, "a cap of 10 tokens should cost less");
    assert_eq!(
        uncapped,
        calc.estimate("gpt-4o", false, 1.0),
        "a non-positive cap must fall back to the plain estimate, not to zero",
    );
}

#[test]
fn the_reservation_grows_with_the_prompt() {
    let calc = calculator(2.0);
    let small = calc.estimate_with_tokens("gpt-4o", 10, 100, false, 1.0);
    let large = calc.estimate_with_tokens("gpt-4o", 100_000, 100, false, 1.0);
    assert!(large > small, "{large} should exceed {small}");
}

#[test]
fn the_rate_multiplier_scales_every_estimate_linearly() {
    let calc = calculator(2.0);
    let full = calc.estimate("gpt-4o", false, 1.0);
    let half = calc.estimate("gpt-4o", false, 0.5);
    assert!(
        (full * 0.5 - half).abs() < 1e-9,
        "{half} should be half of {full}"
    );
}

#[test]
fn compute_returns_the_number_that_gets_debited() {
    let calc = calculator(2.0);
    let tokens = TokenUsage {
        input: 1_000,
        output: 2_000,
        cached: 0,
        reasoning: 0,
    };
    let debited = calc.compute("gpt-4o", tokens, 1.0);
    let breakdown = calc.inner().compute("gpt-4o", into_pricing(tokens), 1.0);
    assert_eq!(
        debited, breakdown.total_cost,
        "the port must surface the total, not one of the itemised columns",
    );
}

#[test]
fn a_zero_token_settlement_costs_nothing() {
    let calc = calculator(2.0);
    assert_eq!(calc.compute("gpt-4o", TokenUsage::default(), 1.0), 0.0);
}
