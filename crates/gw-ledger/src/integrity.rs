//! Money-integrity guard: replay the journal, compare against the stored
//! balance.

use crate::ledger::BALANCE_EXPR;
use crate::{Ledger, LedgerError, log_type};

/// Converts USD to integer micro-USD (USD × 1e6, rounded) — the exact unit for
/// money integrity math, free of float accumulation.
///
/// Values beyond `i64`'s range saturate rather than wrapping; at 1e6 scale
/// that bound is ±9.2 trillion USD, so reaching it means the balance was
/// already corrupt.
#[must_use]
pub fn usd_to_micro(v: f64) -> i64 {
    (v * 1e6).round() as i64
}

impl Ledger {
    /// Recomputes a user's balance from the append-only `balance_logs`
    /// journal using exact integer micro-USD arithmetic, and returns the drift
    /// (stored float balance minus recomputed), in micro-USD.
    ///
    /// Zero means the running float balance matches the journal exactly. A
    /// non-zero value flags accumulated `f64` drift or tampering.
    ///
    /// This is the money-integrity guard that lives *within* the constraint
    /// that `users.balance` stays a float column: rather than change the
    /// representation, it lets operators detect and quantify the drift.
    /// Read-only.
    ///
    /// # Errors
    /// [`LedgerError::UserNotFound`] when the user does not exist; otherwise
    /// the underlying query error.
    pub async fn verify_balance_integrity(&self, user_id: i64) -> Result<i64, LedgerError> {
        // `::float8` and the Option are both load-bearing — see BALANCE_EXPR.
        let balance: Option<f64> =
            sqlx::query_scalar(&format!("SELECT {BALANCE_EXPR} FROM users WHERE id = $1"))
                .bind(user_id)
                .fetch_optional(self.db())
                .await?
                .ok_or(LedgerError::UserNotFound)?;

        // Only credit/debit/settle rows move the balance; hold/release/
        // settle_failed rows are audit trail and must not be replayed.
        let amounts: Vec<Option<f64>> = sqlx::query_scalar(
            "SELECT amount::float8 FROM balance_logs WHERE user_id = $1 AND type IN ($2, $3, $4)",
        )
        .bind(user_id)
        .bind(log_type::BALANCE_MOVING[0])
        .bind(log_type::BALANCE_MOVING[1])
        .bind(log_type::BALANCE_MOVING[2])
        .fetch_all(self.db())
        .await?;

        // Round each row before summing: rounding the sum instead would let
        // sub-micro dust from many rows accumulate into a phantom drift.
        let journal_micro: i64 = amounts
            .into_iter()
            .map(|a| usd_to_micro(a.unwrap_or_default()))
            .sum();
        Ok(usd_to_micro(balance.unwrap_or_default()) - journal_micro)
    }
}

#[cfg(test)]
mod tests;
