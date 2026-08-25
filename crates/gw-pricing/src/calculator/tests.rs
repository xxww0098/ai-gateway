//! 报价的铸造：命中、未命中，以及**冻结**——在途请求不会被改价追上。

use std::sync::Arc;

use super::Calculator;
use crate::cache::ModelPriceCache;
use crate::money::TokenCount;
use crate::quote::{ESTIMATED_TOKENS, TOKENS_PER_UNIT};
use crate::testsupport::{Rng, priced};
use crate::usage::{BillableUsage, ObservedUsage, UsageDialect};

const EPSILON: f64 = 1e-9;

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() <= EPSILON
}

/// 一个建在内存快照上的计价器，既保持 hermetic 又走真实的 `get`/normalize 路径。
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

/// 一份四列都不为零的用量，用来观察「报价换了没有」。
fn sample_usage() -> BillableUsage {
    ObservedUsage::new(1_000_000, 1_000_000, 0, 0)
        .expect("a non-negative envelope")
        .normalize(UsageDialect::OpenAi)
        .expect("a consistent envelope")
}

// ---------------------------------------------------------------- 冻结

/// **这一条是这把刀的理由。**
///
/// Hold 处冻下来的报价，之后无论价目表怎么变，算出来的钱都不变；
/// 而**重新**报一次价必须看到新价 —— 否则这条测的就只是「缓存没生效」。
#[test]
fn a_quote_frozen_at_hold_time_survives_a_mid_flight_price_edit() {
    let cache = Arc::new(ModelPriceCache::from_rows([priced(
        "gpt-4o", 1.0, 1.0, 0.0, 0.0,
    )]));
    let calc = Calculator::new(Some(Arc::clone(&cache)), 0.0);

    let frozen = calc.quote("gpt-4o", 1.0);
    let before = frozen.compute(sample_usage()).total_cost;
    assert!(before > 0.0, "基线必须为正，否则改价看不出差别");

    // 管理员在途改价（或周期刷新拉到了新行）。
    cache.store_rows([priced("gpt-4o", 9.0, 9.0, 0.0, 0.0)]);

    assert!(
        approx_eq(frozen.compute(sample_usage()).total_cost, before),
        "冻结的报价被改价追上了：{} != {before}",
        frozen.compute(sample_usage()).total_cost,
    );
    let refreshed = calc.quote("gpt-4o", 1.0).compute(sample_usage()).total_cost;
    assert!(
        refreshed > before,
        "新报的价必须看到新价目表：{refreshed} 应大于 {before}",
    );
    assert!(
        calc.quote("gpt-4o", 1.0).version() > frozen.version(),
        "改价必须推进缓存代次，否则事后无从回答这笔账按第几版算的",
    );
}

/// 价格键来自**请求**里的模型名，且与公开的 normalize 同口径。
/// 大小写与空白不改变报价 —— 三个 estimator 共用一个已归一化的键。
#[test]
fn the_price_key_is_the_normalized_request_model() {
    let calc = io_calc("mix-id", 3.0, 7.0);
    let canonical = calc.quote("mix-id", 1.0);
    for variant in ["Mix-Id", "MIX-ID", "  mix-id  "] {
        let quote = calc.quote(variant, 1.0);
        assert_eq!(quote.price_key(), crate::normalize_model_key(variant));
        assert_eq!(
            quote.price_key(),
            canonical.price_key(),
            "{variant} 必须和规范形共用一个价格键",
        );
        assert!(approx_eq(
            quote.compute(sample_usage()).total_cost,
            canonical.compute(sample_usage()).total_cost,
        ));
    }
    assert_ne!(
        calc.quote("other-id", 1.0)
            .compute(sample_usage())
            .total_cost,
        canonical.compute(sample_usage()).total_cost,
        "另一个模型不许和它撞价",
    );
}

// ---------------------------------------------------------------- 命中 / 未命中

/// 命中：四列各取该行的价。每列 1M token 让每一项恰好等于那一列的价。
#[test]
fn a_cache_hit_carries_all_four_columns() {
    let calc = calc_with(&[("o3", 10.0, 40.0, 2.5, 60.0)], 0.0);
    let quote = calc.quote("o3", 1.0);
    let usage = BillableUsage {
        uncached_input: TokenCount::new(1_000_000).expect("count"),
        cached_input: TokenCount::new(1_000_000).expect("count"),
        visible_output: TokenCount::new(1_000_000).expect("count"),
        reasoning_output: TokenCount::new(1_000_000).expect("count"),
    };
    let got = quote.compute(usage);
    assert!(approx_eq(got.input_cost, quote.input_price().get()));
    assert!(approx_eq(got.output_cost, quote.output_price().get()));
    assert!(approx_eq(got.cached_cost, quote.cached_price().get()));
    assert!(approx_eq(got.reasoning_cost, quote.reasoning_price().get()));
}

/// 未命中：四列**都**是兜底价，未知模型不会静默变成免费。
/// 兜底价为零时才是零 —— 这样非零结果就一定来自兜底，而不是一次意外命中。
#[test]
fn an_unknown_model_prices_every_column_at_the_default() {
    let calc = calc_with(&[], 7.0);
    let quote = calc.quote("mystery", 1.0);
    for price in [
        quote.input_price(),
        quote.output_price(),
        quote.cached_price(),
        quote.reasoning_price(),
    ] {
        assert!(approx_eq(price.get(), 7.0));
    }
    assert!(quote.estimate(false) > 0.0);

    let unpriced = calc_with(&[], 0.0).quote("mystery", 1.0);
    assert_eq!(unpriced.estimate(false), 0.0);
}

/// 一行里带 `NaN` 或负数的那一列退回兜底价，而不是毒化整笔账。
#[test]
fn a_hostile_price_column_falls_back_to_the_default() {
    let calc = calc_with(&[("m", f64::NAN, -1.0, 0.0, 0.0)], 5.0);
    let quote = calc.quote("m", 1.0);
    assert!(approx_eq(quote.input_price().get(), 5.0));
    assert!(approx_eq(quote.output_price().get(), 5.0));
    assert!(quote.cached_price().is_zero(), "合法的零价必须原样保留");
}

/// 零值 `Calculator` 的契约：无缓存 + 零兜底价 ⇒ 每个报价都收零。
#[test]
fn the_default_calculator_prices_everything_at_zero() {
    let quote = Calculator::default().quote("gpt-4o", 1.0);
    assert_eq!(quote.estimate(true), 0.0);
    assert_eq!(quote.estimate_with_max_tokens(4096, true), 0.0);
    assert_eq!(quote.estimate_with_tokens(1000, 0, false), 0.0);
    assert_eq!(quote.compute(sample_usage()).total_cost, 0.0);
    assert_eq!(quote.version(), 0, "没有缓存就没有代次");
}

// ---------------------------------------------------------------- estimator

/// preflight 闸门只有在「带上限的估算不低于它取代的流式估算」时才是可靠的。
#[test]
fn the_capped_estimate_dominates_the_plain_one_above_the_nominal_floor() {
    let mut rng = Rng::new(0x5EED_0001);
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
        let rate_mult = rng.f64_range(0.0, 10.0);
        let max_tokens = rng.i64_range(ESTIMATED_TOKENS + 1, 2_000_000);
        let quote = calc.quote("m", rate_mult);

        let base = quote.estimate(true);
        let upper = quote.estimate_with_max_tokens(max_tokens, true);
        assert!(upper >= base, "iteration {i}: upper={upper} < base={base}");
    }
}

/// 预扣对输入 token 数单调不减 —— 更大的 prompt 不许预留得更少。
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
        let quote = calc.quote("m", 1.0);
        let lo = rng.i64_range(0, 500_000);
        let hi = lo + rng.i64_range(0, 500_000);
        let stream = rng.bool();

        assert!(
            quote.estimate_with_tokens(hi, 0, stream) >= quote.estimate_with_tokens(lo, 0, stream),
            "iteration {i}: {hi} 个 token 的预扣低于 {lo} 个",
        );
    }
}

/// 估算就是「输入价 × 输入量 + 输出价 × 输出量」，除一次 —— 逐字算一遍，
/// 不抄源码常数（两个 token 数与两列价都是这里给的）。
#[test]
fn the_estimate_is_the_two_priced_columns_over_one_unit() {
    let calc = io_calc("gpt-4o", 10.0, 30.0);
    let quote = calc.quote("gpt-4o", 1.0);
    const IN_TOK: i64 = 50_000;
    const OUT_CAP: i64 = 8_000;

    let want = (quote.input_price().get() * IN_TOK as f64
        + quote.output_price().get() * OUT_CAP as f64)
        / TOKENS_PER_UNIT;
    let got = quote.estimate_with_tokens(IN_TOK, OUT_CAP, false);
    assert!(approx_eq(got, want), "{got} != {want}");
}
