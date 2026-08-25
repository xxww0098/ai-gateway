//! Hold / Settle / Release balance ledger (Redis reservation + Postgres truth).
//!
//! The three-phase shape, from `AGENTS.md`:
//!
//! ```text
//! Hold    reserve an estimated cost in Redis, admitted only if
//!         cached_balance - sum(live holds) covers it
//! Settle  debit min(balance, actual) in Postgres, record any shortfall,
//!         and only then clear the Redis reservation
//! Release drop the reservation without charging
//! ```
//!
//! Two rules make the whole thing safe, and both are easy to break by
//! accident:
//!
//! * **The reservation is cleared last.** A settle whose transaction failed —
//!   or whose Redis call failed after the commit — leaves the hold in place so
//!   the request can be reconciled. Releasing early would hand out the same
//!   money twice.
//! * **The expiry cutoff is uniform.** Hold and get-balance both compute
//!   `now - hold_ttl` from the ledger's *configured* TTL, never from a
//!   per-request value, or a short-lived request would evict a long-lived
//!   request's reservation.
//!
//! The `hold` / `settle` / `release` signatures and semantics are a hard
//! project constraint — see `AGENTS.md`.
//!
//! OWNER: worker `ledger-pricing`.

// Rule 5.3's ratchet: this crate's public paths carry no `todo!()` /
// `unimplemented!()`, so pin those two shut here. The root manifest keeps them
// at `warn` for the crates that still have skeleton stubs; the coordinator
// flips the workspace-level deny once the last one is clear.
#![deny(clippy::todo, clippy::unimplemented)]

mod ids;
mod integrity;
mod keys;
mod ledger;
pub mod log_type;
pub mod operation;
mod operation_store;
mod scripts;
mod settlement;

#[cfg(test)]
mod testsupport;

pub use ids::{BillingOperationId, ClientTraceId, IdempotencyScope, UpstreamAttemptId};
pub use integrity::usd_to_micro;
pub use keys::{
    BALANCE_KEY_PREFIX, HOLDS_KEY_PREFIX, HOLDS_TS_KEY_PREFIX, balance_key, holds_key,
    holds_ts_key, shortfall_resolve_reference,
};
pub use ledger::{DEFAULT_BALANCE_TTL, DEFAULT_HOLD_TTL, HoldOutcome, Ledger, SettleOutcome};
pub use operation::{
    Admission, HoldError, NewOperation, NonTerminalOperation, OperationConflict, OperationRecord,
    OperationState, ReleaseOnce, SettleOnce,
};
pub use settlement::Settlement;

/// Everything the ledger can fail with.
///
/// [`InsufficientBalance`](Self::InsufficientBalance) and
/// [`UserNotFound`](Self::UserNotFound) are sentinel errors; match on them
/// rather than on message text.
///
/// Note what is *not* here: a settle that cannot be covered by the balance is
/// not an error. It debits what it can and records the rest as a shortfall —
/// see [`Settlement`] and [`Ledger::has_unresolved_shortfall`].
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    /// The available balance (cached balance minus live holds) does not cover
    /// the requested hold.
    #[error("insufficient balance")]
    InsufficientBalance,

    /// No `users` row for the given id.
    #[error("user not found")]
    UserNotFound,

    /// The user owns at least one unresolved shortfall, so billable work is
    /// refused until the debt is settled. Raised by the callers that preflight
    /// with [`Ledger::has_unresolved_shortfall`], and surfaced as HTTP 402
    /// `outstanding_debt`.
    #[error("outstanding debt")]
    OutstandingDebt,

    /// The reservation a caller expected to still be live was not in Redis.
    #[error("hold not found")]
    HoldNotFound,

    /// A hold-path method was called on a ledger built without Redis. Holds
    /// have nowhere to live without it, so this is a wiring bug, not a
    /// runtime condition.
    #[error("redis client not configured")]
    RedisNotConfigured,

    /// A caller-supplied argument was rejected before any state was touched
    /// (non-positive amount, empty request id, zero TTL).
    #[error("{0}")]
    InvalidArgument(&'static str),

    #[error(transparent)]
    Redis(#[from] redis::RedisError),

    #[error(transparent)]
    Db(#[from] sqlx::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
