//! Redis key layout and the shortfall-resolution reference format.
//!
//! These strings are a wire contract, not an implementation detail: an
//! existing deployment's Redis already holds keys in this shape, and any
//! newer binary must read that same Redis during a cutover. Do not "tidy"
//! them.

/// `String` — the user's cached persistent balance.
pub const BALANCE_KEY_PREFIX: &str = "cpa-gateway:billing:balance:";

/// `Sorted Set` — member = request id, score = hold amount.
pub const HOLDS_KEY_PREFIX: &str = "cpa-gateway:billing:holds:";

/// `Hash` — field = request id, value = hold creation unix timestamp.
///
/// Note this prefix *extends* [`HOLDS_KEY_PREFIX`], so a `SCAN` for hold sets
/// also matches every timestamp hash. [`crate::Ledger::scan_stale_holds`]
/// depends on [`user_id_from_holds_key`] rejecting those.
pub const HOLDS_TS_KEY_PREFIX: &str = "cpa-gateway:billing:holds:ts:";

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

/// The glob a stale-hold scan issues. Deliberately matches the timestamp
/// hashes too — Redis has no "prefix but not that other prefix" glob, so the
/// filtering happens in [`user_id_from_holds_key`].
pub(crate) const HOLDS_SCAN_PATTERN: &str = "cpa-gateway:billing:holds:*";

/// Recovers the owning user id from a hold sorted-set key, rejecting anything
/// that is not one — most importantly the companion timestamp hashes, which
/// share the same prefix.
pub(crate) fn user_id_from_holds_key(key: &str) -> Option<i64> {
    let suffix = key.strip_prefix(HOLDS_KEY_PREFIX)?;
    // `ts:<id>` shares the prefix; it is a timestamp hash, not a hold set.
    // The parse below would reject it anyway, but rejecting it explicitly
    // keeps the intent legible.
    if key.starts_with(HOLDS_TS_KEY_PREFIX) {
        return None;
    }
    suffix.parse::<i64>().ok()
}

/// The reference a compensating credit must carry to resolve one shortfall
/// row, as defined by the billing-security-hardening design:
///
/// ```text
/// shortfall_resolve:<settle row's reference>:<settle row's id>
/// ```
///
/// `request_id` is the reference of the settle row that recorded the debt
/// (i.e. the request id), and `debit_log_id` is that row's `balance_logs.id`.
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
