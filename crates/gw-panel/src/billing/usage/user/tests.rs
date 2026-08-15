//! 用户侧用量视图的单元测试。
//!
//! 需要数据库的那几半（三个 handler 的 SQL）在集成套件里跑；这里钉的是**不查库
//! 也成立的性质**：筛选参数怎么解析、时间窗怎么划、以及 `/user/usage` 那个
//! 「实体原样」的信封长什么样 —— 后者是与旧实现 `encoding/json` 的对账，不是把实现
//! 抄进断言（规范 2.11）。

use super::*;
use chrono::{TimeZone as _, Timelike as _};

/// 把一组 query 参数攒成 [`UsageQuery`]，省得每个用例写 7 个 `None`。
fn query(pairs: &[(&str, &str)]) -> UsageQuery {
    let mut q = UsageQuery::default();
    for (key, value) in pairs {
        let value = Some((*value).to_owned());
        match *key {
            "page" => q.page = value,
            "page_size" => q.page_size = value,
            "api_key_id" => q.api_key_id = value,
            "model" => q.model = value,
            "start_date" => q.start_date = value,
            "end_date" => q.end_date = value,
            "status" => q.status = value,
            other => panic!("unknown query parameter {other}"),
        }
    }
    q
}

// ------------------------------------------------------------ status 过滤

#[test]
fn success_and_failed_are_opposite_predicates() {
    let success = user_status_filter(Some("success")).expect("recognised");
    let failed = user_status_filter(Some("failed")).expect("recognised");
    assert_ne!(success, failed);
    assert_eq!(success, failed.map(|value| !value));
}

#[test]
fn an_absent_or_all_status_filters_nothing() {
    for raw in [None, Some(""), Some("  "), Some("all"), Some(" all ")] {
        assert_eq!(
            user_status_filter(raw),
            Ok(None),
            "{raw:?} should leave the query unfiltered"
        );
    }
}

#[test]
fn an_unrecognised_status_is_rejected_rather_than_ignored() {
    // 与管理员那条的差别就在这里：那条静默忽略，这条 400。混用会让用户以为
    // `?status=Success` 生效了，其实看到的是全量。
    for raw in ["Success", "ok", "true", "0"] {
        assert!(
            user_status_filter(Some(raw)).is_err(),
            "{raw} should be rejected"
        );
    }
    assert!(super::super::status_filter(Some("Success")).is_none());
}

// ------------------------------------------------------------ 日期区间

#[test]
fn a_single_day_range_is_half_open_and_covers_that_whole_day() {
    let filters = parse_detail_filters(&query(&[
        ("start_date", "2026-03-08"),
        ("end_date", "2026-03-08"),
    ]))
    .expect("valid dates");

    let start = filters.start.expect("start bound");
    let end = filters.end_exclusive.expect("end bound");
    // 上界必须严格大于下界，否则「只筛一天」会筛出 0 行。
    assert!(end > start, "end bound must be exclusive and after start");
    // 而且恰好是一整天 —— 用当地日历长度算，不写死 86400（DST 那天不是 24 小时）。
    let expected_end = local_midnight(
        NaiveDate::from_ymd_opt(2026, 3, 8)
            .expect("valid date")
            .succ_opt()
            .expect("next day"),
    );
    assert_eq!(end, expected_end);
}

#[test]
fn date_bounds_land_on_local_midnight() {
    let filters = parse_detail_filters(&query(&[("start_date", "2026-01-15")])).expect("valid");
    let start = filters.start.expect("start bound");
    // 边界是「当地零点」这件事，只能在当地时区里看出来。
    let local = start.with_timezone(&Local);
    assert_eq!(local.date_naive().to_string(), "2026-01-15");
    assert_eq!((local.time().hour(), local.time().minute()), (0, 0));
}

#[test]
fn a_malformed_date_names_which_end_was_wrong() {
    // 两个端点的文案不同，前端直接弹给用户；混用会让人找错输入框。
    let start_err = parse_detail_filters(&query(&[("start_date", "2026-13-40")])).unwrap_err();
    let end_err = parse_detail_filters(&query(&[("end_date", "not-a-date")])).unwrap_err();
    assert_ne!(start_err, end_err);
    assert!(start_err.contains("开始"));
    assert!(end_err.contains("结束"));
}

#[test]
fn a_timestamp_is_not_accepted_where_a_date_is_expected() {
    // 旧实现的 layout 是 "2006-01-02"，多出来的时间部分让 ParseInLocation 失败。
    assert!(parse_detail_filters(&query(&[("start_date", "2026-01-15T00:00:00Z")])).is_err());
}

// ------------------------------------------------------------ api_key_id

#[test]
fn a_non_positive_api_key_id_is_rejected() {
    // 自增主键从 1 起，`0` 只可能来自伪造的 URL；负数同理。
    for raw in ["0", "-1", "abc", "1.5", ""] {
        let parsed = parse_detail_filters(&query(&[("api_key_id", raw)]));
        if raw.is_empty() {
            // 空串等同于没传，不是错误。
            assert_eq!(parsed.expect("empty is absent").api_key_id, None);
        } else {
            assert!(parsed.is_err(), "{raw} should be rejected");
        }
    }
}

#[test]
fn a_well_formed_api_key_id_survives_parsing() {
    let filters = parse_detail_filters(&query(&[("api_key_id", " 42 ")])).expect("valid");
    assert_eq!(filters.api_key_id, Some(42));
}

// ------------------------------------------------------------ model 过滤

#[test]
fn the_model_filter_is_a_substring_match_that_does_not_escape_wildcards() {
    // 旧实现直接 `"%"+v+"%"`。转义 `_` 会让「搜 gpt_4」的结果与旧实现不同 —— 这里钉的
    // 是"输入原样出现在 pattern 里"，而不是 pattern 的具体拼法。
    let raw = "gpt_4%o";
    let filters = parse_detail_filters(&query(&[("model", raw)])).expect("valid");
    let pattern = filters.model_like.expect("pattern");
    assert!(pattern.contains(raw), "{pattern} must carry {raw} verbatim");
    assert!(pattern.starts_with('%') && pattern.ends_with('%'));
}

#[test]
fn blank_parameters_behave_like_absent_ones() {
    let filters = parse_detail_filters(&query(&[
        ("model", "   "),
        ("start_date", ""),
        ("end_date", "  "),
    ]))
    .expect("blank is absent");
    assert_eq!(filters, DetailFilters::default());
}

// ------------------------------------------------------------ 统计时间窗

#[test]
fn the_three_windows_are_strictly_ordered_oldest_last() {
    let (today, week, month) = stats_windows(Local::now());
    assert!(
        month < week,
        "the 30-day window must start before the 7-day one"
    );
    assert!(week < today, "the 7-day window must start before today");
}

#[test]
fn the_windows_are_counted_in_local_days_not_in_seconds() {
    // 跨 DST 的那一周不是 7×86400 秒。用日历天数比，不用秒数比。
    let now = Local::now();
    let (today, week, month) = stats_windows(now);
    let day_of = |at: DateTime<Utc>| at.with_timezone(&Local).date_naive();
    assert_eq!((day_of(today) - day_of(week)).num_days(), 6);
    assert_eq!((day_of(today) - day_of(month)).num_days(), 29);
    assert_eq!(day_of(today), now.date_naive());
}

// ------------------------------------------------------------ 实体信封

fn sample_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 4, 3, 2, 1)
        .single()
        .expect("valid instant")
}

#[test]
fn the_entity_envelope_uses_old_field_names_verbatim() {
    // `model.UsageLog` 没有 json tag，所以旧实现发的是字段名原样。这一条是与旧实现的
    // 对账：任何"顺手改成 snake_case"都会在这里红。
    let json = entity_json(&EntityRow::blank(sample_time()));
    let object = json.as_object().expect("an object");
    for key in [
        "ID",
        "UserID",
        "ApiKeyID",
        "GroupID",
        "RequestID",
        "IdempotencyKey",
        "EventKey",
        "Model",
        "Provider",
        "AuthID",
        "TokensIn",
        "TokensOut",
        "InputTokens",
        "OutputTokens",
        "ReasoningTokens",
        "CachedTokens",
        "InputCost",
        "OutputCost",
        "TotalCost",
        "ActualCost",
        "Cost",
        "RateMultiplier",
        "Stream",
        "DurationMs",
        "IPAddress",
        "RawMetadata",
        "Failed",
        "CreatedAt",
    ] {
        assert!(object.contains_key(key), "missing {key}");
    }
    // 字段数也要对得上：多发一列就是泄露，少发一列就是前端拿不到。
    assert_eq!(object.len(), 28);
}

#[test]
fn a_nil_pointer_column_is_null_and_a_nil_byte_slice_is_null_too() {
    let json = entity_json(&EntityRow::blank(sample_time()));
    assert!(json["GroupID"].is_null());
    assert!(json["RawMetadata"].is_null());
    // 而字符串列在旧实现那边是零值 ""，不是 null。
    assert_eq!(json["Provider"], "");
    assert_eq!(json["TokensIn"], 0);
}

#[test]
fn raw_metadata_is_base64_of_the_json_text_not_a_nested_object() {
    // 旧实现的字段是 []byte，`encoding/json` 把它编成 base64 字符串。解成对象再发出去
    // 是另一种形状，前端的解码会直接炸。
    let mut row = EntityRow::blank(sample_time());
    let text = r#"{"shortfall_usd": 0.5}"#;
    row.raw_metadata = Some(text.to_owned());

    let json = entity_json(&row);
    let encoded = json["RawMetadata"]
        .as_str()
        .expect("a string, not an object");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("valid base64");
    assert_eq!(String::from_utf8(decoded).expect("utf-8"), text);
}
