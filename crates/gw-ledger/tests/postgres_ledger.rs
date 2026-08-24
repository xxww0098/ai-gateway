//! Ledger behaviour that lives entirely in Postgres: credit, debit, the
//! partial-debit settle, the unresolved-shortfall predicate, and the balance
//! integrity check.
//!
//! Every test here is `#[ignore]`d — see `tests/common/mod.rs` for how to run
//! them.

mod common;

use common::{FAULT_PREFIX, Fixture, Rng};
use gw_ledger::{
    Admission, BillingOperationId, LedgerError, NewOperation, OperationConflict, OperationState,
    ReleaseOnce, SettleOnce, shortfall_resolve_reference,
};

const EPSILON: f64 = 1e-9;

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() <= EPSILON
}

/// Money in and money out both land on the balance and leave a journal row, so
/// the two never disagree.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn credit_and_debit_move_the_balance_and_journal_it() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(10.0).await;

    fx.ledger
        .credit(user, 5.0, "topup-1")
        .await
        .expect("credit");
    assert!(approx(fx.balance(user).await, 15.0));

    fx.ledger
        .debit(user, 4.0, "purchase-1")
        .await
        .expect("debit");
    assert!(approx(fx.balance(user).await, 11.0));

    let credit = fx.logs_for(user, "topup-1").await;
    assert_eq!(credit.len(), 1);
    assert_eq!(credit[0].0, "credit");
    assert!(
        approx(credit[0].1, 5.0),
        "credit rows carry a positive amount"
    );

    let debit = fx.logs_for(user, "purchase-1").await;
    assert_eq!(debit.len(), 1);
    assert_eq!(debit[0].0, "debit");
    assert!(
        approx(debit[0].1, -4.0),
        "debit rows carry a negative amount"
    );

    fx.cleanup().await;
}

/// A debit the balance cannot cover changes nothing at all. Purchases must not
/// be able to create debt — only the request path may, and only through the
/// shortfall mechanism.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn a_debit_cannot_overdraw() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(3.0).await;

    let err = fx.ledger.debit(user, 3.01, "too-much").await.unwrap_err();
    assert!(matches!(err, LedgerError::InsufficientBalance), "{err:?}");

    assert!(
        approx(fx.balance(user).await, 3.0),
        "balance must be untouched"
    );
    assert!(
        fx.logs_for(user, "too-much").await.is_empty(),
        "a refused debit must not journal anything"
    );

    fx.cleanup().await;
}

/// An unknown user is reported as such rather than silently creating rows for
/// an id that does not exist.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn money_operations_reject_unknown_users() {
    let fx = Fixture::postgres_only().await;
    let ghost = common::next_user_id();

    for err in [
        fx.ledger.credit(ghost, 1.0, "r").await.unwrap_err(),
        fx.ledger.debit(ghost, 1.0, "r").await.unwrap_err(),
        fx.ledger.settle(ghost, "r", 1.0).await.unwrap_err(),
        fx.ledger.verify_balance_integrity(ghost).await.unwrap_err(),
    ] {
        assert!(matches!(err, LedgerError::UserNotFound), "{err:?}");
    }

    fx.cleanup().await;
}

/// The first two conservation invariants:
///
/// * `final_balance - Σshortfall == initial_balance - actual` — the ledger
///   never over-debits, and every dollar it could not take is recorded.
/// * a `shortfall_usd` row exists **iff** the cost overran the balance — no
///   spurious debt on a covered request, no silent debt on an overrun.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn settling_conserves_the_cost_and_records_any_overrun() {
    let mut fx = Fixture::postgres_only().await;
    let mut rng = Rng::new(0x5E77_1E00);

    for i in 0..60 {
        let initial = rng.f64_range(0.0, 1000.0);
        let actual = rng.f64_range(0.0, 1000.0);
        let user = fx.seed_user(initial).await;
        let request_id = format!("settle-cons-{i}");

        fx.ledger
            .settle(user, &request_id, actual)
            .await
            .expect("settle");

        let final_balance = fx.balance(user).await;
        let rows = fx.logs_for(user, &request_id).await;
        let total_shortfall: f64 = rows
            .iter()
            .filter_map(|(_, _, meta)| meta.get("shortfall_usd").and_then(|v| v.as_f64()))
            .filter(|v| *v > 0.0)
            .sum();
        let shortfall_rows = rows
            .iter()
            .filter(|(_, _, meta)| {
                meta.get("shortfall_usd")
                    .and_then(|v| v.as_f64())
                    .is_some_and(|v| v > 0.0)
            })
            .count();

        // Conservation. The tolerance is relative because the balance is
        // stored as numeric and read back as float8.
        let lhs = final_balance - total_shortfall;
        let rhs = initial - actual;
        assert!(
            (lhs - rhs).abs() <= 1e-6,
            "iteration {i}: conservation violated — final={final_balance} \
             Σshortfall={total_shortfall} initial={initial} actual={actual}"
        );

        assert_eq!(
            shortfall_rows > 0,
            actual > initial,
            "iteration {i}: shortfall row presence must track the overrun \
             (initial={initial} actual={actual} rows={rows:?})"
        );
        assert!(
            final_balance >= -1e-6,
            "iteration {i}: settle drove the balance negative ({final_balance})"
        );
    }

    fx.cleanup().await;
}

/// A settle whose cost the balance covers takes it in full, journals exactly
/// one row, and leaves no debt marker behind.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn a_covered_settle_writes_one_row_and_no_debt() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(10.0).await;

    fx.ledger
        .settle(user, "req-covered", 2.5)
        .await
        .expect("settle");

    assert!(approx(fx.balance(user).await, 7.5));
    let rows = fx.logs_for(user, "req-covered").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(approx(rows[0].1, -2.5));
    assert_eq!(rows[0].2["actual_cost"].as_f64(), Some(2.5));
    assert!(rows[0].2.get("shortfall_usd").is_none());
    assert!(
        !fx.ledger
            .has_unresolved_shortfall(user)
            .await
            .expect("probe")
    );

    fx.cleanup().await;
}

/// A zero-cost settle still leaves an audit trail, so a request that consumed
/// nothing is distinguishable from one that never settled at all.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn a_zero_cost_settle_audits_without_moving_money() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(10.0).await;

    fx.ledger
        .settle(user, "req-free", 0.0)
        .await
        .expect("settle");

    assert!(approx(fx.balance(user).await, 10.0));
    let rows = fx.logs_for(user, "req-free").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].0, "settle");
    assert!(approx(rows[0].1, 0.0));

    fx.cleanup().await;
}

/// The reason `settle_tx` exists: the balance debit and the caller's own write
/// — in production, the `usage_logs` insert — commit or roll back together.
/// A rollback must leave *neither*, or the two records diverge.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn settle_tx_commits_or_rolls_back_with_the_callers_own_write() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(10.0).await;

    // Rolled back: the debit and the companion row both vanish.
    {
        let mut tx = fx.pool.begin().await.expect("begin");
        fx.ledger
            .settle_tx(&mut tx, user, "req-rollback", 3.0)
            .await
            .expect("settle_tx");
        sqlx::query(
            "INSERT INTO balance_logs (user_id, amount, type, reference, created_at) \
             VALUES ($1, 0, 'credit', 'companion-rollback', NOW())",
        )
        .bind(user)
        .execute(&mut *tx)
        .await
        .expect("companion write");
        tx.rollback().await.expect("rollback");
    }
    assert!(
        approx(fx.balance(user).await, 10.0),
        "rollback must undo the debit"
    );
    assert!(fx.logs_for(user, "req-rollback").await.is_empty());
    assert!(fx.logs_for(user, "companion-rollback").await.is_empty());

    // Committed: both are durable.
    {
        let mut tx = fx.pool.begin().await.expect("begin");
        let outcome = fx
            .ledger
            .settle_tx(&mut tx, user, "req-commit", 3.0)
            .await
            .expect("settle_tx");
        assert!(approx(outcome.debited, 3.0));
        assert_eq!(outcome.shortfall, 0.0);
        assert!(outcome.shortfall_log_id.is_none());
        sqlx::query(
            "INSERT INTO balance_logs (user_id, amount, type, reference, created_at) \
             VALUES ($1, 0, 'credit', 'companion-commit', NOW())",
        )
        .bind(user)
        .execute(&mut *tx)
        .await
        .expect("companion write");
        tx.commit().await.expect("commit");
    }
    assert!(approx(fx.balance(user).await, 7.0));
    assert_eq!(fx.logs_for(user, "req-commit").await.len(), 1);
    assert_eq!(fx.logs_for(user, "companion-commit").await.len(), 1);

    fx.cleanup().await;
}

/// A settle whose journal insert fails must surface the error and leave the
/// balance exactly as it was. The reservation half needs Redis and lives in
/// `redis_ledger.rs`.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn a_failed_settle_leaves_the_balance_untouched() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(100.0).await;
    let request_id = format!("{FAULT_PREFIX}settle-1");

    let err = fx.ledger.settle(user, &request_id, 3.0).await.unwrap_err();
    assert!(matches!(err, LedgerError::Db(_)), "{err:?}");

    assert!(approx(fx.balance(user).await, 100.0));
    assert!(fx.logs_for(user, &request_id).await.is_empty());

    fx.cleanup().await;
}

/// The shrink target end to end: a $0.01 balance meeting a $1.00 cost is
/// fully consumed, the $0.99 remainder is recorded as debt, and the user is
/// blocked from further billable work.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn an_overrun_records_debt_and_blocks_the_user() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(0.01).await;

    assert!(
        !fx.ledger
            .has_unresolved_shortfall(user)
            .await
            .expect("probe")
    );

    fx.ledger
        .settle(user, "req-overrun", 1.00)
        .await
        .expect("settle");

    assert!(approx(fx.balance(user).await, 0.0));
    let rows = fx.logs_for(user, "req-overrun").await;
    assert_eq!(rows.len(), 2, "debited row + shortfall marker: {rows:?}");
    let shortfall = rows
        .iter()
        .find_map(|(_, _, meta)| meta.get("shortfall_usd").and_then(|v| v.as_f64()))
        .expect("a shortfall marker row");
    assert!(approx(shortfall, 0.99), "{shortfall}");

    assert!(
        fx.ledger
            .has_unresolved_shortfall(user)
            .await
            .expect("probe"),
        "an unpaid shortfall must block billable work"
    );

    fx.cleanup().await;
}

/// The compensating credit clears the debt, and only through the paired
/// reference — which is the whole point of pinning it to the debt row's id.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn a_paired_credit_resolves_the_debt_and_an_unpaired_one_does_not() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(0.0).await;

    let debit_id = fx.insert_shortfall_row(user, "req-debt", 5.0).await;
    assert!(
        fx.ledger
            .has_unresolved_shortfall(user)
            .await
            .expect("probe")
    );

    // A credit that does not name this row leaves the debt standing.
    fx.ledger
        .credit(user, 5.0, "a-generous-but-unrelated-topup")
        .await
        .expect("credit");
    assert!(
        fx.ledger
            .has_unresolved_shortfall(user)
            .await
            .expect("probe"),
        "an unpaired credit must not clear a tracked debt"
    );

    // The paired reference does.
    fx.ledger
        .resolve_shortfall(user, "req-debt", debit_id, 5.0)
        .await
        .expect("resolve");
    assert!(
        !fx.ledger
            .has_unresolved_shortfall(user)
            .await
            .expect("probe")
    );

    // And the resolving row is exactly the reference the SQL predicate builds.
    let reference = shortfall_resolve_reference("req-debt", debit_id);
    assert_eq!(fx.logs_for(user, &reference).await.len(), 1);

    fx.cleanup().await;
}

/// Over any mixture of debts, paired resolves, and orphan resolves, the
/// predicate is true exactly when some debt went unpaired. Orphan credits —
/// pointing at rows that do not exist — must never clear a real debt.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn the_shortfall_predicate_tracks_unpaired_debts_only() {
    let mut fx = Fixture::postgres_only().await;
    let mut rng = Rng::new(0x5407_FA11);

    for i in 0..40 {
        let user = fx.seed_user(0.0).await;
        let debts = rng.i64_range(0, 8);
        let orphans = rng.i64_range(0, 4);
        let shortfall = rng.f64_range(0.0001, 1000.0);

        let mut paired = 0;
        for d in 0..debts {
            let reference = format!("req-{d}");
            let debit_id = fx.insert_shortfall_row(user, &reference, shortfall).await;
            if rng.bool() {
                fx.insert_credit_row(user, &shortfall_resolve_reference(&reference, debit_id))
                    .await;
                paired += 1;
            }
        }
        for o in 0..orphans {
            fx.insert_credit_row(user, &format!("shortfall_resolve:orphan-{o}:9999999"))
                .await;
        }

        let want = paired < debts;
        let got = fx
            .ledger
            .has_unresolved_shortfall(user)
            .await
            .expect("probe");
        assert_eq!(
            got, want,
            "iteration {i}: debts={debts} paired={paired} orphans={orphans}"
        );
    }

    fx.cleanup().await;
}

/// The categorical edge cases for the unresolved-shortfall predicate.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn the_shortfall_predicate_edge_cases() {
    let mut fx = Fixture::postgres_only().await;

    // No debts at all.
    let clean = fx.seed_user(0.0).await;
    assert!(
        !fx.ledger
            .has_unresolved_shortfall(clean)
            .await
            .expect("probe")
    );

    // Orphan resolves with no debts behind them.
    let phantom = fx.seed_user(0.0).await;
    for j in 0..3 {
        fx.insert_credit_row(phantom, &format!("shortfall_resolve:phantom-{j}:123"))
            .await;
    }
    assert!(
        !fx.ledger
            .has_unresolved_shortfall(phantom)
            .await
            .expect("probe")
    );

    // A settle row with no shortfall key must not count as debt.
    let settled = fx.seed_user(10.0).await;
    fx.ledger
        .settle(settled, "req-ok", 1.0)
        .await
        .expect("settle");
    assert!(
        !fx.ledger
            .has_unresolved_shortfall(settled)
            .await
            .expect("probe")
    );

    // Everything paired, plus extra orphans that must not flip the answer.
    let resolved = fx.seed_user(0.0).await;
    for i in 0..2 {
        let reference = format!("req-{i}");
        let id = fx.insert_shortfall_row(resolved, &reference, 0.25).await;
        fx.insert_credit_row(resolved, &shortfall_resolve_reference(&reference, id))
            .await;
    }
    for j in 0..3 {
        fx.insert_credit_row(resolved, &format!("shortfall_resolve:orphan-{j}:9999999"))
            .await;
    }
    assert!(
        !fx.ledger
            .has_unresolved_shortfall(resolved)
            .await
            .expect("probe")
    );

    // Partially paired: one debt left standing is enough.
    let partial = fx.seed_user(0.0).await;
    for i in 0..3 {
        let reference = format!("req-{i}");
        let id = fx.insert_shortfall_row(partial, &reference, 0.75).await;
        if i < 2 {
            fx.insert_credit_row(partial, &shortfall_resolve_reference(&reference, id))
                .await;
        }
    }
    assert!(
        fx.ledger
            .has_unresolved_shortfall(partial)
            .await
            .expect("probe")
    );

    fx.cleanup().await;
}

/// A balance that matches its journal reports no drift, and one edited behind
/// the journal's back is caught with the exact discrepancy.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn balance_integrity_detects_a_balance_edited_behind_the_journal() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(0.0).await;

    fx.ledger.credit(user, 10.0, "c1").await.expect("credit");
    fx.ledger.debit(user, 3.0, "d1").await.expect("debit");
    assert_eq!(
        fx.ledger
            .verify_balance_integrity(user)
            .await
            .expect("verify"),
        0,
        "a journal-consistent balance must report no drift"
    );

    fx.tamper_balance(user, 999.0).await;
    assert_eq!(
        fx.ledger
            .verify_balance_integrity(user)
            .await
            .expect("verify"),
        992_000_000,
        "the drift must be reported exactly, in micro-USD"
    );

    fx.cleanup().await;
}

/// Audit-only rows must not be replayed as balance movements — counting a
/// `hold` or `release` would report drift on a perfectly consistent user.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn balance_integrity_ignores_audit_only_rows() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(0.0).await;

    fx.ledger.credit(user, 10.0, "c1").await.expect("credit");
    // Release journals a zero-amount audit row; a hold-typed row with a
    // non-zero amount is what a real hold writes.
    fx.ledger.release(user, "req-audit").await.expect("release");
    sqlx::query(
        "INSERT INTO balance_logs (user_id, amount, type, reference, created_at) \
         VALUES ($1, 42, 'hold', 'req-audit', NOW())",
    )
    .bind(user)
    .execute(&fx.pool)
    .await
    .expect("audit row");

    assert_eq!(
        fx.ledger
            .verify_balance_integrity(user)
            .await
            .expect("verify"),
        0,
        "hold/release rows must not move the recomputed balance"
    );

    fx.cleanup().await;
}

// =================================================== billing_operations (SM)

/// Builds a fresh operation for `user`, reserving `amount`.
fn operation_for(user: i64, amount: f64, fingerprint: &str) -> NewOperation {
    NewOperation {
        operation_id: BillingOperationId::mint(),
        user_id: user,
        reserved_amount: amount,
        admitted_liability: amount,
        request_fingerprint: fingerprint.to_owned(),
        client_trace_id: "trace-the-client-chose".to_owned(),
    }
}

/// The headline invariant, against the real conditional `UPDATE`: settling one
/// operation a hundred times moves the balance exactly once.
///
/// The in-memory twin of this lives in `src/operation/tests.rs`; both pin the
/// same property so the SQL and the pure decision cannot drift apart.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn settling_one_operation_a_hundred_times_debits_the_balance_once() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(100.0).await;
    let op = operation_for(user, 10.0, "fingerprint");

    assert_eq!(
        fx.ledger.begin_operation(&op).await.expect("begin"),
        Admission::Created
    );

    let mut debited = 0;
    for _ in 0..100 {
        match fx
            .ledger
            .settle_once(&op.operation_id, user, 4.0)
            .await
            .expect("settle_once")
        {
            SettleOnce::Debited(outcome) => {
                debited += 1;
                assert!(approx(outcome.debited, 4.0));
            }
            SettleOnce::AlreadyTerminal(state) => assert_eq!(state, OperationState::Settled),
        }
    }

    assert_eq!(debited, 1, "more than one settle moved money");
    assert!(
        approx(fx.balance(user).await, 96.0),
        "balance reflects exactly one 4.0 debit"
    );

    let record = fx
        .ledger
        .operation(&op.operation_id)
        .await
        .expect("read")
        .expect("row exists");
    assert_eq!(record.state, OperationState::Settled);

    // One settle row, not a hundred.
    let rows = fx.logs_for(user, op.operation_id.as_str()).await;
    assert_eq!(rows.len(), 1, "one terminal settle wrote one journal row");

    fx.cleanup().await;
}

/// Two settles racing on one operation: the row lock the conditional `UPDATE`
/// takes is what makes exactly one of them the debiter.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn concurrent_settles_of_one_operation_debit_once() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(100.0).await;
    let op = operation_for(user, 10.0, "fingerprint");
    fx.ledger.begin_operation(&op).await.expect("begin");

    let gate = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let ledger = fx.ledger.clone();
            let id = op.operation_id.clone();
            let gate = std::sync::Arc::clone(&gate);
            tokio::spawn(async move {
                gate.wait().await;
                matches!(
                    ledger.settle_once(&id, user, 7.0).await.expect("settle"),
                    SettleOnce::Debited(_)
                )
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
    assert!(approx(fx.balance(user).await, 93.0));

    fx.cleanup().await;
}

/// Re-holding an operation id with different money — or for a different
/// request — is a conflict, and leaves the stored row untouched.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn re_holding_an_operation_id_with_different_facts_conflicts() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(100.0).await;
    let op = operation_for(user, 10.0, "fingerprint-a");
    fx.ledger.begin_operation(&op).await.expect("begin");

    let cases = [
        (
            NewOperation {
                reserved_amount: op.reserved_amount + 1.0,
                admitted_liability: op.admitted_liability + 1.0,
                ..op.clone()
            },
            OperationConflict::Amount,
        ),
        (
            NewOperation {
                request_fingerprint: "fingerprint-b".to_owned(),
                ..op.clone()
            },
            OperationConflict::Fingerprint,
        ),
        (
            NewOperation {
                user_id: user + 1,
                ..op.clone()
            },
            OperationConflict::Tenant,
        ),
    ];
    for (candidate, expected) in cases {
        assert_eq!(
            fx.ledger.begin_operation(&candidate).await.expect("begin"),
            Admission::Conflict(expected)
        );
    }

    // Nothing was overwritten by any of the refused re-holds.
    let record = fx
        .ledger
        .operation(&op.operation_id)
        .await
        .expect("read")
        .expect("row exists");
    assert_eq!(record.user_id, user);
    assert!(approx(record.reserved_amount, 10.0));
    assert_eq!(record.request_fingerprint, "fingerprint-a");
    assert_eq!(record.state, OperationState::Held);

    // The identical hold, by contrast, resumes.
    assert_eq!(
        fx.ledger.begin_operation(&op).await.expect("begin"),
        Admission::Resumed
    );

    fx.cleanup().await;
}

/// Prepaid reservations record the admitted liability itself, not a smaller
/// floor: what was checked against the balance is what is reserved.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn a_prepaid_reservation_records_the_admitted_liability() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(100.0).await;
    let mut rng = Rng::new(0x0B11_1146);

    for _ in 0..20 {
        let amount = rng.f64_range(0.01, 50.0);
        let op = operation_for(user, amount, "fingerprint");
        fx.ledger.begin_operation(&op).await.expect("begin");
        let record = fx
            .ledger
            .operation(&op.operation_id)
            .await
            .expect("read")
            .expect("row exists");
        assert!(
            approx(record.reserved_amount, record.admitted_liability),
            "reserved {} != admitted {}",
            record.reserved_amount,
            record.admitted_liability
        );
    }

    fx.cleanup().await;
}

/// Release is once, too, and it never touches the balance.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn releasing_one_operation_repeatedly_never_moves_money() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(100.0).await;
    let op = operation_for(user, 10.0, "fingerprint");
    fx.ledger.begin_operation(&op).await.expect("begin");

    assert_eq!(
        fx.ledger
            .release_once(&op.operation_id, user)
            .await
            .expect("release"),
        ReleaseOnce::Released
    );
    for _ in 0..20 {
        assert_eq!(
            fx.ledger
                .release_once(&op.operation_id, user)
                .await
                .expect("release"),
            ReleaseOnce::AlreadyTerminal(OperationState::Released)
        );
    }
    // And a settle afterwards must not resurrect the charge.
    assert!(matches!(
        fx.ledger
            .settle_once(&op.operation_id, user, 9.0)
            .await
            .expect("settle"),
        SettleOnce::AlreadyTerminal(OperationState::Released)
    ));
    assert!(approx(fx.balance(user).await, 100.0));

    fx.cleanup().await;
}

/// Reconciliation reads Postgres. A terminal operation never shows up, however
/// old it is, and a held one shows up once it is older than the cutoff.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn the_reconcile_scan_reports_only_non_terminal_operations() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(100.0).await;

    let held = operation_for(user, 3.0, "still-running");
    let settled = operation_for(user, 4.0, "already-done");
    fx.ledger.begin_operation(&held).await.expect("begin");
    fx.ledger.begin_operation(&settled).await.expect("begin");
    fx.ledger
        .settle_once(&settled.operation_id, user, 2.0)
        .await
        .expect("settle");

    // Age both rows past the cutoff so the only thing separating them is state.
    sqlx::query("UPDATE billing_operations SET created_at = NOW() - interval '2 hours' WHERE user_id = $1")
        .bind(user)
        .execute(&fx.pool)
        .await
        .expect("age the rows");

    let found = fx
        .ledger
        .scan_non_terminal_operations(std::time::Duration::from_secs(600), 100)
        .await
        .expect("scan");
    let mine: Vec<_> = found.into_iter().filter(|op| op.user_id == user).collect();

    assert_eq!(mine.len(), 1, "only the held operation is reconcilable: {mine:?}");
    assert_eq!(mine[0].operation_id, held.operation_id);
    assert!(approx(mine[0].reserved_amount, 3.0));
    assert!(mine[0].age_seconds >= 3600);

    // A fresh held operation is not yet reconcilable.
    let fresh = operation_for(user, 1.0, "just-started");
    fx.ledger.begin_operation(&fresh).await.expect("begin");
    let found = fx
        .ledger
        .scan_non_terminal_operations(std::time::Duration::from_secs(600), 100)
        .await
        .expect("scan");
    assert!(
        !found.iter().any(|op| op.operation_id == fresh.operation_id),
        "a live request was reported as orphaned"
    );

    fx.cleanup().await;
}

/// Settling an operation that was never held is an error, not a silent debit.
#[tokio::test]
#[ignore = "requires a local Postgres (set GW_TEST_DATABASE_URL)"]
async fn settling_an_unknown_operation_is_refused() {
    let mut fx = Fixture::postgres_only().await;
    let user = fx.seed_user(100.0).await;

    let err = fx
        .ledger
        .settle_once(&BillingOperationId::mint(), user, 5.0)
        .await
        .unwrap_err();
    assert!(matches!(err, LedgerError::HoldNotFound), "{err:?}");
    assert!(approx(fx.balance(user).await, 100.0));

    fx.cleanup().await;
}
