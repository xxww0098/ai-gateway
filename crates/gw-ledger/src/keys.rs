//! Redis key layout and the `balance_logs.reference` formats the ledger owns.
//!
//! These strings are the current Redis key layout. Do not rename them
//! casually — a running process and a later binary must agree on the prefix.

/// `String` — the user's cached persistent balance.
pub const BALANCE_KEY_PREFIX: &str = "ai-gateway:billing:balance:";

/// `String` — the last published `users.balance_version` for this user.
pub const BALANCE_VER_KEY_PREFIX: &str = "ai-gateway:billing:balance:ver:";

/// `Sorted Set` — member = request id, score = hold amount.
pub const HOLDS_KEY_PREFIX: &str = "ai-gateway:billing:holds:";

/// `Hash` — field = request id, value = hold creation unix timestamp.
///
/// Note this prefix *extends* [`HOLDS_KEY_PREFIX`], so a `SCAN` for hold sets
/// also matches every timestamp hash. [`crate::Ledger::scan_stale_holds`]
/// depends on [`user_id_from_holds_key`] rejecting those.
pub const HOLDS_TS_KEY_PREFIX: &str = "ai-gateway:billing:holds:ts:";

#[must_use]
pub fn balance_key(user_id: i64) -> String {
    format!("{BALANCE_KEY_PREFIX}{user_id}")
}

#[must_use]
pub fn balance_ver_key(user_id: i64) -> String {
    format!("{BALANCE_VER_KEY_PREFIX}{user_id}")
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
pub(crate) const HOLDS_SCAN_PATTERN: &str = "ai-gateway:billing:holds:*";

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

/// The `balance_logs.reference` namespace of a service-surface credit.
///
/// Declared here rather than typed again at the call sites: `gw-ledger`'s
/// dedup table uses it to decide which credits are idempotent, and the panel's
/// `POST /api/service/users/{user_id}/credits` uses it to build the reference.
/// `migrations/0015_service_credit_unique.sql` pins the same literal in its
/// partial index; the three cannot drift without a test noticing.
pub const SERVICE_CREDIT_REF_PREFIX: &str = "service_credit:";

/// The reference for one idempotent service-surface credit.
///
/// The caller's `idempotency_key` is the whole identity: a replayed request
/// rebuilds the identical reference, which the `0015` partial unique index and
/// the `credit_tx` pre-check both recognise as already applied.
#[must_use]
pub fn service_credit_reference(idempotency_key: &str) -> String {
    format!("{SERVICE_CREDIT_REF_PREFIX}{idempotency_key}")
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
