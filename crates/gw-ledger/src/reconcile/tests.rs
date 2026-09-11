//! The stale-hold pairing step on its own, so the invariant is covered
//! without a Redis. The end-to-end scan lives in `tests/redis_ledger.rs`.

use std::collections::HashMap;

use super::collect_stale;

const NOW: i64 = 1_700_000_000;
/// Fixture: anything older than ten minutes is stale.
const CUTOFF: i64 = NOW - 600;

fn timestamps(pairs: &[(&str, i64)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.to_string()))
        .collect()
}

/// Fixture: one hold created just now, one an hour ago. Only the old one is
/// stale, and its age and amount survive the pairing.
#[test]
fn only_holds_older_than_the_cutoff_are_reported() {
    let members = vec![
        ("req-fresh".to_string(), 5.0),
        ("req-stale".to_string(), 7.0),
    ];
    let ts = timestamps(&[("req-fresh", NOW), ("req-stale", NOW - 3600)]);

    let stale = collect_stale(1, &members, &ts, CUTOFF, NOW);

    assert_eq!(stale.len(), 1, "fresh holds must be excluded: {stale:?}");
    let hold = &stale[0];
    assert_eq!(hold.user_id, 1);
    assert_eq!(hold.request_id, "req-stale");
    assert_eq!(hold.amount, 7.0);
    assert_eq!(hold.age_seconds, 3600);
}

/// A hold created exactly at the cutoff is not yet stale. The boundary belongs
/// to the live side (`ts >= cutoff` is skipped), so a reconciler can never
/// flag a request the moment its deadline arrives.
#[test]
fn the_cutoff_boundary_counts_as_live() {
    let members = vec![
        ("at-cutoff".to_string(), 1.0),
        ("one-older".to_string(), 1.0),
    ];
    let ts = timestamps(&[("at-cutoff", CUTOFF), ("one-older", CUTOFF - 1)]);

    let stale = collect_stale(1, &members, &ts, CUTOFF, NOW);

    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].request_id, "one-older");
}

/// An unknown age is not evidence of staleness. A member with a missing,
/// unparseable, or zero timestamp is left alone — reporting it would send a
/// reconciler after a hold whose age nobody can establish.
#[test]
fn holds_with_an_unknown_age_are_left_alone() {
    let members = vec![
        ("no-timestamp".to_string(), 1.0),
        ("garbage-timestamp".to_string(), 2.0),
        ("zero-timestamp".to_string(), 3.0),
        ("genuinely-old".to_string(), 4.0),
    ];
    let ts = timestamps(&[("zero-timestamp", 0), ("genuinely-old", NOW - 7200)]);
    let mut ts = ts;
    ts.insert("garbage-timestamp".to_string(), "not-a-number".to_string());

    let stale = collect_stale(1, &members, &ts, CUTOFF, NOW);

    assert_eq!(stale.len(), 1, "{stale:?}");
    assert_eq!(stale[0].request_id, "genuinely-old");
}

/// Timestamps with no matching member are ignored: a leftover hash field
/// cannot conjure a reservation that the sorted set does not hold.
#[test]
fn orphan_timestamps_do_not_invent_holds() {
    let ts = timestamps(&[("ghost", NOW - 99_999)]);
    assert!(collect_stale(1, &[], &ts, CUTOFF, NOW).is_empty());
}

/// Age is measured from the hold's own timestamp, so it grows with the hold's
/// real age rather than with anything about the scan.
#[test]
fn the_reported_age_tracks_the_holds_own_timestamp() {
    let members = vec![("old".to_string(), 1.0), ("older".to_string(), 1.0)];
    let ts = timestamps(&[("old", NOW - 1000), ("older", NOW - 5000)]);

    let stale = collect_stale(9, &members, &ts, CUTOFF, NOW);
    let by_id: HashMap<_, _> = stale.iter().map(|h| (h.request_id.as_str(), h)).collect();

    assert_eq!(by_id["old"].age_seconds, 1000);
    assert_eq!(by_id["older"].age_seconds, 5000);
    assert!(by_id["older"].age_seconds > by_id["old"].age_seconds);
    assert!(stale.iter().all(|h| h.user_id == 9));
}
