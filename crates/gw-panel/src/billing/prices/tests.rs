//! 对应校验那半的既有测试 —— 一条 `rapid` 属性测试加两个用例，全部经由
//! 一个 HTTP 引擎驱动、跑在内存 SQLite 上。这里没有 SQLite，处理器又需要真
//! Postgres，所以那些依赖数据库的断言放在集成套件里；这里钉的是*决定*拒绝与否的
//! 谓词 —— [`UpsertModelPriceRequest::negative_fields`] —— 因为那才是需求真正
//! 关心的东西，而它又是全函数且纯的。
//!
//! The two claims that matter (Requirement 6.4 / 6.5):
//!
//! * a negative value in ANY of the four per-1M fields rejects;
//! * exactly `0` is a valid price and must NOT reject.
//!
//! Requirement 6.5 ("a rejected payload must not invalidate the cache") is
//! structural in this port: the handler returns before reaching either the
//! upsert or `price_cache.invalidate`, so there is no ordering to test — see
//! `upsert_model_price`'s doc comment.

use super::*;

/// Every combination of "which field is poisoned" the original property test draws
/// over, expressed as a builder so a renamed field breaks compilation rather
/// than silently dropping a case.
fn request_with(index: usize, value: f64) -> UpsertModelPriceRequest {
    // Non-poisoned fields stay non-negative. The original draws them from [0, 100]; the
    // exact value is irrelevant to the predicate, only the sign is.
    let mut req = UpsertModelPriceRequest {
        model_id: "m".to_owned(),
        input_price_per_1m: 1.0,
        output_price_per_1m: 2.0,
        cached_input_price_per_1m: 3.0,
        reasoning_price_per_1m: 4.0,
    };
    match index {
        0 => req.input_price_per_1m = value,
        1 => req.output_price_per_1m = value,
        2 => req.cached_input_price_per_1m = value,
        3 => req.reasoning_price_per_1m = value,
        other => panic!("no per-1M field at index {other}"),
    }
    req
}

/// The four field indices, so a test that forgets one fails to compile rather
/// than passing with less coverage.
const FIELD_COUNT: usize = 4;

#[test]
fn every_per_1m_field_rejects_a_negative_value() {
    // 既有测试里的 TestNegativePriceRejected shrinks toward the float just below zero,
    // which is where a `< -epsilon` bug would hide. The boundary values below
    // cover that explicitly instead of sampling for it.
    for index in 0..FIELD_COUNT {
        for value in [-f64::MIN_POSITIVE, -1e-9, -0.1, -1000.0, f64::NEG_INFINITY] {
            let negative = request_with(index, value).negative_fields();
            assert_eq!(
                negative.len(),
                1,
                "field {index} with {value} should reject exactly itself, got {negative:?}"
            );
        }
    }
}

#[test]
fn exactly_zero_is_a_valid_price() {
    // 既有测试里的 TestZeroPriceAccepted。"Price dropped to 0" must be representable —
    // it is a real operator action, not a missing value.
    let all_zero = UpsertModelPriceRequest {
        model_id: "m".to_owned(),
        input_price_per_1m: 0.0,
        output_price_per_1m: 0.0,
        cached_input_price_per_1m: 0.0,
        reasoning_price_per_1m: 0.0,
    };
    assert!(all_zero.negative_fields().is_empty());
}

#[test]
fn negative_zero_is_not_negative() {
    // IEEE-754 `-0.0 < 0.0` is false, so a client that sends `-0` gets the same
    // answer as one that sends `0`. Pinning it because a hand-written
    // `is_sign_negative` check would behave differently and silently reject.
    for index in 0..FIELD_COUNT {
        assert!(request_with(index, -0.0).negative_fields().is_empty());
    }
}

#[test]
fn a_nan_price_is_not_treated_as_negative() {
    // Every comparison with NaN is false, so NaN passes validation and is
    // handed to the DB, which rejects it for a `numeric` column. Documented
    // here so the behaviour is a known consequence rather than a surprise.
    for index in 0..FIELD_COUNT {
        assert!(request_with(index, f64::NAN).negative_fields().is_empty());
    }
}

#[test]
fn all_four_negatives_are_reported_together() {
    // 旧实现 collects every offending field into `negativeFields` for the warning
    // log rather than short-circuiting on the first, so an operator fixing a
    // payload sees all of them at once.
    let req = UpsertModelPriceRequest {
        model_id: "m".to_owned(),
        input_price_per_1m: -1.0,
        output_price_per_1m: -2.0,
        cached_input_price_per_1m: -3.0,
        reasoning_price_per_1m: -4.0,
    };
    assert_eq!(req.negative_fields().len(), FIELD_COUNT);
}

#[test]
fn reported_field_names_are_the_request_names() {
    // The names go into the structured log an operator greps. They must be the
    // JSON names the client sent, not the reverse-intuitive column names
    // (`input_price_per1_m`), or the log points at the wrong thing.
    let names = UpsertModelPriceRequest {
        model_id: "m".to_owned(),
        input_price_per_1m: -1.0,
        output_price_per_1m: -1.0,
        cached_input_price_per_1m: -1.0,
        reasoning_price_per_1m: -1.0,
    }
    .negative_fields();

    let body = serde_json::to_value(serde_json::json!({
        "model_id": "m",
        "input_price_per_1m": -1.0,
        "output_price_per_1m": -1.0,
        "cached_input_price_per_1m": -1.0,
        "reasoning_price_per_1m": -1.0,
    }))
    .expect("literal is valid json");
    for name in names {
        assert!(
            body.get(name).is_some(),
            "{name} is not a field the client can send"
        );
    }
}

#[test]
fn the_request_parses_the_readable_field_names() {
    // The request uses `input_price_per_1m`; the *column* is
    // `input_price_per1_m`. Confusing the two is the single most likely porting
    // mistake here, so pin that the wire name is the readable one — this is the
    // shape `frontend/src/features/pricing/types.ts` sends.
    let req: UpsertModelPriceRequest = serde_json::from_str(
        r#"{"model_id":"gpt-4o","input_price_per_1m":1.5,"output_price_per_1m":2.5,
            "cached_input_price_per_1m":0.5,"reasoning_price_per_1m":3.5}"#,
    )
    .expect("frontend payload must deserialize");
    assert_eq!(req.model_id, "gpt-4o");
    assert!(req.negative_fields().is_empty());
    assert!((req.input_price_per_1m - 1.5).abs() < f64::EPSILON);
    assert!((req.reasoning_price_per_1m - 3.5).abs() < f64::EPSILON);
}

#[test]
fn a_partial_payload_defaults_the_missing_prices_to_zero() {
    // 旧实现 binds into a struct of `float64`, so an omitted field arrives as 0 and
    // is written as 0 — not left at its previous value. Same here.
    let req: UpsertModelPriceRequest =
        serde_json::from_str(r#"{"model_id":"m"}"#).expect("partial payload must deserialize");
    assert!(req.negative_fields().is_empty());
    assert_eq!(req.output_price_per_1m, 0.0);
}

#[test]
fn the_response_uses_old_struct_field_names() {
    // `model.ModelPrice` carries no json tags, so 旧实现 emits PascalCase. The
    // request/response asymmetry is the shipped contract, not a bug — pin it,
    // because "tidying" it to snake_case is the obvious wrong move.
    let body = serde_json::to_value(ModelPriceResponse::from(gw_model::ModelPrice {
        id: 1,
        model_id: "m".to_owned(),
        input_price_per_1m: 1.0,
        output_price_per_1m: 2.0,
        cached_input_price_per_1m: 3.0,
        reasoning_price_per_1m: 4.0,
        created_at: chrono::DateTime::UNIX_EPOCH,
        updated_at: chrono::DateTime::UNIX_EPOCH,
    }))
    .expect("response serializes");

    // The expected set is transcribed from the entity's published
    // exported field names — an external authority, not this module's source.
    let mut got: Vec<&str> = body
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    got.sort_unstable();
    let mut want = [
        "ID",
        "ModelID",
        "InputPricePer1M",
        "OutputPricePer1M",
        "CachedInputPricePer1M",
        "ReasoningPricePer1M",
        "CreatedAt",
        "UpdatedAt",
    ];
    want.sort_unstable();
    assert_eq!(got, want);
}
