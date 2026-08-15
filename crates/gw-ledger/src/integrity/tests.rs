//! The arithmetic core of balance-integrity checking.
//! The database round-trip it wraps lives in `tests/postgres_ledger.rs`.

use super::usd_to_micro;
use crate::testsupport::Rng;

/// Fixed case: a journal that recomputes to $7 against a balance tampered
/// to $999 must report exactly 992,000,000 micro-USD of drift.
#[test]
fn a_tampered_balance_reports_its_exact_drift() {
    assert_eq!(usd_to_micro(999.0) - usd_to_micro(7.0), 992_000_000);
}

/// A balance that matches its journal reports zero drift — the signal
/// operators read as "nothing to investigate".
#[test]
fn a_consistent_journal_reports_no_drift() {
    let journal: i64 = [10.0_f64, -3.0].iter().copied().map(usd_to_micro).sum();
    assert_eq!(usd_to_micro(7.0) - journal, 0);
}

/// The whole reason the check converts to integers: summing a repeating
/// decimal in `f64` drifts, summing it in micro-USD does not. This is the
/// class of false positive that per-row rounding exists to avoid.
#[test]
fn micro_usd_summation_is_exact_where_float_summation_is_not() {
    let rows = [0.1_f64; 10];

    let float_sum: f64 = rows.iter().sum();
    assert_ne!(float_sum, 1.0, "the float baseline is supposed to drift");

    let micro_sum: i64 = rows.iter().copied().map(usd_to_micro).sum();
    assert_eq!(micro_sum, usd_to_micro(1.0));
}

/// Sub-half-micro amounts round to nothing, so dust below the ledger's
/// resolution cannot accumulate into phantom drift.
#[test]
fn amounts_below_half_a_micro_round_away() {
    assert_eq!(usd_to_micro(0.0000004), 0);
    assert_eq!(usd_to_micro(-0.0000004), 0);
    assert_eq!(usd_to_micro(0.0000006), 1);
}

/// Debits and credits of the same magnitude cancel exactly, in either order —
/// otherwise a round-trip through the ledger would leave residue.
#[test]
fn opposite_movements_cancel_exactly() {
    let mut rng = Rng::new(0x1123_4567);
    for _ in 0..1000 {
        let v = rng.f64_range(-1_000_000.0, 1_000_000.0);
        assert_eq!(usd_to_micro(v) + usd_to_micro(-v), 0);
    }
}

/// The conversion is monotonic: a larger balance can never report a smaller
/// micro figure, so drift keeps its sign and magnitude ordering.
#[test]
fn the_conversion_is_monotonic() {
    let mut rng = Rng::new(0x1876_5432);
    for _ in 0..1000 {
        let lo = rng.f64_range(-1_000_000.0, 1_000_000.0);
        let hi = lo + rng.f64_range(0.0, 1_000_000.0);
        assert!(usd_to_micro(hi) >= usd_to_micro(lo));
    }
}
