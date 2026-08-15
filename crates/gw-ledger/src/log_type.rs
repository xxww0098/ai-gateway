//! The `balance_logs.type` vocabulary.
//!
//! Existing rows in a live database already carry these exact strings, so the
//! panel's audit feed and any reporting query must use them verbatim.

/// Money added to the balance (top-up, refund, compensating credit).
pub const CREDIT: &str = "credit";

/// Money taken off the balance outside the request path (a purchase).
pub const DEBIT: &str = "debit";

/// Audit trail for a reservation being placed. Moves no money — a hold lives
/// in Redis, not in the balance.
pub const HOLD: &str = "hold";

/// The request-path debit, plus the zero-amount `shortfall_usd` marker row
/// when the balance could not cover the cost.
pub const SETTLE: &str = "settle";

/// Audit trail for a reservation being dropped without a charge.
pub const RELEASE: &str = "release";

/// Audit trail written outside the failed transaction when a settle could not
/// commit. The hold deliberately survives so the request can be reconciled.
pub const SETTLE_FAILED: &str = "settle_failed";

/// The types that actually move `users.balance`, and therefore the only ones
/// [`crate::Ledger::verify_balance_integrity`] replays. `hold` / `release` /
/// `settle_failed` are audit-only.
pub const BALANCE_MOVING: [&str; 3] = [CREDIT, DEBIT, SETTLE];
