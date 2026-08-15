//! Process-local batch pre-deduction.

use std::time::Duration;

use super::*;

#[test]
fn a_user_without_a_token_always_falls_back_to_the_ledger() {
    let store = BudgetTokenStore::new();
    assert!(!store.try_deduct(1, 0.01));
    assert_eq!(store.remaining(1), None);
}

#[test]
fn deductions_come_out_of_the_batch_until_it_cannot_cover_them() {
    let store = BudgetTokenStore::new();
    store.acquire(1, "batch-1", 1.0, Duration::from_secs(60));

    assert!(store.try_deduct(1, 0.6));
    assert_eq!(store.remaining(1), Some(0.4));

    assert!(
        !store.try_deduct(1, 0.5),
        "a deduction the batch cannot cover must fall through to a Redis hold",
    );
    assert_eq!(
        store.remaining(1),
        Some(0.4),
        "a refused deduction must not consume budget",
    );
}

#[test]
fn an_expired_batch_stops_being_spendable() {
    let store = BudgetTokenStore::new();
    store.acquire(1, "batch-1", 10.0, Duration::from_nanos(1));
    std::thread::sleep(Duration::from_millis(2));
    assert!(!store.try_deduct(1, 0.01));
}

#[test]
fn settlement_may_drive_the_batch_negative_to_signal_it_is_spent() {
    let store = BudgetTokenStore::new();
    store.acquire(1, "batch-1", 1.0, Duration::from_secs(60));
    store.deduct_settle(1, 1.5);
    assert_eq!(store.remaining(1), Some(-0.5));
    assert!(
        !store.try_deduct(1, 0.01),
        "an overspent batch must force the next request back to the ledger",
    );
}

#[test]
fn settling_for_an_unknown_user_is_a_no_op() {
    let store = BudgetTokenStore::new();
    store.deduct_settle(99, 1.0);
    assert_eq!(store.remaining(99), None);
}

#[test]
fn releasing_returns_the_unused_budget_so_it_can_go_back_to_redis() {
    let store = BudgetTokenStore::new();
    store.acquire(1, "batch-1", 2.0, Duration::from_secs(60));
    store.try_deduct(1, 0.5);

    let (batch_id, remaining) = store.release(1).expect("a token to release");
    assert_eq!(batch_id, "batch-1");
    assert_eq!(remaining, 1.5);
    assert_eq!(store.remaining(1), None, "release must drop the token");
    assert!(store.release(1).is_none());
}

#[test]
fn acquiring_again_replaces_the_previous_batch() {
    let store = BudgetTokenStore::new();
    store.acquire(1, "batch-1", 1.0, Duration::from_secs(60));
    store.acquire(1, "batch-2", 5.0, Duration::from_secs(60));
    assert_eq!(store.remaining(1), Some(5.0));
    assert_eq!(store.release(1).expect("token").0, "batch-2");
}
