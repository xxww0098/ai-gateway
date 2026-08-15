use chrono::{Duration, TimeZone, Timelike, Weekday};

use super::*;

fn at(y: i32, m: u32, d: u32, hh: u32, mm: u32, ss: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, hh, mm, ss)
        .single()
        .expect("测试用时刻必须唯一")
}

/// 一天里的任意时刻（含零点本身）都必须映射到严格更晚的一个 UTC 零点。
/// 这是配额能归零的前提："strictly after" 就是这条性质。
#[test]
fn daily_reset_is_strictly_after_and_at_midnight() {
    let samples = [
        at(2026, 1, 1, 0, 0, 0),
        at(2026, 2, 28, 23, 59, 59),
        at(2026, 6, 15, 12, 0, 0),
        at(2028, 2, 29, 8, 30, 0), // 闰日
    ];
    for t in samples {
        let next = next_daily_reset_after(t);
        assert!(next > t, "{t} 的下一个日重置点必须严格更晚");
        assert_eq!((next.hour(), next.minute(), next.second()), (0, 0, 0));
        assert!(next - t <= Duration::days(1));
    }
}

/// 周重置必须落在 UTC 周一 00:00，且严格更晚、最多一周之后。
#[test]
fn weekly_reset_lands_on_monday_midnight() {
    let mut t = at(2026, 3, 1, 0, 0, 0); // 2026-03-01 是周日
    for _ in 0..14 {
        let next = next_weekly_reset_after(t);
        assert_eq!(next.weekday(), Weekday::Mon, "{t} → {next} 必须是周一");
        assert_eq!((next.hour(), next.minute(), next.second()), (0, 0, 0));
        assert!(next > t);
        assert!(next - t <= Duration::days(7));
        t += Duration::hours(13);
    }
}

/// 周一零点本身是最容易错的边界：必须跳到下一个周一，而不是原地不动。
#[test]
fn weekly_reset_on_monday_midnight_advances_a_full_week() {
    let monday = at(2026, 3, 2, 0, 0, 0);
    assert_eq!(monday.weekday(), Weekday::Mon);
    assert_eq!(next_weekly_reset_after(monday) - monday, Duration::days(7));
}

/// 月重置必须落在下个月 1 号 00:00，跨年、跨闰月都不例外。
#[test]
fn monthly_reset_lands_on_first_of_next_month() {
    let samples = [
        at(2026, 1, 31, 23, 0, 0),
        at(2026, 2, 1, 0, 0, 0),
        at(2026, 12, 31, 23, 59, 59),
        at(2028, 1, 30, 6, 0, 0), // 下个月是闰年 2 月
    ];
    for t in samples {
        let next = next_monthly_reset_after(t);
        assert!(next > t, "{t} 的下一个月重置点必须严格更晚");
        assert_eq!(next.day(), 1);
        assert_eq!((next.hour(), next.minute(), next.second()), (0, 0, 0));
        // 跨过的月份恰好是一个
        assert_eq!(
            (next.year() - t.year()) * 12 + next.month() as i32 - t.month() as i32,
            1
        );
    }
}

/// 三个重置点单调不减：时间往后走，重置点不会往回跳。
#[test]
fn reset_points_are_monotonic() {
    let mut t = at(2026, 1, 1, 0, 0, 0);
    let (mut d, mut w, mut m) = (
        next_daily_reset_after(t),
        next_weekly_reset_after(t),
        next_monthly_reset_after(t),
    );
    for _ in 0..200 {
        t += Duration::hours(7);
        let (nd, nw, nm) = (
            next_daily_reset_after(t),
            next_weekly_reset_after(t),
            next_monthly_reset_after(t),
        );
        assert!(nd >= d && nw >= w && nm >= m, "在 {t} 处重置点回退了");
        (d, w, m) = (nd, nw, nm);
    }
}
