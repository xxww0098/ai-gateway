//! Key-layout invariants. The reservation keys and their companion timestamp
//! hashes share one prefix, and a running process and a later binary have to
//! agree on it.

use super::{HOLDS_KEY_PREFIX, balance_key, holds_key, holds_ts_key, shortfall_resolve_reference};
use crate::testsupport::Rng;

/// Reservation keys and their companion timestamp hashes share one prefix, so
/// anything globbing the prefix sees both. Nothing scans them any more —
/// reconciliation reads `billing_operations` — but the layout is still what a
/// running process and a later binary have to agree on.
#[test]
fn timestamp_hashes_share_the_hold_set_prefix() {
    assert!(holds_ts_key(42).starts_with(HOLDS_KEY_PREFIX));
    assert!(holds_key(42).starts_with(HOLDS_KEY_PREFIX));
}

/// Keys for different users never collide, in either namespace — a collision
/// would leak one user's reservations into another's available balance.
#[test]
fn distinct_users_get_distinct_keys() {
    let mut rng = Rng::new(0x0C0F_FEE2);
    for _ in 0..500 {
        let a = rng.i64_range(1, 1_000_000);
        let b = a + rng.i64_range(1, 1_000_000);
        assert_ne!(holds_key(a), holds_key(b));
        assert_ne!(holds_ts_key(a), holds_ts_key(b));
        assert_ne!(balance_key(a), balance_key(b));
        assert_ne!(holds_key(a), balance_key(a));
    }
}

/// The resolve reference pins a credit to one specific debt row. Two debts of
/// the same request, or the same row id under a different request, must never
/// share a reference — otherwise one credit would clear two debts.
#[test]
fn a_resolve_reference_identifies_exactly_one_debt_row() {
    let a = shortfall_resolve_reference("req-1", 10);
    assert_ne!(a, shortfall_resolve_reference("req-1", 11));
    assert_ne!(a, shortfall_resolve_reference("req-2", 10));
    assert_eq!(a, shortfall_resolve_reference("req-1", 10));
}

/// The SQL predicate concatenates `'shortfall_resolve:' || reference || ':' ||
/// id`; this asserts the Rust builder produces that same shape, since the two
/// live in different files and nothing else couples them.
#[test]
fn the_resolve_reference_matches_the_sql_concatenation() {
    let request_id = "req-abc";
    let debit_id = 4321_i64;
    let sql_shape = format!("shortfall_resolve:{request_id}:{debit_id}");
    assert_eq!(
        shortfall_resolve_reference(request_id, debit_id),
        sql_shape,
        "the builder and has_unresolved_shortfall's SQL must agree"
    );
}
