//! Unit tests for the dashboard's time window.
//!
//! The counts themselves are plain `COUNT(*)`/`SUM()` and are covered by the
//! integration suite. What is worth pinning without a database is the pair of
//! boundaries, because getting "this week" wrong shifts every number on the
//! console's landing page by a day.

use super::*;
use chrono::Duration;

#[test]
fn today_starts_at_local_midnight() {
    let now = Local::now();
    let (today, _) = day_bounds(now);
    assert_eq!(
        today.with_timezone(&Local).date_naive(),
        now.date_naive(),
        "today's boundary must be on today's local date"
    );
    let local = today.with_timezone(&Local);
    // A DST spring-forward can delete midnight, in which case the boundary is
    // the first instant that does exist on that date.
    assert!(local.time() < chrono::NaiveTime::from_hms_opt(4, 0, 0).expect("valid"));
}

#[test]
fn today_is_not_in_the_future() {
    let now = Local::now();
    let (today, week) = day_bounds(now);
    assert!(today <= now.with_timezone(&Utc));
    assert!(week <= today);
}

#[test]
fn the_week_window_covers_seven_calendar_days_including_today() {
    // 旧实现：`todayStart.AddDate(0, 0, -6)`。往前六天加上今天共七天 ——
    // an off-by-one here silently changes what "week_requests" means.
    let now = Local::now();
    let (today, week) = day_bounds(now);
    let days = today.with_timezone(&Local).date_naive() - week.with_timezone(&Local).date_naive();
    assert_eq!(days, Duration::days(WEEK_DAYS_BACK));
}

#[test]
fn the_week_boundary_is_also_a_local_midnight() {
    let (_, week) = day_bounds(Local::now());
    let local = week.with_timezone(&Local);
    assert!(local.time() < chrono::NaiveTime::from_hms_opt(4, 0, 0).expect("valid"));
}

#[test]
fn the_boundaries_move_with_the_clock() {
    // Two calls a day apart must produce boundaries a day apart; a cached or
    // process-start-pinned "today" would make the dashboard go stale.
    let now = Local::now();
    let (today, _) = day_bounds(now);
    let (tomorrow, _) = day_bounds(now + Duration::days(1));
    assert!(tomorrow > today);
}
