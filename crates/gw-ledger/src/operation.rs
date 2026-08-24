//! The Billing Operation state machine — three states, two terminal.
//!
//! ```text
//!            admit                settle_once
//!   (none) ────────► Held ──────────────────► Settled   (terminal)
//!                     │        release_once
//!                     └──────────────────────► Released (terminal)
//! ```
//!
//! Everything money-shaped keys on a [`BillingOperationId`], and the row that
//! carries the state lives in Postgres (`billing_operations`). Redis holds a
//! *reservation* for the same operation, but a reservation is a cache: it can
//! expire, be evicted, or be lost with the box, and none of that changes what
//! this table says. That is why reconciliation scans Postgres and not a Redis
//! TTL.
//!
//! # Why the decisions are pure functions
//!
//! [`admit`] and [`terminate`] contain the whole policy and touch no I/O. The
//! Postgres implementation expresses the same two rules as conditional SQL
//! (`INSERT ... ON CONFLICT DO NOTHING`, `UPDATE ... WHERE state = 'held'`),
//! and an in-memory store expresses them by calling these directly. Keeping
//! the *decision* in one place is what stops the two from drifting — the
//! `#[ignore]`d Postgres tests pin the SQL against the same properties the
//! in-memory tests pin the functions against.

use crate::LedgerError;
use crate::ids::BillingOperationId;

/// Where one operation is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationState {
    /// Funds reserved, outcome unknown. The only non-terminal state, and
    /// therefore the only one reconciliation looks at.
    Held,
    /// Debited against the real cost. Terminal.
    Settled,
    /// Given back without a debit. Terminal.
    Released,
}

impl OperationState {
    /// The `billing_operations.state` literal.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Held => "held",
            Self::Settled => "settled",
            Self::Released => "released",
        }
    }

    /// Parses a stored literal. An unrecognised value is `None` — treating an
    /// unknown state as `Held` would let a future state be silently
    /// re-charged.
    #[must_use]
    pub fn from_str(raw: &str) -> Option<Self> {
        match raw {
            "held" => Some(Self::Held),
            "settled" => Some(Self::Settled),
            "released" => Some(Self::Released),
            _ => None,
        }
    }

    /// Whether the operation can still move.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Held)
    }
}

/// What a caller wants to reserve, keyed by a server-minted id.
#[derive(Debug, Clone, PartialEq)]
pub struct NewOperation {
    pub operation_id: BillingOperationId,
    pub user_id: i64,
    /// The upper bound actually reserved. For a prepaid tenant this equals
    /// [`admitted_liability`](Self::admitted_liability) — the reservation is
    /// the bound that was checked, not some smaller floor.
    pub reserved_amount: f64,
    /// The liability the pre-flight admitted against the balance.
    pub admitted_liability: f64,
    /// Identifies *which request* this operation is for. Two different
    /// requests must never share one operation id, and this is how that is
    /// detected rather than assumed.
    pub request_fingerprint: String,
    /// Observability only; never compared, never used to key anything.
    pub client_trace_id: String,
}

/// The persisted facts [`admit`] compares an incoming hold against.
#[derive(Debug, Clone, PartialEq)]
pub struct OperationRecord {
    pub operation_id: BillingOperationId,
    pub user_id: i64,
    pub state: OperationState,
    pub reserved_amount: f64,
    pub admitted_liability: f64,
    pub request_fingerprint: String,
}

/// What admitting an operation resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// No row existed. Insert it as [`OperationState::Held`].
    Created,
    /// An identical, still-held row existed — the caller retried the *same*
    /// hold. Idempotent: reserve nothing new, charge nothing new.
    Resumed,
    /// The id is taken by something that is not this hold.
    Conflict(OperationConflict),
}

/// Why a re-hold on an existing operation id was refused.
///
/// Every variant means the same thing operationally: **do not overwrite, do
/// not proceed.** They are separate so the log line says which invariant the
/// caller broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OperationConflict {
    /// The id belongs to a different tenant.
    #[error("operation belongs to another user")]
    Tenant,
    /// Same id, different money.
    #[error("operation was admitted for a different amount")]
    Amount,
    /// Same id, different request.
    #[error("operation was admitted for a different request")]
    Fingerprint,
    /// The operation already settled or released. A terminal operation cannot
    /// be re-opened — that is precisely how one request gets charged twice.
    #[error("operation is already terminal")]
    AlreadyTerminal,
}

/// Money comparison tolerance for [`admit`].
///
/// Amounts round-trip through Postgres `numeric` and back into `f64`, so
/// bit-equality is the wrong test. A tenth of a micro-dollar is far below the
/// smallest amount the ledger can act on and far above the round-trip error.
const AMOUNT_EPSILON: f64 = 1e-9;

fn same_amount(a: f64, b: f64) -> bool {
    (a - b).abs() <= AMOUNT_EPSILON
}

/// Decides whether an incoming hold may use an operation id.
///
/// The rule in one line: **an operation id may be reused only by a byte-identical,
/// still-held hold.** Anything else — a different tenant, a different amount, a
/// different request, or an already-terminal row — is a conflict, never an
/// overwrite.
#[must_use]
pub fn admit(existing: Option<&OperationRecord>, incoming: &NewOperation) -> Admission {
    let Some(existing) = existing else {
        return Admission::Created;
    };
    if existing.user_id != incoming.user_id {
        return Admission::Conflict(OperationConflict::Tenant);
    }
    if existing.request_fingerprint != incoming.request_fingerprint {
        return Admission::Conflict(OperationConflict::Fingerprint);
    }
    if !same_amount(existing.reserved_amount, incoming.reserved_amount)
        || !same_amount(existing.admitted_liability, incoming.admitted_liability)
    {
        return Admission::Conflict(OperationConflict::Amount);
    }
    if existing.state.is_terminal() {
        return Admission::Conflict(OperationConflict::AlreadyTerminal);
    }
    Admission::Resumed
}

/// Whether a terminating call is the one that gets to act.
///
/// Returns [`Transition::Apply`] exactly once per operation, however many
/// callers race: the first settle or release wins and every later one — of
/// either kind — is told the operation is already terminal.
#[must_use]
pub fn terminate(current: OperationState, to: OperationState) -> Transition {
    debug_assert!(to.is_terminal(), "terminate() targets a terminal state");
    if current.is_terminal() {
        return Transition::AlreadyTerminal(current);
    }
    Transition::Apply(to)
}

/// The outcome of [`terminate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// This caller owns the terminal write: perform the debit (or the plain
    /// release) and move the row.
    Apply(OperationState),
    /// Someone already terminated it. **Do nothing** — in particular, do not
    /// debit again.
    AlreadyTerminal(OperationState),
}

/// Result of [`crate::Ledger::settle_once`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettleOnce {
    /// This call performed the one debit for the operation.
    Debited(crate::SettleOutcome),
    /// The operation was already settled or released; nothing was debited.
    AlreadyTerminal(OperationState),
}

/// Result of [`crate::Ledger::release_once`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseOnce {
    /// This call released the reservation.
    Released,
    /// The operation was already settled or released.
    AlreadyTerminal(OperationState),
}

/// A `held` row old enough that its request is presumed dead.
///
/// The reconcile input, read from Postgres — never from a Redis TTL.
#[derive(Debug, Clone, PartialEq)]
pub struct NonTerminalOperation {
    pub operation_id: BillingOperationId,
    pub user_id: i64,
    /// What was reserved, i.e. the most this operation may be charged.
    pub reserved_amount: f64,
    /// The trace the client saw, so a reconciled `usage_logs` row still joins
    /// to whatever the tenant has in their own logs. Observability only.
    pub client_trace_id: String,
    /// Seconds between the row's creation and the scan.
    pub age_seconds: i64,
}

/// Everything admitting a hold can fail with.
#[derive(Debug, thiserror::Error)]
pub enum HoldError {
    /// The operation id is taken by a different hold. **Not OK, not an
    /// overwrite** — the caller minted a colliding id or replayed a finished
    /// one, and either way proceeding would corrupt the ledger.
    #[error("billing operation conflict: {0}")]
    OperationConflict(#[from] OperationConflict),

    /// Available balance does not cover the admitted liability. No reservation
    /// and no `held` row survive this.
    #[error("insufficient balance")]
    InsufficientBalance {
        /// `cached_balance - sum(live holds)` at refusal.
        available: f64,
    },

    #[error(transparent)]
    Ledger(#[from] LedgerError),
}

#[cfg(test)]
mod tests;
