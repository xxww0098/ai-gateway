//! Transaction-aware balance changes and standalone transaction composition.
//!
//! The caller of `_tx` owns commit/rollback and post-commit cache invalidation.
//! Neither primitive begins a transaction or touches Redis. Credits in a
//! deduplicated reference namespace — `payment_order:` for payment settlement,
//! `service_credit:` for the ozon-pod service surface — collapse to
//! [`BalanceChange::AlreadyApplied`] on replay; every other namespace keeps its
//! existing reference semantics and owns its own retry rules.
//! A matching credit must agree on user and amount. The user-row lock and the
//! partial unique indexes guard concurrent writers; conflicting data fails closed.

use redis::AsyncCommands;
use sqlx::PgConnection;

use super::Ledger;
use crate::keys::{SERVICE_CREDIT_REF_PREFIX, balance_key};
use crate::{LedgerError, log_type};

/// One `balance_logs.reference` namespace whose credits are idempotent.
///
/// This table is the single declaration of a dedup prefix: the pre-check in
/// [`Ledger::credit_tx`] interpolates [`like`](Self::like) verbatim, so a
/// prefix whose migration is missing — or whose predicate drifted from the
/// index it belongs to — is one visibly incomplete entry here instead of two
/// hand-synchronized string literals.
///
/// `pub(super)` because the unit tests in `ledger/tests.rs` assert the table
/// against the migrations; nothing outside this module writes a credit.
pub(super) struct DedupPrefix {
    /// The prefix that opts a credit into idempotency.
    pub(super) prefix: &'static str,
    /// The *literal* predicate the matching partial unique index declares,
    /// `ESCAPE '!'` included. Postgres only uses the index when it can prove
    /// the query predicate implies the index predicate, so this must stay
    /// character-for-character identical to the migration.
    pub(super) like: &'static str,
    /// The conflict error for this namespace. `LedgerError::InvalidArgument`
    /// carries a `&'static str`, so the message is rendered here.
    conflict: &'static str,
}

/// Every namespace that has a partial unique index in `migrations/`:
/// `0010_control_plane_credit_unique.sql` (payment orders) and
/// `0015_service_credit_unique.sql` (service-surface credits). Keeping the
/// `starts_with` branch and the SQL predicate in one row is what stops them
/// from being edited apart.
pub(super) const DEDUP_CREDIT_PREFIXES: [DedupPrefix; 2] = [
    DedupPrefix {
        prefix: "payment_order:",
        like: "'payment!_order:%' ESCAPE '!'",
        conflict: "payment_order credit reference already applied with a different user or amount",
    },
    DedupPrefix {
        prefix: SERVICE_CREDIT_REF_PREFIX,
        like: "'service!_credit:%' ESCAPE '!'",
        conflict: "service_credit reference already applied with a different user or amount",
    },
];

/// The dedup namespace `reference` belongs to, if any.
#[must_use]
pub(super) fn dedup_prefix(reference: &str) -> Option<&'static DedupPrefix> {
    DEDUP_CREDIT_PREFIXES
        .iter()
        .find(|dedup| reference.starts_with(dedup.prefix))
}

fn validate_amount(amount: f64) -> Result<(), LedgerError> {
    if amount.is_finite() && amount > 0.0 {
        Ok(())
    } else {
        Err(LedgerError::InvalidArgument(
            "amount must be positive and finite",
        ))
    }
}

/// What a transaction-aware balance movement did.
///
/// `Applied` carries the calculated balance submitted to PostgreSQL, which
/// applies the existing float8-to-numeric storage conversion. The caller still
/// owns commit. The payment path invalidates the cache only after that commit;
/// this result does not authorize an unversioned cache publication.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BalanceChange {
    /// This transaction submitted a balance change and its journal entry.
    Applied {
        /// Calculated balance before PostgreSQL's numeric storage conversion.
        balance_after: f64,
        /// `users.balance_version` after this write.
        balance_version: i64,
    },
    /// No new money was written: a matching payment credit is already visible
    /// to this transaction. This includes retries after a historical repair;
    /// the first successful repair itself returns `Applied`.
    AlreadyApplied,
}

impl Ledger {
    /// Adds `amount` to a user's balance and writes a `credit` journal row in
    /// one transaction; invalidates the Redis balance cache after the commit.
    ///
    /// Thin convenience wrapper: `begin` + [`credit_tx`](Self::credit_tx) +
    /// `commit` + best-effort cache invalidation. A credit already applied
    /// under the same deduplicated reference collapses to a no-op success.
    /// Callers that
    /// need the credit atomic with their own writes should open the
    /// transaction themselves and call [`credit_tx`](Self::credit_tx)
    /// directly.
    ///
    /// # Errors
    /// [`LedgerError::InvalidArgument`] for a non-positive or non-finite
    /// amount, [`LedgerError::UserNotFound`] for an unknown user, otherwise
    /// the underlying query error.
    pub async fn credit(
        &self,
        user_id: i64,
        amount: f64,
        reference: &str,
    ) -> Result<(), LedgerError> {
        validate_amount(amount)?;
        let mut tx = self.db.begin().await?;
        let change = self.credit_tx(&mut tx, user_id, amount, reference).await?;
        tx.commit().await?;
        if let BalanceChange::Applied {
            balance_after,
            balance_version,
        } = change
        {
            let _ = self
                .publish_balance(user_id, balance_after, balance_version)
                .await;
        }
        Ok(())
    }

    /// Subtracts `amount` from a user's balance and writes a `debit` journal
    /// row in one transaction; publishes the Redis balance cache after the
    /// commit.
    ///
    /// Unlike settle, this refuses to overdraw: an amount the balance cannot
    /// cover fails with [`LedgerError::InsufficientBalance`] and changes
    /// nothing. Purchases must not be able to create debt.
    ///
    /// # Errors
    /// [`LedgerError::InvalidArgument`] for a non-positive or non-finite
    /// amount, [`LedgerError::UserNotFound`] for an unknown user,
    /// [`LedgerError::InsufficientBalance`] when the balance is too low.
    pub async fn debit(
        &self,
        user_id: i64,
        amount: f64,
        reference: &str,
    ) -> Result<(), LedgerError> {
        validate_amount(amount)?;
        let mut tx = self.db.begin().await?;
        let change = self.debit_tx(&mut tx, user_id, amount, reference).await?;
        tx.commit().await?;
        if let BalanceChange::Applied {
            balance_after,
            balance_version,
        } = change
        {
            let _ = self
                .publish_balance(user_id, balance_after, balance_version)
                .await;
        }
        Ok(())
    }

    /// The transaction-aware half of a credit, run inside the caller's
    /// transaction.
    ///
    /// Locks the user row (`SELECT ... FOR UPDATE`), credits the balance and
    /// writes the `credit` journal row. It does **not** begin, commit, roll
    /// back, or touch Redis — see the module docs for the caller's
    /// obligations.
    ///
    /// # Idempotency
    ///
    /// Only for references in a deduplicated namespace — every prefix in
    /// [`DEDUP_CREDIT_PREFIXES`]: after taking the user-row lock, an existing
    /// `credit` row with the same reference is looked up **before any write**.
    /// Matching user and amount (compared in SQL using the same
    /// `float8 -> numeric` conversion as journal insertion) returns
    /// [`BalanceChange::AlreadyApplied`] with nothing written; a mismatched
    /// user or amount is a hard error, never a silent success. References in
    /// any other namespace are written as-is, preserving the legacy
    /// repeat-friendly behavior. The matching partial unique index backs all
    /// of this up at the database level.
    ///
    /// # Errors
    /// [`LedgerError::InvalidArgument`] for a non-positive or non-finite
    /// amount, a non-finite resulting balance, or a deduplicated reference
    /// already applied with a different user or amount;
    /// [`LedgerError::UserNotFound`] for an unknown user; otherwise the
    /// underlying query error (a unique violation from the backstop index
    /// propagates rather than being swallowed).
    pub async fn credit_tx(
        &self,
        conn: &mut PgConnection,
        user_id: i64,
        amount: f64,
        reference: &str,
    ) -> Result<BalanceChange, LedgerError> {
        validate_amount(amount)?;

        // Lock order: payment-orders row (caller, if any) -> users row ->
        // journal insert. The user-row lock serializes concurrent movements
        // for one user, which is what makes the pre-check below race-free
        // against a committed repair.
        let balance = Self::lock_balance(&mut *conn, user_id).await?;

        // Reference-level idempotency pre-check. It must run BEFORE any write:
        // a PostgreSQL statement error aborts the caller's whole transaction,
        // so detecting the duplicate through the partial unique index after
        // mutating the balance could only be answered by throwing that
        // transaction away. A replayed credit takes the AlreadyApplied exit
        // here; the index stays as the database-level backstop.
        if let Some(dedup) = dedup_prefix(reference)
            && let Some((owner, amount_matches)) =
                Self::find_dedup_credit(&mut *conn, reference, amount, dedup.like).await?
        {
            if owner == user_id && amount_matches {
                return Ok(BalanceChange::AlreadyApplied);
            }
            // A reference collision must never acknowledge money paid to
            // another user or in a different persisted amount.
            return Err(LedgerError::InvalidArgument(dedup.conflict));
        }

        let balance_after = balance + amount;
        if !balance_after.is_finite() {
            return Err(LedgerError::InvalidArgument(
                "resulting balance must be finite",
            ));
        }
        let balance_version = Self::set_balance(&mut *conn, user_id, balance_after).await?;
        Self::insert_balance_log(
            &mut *conn,
            user_id,
            amount,
            log_type::CREDIT,
            reference,
            None,
        )
        .await?;
        Ok(BalanceChange::Applied {
            balance_after,
            balance_version,
        })
    }

    /// The transaction-aware half of a debit, run inside the caller's
    /// transaction: locks the user row, refuses to overdraw
    /// ([`LedgerError::InsufficientBalance`] — the caller's transaction must
    /// then abort) and writes the `debit` journal row. No dedup today:
    /// reference-level idempotency for debits is slice 05's subject, so
    /// [`BalanceChange::AlreadyApplied`] is currently never produced here.
    /// It does **not** begin, commit, roll back, or touch Redis.
    ///
    /// # Errors
    /// [`LedgerError::InvalidArgument`] for a non-positive or non-finite
    /// amount or a non-finite resulting balance;
    /// [`LedgerError::UserNotFound`] for an unknown user;
    /// [`LedgerError::InsufficientBalance`] when the balance is too low;
    /// otherwise the underlying query error.
    pub async fn debit_tx(
        &self,
        conn: &mut PgConnection,
        user_id: i64,
        amount: f64,
        reference: &str,
    ) -> Result<BalanceChange, LedgerError> {
        validate_amount(amount)?;

        let balance = Self::lock_balance(&mut *conn, user_id).await?;
        if balance < amount {
            return Err(LedgerError::InsufficientBalance);
        }

        let balance_after = balance - amount;
        if !balance_after.is_finite() {
            return Err(LedgerError::InvalidArgument(
                "resulting balance must be finite",
            ));
        }
        let balance_version = Self::set_balance(&mut *conn, user_id, balance_after).await?;
        Self::insert_balance_log(
            &mut *conn,
            user_id,
            -amount,
            log_type::DEBIT,
            reference,
            None,
        )
        .await?;
        Ok(BalanceChange::Applied {
            balance_after,
            balance_version,
        })
    }

    /// Drops the cached balance so the next read re-derives it from Postgres.
    ///
    /// The documented post-commit obligation of every `*_tx` caller: run it
    /// **after** the transaction commits, never inside it and never on the
    /// failure path (a rollback left nothing to invalidate). Best-effort: a
    /// stale cache entry expires on its own within `balance_ttl`, so a failed
    /// invalidation is logged and swallowed, never propagated.
    pub async fn invalidate_balance_cache(&self, user_id: i64) {
        let Some(mut conn) = self.redis_conn() else {
            return;
        };
        if let Err(err) = conn.del::<_, ()>(balance_key(user_id)).await {
            tracing::debug!(error = %err, user_id, "failed to invalidate balance cache");
        }
    }

    /// Returns the owner and whether the amount matches the journal's numeric
    /// representation. Comparing in SQL avoids a lossy numeric/float8 round trip.
    /// Searching across users exposes identity conflicts rather than hiding them.
    ///
    /// `like` is the namespace's literal index predicate from
    /// [`DEDUP_CREDIT_PREFIXES`]. Interpolating the migration's own text keeps
    /// the planner able to prove the query implies the partial index (and the
    /// `ESCAPE '!'` keeps `_` literal, so `paymentXorder:` does not match);
    /// a parameterized LIKE would silently degrade every replay to a scan.
    async fn find_dedup_credit(
        conn: &mut PgConnection,
        reference: &str,
        amount: f64,
        like: &str,
    ) -> Result<Option<(i64, bool)>, LedgerError> {
        let row = sqlx::query_as(&format!(
            "SELECT user_id, COALESCE(amount = ($2::float8)::numeric, FALSE) \
             FROM balance_logs \
             WHERE type = 'credit' AND reference = $1 \
               AND reference LIKE {like} \
             LIMIT 1"
        ))
        .bind(reference)
        .bind(amount)
        .fetch_optional(conn)
        .await?;
        Ok(row)
    }
}
