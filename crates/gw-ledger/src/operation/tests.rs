//! Properties of the Billing Operation state machine.
//!
//! Two of these are the headline invariants of the whole billing path:
//!
//! * settling one operation any number of times debits **once**, and
//! * re-holding an operation id with different money or a different request is
//!   a conflict, never an overwrite.
//!
//! They are tested against an in-memory store that calls the same [`admit`] /
//! [`terminate`] the Postgres statements encode, so what is pinned here is the
//! *decision*, not one backend's SQL. The Postgres side is pinned separately
//! by the `#[ignore]`d integration tests in `tests/postgres_ledger.rs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::*;
use crate::ids::BillingOperationId;
use crate::testsupport::Rng;

// ------------------------------------------------------------- test double

/// One operation as the store keeps it, plus the debits it caused.
#[derive(Debug, Clone)]
struct Row {
    record: OperationRecord,
}

/// An in-memory [`OperationStore`]-shaped double.
///
/// It exists to make "how many debits happened" observable. The mutex is the
/// stand-in for Postgres' row lock — the point of the concurrency test is that
/// *whatever* serializes the two callers, only one of them may debit.
#[derive(Clone, Default)]
struct MemoryOperations {
    rows: Arc<Mutex<HashMap<String, Row>>>,
    /// Every debit that actually moved money, in order.
    debits: Arc<Mutex<Vec<(String, f64)>>>,
}

impl MemoryOperations {
    fn begin(&self, op: &NewOperation) -> Admission {
        let mut rows = self.rows.lock().expect("poisoned");
        let key = op.operation_id.to_string();
        let decision = admit(rows.get(&key).map(|row| &row.record), op);
        if decision == Admission::Created {
            rows.insert(
                key,
                Row {
                    record: OperationRecord {
                        operation_id: op.operation_id.clone(),
                        user_id: op.user_id,
                        state: OperationState::Held,
                        reserved_amount: op.reserved_amount,
                        admitted_liability: op.admitted_liability,
                        request_fingerprint: op.request_fingerprint.clone(),
                    },
                },
            );
        }
        decision
    }

    /// The double's whole point: the terminal move and the debit happen under
    /// one lock, exactly as the Postgres path does them under one row lock in
    /// one transaction.
    fn settle_once(&self, id: &BillingOperationId, actual: f64) -> SettleOnce {
        let mut rows = self.rows.lock().expect("poisoned");
        let Some(row) = rows.get_mut(id.as_str()) else {
            panic!("settle of an operation that was never held");
        };
        match terminate(row.record.state, OperationState::Settled) {
            Transition::AlreadyTerminal(state) => SettleOnce::AlreadyTerminal(state),
            Transition::Apply(to) => {
                row.record.state = to;
                self.debits
                    .lock()
                    .expect("poisoned")
                    .push((id.to_string(), actual));
                SettleOnce::Debited(crate::SettleOutcome {
                    debited: actual,
                    shortfall: 0.0,
                    shortfall_log_id: None,
                })
            }
        }
    }

    fn release_once(&self, id: &BillingOperationId) -> ReleaseOnce {
        let mut rows = self.rows.lock().expect("poisoned");
        let Some(row) = rows.get_mut(id.as_str()) else {
            panic!("release of an operation that was never held");
        };
        match terminate(row.record.state, OperationState::Released) {
            Transition::AlreadyTerminal(state) => ReleaseOnce::AlreadyTerminal(state),
            Transition::Apply(to) => {
                row.record.state = to;
                ReleaseOnce::Released
            }
        }
    }

    fn state(&self, id: &BillingOperationId) -> OperationState {
        self.rows
            .lock()
            .expect("poisoned")
            .get(id.as_str())
            .expect("operation exists")
            .record
            .state
    }

    fn debits_for(&self, id: &BillingOperationId) -> Vec<f64> {
        self.debits
            .lock()
            .expect("poisoned")
            .iter()
            .filter(|(key, _)| key == id.as_str())
            .map(|(_, amount)| *amount)
            .collect()
    }
}

fn new_operation(user_id: i64, amount: f64, fingerprint: &str) -> NewOperation {
    NewOperation {
        operation_id: BillingOperationId::mint(),
        user_id,
        reserved_amount: amount,
        admitted_liability: amount,
        request_fingerprint: fingerprint.to_owned(),
        client_trace_id: "trace-the-client-chose".to_owned(),
    }
}

// --------------------------------------------------------------- settle_once

#[test]
fn settling_one_operation_a_hundred_times_debits_once() {
    let store = MemoryOperations::default();
    let op = new_operation(7, 1.25, "fingerprint");
    assert_eq!(store.begin(&op), Admission::Created);

    let mut terminal = 0;
    let mut debited = 0;
    for _ in 0..100 {
        match store.settle_once(&op.operation_id, 0.75) {
            SettleOnce::Debited(_) => debited += 1,
            SettleOnce::AlreadyTerminal(state) => {
                assert_eq!(state, OperationState::Settled);
                terminal += 1;
            }
        }
    }

    assert_eq!(debited, 1, "more than one settle moved money");
    assert_eq!(terminal, 99);
    assert_eq!(store.debits_for(&op.operation_id), vec![0.75]);
    assert_eq!(store.state(&op.operation_id), OperationState::Settled);
}

#[tokio::test]
async fn concurrent_settles_of_one_operation_produce_exactly_one_debit() {
    for _ in 0..64 {
        let store = MemoryOperations::default();
        let op = new_operation(11, 2.0, "fingerprint");
        store.begin(&op);

        // Two racers released together by a barrier, so which one reaches the
        // terminal write first is up to the scheduler rather than to the order
        // the tasks were spawned.
        let gate = Arc::new(tokio::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let store = store.clone();
                let id = op.operation_id.clone();
                let gate = Arc::clone(&gate);
                tokio::spawn(async move {
                    gate.wait().await;
                    matches!(store.settle_once(&id, 1.5), SettleOnce::Debited(_))
                })
            })
            .collect();

        let mut winners = 0;
        for handle in handles {
            if handle.await.expect("task") {
                winners += 1;
            }
        }
        assert_eq!(winners, 1, "both racers debited the same operation");
        assert_eq!(store.debits_for(&op.operation_id).len(), 1);
    }
}

#[test]
fn a_released_operation_can_never_be_settled_afterwards() {
    // The dangerous direction: release frees the reservation, so a later
    // settle would debit money the tenant already got back.
    let store = MemoryOperations::default();
    let op = new_operation(3, 5.0, "fingerprint");
    store.begin(&op);

    assert_eq!(store.release_once(&op.operation_id), ReleaseOnce::Released);
    assert!(matches!(
        store.settle_once(&op.operation_id, 4.0),
        SettleOnce::AlreadyTerminal(OperationState::Released)
    ));
    assert!(store.debits_for(&op.operation_id).is_empty());
}

#[test]
fn releasing_repeatedly_is_a_no_op_after_the_first() {
    let store = MemoryOperations::default();
    let op = new_operation(3, 5.0, "fingerprint");
    store.begin(&op);

    assert_eq!(store.release_once(&op.operation_id), ReleaseOnce::Released);
    for _ in 0..50 {
        assert_eq!(
            store.release_once(&op.operation_id),
            ReleaseOnce::AlreadyTerminal(OperationState::Released)
        );
    }
    assert_eq!(store.state(&op.operation_id), OperationState::Released);
}

#[test]
fn a_settled_operation_cannot_be_released_back() {
    let store = MemoryOperations::default();
    let op = new_operation(3, 5.0, "fingerprint");
    store.begin(&op);

    assert!(matches!(
        store.settle_once(&op.operation_id, 4.0),
        SettleOnce::Debited(_)
    ));
    assert_eq!(
        store.release_once(&op.operation_id),
        ReleaseOnce::AlreadyTerminal(OperationState::Settled)
    );
}

// ------------------------------------------------------------- re-hold

#[test]
fn re_holding_the_identical_operation_is_idempotent() {
    let store = MemoryOperations::default();
    let op = new_operation(7, 1.25, "fingerprint");
    assert_eq!(store.begin(&op), Admission::Created);
    for _ in 0..10 {
        assert_eq!(store.begin(&op), Admission::Resumed);
    }
    assert_eq!(store.state(&op.operation_id), OperationState::Held);
}

#[test]
fn re_holding_with_a_different_amount_conflicts() {
    let mut rng = Rng::new(0x0BAD_F00D);
    let store = MemoryOperations::default();
    let op = new_operation(7, 1.25, "fingerprint");
    store.begin(&op);

    for _ in 0..200 {
        // Any perturbation the ledger could act on must be caught. The nudge
        // is generated, never a literal copied from the implementation.
        let nudge = rng.f64_range(1e-6, 1_000.0);
        for candidate in [
            NewOperation {
                reserved_amount: op.reserved_amount + nudge,
                ..op.clone()
            },
            NewOperation {
                admitted_liability: op.admitted_liability + nudge,
                ..op.clone()
            },
        ] {
            assert_eq!(
                store.begin(&candidate),
                Admission::Conflict(OperationConflict::Amount),
                "a re-hold for a different amount was admitted"
            );
        }
    }
    // ... and nothing was overwritten.
    assert_eq!(store.state(&op.operation_id), OperationState::Held);
}

#[test]
fn re_holding_for_a_different_request_conflicts() {
    let store = MemoryOperations::default();
    let op = new_operation(7, 1.25, "fingerprint-a");
    store.begin(&op);

    let other = NewOperation {
        request_fingerprint: "fingerprint-b".to_owned(),
        ..op.clone()
    };
    assert_eq!(
        store.begin(&other),
        Admission::Conflict(OperationConflict::Fingerprint)
    );
}

#[test]
fn re_holding_under_a_different_tenant_conflicts() {
    let store = MemoryOperations::default();
    let op = new_operation(7, 1.25, "fingerprint");
    store.begin(&op);

    let other = NewOperation {
        user_id: op.user_id + 1,
        ..op.clone()
    };
    assert_eq!(
        store.begin(&other),
        Admission::Conflict(OperationConflict::Tenant)
    );
}

#[test]
fn a_terminal_operation_cannot_be_re_held_even_by_an_identical_hold() {
    // Re-opening a settled operation is exactly how one request gets charged
    // twice, so "identical" is not enough — terminal is terminal.
    for terminate_with in [OperationState::Settled, OperationState::Released] {
        let store = MemoryOperations::default();
        let op = new_operation(7, 1.25, "fingerprint");
        store.begin(&op);
        match terminate_with {
            OperationState::Settled => {
                store.settle_once(&op.operation_id, 1.0);
            }
            _ => {
                store.release_once(&op.operation_id);
            }
        }
        assert_eq!(
            store.begin(&op),
            Admission::Conflict(OperationConflict::AlreadyTerminal)
        );
    }
}

#[test]
fn a_client_trace_id_never_affects_admission() {
    // The trace id is observability. Two holds that differ *only* there are
    // the same hold; if this ever changed, a client could fork one operation
    // into two by varying a header.
    let store = MemoryOperations::default();
    let op = new_operation(7, 1.25, "fingerprint");
    store.begin(&op);

    let relabelled = NewOperation {
        client_trace_id: "a-completely-different-trace".to_owned(),
        ..op.clone()
    };
    assert_eq!(store.begin(&relabelled), Admission::Resumed);
}

// ------------------------------------------------------------ pure decisions

#[test]
fn amounts_that_differ_only_by_numeric_round_trip_noise_are_the_same_amount() {
    // Amounts go to Postgres as `numeric` and come back through `::float8`.
    // Treating that noise as a conflict would make every retry fail.
    let mut rng = Rng::new(0x00FF_1CE5);
    for _ in 0..500 {
        let amount = rng.f64_range(0.0, 10_000.0);
        let noisy = f64::from_bits(amount.to_bits() ^ 1);
        let op = new_operation(1, amount, "fingerprint");
        let existing = OperationRecord {
            operation_id: op.operation_id.clone(),
            user_id: op.user_id,
            state: OperationState::Held,
            reserved_amount: noisy,
            admitted_liability: noisy,
            request_fingerprint: op.request_fingerprint.clone(),
        };
        assert_eq!(admit(Some(&existing), &op), Admission::Resumed);
    }
}

#[test]
fn terminate_yields_apply_only_from_the_held_state() {
    for to in [OperationState::Settled, OperationState::Released] {
        assert_eq!(terminate(OperationState::Held, to), Transition::Apply(to));
        for from in [OperationState::Settled, OperationState::Released] {
            assert_eq!(
                terminate(from, to),
                Transition::AlreadyTerminal(from),
                "a terminal operation accepted another terminal write"
            );
        }
    }
}

#[test]
fn only_held_is_non_terminal() {
    assert!(!OperationState::Held.is_terminal());
    assert!(OperationState::Settled.is_terminal());
    assert!(OperationState::Released.is_terminal());
}

#[test]
fn every_state_round_trips_through_its_stored_literal() {
    for state in [
        OperationState::Held,
        OperationState::Settled,
        OperationState::Released,
    ] {
        assert_eq!(OperationState::parse(state.as_str()), Some(state));
    }
}

#[test]
fn an_unknown_stored_state_is_not_mistaken_for_held() {
    // `held` is the only state that permits a debit. A future or corrupted
    // literal must never decode into it.
    for unknown in ["", "HELD", "settling", "abandoned", "0"] {
        assert_eq!(
            OperationState::parse(unknown),
            None,
            "{unknown:?} decoded as a state"
        );
    }
}
