//! The conservation invariants, stated directly against the arithmetic
//! rather than through a database round-trip.

use super::Settlement;
use crate::testsupport::Rng;

const EPSILON: f64 = 1e-9;

/// Conservation, plus the three guards that make it safe.
///
/// The property is `final_balance - Σshortfall == initial_balance - actual`,
/// which — since `final = initial - debited` — is exactly
/// `debited + shortfall == actual`.
///
/// Inputs are bounded to a `[0, 1000]` range, which also keeps
/// `actual - debited` clear of the float cancellation that would make the
/// "shortfall iff overrun" clause ill-defined at astronomical magnitudes.
#[test]
fn a_settlement_conserves_the_cost_and_never_overdraws() {
    let mut rng = Rng::new(0xB00C_1234);
    for i in 0..2000 {
        let balance = rng.f64_range(0.0, 1000.0);
        let actual = rng.f64_range(0.0, 1000.0);
        let split = Settlement::split(balance, actual);

        assert!(
            (split.debited + split.shortfall - actual).abs() <= EPSILON,
            "iteration {i}: {} + {} != {actual}",
            split.debited,
            split.shortfall
        );
        assert!(
            split.debited <= balance + EPSILON,
            "iteration {i}: debited {} exceeds balance {balance}",
            split.debited
        );
        assert!(split.debited >= 0.0, "iteration {i}: negative debit");
        assert!(split.shortfall >= 0.0, "iteration {i}: negative shortfall");
        assert_eq!(
            split.has_shortfall(),
            actual > balance,
            "iteration {i}: shortfall presence must track the overrun \
             (balance={balance} actual={actual} split={split:?})"
        );
    }
}

/// Shrink target: a $0.01 balance meeting a $1.00 cost is fully consumed
/// and leaves $0.99 of tracked debt.
#[test]
fn an_overrun_consumes_the_balance_and_records_the_remainder() {
    let split = Settlement::split(0.01, 1.00);
    assert!((split.debited - 0.01).abs() <= EPSILON, "{split:?}");
    assert!((split.shortfall - 0.99).abs() <= EPSILON, "{split:?}");
    assert!(split.has_shortfall());
}

/// A cost the balance covers is taken in full, with nothing left owing — and
/// no spurious shortfall marker row to make the user look indebted.
#[test]
fn a_covered_cost_is_debited_in_full_with_no_debt() {
    let split = Settlement::split(100.0, 12.5);
    assert_eq!(split.debited, 12.5);
    assert_eq!(split.shortfall, 0.0);
    assert!(!split.has_shortfall());
}

/// Settling exactly the whole balance is the boundary case: fully debited,
/// still no debt.
#[test]
fn settling_the_entire_balance_leaves_no_debt() {
    let split = Settlement::split(7.5, 7.5);
    assert_eq!(split.debited, 7.5);
    assert_eq!(split.shortfall, 0.0);
    assert!(!split.has_shortfall());
}

/// A non-positive cost settles to nothing. This is the fast path where the
/// upstream attributed no cost to the request.
#[test]
fn a_non_positive_cost_settles_to_nothing() {
    for actual in [0.0, -0.0, -1.0, -1e9] {
        let split = Settlement::split(100.0, actual);
        assert_eq!(split, Settlement::default(), "actual={actual}");
    }
}

/// An already-negative balance must not turn a settle into a *credit*: the
/// debit floors at zero and the whole cost becomes tracked debt.
#[test]
fn a_negative_balance_never_credits_the_user_through_a_settle() {
    let split = Settlement::split(-25.0, 4.0);
    assert_eq!(split.debited, 0.0, "a settle must never pay money out");
    assert!((split.shortfall - 4.0).abs() <= EPSILON, "{split:?}");
}

/// More money on hand can only mean more of the cost is actually paid, and
/// correspondingly less debt.
#[test]
fn a_larger_balance_pays_more_and_owes_less() {
    let mut rng = Rng::new(0xB00C_5678);
    for i in 0..1000 {
        let actual = rng.f64_range(0.01, 1000.0);
        let lo = rng.f64_range(0.0, 1000.0);
        let hi = lo + rng.f64_range(0.0, 1000.0);

        let poor = Settlement::split(lo, actual);
        let rich = Settlement::split(hi, actual);

        assert!(
            rich.debited >= poor.debited,
            "iteration {i}: balance {hi} debited {} < balance {lo} debited {}",
            rich.debited,
            poor.debited
        );
        assert!(
            rich.shortfall <= poor.shortfall,
            "iteration {i}: balance {hi} owes {} > balance {lo} owes {}",
            rich.shortfall,
            poor.shortfall
        );
    }
}
