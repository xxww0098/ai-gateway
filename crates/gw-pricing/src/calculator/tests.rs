//! Unit tests for the pricing calculator.

use std::sync::Arc;

use super::{Calculator, ESTIMATED_TOKENS, STREAM_MULTIPLIER, TokenUsage};
use crate::cache::ModelPriceCache;
use crate::testsupport::{Rng, priced};

const EPSILON: f64 = 1e-9;

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() <= EPSILON
}

/// Builds a calculator over an in-memory snapshot, so the tests stay
/// hermetic while still going through the real `get`/normalize path.
fn calc_with(prices: &[(&str, f64, f64, f64, f64)], default_price_per_1m: f64) -> Calculator {
    let rows = prices
        .iter()
        .map(|&(id, input, output, cached, reasoning)| {
            priced(id, input, output, cached, reasoning)
        });
    Calculator::new(
        Some(Arc::new(ModelPriceCache::from_rows(rows))),
        default_price_per_1m,
    )
}

fn io_calc(model: &str, input: f64, output: f64) -> Calculator {
    calc_with(&[(model, input, output, 0.0, 0.0)], 1.0)
}

// ---------------------------------------------------------------- estimate

/// A cache hit with non-zero prices must not price a request at nothing.
#[test]
fn estimate_of_a_priced_model_is_positive() {
    let calc = io_calc("gpt-4o", 2.50, 10.00);
    assert!(calc.estimate("gpt-4o", false, 1.0) > 0.0);
}

/// The streaming headroom the hold middleware relies on to avoid
/// under-reserving long responses.
#[test]
fn streaming_estimate_exceeds_the_non_streaming_one() {
    let calc = io_calc("gpt-4o", 2.50, 10.00);
    let non_stream = calc.estimate("gpt-4o", false, 1.0);
    let streamed = calc.estimate("gpt-4o", true, 1.0);
    assert!(
        streamed > non_stream,
        "stream={streamed} must exceed non-stream={non_stream}"
    );
}

/// An unpriced model still reserves funds via the default price — and a
/// zero default is what proves the non-zero result really came from that
/// fallback rather than from a stray cache hit.
#[test]
fn unknown_models_fall_back_to_the_default_price() {
    let priced = calc_with(&[], 2.0);
    assert!(priced.estimate("does-not-exist", false, 1.0) > 0.0);

    let unpriced = calc_with(&[], 0.0);
    assert_eq!(unpriced.estimate("does-not-exist", false, 1.0), 0.0);
}

/// The zero-value `Calculator` contract: a calculator with no cache and a
/// zero default price returns 0 from every method.
#[test]
fn the_default_calculator_prices_everything_at_zero() {
    let calc = Calculator::default();
    assert_eq!(calc.estimate("gpt-4o", true, 1.0), 0.0);
    assert_eq!(
        calc.estimate_with_max_tokens("gpt-4o", 4096, true, 1.0),
        0.0
    );
    assert_eq!(
        calc.estimate_with_tokens("gpt-4o", 1000, 0, false, 1.0),
        0.0
    );
    assert_eq!(
        calc.compute(
            "gpt-4o",
            TokenUsage {
                input: 1_000_000,
                output: 1_000_000,
                cached: 1_000_000,
                reasoning: 1_000_000,
            },
            1.0,
        )
        .total_cost,
        0.0
    );
}

// ------------------------------------------------- estimate_with_max_tokens

/// The preflight gate is only sound if the max-tokens-aware estimate is
/// never *below* the streaming estimate it replaces, whenever the client's
/// cap exceeds the nominal floor.
#[test]
fn max_tokens_estimate_is_an_upper_bound_on_the_plain_estimate() {
    let mut rng = Rng::new(0x5EED_0001);
    for i in 0..500 {
        let input_per_1m = rng.f64_range(0.0, 100.0);
        let output_per_1m = rng.f64_range(0.0, 100.0);
        let max_tokens = rng.i64_range(ESTIMATED_TOKENS + 1, 2_000_000);
        let rate_mult = rng.f64_range(0.0, 10.0);

        let calc = calc_with(&[("m", input_per_1m, output_per_1m, 0.0, 0.0)], 0.0);
        let base = calc.estimate("m", true, rate_mult);
        let upper = calc.estimate_with_max_tokens("m", max_tokens, true, rate_mult);

        assert!(
            upper >= base,
            "iteration {i}: upper={upper} must be >= base={base} \
             (input={input_per_1m} output={output_per_1m} max={max_tokens} rate={rate_mult})"
        );
    }
}

/// A client that omits or malforms the cap gets the conservative estimate,
/// bit for bit — no arithmetic divergence is tolerated on the fallback branch.
#[test]
fn a_non_positive_cap_falls_back_to_the_plain_estimate_exactly() {
    let mut rng = Rng::new(0x5EED_0002);
    for i in 0..500 {
        let input_per_1m = rng.f64_range(0.0, 100.0);
        let output_per_1m = rng.f64_range(0.0, 100.0);
        let max_tokens = rng.i64_range(-1_000_000, 0);
        let rate_mult = rng.f64_range(0.0, 10.0);
        let streaming = rng.bool();

        let calc = calc_with(&[("m", input_per_1m, output_per_1m, 0.0, 0.0)], 0.0);
        let base = calc.estimate("m", streaming, rate_mult);
        let fallback = calc.estimate_with_max_tokens("m", max_tokens, streaming, rate_mult);

        assert_eq!(
            fallback, base,
            "iteration {i}: max_tokens={max_tokens} must delegate to estimate verbatim"
        );
    }
}

/// A larger client cap can only raise the upper bound, never lower it —
/// otherwise a generous `max_tokens` could sneak past a gate a stingier one
/// would have blocked.
#[test]
fn the_upper_bound_is_monotonic_in_the_client_cap() {
    let mut rng = Rng::new(0x5EED_0003);
    for i in 0..300 {
        let output_per_1m = rng.f64_range(0.0, 100.0);
        let lo = rng.i64_range(1, 1_000_000);
        let hi = lo + rng.i64_range(0, 1_000_000);
        let calc = calc_with(
            &[("m", rng.f64_range(0.0, 100.0), output_per_1m, 0.0, 0.0)],
            0.0,
        );

        let lo_cost = calc.estimate_with_max_tokens("m", lo, false, 1.0);
        let hi_cost = calc.estimate_with_max_tokens("m", hi, false, 1.0);
        assert!(
            hi_cost >= lo_cost,
            "iteration {i}: cap {hi} priced at {hi_cost} below cap {lo} at {lo_cost}"
        );
    }
}

// ----------------------------------------------------- estimate_with_tokens

/// The reservation tracks the real prompt size, which is what stops a
/// 200k-token request from under-holding and draining the balance below zero
/// at settle.
#[test]
fn the_reservation_scales_with_the_real_input_size() {
    let calc = io_calc("gpt-4o", 10.00, 30.00);

    let small = calc.estimate_with_tokens("gpt-4o", 1_000, 0, false, 1.0);
    let large = calc.estimate_with_tokens("gpt-4o", 200_000, 0, false, 1.0);
    assert!(large > small, "large={large} must exceed small={small}");

    let flat = calc.estimate("gpt-4o", false, 1.0);
    assert!(
        large > flat * 10.0,
        "a 200k prompt ({large}) must dwarf the flat estimate ({flat})"
    );
}

/// Tiny prompts still reserve the historical nominal floor, so small-request
/// behaviour is unchanged.
#[test]
fn a_tiny_prompt_is_priced_at_the_nominal_floor() {
    let calc = io_calc("gpt-4o", 10.00, 30.00);
    let floored = calc.estimate_with_tokens("gpt-4o", 10, 0, false, 1.0);
    let nominal = calc.estimate_with_tokens("gpt-4o", ESTIMATED_TOKENS, 0, false, 1.0);
    assert!(approx_eq(floored, nominal), "{floored} != {nominal}");
}

/// 50k input at $10/1M plus an 8k output cap at $30/1M is $0.74.
#[test]
fn the_output_cap_stream_flag_and_rate_multiplier_all_apply() {
    let calc = io_calc("gpt-4o", 10.00, 30.00);
    const IN_TOK: i64 = 50_000;
    const OUT_CAP: i64 = 8_000;
    const WANT: f64 = 0.74;

    let got = calc.estimate_with_tokens("gpt-4o", IN_TOK, OUT_CAP, false, 1.0);
    assert!(approx_eq(got, WANT), "{got} != {WANT}");

    let streamed = calc.estimate_with_tokens("gpt-4o", IN_TOK, OUT_CAP, true, 1.0);
    assert!(approx_eq(streamed, WANT * STREAM_MULTIPLIER));

    let scaled = calc.estimate_with_tokens("gpt-4o", IN_TOK, OUT_CAP, false, 2.0);
    assert!(approx_eq(scaled, WANT * 2.0));
}

/// Every estimate is monotonically non-decreasing in the input token count —
/// a bigger prompt can never reserve less.
#[test]
fn the_reservation_is_monotonic_in_the_input_token_count() {
    let mut rng = Rng::new(0x5EED_0004);
    for i in 0..300 {
        let calc = calc_with(
            &[(
                "m",
                rng.f64_range(0.0, 100.0),
                rng.f64_range(0.0, 100.0),
                0.0,
                0.0,
            )],
            0.0,
        );
        let lo = rng.i64_range(0, 500_000);
        let hi = lo + rng.i64_range(0, 500_000);
        let stream = rng.bool();

        let lo_cost = calc.estimate_with_tokens("m", lo, 0, stream, 1.0);
        let hi_cost = calc.estimate_with_tokens("m", hi, 0, stream, 1.0);
        assert!(
            hi_cost >= lo_cost,
            "iteration {i}: {hi} tokens priced at {hi_cost} below {lo} tokens at {lo_cost}"
        );
    }
}

// ----------------------------------------------------------------- compute

/// Each of the four columns is multiplied by its own price. Exercising 1M
/// tokens per column makes each contribution equal the price itself.
#[test]
fn compute_prices_each_token_column_against_its_own_rate() {
    let calc = calc_with(&[("o3", 10.0, 40.0, 2.5, 60.0)], 0.0);
    let tokens = TokenUsage {
        input: 1_000_000,
        output: 1_000_000,
        cached: 1_000_000,
        reasoning: 1_000_000,
    };

    let got = calc.compute("o3", tokens, 1.0);
    assert!(approx_eq(got.total_cost, 112.5), "{}", got.total_cost);
    assert!(approx_eq(got.input_cost, 10.0));
    assert!(approx_eq(got.output_cost, 40.0));
    assert!(approx_eq(got.cached_cost, 2.5));
    assert!(approx_eq(got.reasoning_cost, 60.0));
}

/// The rate multiplier is a final linear scaling, so doubling it doubles
/// the bill.
#[test]
fn the_rate_multiplier_scales_the_cost_linearly() {
    let calc = calc_with(&[("gpt-4o", 2.5, 10.0, 0.0, 0.0)], 0.0);
    let tokens = TokenUsage {
        input: 1_000_000,
        output: 500_000,
        ..TokenUsage::default()
    };

    let single = calc.compute("gpt-4o", tokens, 1.0).total_cost;
    let doubled = calc.compute("gpt-4o", tokens, 2.0).total_cost;

    assert!(single > 0.0, "baseline must be positive, got {single}");
    assert!(
        approx_eq(doubled, single * 2.0),
        "{doubled} != {single} * 2"
    );
}

/// A request that consumed nothing costs exactly nothing, whatever the
/// model's prices or the default fallback. This is the settle path for a
/// request that failed before producing tokens.
#[test]
fn zero_tokens_cost_exactly_zero() {
    let calc = calc_with(&[("gpt-4o", 2.5, 10.0, 0.1, 5.0)], 5.0);
    let got = calc.compute("gpt-4o", TokenUsage::default(), 1.0);
    assert_eq!(got.total_cost, 0.0);
    assert_eq!(got.input_cost, 0.0);
    assert_eq!(got.output_cost, 0.0);
    assert_eq!(got.cached_cost, 0.0);
    assert_eq!(got.reasoning_cost, 0.0);
}

/// An unpriced model bills every column at the default rate — the same shape
/// as the estimate fallback, so an unknown model is never silently free.
#[test]
fn compute_prices_unknown_models_at_the_default_rate() {
    let calc = calc_with(&[], 7.0);
    let one_million_each = TokenUsage {
        input: 1_000_000,
        output: 1_000_000,
        cached: 1_000_000,
        reasoning: 1_000_000,
    };
    let got = calc.compute("mystery", one_million_each, 1.0);
    assert!(approx_eq(got.total_cost, 28.0), "{}", got.total_cost);
}

/// The itemised columns must account for the whole bill: no cost may hide
/// outside the four components the panel renders.
#[test]
fn the_itemised_columns_account_for_the_total() {
    let mut rng = Rng::new(0x5EED_0005);
    for i in 0..300 {
        let calc = calc_with(
            &[(
                "m",
                rng.f64_range(0.0, 100.0),
                rng.f64_range(0.0, 100.0),
                rng.f64_range(0.0, 100.0),
                rng.f64_range(0.0, 100.0),
            )],
            0.0,
        );
        let tokens = TokenUsage {
            input: rng.i64_range(0, 2_000_000),
            output: rng.i64_range(0, 2_000_000),
            cached: rng.i64_range(0, 2_000_000),
            reasoning: rng.i64_range(0, 2_000_000),
        };
        let rate = rng.f64_range(0.0, 5.0);

        let got = calc.compute("m", tokens, rate);
        let summed = got.input_cost + got.output_cost + got.cached_cost + got.reasoning_cost;
        assert!(
            (summed - got.total_cost).abs() <= 1e-9 * got.total_cost.abs().max(1.0),
            "iteration {i}: components sum to {summed}, total is {}",
            got.total_cost
        );
    }
}

/// The calculator reads through the shared cache handle, so a price edit
/// published by the panel is visible to billing on the very next request —
/// no restart, no second cache.
#[tokio::test]
async fn a_price_edit_is_visible_to_an_already_built_calculator() {
    let cache = Arc::new(ModelPriceCache::from_rows([priced(
        "gpt-test", 1.0, 0.0, 0.0, 0.0,
    )]));
    let calc = Calculator::new(Some(Arc::clone(&cache)), 0.0);

    let before = calc.compute(
        "gpt-test",
        TokenUsage {
            input: 1_000_000,
            ..TokenUsage::default()
        },
        1.0,
    );

    // Whatever republishes the snapshot — an admin edit through the panel or a
    // periodic refresh — the calculator sees it without being rebuilt.
    cache.store_rows([priced("gpt-test", 2.0, 0.0, 0.0, 0.0)]);

    let after = calc.compute(
        "gpt-test",
        TokenUsage {
            input: 1_000_000,
            ..TokenUsage::default()
        },
        1.0,
    );

    assert!(
        after.total_cost > before.total_cost,
        "calculator kept the stale price: before={} after={}",
        before.total_cost,
        after.total_cost
    );
}
