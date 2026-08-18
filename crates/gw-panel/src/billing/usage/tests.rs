//! Unit tests for the usage aggregates.
//!
//! The DB-backed halves (`list_usage_logs`, the two route handlers) need a live
//! Postgres and live in the integration suite. What is pinned here is the
//! folding logic the original implementation keeps in `buildUsageTrend` / `buildUsageModels` — the part
//! that decides which day a request lands on, which column wins when two are
//! populated, and how ties are ordered.

use super::*;
use chrono::TimeDelta;

fn day(offset: i64) -> DateTime<Utc> {
    // Anchored well inside a day so a timezone offset cannot move the sample
    // across a date boundary and make the test depend on where it runs.
    let noon = Local::now()
        .date_naive()
        .and_hms_opt(12, 0, 0)
        .expect("noon exists");
    let at = Local
        .from_local_datetime(&noon)
        .earliest()
        .expect("noon is unambiguous");
    (at + TimeDelta::days(offset)).with_timezone(&Utc)
}

// ---------------------------------------------------------------- trend

#[test]
fn the_trend_has_one_point_per_requested_day() {
    for days in [1, 7, 30] {
        let points = build_trend(&[], Local::now(), days);
        assert_eq!(i64::try_from(points.len()).unwrap_or(-1), days);
    }
}

#[test]
fn empty_days_are_present_and_zeroed() {
    // A chart with holes is worse than a chart of zeroes; 旧实现 fills every
    // bucket for exactly this reason.
    let points = build_trend(&[AggregateRow::at(day(0), "m", 5, 1.0)], Local::now(), 3);
    assert!(points.iter().all(|point| !point.date.is_empty()));
    assert_eq!(points.iter().filter(|point| point.requests == 0).count(), 2);
}

#[test]
fn the_trend_runs_oldest_first() {
    let points = build_trend(&[], Local::now(), 5);
    let dates: Vec<&String> = points.iter().map(|point| &point.date).collect();
    let mut sorted = dates.clone();
    sorted.sort();
    assert_eq!(dates, sorted);
}

#[test]
fn todays_row_lands_on_the_last_point() {
    let points = build_trend(&[AggregateRow::at(day(0), "m", 5, 1.5)], Local::now(), 7);
    let last = points.last().expect("non-empty window");
    assert_eq!(last.requests, 1);
    assert_eq!(last.tokens, 5);
    assert!((last.cost - 1.5).abs() < f64::EPSILON);
    assert_eq!(
        points[..points.len() - 1]
            .iter()
            .map(|p| p.requests)
            .sum::<i64>(),
        0
    );
}

#[test]
fn rows_older_than_the_window_are_dropped() {
    // The SQL lower bound should already exclude them; the fold must not
    // mis-bucket one that slips through rather than silently adding it to day 0.
    let points = build_trend(&[AggregateRow::at(day(-99), "m", 5, 1.0)], Local::now(), 7);
    assert!(points.iter().all(|point| point.requests == 0));
}

#[test]
fn same_day_rows_accumulate() {
    let rows = [
        AggregateRow::at(day(0), "a", 3, 0.25),
        AggregateRow::at(day(0), "b", 4, 0.75),
    ];
    let last = build_trend(&rows, Local::now(), 2)
        .pop()
        .expect("non-empty window");
    assert_eq!(last.requests, 2);
    assert_eq!(last.tokens, 7);
    assert!((last.cost - 1.0).abs() < 1e-12);
}

#[test]
fn a_one_day_window_starts_today() {
    // `days - 1` days back: with days = 1 the window must not reach yesterday.
    let now = Local::now();
    let start = window_start(now, 1);
    assert!(start <= now.with_timezone(&Utc));
    assert_eq!(local_day(start), now.date_naive().to_string());
}

#[test]
fn the_window_start_moves_back_one_day_per_extra_day() {
    let now = Local::now();
    let short = window_start(now, 1);
    let long = window_start(now, 8);
    // Seven whole days apart, give or take a DST hour.
    let gap = short - long;
    assert!(gap >= TimeDelta::days(7) - TimeDelta::hours(1));
    assert!(gap <= TimeDelta::days(7) + TimeDelta::hours(1));
}

// ---------------------------------------------------------------- models

#[test]
fn models_are_ordered_by_request_count_descending() {
    let rows = [
        AggregateRow::at(day(0), "rare", 1, 0.1),
        AggregateRow::at(day(0), "common", 1, 0.1),
        AggregateRow::at(day(0), "common", 1, 0.1),
    ];
    let points = build_models(&rows);
    assert_eq!(points.first().map(|p| p.model.as_str()), Some("common"));
}

#[test]
fn ties_break_on_the_model_name() {
    // Without a tiebreak the order comes out of a hash map and the dashboard
    // reshuffles on every refresh.
    let rows = [
        AggregateRow::at(day(0), "zeta", 1, 0.1),
        AggregateRow::at(day(0), "alpha", 1, 0.1),
        AggregateRow::at(day(0), "mid", 1, 0.1),
    ];
    let names: Vec<String> = build_models(&rows)
        .into_iter()
        .map(|point| point.model)
        .collect();
    assert_eq!(names, ["alpha", "mid", "zeta"]);
}

#[test]
fn a_blank_model_is_labelled_rather_than_dropped() {
    let rows = [
        AggregateRow::at(day(0), "", 1, 0.1),
        AggregateRow::at(day(0), "   ", 2, 0.2),
    ];
    let points = build_models(&rows);
    assert_eq!(points.len(), 1, "blank and whitespace must fold together");
    assert!(!points[0].model.is_empty());
    assert_eq!(points[0].requests, 2);
}

#[test]
fn the_model_list_is_truncated() {
    let rows: Vec<AggregateRow> = (0..MODEL_POINT_LIMIT + 5)
        .map(|index| AggregateRow::at(day(0), &format!("model-{index:03}"), 1, 0.1))
        .collect();
    assert_eq!(build_models(&rows).len(), MODEL_POINT_LIMIT);
}

#[test]
fn truncation_keeps_the_busiest_models() {
    // Dropping the tail is only acceptable if the tail is the least used.
    let mut rows: Vec<AggregateRow> = (0..MODEL_POINT_LIMIT + 5)
        .map(|index| AggregateRow::at(day(0), &format!("model-{index:03}"), 1, 0.1))
        .collect();
    rows.push(AggregateRow::at(day(0), "model-999", 1, 0.1));
    rows.push(AggregateRow::at(day(0), "model-999", 1, 0.1));

    let points = build_models(&rows);
    assert_eq!(points.first().map(|p| p.model.as_str()), Some("model-999"));
}

// ---------------------------------------------------------------- columns

#[test]
fn the_newer_token_columns_win_when_populated() {
    // Rows written before the four-column split only have tokens_in/out; rows
    // written after have both. Reading the wrong one silently halves the chart.
    assert_eq!(prefer_positive(Some(7), Some(3)), 7);
    assert_eq!(prefer_positive(Some(0), Some(3)), 3);
    assert_eq!(prefer_positive(None, Some(3)), 3);
    assert_eq!(prefer_positive(None, None), 0);
}

#[test]
fn a_zero_primary_cost_falls_back_to_the_legacy_column() {
    assert!((prefer_positive_f64(Some(1.5), Some(9.0)) - 1.5).abs() < f64::EPSILON);
    assert!((prefer_positive_f64(Some(0.0), Some(9.0)) - 9.0).abs() < f64::EPSILON);
    assert!((prefer_positive_f64(None, None)).abs() < f64::EPSILON);
}

#[test]
fn a_missing_rate_multiplier_reads_as_the_baseline() {
    // Zero would make every historical cost display as free.
    let mut row = usage_log_row();
    row.rate_multiplier = None;
    assert!((row.rate_multiplier() - 1.0).abs() < f64::EPSILON);
    row.rate_multiplier = Some(0.0);
    assert!((row.rate_multiplier() - 1.0).abs() < f64::EPSILON);
    row.rate_multiplier = Some(0.5);
    assert!((row.rate_multiplier() - 0.5).abs() < f64::EPSILON);
}

fn usage_log_row() -> UsageLogRow {
    UsageLogRow {
        id: 1,
        request_id: None,
        user_id: 1,
        api_key_id: 1,
        model: None,
        provider: None,
        tokens_in: None,
        tokens_out: None,
        input_tokens: None,
        output_tokens: None,
        reasoning_tokens: None,
        cached_tokens: None,
        input_cost: None,
        output_cost: None,
        total_cost: None,
        actual_cost: None,
        cost: None,
        rate_multiplier: None,
        stream: None,
        duration_ms: None,
        failed: None,
        created_at: DateTime::UNIX_EPOCH,
    }
}

// ---------------------------------------------------------------- filters

#[test]
fn only_the_two_named_statuses_filter() {
    assert_eq!(status_filter(Some("success")), Some(false));
    assert_eq!(status_filter(Some("failed")), Some(true));
    for ignored in [None, Some(""), Some("all"), Some("SUCCESS"), Some("error")] {
        assert_eq!(
            status_filter(ignored),
            None,
            "{ignored:?} must leave the query unfiltered"
        );
    }
}

#[test]
fn a_blank_parameter_is_the_same_as_an_absent_one() {
    // `?model=` must not turn into `WHERE model = ''`, which matches nothing.
    assert_eq!(non_empty(Some(&String::new())), None);
    assert_eq!(non_empty(Some(&"   ".to_owned())), None);
    assert_eq!(non_empty(Some(&" gpt-4o ".to_owned())), Some("gpt-4o"));
    assert_eq!(non_empty(None), None);
}

/// sqlx types a `None::<&str>` date bind as `text`. Comparing that to
/// `created_at::date` without a cast is `date >= text` and 500s the empty
/// admin usage page. This query is the production filter with no values set.
#[tokio::test]
#[ignore = "needs a local Postgres: set GW_TEST_DATABASE_URL"]
async fn an_unfiltered_usage_count_type_checks_when_date_binds_are_null() {
    let url = std::env::var("GW_TEST_DATABASE_URL").expect(
        "连库集成测试需要 GW_TEST_DATABASE_URL，例如：\n  \
         GW_TEST_DATABASE_URL=postgres://ai_gateway:agw_dev_password@127.0.0.1:5432/ai_gateway",
    );
    let pool = sqlx::PgPool::connect(&url)
        .await
        .expect("连不上 GW_TEST_DATABASE_URL 指向的库");
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM usage_logs WHERE {USAGE_LOG_FILTER}"
    ))
    .bind(None::<&str>)
    .bind(None::<bool>)
    .bind(None::<&str>)
    .bind(None::<&str>)
    .fetch_one(&pool)
    .await
    .expect("NULL date binds must compare as date, not text");
    assert!(total >= 0);
}
