//! Error translation. The mapping is the whole adapter: the middleware branches
//! on these variants to choose between a structured 402 and a generic one.

use super::*;

#[test]
fn the_two_conditions_the_preflight_distinguishes_survive_translation() {
    assert!(matches!(
        map_error(LedgerError::InsufficientBalance),
        BillingError::InsufficientBalance
    ));
    assert!(matches!(
        map_error(LedgerError::OutstandingDebt),
        BillingError::OutstandingDebt
    ));
}

#[test]
fn a_missing_reservation_stays_distinguishable_from_an_outage() {
    // The settlement fallback reads "no hold" as a definite zero; folding it in
    // with infrastructure failures would bill such a request at zero.
    assert!(matches!(
        map_error(LedgerError::HoldNotFound),
        BillingError::HoldNotFound
    ));
}

#[test]
fn every_infrastructure_failure_lands_in_the_catch_all() {
    // These must NOT read as insufficient balance: the middleware answers that
    // with a structured 402 quoting a balance it never actually looked up.
    for err in [
        LedgerError::UserNotFound,
        LedgerError::RedisNotConfigured,
        LedgerError::InvalidArgument("requestID is required"),
    ] {
        let label = err.to_string();
        assert!(
            matches!(map_error(err), BillingError::Other(_)),
            "{label} escaped the catch-all",
        );
    }
}

#[test]
fn translation_preserves_the_original_message_for_the_operator() {
    let mapped = map_error(LedgerError::UserNotFound);
    assert!(
        mapped.to_string().contains("user not found"),
        "the cause must survive into the log line, got {mapped}",
    );
}
