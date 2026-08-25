//! 冻结报价：估算、精算，以及「在途请求不会被改价追上」这条性质。

use super::*;
use crate::usage::{ObservedUsage, UsageDialect};

const EPSILON: f64 = 1e-9;

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() <= EPSILON
}

/// 四列分别定价的报价。
fn quote(input: f64, output: f64, cached: f64, reasoning: f64) -> PricingQuote {
    PricingQuote::new(
        "m".to_owned(),
        UnitPrice::new(input).expect("input price"),
        UnitPrice::new(output).expect("output price"),
        UnitPrice::new(cached).expect("cached price"),
        UnitPrice::new(reasoning).expect("reasoning price"),
        RateMultiplier::ONE,
        0,
    )
}

/// 按方言折出来的可计价视图。归一化失败在这里是测试写错了。
fn billable(dialect: UsageDialect, observed: ObservedUsage) -> BillableUsage {
    observed.normalize(dialect).expect("a consistent envelope")
}

// ---------------------------------------------------------------- compute

/// 四列各按各自的价。每列各 1M token，于是每一项的贡献恰好等于那一列的价，
/// 断言里就没有从源码抄来的常数。
#[test]
fn each_column_is_priced_against_its_own_rate() {
    let quote = quote(10.0, 40.0, 2.5, 60.0);
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
    assert!(approx_eq(
        got.total_cost,
        quote.input_price().get()
            + quote.output_price().get()
            + quote.cached_price().get()
            + quote.reasoning_price().get(),
    ));
}

/// **OpenAI 的思考 token 不许收两遍。**
///
/// `completion_tokens` 含 `reasoning_tokens`，所以可见输出是两者之差。
/// 把归一化里的减法去掉（`visible_output = output`），这条就会红。
#[test]
fn openai_reasoning_is_not_charged_twice() {
    // 两列价不同且都为正 —— 否则「按哪一列收」根本观察不到。
    let quote = quote(1.0, 40.0, 0.0, 7.0);
    let observed = ObservedUsage::new(100, 50, 0, 20).expect("a non-negative envelope");
    let usage = billable(UsageDialect::OpenAi, observed);

    // 期望值从**上游原话**推，不从生产代码抄：可见输出 = 输出 - 思考。
    let priced = |input: i64, output: i64, reasoning: i64| {
        (quote.input_price().get() * input as f64
            + quote.output_price().get() * output as f64
            + quote.reasoning_price().get() * reasoning as f64)
            / TOKENS_PER_UNIT
    };
    let want = priced(
        observed.input,
        observed.output - observed.reasoning,
        observed.reasoning,
    );

    let got = quote.compute(usage).total_cost;
    assert!(approx_eq(got, want), "{got} != {want}");

    // 少收也是错，但这条测的是**多收**：按 `output` 全额再加 `reasoning` 收，
    // 就是把那 20 个思考 token 收了两遍。
    let double_counted = priced(observed.input, observed.output, observed.reasoning);
    assert!(
        got < double_counted,
        "思考 token 被收了两遍：{got} 应当低于 {double_counted}",
    );
}

/// 缓存输入是输入的子集，所以两列相加等于输入总量 —— 不是输入 + 缓存。
#[test]
fn cached_input_is_carved_out_of_the_input_rather_than_added_to_it() {
    let quote = quote(30.0, 1.0, 3.0, 0.0);
    let observed = ObservedUsage::new(100, 10, 40, 0).expect("a non-negative envelope");
    let usage = billable(UsageDialect::OpenAi, observed);

    assert_eq!(
        usage.uncached_input.get() + usage.cached_input.get(),
        observed.input,
        "两列之和必须正好是上游报的输入总量",
    );
    let got = quote.compute(usage);
    let all_uncached = quote.input_price().get() * observed.input as f64 / TOKENS_PER_UNIT;
    assert!(
        got.input_cost < all_uncached,
        "命中缓存的部分必须按更便宜的缓存价收：{} 应低于 {all_uncached}",
        got.input_cost,
    );
}

/// `reasoning` 这一列没有价时，思考按**输出价**收 —— 不是免费。
///
/// 这条对两族方言都成立，而且是同一个理由：这个部署没有为思考单独定价。
#[test]
fn thinking_is_never_free_when_the_reasoning_column_is_unpriced() {
    let quote = quote(0.0, 40.0, 0.0, 0.0);
    for dialect in [
        UsageDialect::OpenAi,
        UsageDialect::Anthropic,
        UsageDialect::Google,
    ] {
        // Google 的 `candidatesTokenCount` 不含思考，OpenAI / Anthropic 的含 ——
        // 所以两边的「上游原话」不一样，但两边的**总输出**都是 100。
        let observed = match dialect {
            UsageDialect::Google => ObservedUsage::new(0, 20, 0, 80),
            _ => ObservedUsage::new(0, 100, 0, 80),
        }
        .expect("a non-negative envelope");

        let quiet = quote.compute(billable(
            dialect,
            ObservedUsage::new(0, 20, 0, 0).expect("q"),
        ));
        let thinking = quote.compute(billable(dialect, observed));
        assert!(
            thinking.total_cost > quiet.total_cost,
            "{dialect:?}: 多了 80 个思考 token 却没多收钱：{} vs {}",
            thinking.total_cost,
            quiet.total_cost,
        );
    }
}

/// 一旦 `reasoning` 列被填上正数，思考只按那一列收一次，
/// **不再**折回可见输出 —— 否则填价的部署反而被重复计价。
#[test]
fn a_priced_reasoning_column_replaces_the_output_fold_instead_of_adding_to_it() {
    let usage = billable(
        UsageDialect::Google,
        ObservedUsage::new(0, 100, 0, 400).expect("a non-negative envelope"),
    );
    let folded = quote(0.0, 10.0, 0.0, 0.0).compute(usage).total_cost;
    let priced = quote(0.0, 10.0, 0.0, 10.0).compute(usage);

    assert!(
        approx_eq(priced.total_cost, folded),
        "同价时两条路必须给出同一个数：{} != {folded}",
        priced.total_cost,
    );
    assert!(
        priced.reasoning_cost > 0.0
            && approx_eq(priced.output_cost, folded - priced.reasoning_cost),
        "填了价之后思考应当只在 reasoning 那一项里出现一次",
    );
}

/// 分组倍率是最后一次线性缩放。
#[test]
fn the_rate_multiplier_scales_the_whole_bill_linearly() {
    let usage = billable(
        UsageDialect::OpenAi,
        ObservedUsage::new(1_000_000, 500_000, 0, 0).expect("envelope"),
    );
    let single = quote(2.5, 10.0, 0.0, 0.0).compute(usage).total_cost;
    let doubled = PricingQuote::new(
        "m".to_owned(),
        UnitPrice::new(2.5).expect("p"),
        UnitPrice::new(10.0).expect("p"),
        UnitPrice::ZERO,
        UnitPrice::ZERO,
        RateMultiplier::new(2.0).expect("rate"),
        0,
    )
    .compute(usage)
    .total_cost;

    assert!(single > 0.0, "基线必须为正，得到 {single}");
    assert!(
        approx_eq(doubled, single * 2.0),
        "{doubled} != {single} * 2"
    );
}

/// 一次什么也没消耗的请求恰好收零，无论价目与倍率。
#[test]
fn zero_tokens_cost_exactly_zero() {
    let got = quote(2.5, 10.0, 0.1, 5.0).compute(BillableUsage::default());
    assert_eq!(got.total_cost, 0.0);
    assert_eq!(got.input_cost, 0.0);
    assert_eq!(got.output_cost, 0.0);
    assert_eq!(got.cached_cost, 0.0);
    assert_eq!(got.reasoning_cost, 0.0);
}

/// 逐项必须把整张账单说完：不许有钱藏在四个分项之外。
#[test]
fn the_itemised_columns_account_for_the_total() {
    let mut rng = crate::testsupport::Rng::new(0x5EED_0005);
    for i in 0..300 {
        let quote = quote(
            rng.f64_range(0.0, 100.0),
            rng.f64_range(0.0, 100.0),
            rng.f64_range(0.0, 100.0),
            rng.f64_range(0.1, 100.0),
        );
        let usage = BillableUsage {
            uncached_input: TokenCount::new(rng.i64_range(0, 2_000_000)).expect("count"),
            cached_input: TokenCount::new(rng.i64_range(0, 2_000_000)).expect("count"),
            visible_output: TokenCount::new(rng.i64_range(0, 2_000_000)).expect("count"),
            reasoning_output: TokenCount::new(rng.i64_range(0, 2_000_000)).expect("count"),
        };

        let got = quote.compute(usage);
        let summed = got.input_cost + got.output_cost + got.cached_cost + got.reasoning_cost;
        assert!(
            (summed - got.total_cost).abs() <= 1e-9 * got.total_cost.abs().max(1.0),
            "iteration {i}: 分项之和 {summed}，总额 {}",
            got.total_cost,
        );
    }
}

// ---------------------------------------------------------------- estimate

#[test]
fn the_streaming_estimate_exceeds_the_unary_one() {
    let quote = quote(2.5, 10.0, 0.0, 0.0);
    assert!(quote.estimate(true) > quote.estimate(false));
}

#[test]
fn a_non_positive_output_cap_falls_back_to_the_plain_estimate_exactly() {
    let mut rng = crate::testsupport::Rng::new(0x5EED_0002);
    for i in 0..200 {
        let quote = quote(
            rng.f64_range(0.0, 100.0),
            rng.f64_range(0.0, 100.0),
            0.0,
            0.0,
        );
        let stream = rng.bool();
        let cap = rng.i64_range(-1_000_000, 0);
        assert_eq!(
            quote.estimate_with_max_tokens(cap, stream),
            quote.estimate(stream),
            "iteration {i}: cap={cap} 必须逐字委托给 estimate",
        );
    }
}

/// 更大的客户端上限只能抬高上界。
#[test]
fn the_upper_bound_is_monotonic_in_the_client_cap() {
    let mut rng = crate::testsupport::Rng::new(0x5EED_0003);
    for i in 0..200 {
        let quote = quote(
            rng.f64_range(0.0, 100.0),
            rng.f64_range(0.0, 100.0),
            0.0,
            0.0,
        );
        let lo = rng.i64_range(1, 1_000_000);
        let hi = lo + rng.i64_range(0, 1_000_000);
        assert!(
            quote.estimate_with_max_tokens(hi, false) >= quote.estimate_with_max_tokens(lo, false),
            "iteration {i}: 上限 {hi} 的估算低于 {lo} 的",
        );
    }
}

/// 预扣随真实 prompt 增长，这是大请求不再预扣不足的前提。
#[test]
fn the_reservation_scales_with_the_real_input_size_above_the_nominal_floor() {
    let quote = quote(10.0, 30.0, 0.0, 0.0);
    let small = quote.estimate_with_tokens(1_000, 0, false);
    let large = quote.estimate_with_tokens(200_000, 0, false);
    assert!(large > small, "large={large} 必须大于 small={small}");

    // 小于名义下限的 prompt 不许低于下限。
    assert!(approx_eq(
        quote.estimate_with_tokens(10, 0, false),
        quote.estimate_with_tokens(ESTIMATED_TOKENS, 0, false),
    ));
}

/// 上限、流式标志、倍率三者都要生效，且互相独立。
#[test]
fn the_cap_the_stream_flag_and_the_multiplier_all_apply() {
    let quote = quote(10.0, 30.0, 0.0, 0.0);
    let base = quote.estimate_with_tokens(50_000, 8_000, false);
    assert!(approx_eq(
        quote.estimate_with_tokens(50_000, 8_000, true),
        base * STREAM_MULTIPLIER,
    ));

    let doubled = PricingQuote::new(
        "m".to_owned(),
        UnitPrice::new(10.0).expect("p"),
        UnitPrice::new(30.0).expect("p"),
        UnitPrice::ZERO,
        UnitPrice::ZERO,
        RateMultiplier::new(2.0).expect("rate"),
        0,
    );
    assert!(approx_eq(
        doubled.estimate_with_tokens(50_000, 8_000, false),
        base * 2.0,
    ));
}

// ---------------------------------------------------------------- flat

/// 兜底形状：四列同价，非法价与非法倍率各自塌到安全的那一边。
#[test]
fn a_flat_quote_prices_every_column_alike_and_refuses_hostile_inputs() {
    let flat = PricingQuote::flat("m", 7.0, 1.0, 3);
    assert_eq!(flat.input_price(), flat.output_price());
    assert_eq!(flat.cached_price(), flat.reasoning_price());
    assert_eq!(flat.version(), 3);

    // 负价不许变成一笔退款。
    assert!(
        PricingQuote::flat("m", -1.0, 1.0, 0)
            .input_price()
            .is_zero()
    );
    assert!(
        PricingQuote::flat("m", f64::NAN, 1.0, 0)
            .input_price()
            .is_zero()
    );
    // 非法倍率退回恒等（按未打折的价收），零倍率保留。
    assert_eq!(
        PricingQuote::flat("m", 1.0, f64::NAN, 0).multiplier(),
        RateMultiplier::ONE,
    );
    assert!(PricingQuote::flat("m", 1.0, 0.0, 0).multiplier().is_zero());
}
