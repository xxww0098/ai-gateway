//! Properties of the four identifiers.
//!
//! The load-bearing one is negative: a minted [`BillingOperationId`] is not a
//! function of anything the client sent. That is asserted here by minting
//! under identical conditions and demanding *different* values — the opposite
//! of what a trace-id-derived key would do.

use std::collections::HashSet;

use super::*;

#[test]
fn minting_never_repeats() {
    let minted: HashSet<String> = (0..10_000)
        .map(|_| BillingOperationId::mint().to_string())
        .collect();
    assert_eq!(
        minted.len(),
        10_000,
        "two operations shared an id; they would share a ledger row"
    );
}

#[test]
fn minting_is_independent_of_any_request_input() {
    // Two requests carrying the *same* client trace id must still get two
    // different operations. If minting ever became a function of the trace
    // header, this is the test that fails.
    let trace = ClientTraceId::new("a-trace-a-client-chose");
    let first = BillingOperationId::mint();
    let second = BillingOperationId::mint();
    assert_ne!(first, second);
    assert!(!first.as_str().contains(trace.as_str()));
    assert!(!second.as_str().contains(trace.as_str()));
}

#[test]
fn a_minted_id_is_never_empty_and_survives_a_storage_round_trip() {
    for _ in 0..256 {
        let id = BillingOperationId::mint();
        assert!(!id.as_str().is_empty());
        assert_eq!(
            BillingOperationId::from_storage(id.as_str()).as_ref(),
            Some(&id)
        );
    }
}

#[test]
fn an_absent_stored_id_is_none_rather_than_an_empty_operation() {
    // `usage_logs.event_key` was an empty string for years. Reading that back
    // as a valid operation id would make every such row share one money key.
    for blank in ["", " ", "\t", "\n", "   \r\n  "] {
        assert!(
            BillingOperationId::from_storage(blank).is_none(),
            "{blank:?} parsed as an operation id"
        );
    }
}

#[test]
fn stored_ids_are_trimmed_so_padding_cannot_fork_one_operation_into_two() {
    let id = BillingOperationId::mint();
    let padded = format!("  {id}\n");
    assert_eq!(
        BillingOperationId::from_storage(&padded).as_ref(),
        Some(&id)
    );
}

#[test]
fn the_observability_ids_keep_what_they_were_given() {
    let trace = ClientTraceId::new("  trace-123  ");
    assert_eq!(trace.as_str(), "trace-123");
    assert!(!trace.is_empty());
    assert!(ClientTraceId::new("   ").is_empty());
    assert!(ClientTraceId::default().is_empty());

    let scope = IdempotencyScope::new("user:7:/v1/messages:abc");
    assert_eq!(scope.as_str(), "user:7:/v1/messages:abc");
}

#[test]
fn attempt_ids_separate_every_attempt_under_one_operation() {
    let operation = BillingOperationId::mint();
    let mut attempts: HashSet<String> = HashSet::new();
    for index in 0..8 {
        for auth in ["auth-a", "auth-b"] {
            attempts.insert(UpstreamAttemptId::for_attempt(&operation, auth, index).to_string());
        }
    }
    assert_eq!(attempts.len(), 16, "attempt ids collided: {attempts:?}");

    // And every one of them names the operation it serves, so an upstream log
    // line joins back to the billing row without a second lookup.
    for attempt in &attempts {
        assert!(attempt.starts_with(operation.as_str()));
    }
}

#[test]
fn two_operations_never_share_an_attempt_id() {
    let a = BillingOperationId::mint();
    let b = BillingOperationId::mint();
    assert_ne!(
        UpstreamAttemptId::for_attempt(&a, "auth", 0),
        UpstreamAttemptId::for_attempt(&b, "auth", 0)
    );
}
