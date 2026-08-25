//! 归一化：三家上游的信封语义，以及无效信封被拒绝而不是被截断。

use super::*;

/// 归一化后的四列。
fn folded(
    dialect: UsageDialect,
    input: i64,
    output: i64,
    cached: i64,
    reasoning: i64,
) -> BillableUsage {
    normalize(
        dialect,
        ObservedUsage {
            input,
            output,
            cached,
            reasoning,
        },
    )
    .expect("a consistent envelope")
}

/// 归一化的定义性质：四列互不重叠，两两之和还原上游报的输入 / 输出总量。
///
/// 「输出总量」按方言不同：思考 ⊂ 输出的方言里是 `output`，
/// Google 里是 `output + reasoning`。
#[test]
fn the_four_columns_partition_the_upstream_totals() {
    let mut rng = crate::testsupport::Rng::new(0x11EE_0001);
    for i in 0..500 {
        let input = rng.i64_range(0, 1_000_000);
        let cached = rng.i64_range(0, input);
        let output = rng.i64_range(0, 1_000_000);
        for dialect in [
            UsageDialect::OpenAi,
            UsageDialect::Anthropic,
            UsageDialect::Google,
        ] {
            let reasoning = match dialect {
                UsageDialect::Google => rng.i64_range(0, 1_000_000),
                _ => rng.i64_range(0, output),
            };
            let got = folded(dialect, input, output, cached, reasoning);

            assert_eq!(
                got.uncached_input.get() + got.cached_input.get(),
                input,
                "iteration {i} {dialect:?}: 输入两列必须正好拼回上游的输入总量",
            );
            let total_output = match dialect {
                UsageDialect::Google => output + reasoning,
                _ => output,
            };
            assert_eq!(
                got.visible_output.get() + got.reasoning_output.get(),
                total_output,
                "iteration {i} {dialect:?}: 输出两列必须正好拼回上游的输出总量",
            );
            assert_eq!(got.reasoning_output.get(), reasoning);
        }
    }
}

/// 思考 ⊂ 输出的两族：可见输出是差，不是原值。这是不重复计价的全部内容。
#[test]
fn reasoning_is_carved_out_of_the_output_for_openai_and_anthropic() {
    for dialect in [UsageDialect::OpenAi, UsageDialect::Anthropic] {
        let got = folded(dialect, 100, 50, 0, 20);
        assert_eq!(got.visible_output.get(), 30, "{dialect:?}");
        assert_eq!(got.reasoning_output.get(), 20, "{dialect:?}");
    }
}

/// Google 的 `thoughtsTokenCount` 与 `candidatesTokenCount` **并列**，
/// 减出去就是凭空少收一整块思考。
#[test]
fn google_keeps_its_visible_output_intact() {
    let got = folded(UsageDialect::Google, 100, 50, 0, 20);
    assert_eq!(got.visible_output.get(), 50);
    assert_eq!(got.reasoning_output.get(), 20);
}

/// 缓存是输入的子集，三家一致。
#[test]
fn cached_input_is_a_subset_of_the_input_in_every_dialect() {
    for dialect in [
        UsageDialect::OpenAi,
        UsageDialect::Anthropic,
        UsageDialect::Google,
    ] {
        let got = folded(dialect, 100, 10, 40, 0);
        assert_eq!(got.uncached_input.get(), 60, "{dialect:?}");
        assert_eq!(got.cached_input.get(), 40, "{dialect:?}");
    }
}

// ---------------------------------------------------------------- 无效信封

/// 负数不 clamp 也不接受：它既不是零消耗，更不是一笔退款。
#[test]
fn any_negative_column_is_refused_rather_than_clamped() {
    assert!(ObservedUsage::new(-1, 0, 0, 0).is_err());
    assert!(ObservedUsage::new(0, -1, 0, 0).is_err());
    assert!(ObservedUsage::new(0, 0, -1, 0).is_err());
    assert!(ObservedUsage::new(0, 0, 0, -1).is_err());
    assert!(ObservedUsage::try_from([0, 0, 0, -1]).is_err());
    assert_eq!(
        ObservedUsage::new(1, 2, 0, 0).expect("valid"),
        ObservedUsage {
            input: 1,
            output: 2,
            cached: 0,
            reasoning: 0
        },
    );

    for dialect in [
        UsageDialect::OpenAi,
        UsageDialect::Anthropic,
        UsageDialect::Google,
    ] {
        let err = normalize(
            dialect,
            ObservedUsage {
                input: 10,
                output: -5,
                cached: 0,
                reasoning: 0,
            },
        )
        .expect_err("a negative column is not billable");
        assert!(
            matches!(err, ValueError::Negative { .. }),
            "{dialect:?}: {err:?}",
        );
    }
}

/// 子集列大过总量 = 信封自相矛盾。拒绝，而不是「两列都收」或「截断到总量」。
#[test]
fn a_subset_column_larger_than_its_total_is_refused() {
    for dialect in [
        UsageDialect::OpenAi,
        UsageDialect::Anthropic,
        UsageDialect::Google,
    ] {
        assert!(
            matches!(
                normalize(
                    dialect,
                    ObservedUsage {
                        input: 10,
                        output: 0,
                        cached: 11,
                        reasoning: 0
                    }
                ),
                Err(ValueError::Inconsistent { .. }),
            ),
            "{dialect:?}: 缓存大过输入必须被拒绝",
        );
    }

    // 思考大过输出只在「思考 ⊂ 输出」的两族里矛盾；Google 那里它完全合法。
    for dialect in [UsageDialect::OpenAi, UsageDialect::Anthropic] {
        assert!(
            matches!(
                normalize(
                    dialect,
                    ObservedUsage {
                        input: 0,
                        output: 10,
                        cached: 0,
                        reasoning: 11
                    }
                ),
                Err(ValueError::Inconsistent { .. }),
            ),
            "{dialect:?}: 思考大过输出必须被拒绝",
        );
    }
    assert_eq!(
        folded(UsageDialect::Google, 0, 10, 0, 11)
            .reasoning_output
            .get(),
        11,
        "Google 的思考与输出并列，谁大谁小都不矛盾",
    );
}

/// 边界：思考恰好等于输出，可见输出为零而不是负数。
#[test]
fn reasoning_equal_to_the_output_leaves_nothing_visible() {
    let got = folded(UsageDialect::OpenAi, 0, 40, 0, 40);
    assert_eq!(got.visible_output.get(), 0);
    assert_eq!(got.reasoning_output.get(), 40);
}
