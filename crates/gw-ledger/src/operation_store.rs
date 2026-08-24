//! The Postgres half of the Billing Operation state machine.
//!
//! Each method below is one conditional statement whose `WHERE` clause *is*
//! the rule from [`crate::operation`]:
//!
//! | rule | SQL that enforces it |
//! | --- | --- |
//! | one row per operation id | `INSERT ... ON CONFLICT (billing_operation_id) DO NOTHING` |
//! | an id may not be silently reused | the follow-up `SELECT` fed to [`admit`] |
//! | first terminal write wins | `UPDATE ... WHERE state = 'held'`, then `rows_affected` |
//! | reconcile reads Postgres, not Redis | [`Ledger::scan_non_terminal_operations`] |
//!
//! The conditional `UPDATE` is what makes settle/release *once*: it takes the
//! row lock, so two concurrent settles serialize, the loser's predicate no
//! longer matches, and it reports zero rows instead of performing a second
//! debit.

use std::time::Duration;

use sqlx::{PgConnection, Row};

use crate::ids::BillingOperationId;
use crate::ledger::BALANCE_EXPR;
use crate::operation::{
    Admission, HoldError, NewOperation, NonTerminalOperation, OperationConflict, OperationRecord,
    OperationState, ReleaseOnce, SettleOnce, Transition, admit, terminate,
};
use crate::{Ledger, LedgerError, SettleOutcome};

/// Columns [`load_operation`] reads back, in one place so the two callers
/// cannot disagree about the cast list. `numeric -> float8` is mandatory, not
/// cosmetic: sqlx will not decode `NUMERIC` into `f64` (see [`BALANCE_EXPR`]).
const OPERATION_COLUMNS: &str = "billing_operation_id, user_id, state, \
     reserved_amount::float8 AS reserved_amount, \
     admitted_liability::float8 AS admitted_liability, \
     request_fingerprint";

impl Ledger {
    /// Records a hold as a durable operation, and reserves for it.
    ///
    /// `redis_ttl` is `Some` for the normal path (write the row, then take the
    /// Redis reservation) and `None` when the reservation was already taken
    /// out of the process-local budget token — the durable row is written
    /// either way, because it is the operation's identity, not its cache
    /// entry.
    ///
    /// Ordering is deliberate: **the durable row first.** A conflict has to be
    /// discovered before any money moves, and a row without a reservation is
    /// merely reconcilable, while a reservation without a row is invisible
    /// money.
    ///
    /// # Errors
    /// [`HoldError::OperationConflict`] when the id is taken by a different
    /// hold, [`HoldError::InsufficientBalance`] when the balance does not
    /// cover `admitted_liability` (nothing durable survives that), otherwise
    /// the underlying query or Redis error.
    pub async fn admit_operation(
        &self,
        op: &NewOperation,
        redis_ttl: Option<Duration>,
    ) -> Result<(), HoldError> {
        match self.begin_operation(op).await? {
            Admission::Created | Admission::Resumed => {}
            Admission::Conflict(conflict) => return Err(HoldError::OperationConflict(conflict)),
        }

        let Some(ttl) = redis_ttl else {
            return Ok(());
        };

        match self
            .hold_with_floor(
                op.user_id,
                op.reserved_amount,
                op.admitted_liability,
                op.operation_id.as_str(),
                ttl,
            )
            .await
        {
            Ok(crate::HoldOutcome::Reserved) => Ok(()),
            Ok(crate::HoldOutcome::Insufficient { available }) => {
                // The gate refused, so this operation never existed as far as
                // billing is concerned. Terminate it now rather than leaving a
                // `held` row for the reconciler to charge.
                self.abandon_operation(&op.operation_id).await;
                Err(HoldError::InsufficientBalance { available })
            }
            Err(err) => {
                self.abandon_operation(&op.operation_id).await;
                Err(HoldError::Ledger(err))
            }
        }
    }

    /// Writes (or re-recognises) the `billing_operations` row.
    ///
    /// # Errors
    /// The underlying query error.
    pub async fn begin_operation(&self, op: &NewOperation) -> Result<Admission, LedgerError> {
        let inserted = sqlx::query(
            "INSERT INTO billing_operations ( \
                billing_operation_id, user_id, state, reserved_amount, admitted_liability, \
                request_fingerprint, client_trace_id, created_at, updated_at \
             ) VALUES ($1, $2, $3, CAST($4 AS numeric), CAST($5 AS numeric), $6, $7, NOW(), NOW()) \
             ON CONFLICT (billing_operation_id) DO NOTHING",
        )
        .bind(op.operation_id.as_str())
        .bind(op.user_id)
        .bind(OperationState::Held.as_str())
        .bind(op.reserved_amount)
        .bind(op.admitted_liability)
        .bind(&op.request_fingerprint)
        .bind(&op.client_trace_id)
        .execute(self.db())
        .await?;

        if inserted.rows_affected() == 1 {
            return Ok(Admission::Created);
        }

        // The id was taken. Whether that is a legitimate retry of the same
        // hold or a collision is exactly what `admit` decides.
        let existing = load_operation(&mut *self.db().acquire().await?, &op.operation_id).await?;
        Ok(admit(existing.as_ref(), op))
    }

    /// Reads one operation row.
    ///
    /// # Errors
    /// The underlying query error.
    pub async fn operation(
        &self,
        operation_id: &BillingOperationId,
    ) -> Result<Option<OperationRecord>, LedgerError> {
        load_operation(&mut *self.db().acquire().await?, operation_id).await
    }

    /// Settles an operation **exactly once**.
    ///
    /// The first terminal caller performs the debit; every later or concurrent
    /// one gets [`SettleOnce::AlreadyTerminal`] and debits nothing. The Redis
    /// reservation is cleared only after the transaction commits, so a settle
    /// whose commit failed stays reconcilable.
    ///
    /// # Errors
    /// [`LedgerError::HoldNotFound`] when no such operation exists;
    /// otherwise the underlying query or Redis error.
    pub async fn settle_once(
        &self,
        operation_id: &BillingOperationId,
        user_id: i64,
        actual_amount: f64,
    ) -> Result<SettleOnce, LedgerError> {
        let mut tx = self.db().begin().await?;
        let outcome = self
            .settle_once_tx(&mut tx, operation_id, user_id, actual_amount)
            .await?;
        tx.commit().await?;

        if matches!(outcome, SettleOnce::Debited(_)) {
            self.clear_hold(user_id, operation_id.as_str()).await?;
        }
        Ok(outcome)
    }

    /// The persistent half of [`settle_once`](Self::settle_once), inside the
    /// caller's transaction.
    ///
    /// Exists so the usage-log insert and the subscription accumulation commit
    /// atomically with the debit. After the caller's transaction commits it
    /// **must** call [`Ledger::clear_hold`]; on rollback it must not, so the
    /// operation stays `held` and reconcilable.
    ///
    /// # Errors
    /// [`LedgerError::HoldNotFound`] for an unknown operation; otherwise the
    /// underlying query error.
    pub async fn settle_once_tx(
        &self,
        conn: &mut PgConnection,
        operation_id: &BillingOperationId,
        user_id: i64,
        actual_amount: f64,
    ) -> Result<SettleOnce, LedgerError> {
        match claim_terminal(&mut *conn, operation_id, OperationState::Settled).await? {
            Transition::AlreadyTerminal(state) => Ok(SettleOnce::AlreadyTerminal(state)),
            Transition::Apply(_) => {
                let outcome: SettleOutcome = self
                    .settle_tx(&mut *conn, user_id, operation_id.as_str(), actual_amount)
                    .await?;
                Ok(SettleOnce::Debited(outcome))
            }
        }
    }

    /// Releases an operation **exactly once**, without touching the balance.
    ///
    /// # Errors
    /// [`LedgerError::HoldNotFound`] for an unknown operation; otherwise the
    /// underlying query or Redis error.
    pub async fn release_once(
        &self,
        operation_id: &BillingOperationId,
        user_id: i64,
    ) -> Result<ReleaseOnce, LedgerError> {
        let mut conn = self.db().acquire().await?;
        let claimed = claim_terminal(&mut conn, operation_id, OperationState::Released).await?;
        drop(conn);

        match claimed {
            Transition::AlreadyTerminal(state) => Ok(ReleaseOnce::AlreadyTerminal(state)),
            Transition::Apply(_) => {
                self.release(user_id, operation_id.as_str()).await?;
                Ok(ReleaseOnce::Released)
            }
        }
    }

    /// Non-terminal operations older than `older_than`, **from Postgres**.
    ///
    /// This is the reconcile input. It is deliberately not a Redis scan: a
    /// reservation that expired, was evicted, or died with its box says
    /// nothing about whether the money was accounted for. The `held` row does.
    ///
    /// `limit` bounds one scan so a large backlog cannot turn a periodic job
    /// into an unbounded read; the remainder is picked up next tick.
    ///
    /// # Errors
    /// The underlying query error.
    pub async fn scan_non_terminal_operations(
        &self,
        older_than: Duration,
        limit: i64,
    ) -> Result<Vec<NonTerminalOperation>, LedgerError> {
        let rows = sqlx::query(
            "SELECT billing_operation_id, user_id, client_trace_id, \
                    reserved_amount::float8 AS reserved_amount, \
                    EXTRACT(EPOCH FROM (NOW() - created_at))::bigint AS age_seconds \
               FROM billing_operations \
              WHERE terminal_at IS NULL \
                AND state = $1 \
                AND created_at < NOW() - make_interval(secs => $2) \
              ORDER BY created_at \
              LIMIT $3",
        )
        .bind(OperationState::Held.as_str())
        .bind(older_than.as_secs_f64())
        .bind(limit)
        .fetch_all(self.db())
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let raw: Option<String> = row.try_get("billing_operation_id").ok()?;
                let operation_id = BillingOperationId::from_storage(raw.as_deref()?)?;
                Some(NonTerminalOperation {
                    operation_id,
                    user_id: row.try_get("user_id").ok()?,
                    reserved_amount: row
                        .try_get::<Option<f64>, _>("reserved_amount")
                        .ok()?
                        .unwrap_or(0.0),
                    client_trace_id: row
                        .try_get::<Option<String>, _>("client_trace_id")
                        .ok()?
                        .unwrap_or_default(),
                    age_seconds: row
                        .try_get::<Option<i64>, _>("age_seconds")
                        .ok()?
                        .unwrap_or(0),
                })
            })
            .collect())
    }

    /// Marks an operation `released` after its reservation was refused.
    ///
    /// Best-effort by design: the caller is already returning an error, and a
    /// stranded `held` row is picked up by reconciliation. Failing the request
    /// a second time over the bookkeeping would be strictly worse.
    async fn abandon_operation(&self, operation_id: &BillingOperationId) {
        let mut conn = match self.db().acquire().await {
            Ok(conn) => conn,
            Err(err) => {
                tracing::warn!(%err, operation = %operation_id, "abandon: no connection");
                return;
            }
        };
        if let Err(err) = claim_terminal(&mut conn, operation_id, OperationState::Released).await {
            tracing::warn!(%err, operation = %operation_id, "abandon: state update failed");
        }
    }

    /// The persisted balance, for tests and callers that need the truth rather
    /// than the cache.
    ///
    /// # Errors
    /// The underlying query error.
    pub async fn persisted_balance(&self, user_id: i64) -> Result<f64, LedgerError> {
        let balance: Option<Option<f64>> =
            sqlx::query_scalar(&format!("SELECT {BALANCE_EXPR} FROM users WHERE id = $1"))
                .bind(user_id)
                .fetch_optional(self.db())
                .await?;
        Ok(balance.flatten().unwrap_or(0.0))
    }
}

/// The conditional terminal write, shared by settle and release.
///
/// `UPDATE ... WHERE state = 'held'` is the whole concurrency argument: the
/// statement takes the row lock, so the second of two racing callers evaluates
/// its predicate against the *already updated* row and matches nothing.
async fn claim_terminal(
    conn: &mut PgConnection,
    operation_id: &BillingOperationId,
    to: OperationState,
) -> Result<Transition, LedgerError> {
    let updated = sqlx::query(
        "UPDATE billing_operations \
            SET state = $2, updated_at = NOW(), terminal_at = NOW() \
          WHERE billing_operation_id = $1 AND state = $3",
    )
    .bind(operation_id.as_str())
    .bind(to.as_str())
    .bind(OperationState::Held.as_str())
    .execute(&mut *conn)
    .await?;

    if updated.rows_affected() == 1 {
        return Ok(terminate(OperationState::Held, to));
    }

    // Zero rows: either already terminal, or no such operation. Those are very
    // different bugs, so they get different answers.
    let existing = load_operation(&mut *conn, operation_id).await?;
    match existing {
        Some(record) => Ok(terminate(record.state, to)),
        None => Err(LedgerError::HoldNotFound),
    }
}

/// Reads one row into the shape [`admit`] compares against.
///
/// An unparseable `state` is treated as a missing row: an unknown state must
/// never be mistaken for `held`, because that is the value that permits a
/// debit.
async fn load_operation(
    conn: &mut PgConnection,
    operation_id: &BillingOperationId,
) -> Result<Option<OperationRecord>, LedgerError> {
    let row = sqlx::query(&format!(
        "SELECT {OPERATION_COLUMNS} FROM billing_operations WHERE billing_operation_id = $1"
    ))
    .bind(operation_id.as_str())
    .fetch_optional(&mut *conn)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };
    let raw_state: Option<String> = row.try_get("state")?;
    let Some(state) = raw_state.as_deref().and_then(OperationState::from_str) else {
        tracing::error!(
            operation = %operation_id,
            state = ?raw_state,
            "billing_operations row carries an unknown state"
        );
        return Ok(None);
    };
    let raw_id: Option<String> = row.try_get("billing_operation_id")?;
    let Some(id) = raw_id.as_deref().and_then(BillingOperationId::from_storage) else {
        return Ok(None);
    };
    let fingerprint: Option<String> = row.try_get("request_fingerprint")?;

    Ok(Some(OperationRecord {
        operation_id: id,
        user_id: row.try_get("user_id")?,
        state,
        reserved_amount: row
            .try_get::<Option<f64>, _>("reserved_amount")?
            .unwrap_or(0.0),
        admitted_liability: row
            .try_get::<Option<f64>, _>("admitted_liability")?
            .unwrap_or(0.0),
        request_fingerprint: fingerprint.unwrap_or_default(),
    }))
}

/// Convenience for the caller that only wants the conflict, not the enum.
impl From<Admission> for Option<OperationConflict> {
    fn from(admission: Admission) -> Self {
        match admission {
            Admission::Conflict(conflict) => Some(conflict),
            Admission::Created | Admission::Resumed => None,
        }
    }
}
