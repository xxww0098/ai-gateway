//! Redis key layout and the shortfall-resolution reference format.
//!
//! These strings are the current Redis key layout. Do not rename them
//! casually — a running process and a later binary must agree on the prefix.

/// `String` — the user's cached persistent balance.
pub const BALANCE_KEY_PREFIX: &str = "ai-gateway:billing:balance:";

/// `Sorted Set` — member = request id, score = hold amount.
pub const HOLDS_KEY_PREFIX: &str = "ai-gateway:billing:holds:";

/// `Hash` — field = request id, value = hold creation unix timestamp.
///
/// Note this prefix *extends* [`HOLDS_KEY_PREFIX`], so anything globbing the
/// hold prefix also matches every timestamp hash.
pub const HOLDS_TS_KEY_PREFIX: &str = "ai-gateway:billing:holds:ts:";

#[must_use]
pub fn balance_key(user_id: i64) -> String {
    format!("{BALANCE_KEY_PREFIX}{user_id}")
}

#[must_use]
pub fn holds_key(user_id: i64) -> String {
    format!("{HOLDS_KEY_PREFIX}{user_id}")
}

#[must_use]
pub fn holds_ts_key(user_id: i64) -> String {
    format!("{HOLDS_TS_KEY_PREFIX}{user_id}")
}

/// The reference a compensating credit must carry to resolve one shortfall
/// row, as defined by the billing-security-hardening design:
///
/// ```text
/// shortfall_resolve:<settle row's reference>:<settle row's id>
/// ```
///
/// `reference` is the reference of the settle row that recorded the debt
/// (the [`crate::BillingOperationId`] text), and `debit_log_id` is that row's
/// `balance_logs.id`.
/// Pinning both halves is what stops one credit from resolving a different
/// request's debt, and what makes an orphan credit (pointing at a row that
/// does not exist) a no-op rather than a false "all clear".
///
/// [`crate::Ledger::has_unresolved_shortfall`] builds the same string in SQL;
/// keep the two in step.
#[must_use]
pub fn shortfall_resolve_reference(request_id: &str, debit_log_id: i64) -> String {
    format!("shortfall_resolve:{request_id}:{debit_log_id}")
}

#[cfg(test)]
mod tests;
