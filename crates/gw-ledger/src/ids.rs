//! The four identifiers a billable request carries, as four distinct types.
//!
//! They used to be one `String` — an inbound-or-generated `X-Trace-ID` that
//! was simultaneously the log correlation id, the Redis hold key, the settle
//! key, the reconcile key and the usage event key. That conflation is the bug:
//! **a client-supplied header was the money key**, so a client could replay a
//! trace id, or two clients could collide on one, and land on the same ledger
//! row.
//!
//! Splitting them makes the mistake unrepresentable rather than merely
//! discouraged — a [`ClientTraceId`] does not coerce into the parameter that
//! wants a [`BillingOperationId`].
//!
//! | type | who mints it | what it keys |
//! | --- | --- | --- |
//! | [`BillingOperationId`] | **the server**, at hold admission | hold / settle / release / reconcile / `usage_logs.event_key` |
//! | [`ClientTraceId`] | the client (`X-Trace-ID`) or the process | logs, response headers, `usage_logs.request_id` |
//! | [`IdempotencyScope`] | derived from the client `Idempotency-Key` | *client* retry de-duplication, never money |
//! | [`UpstreamAttemptId`] | the dispatcher, per HTTP attempt | one attempt under a route plan / failover |

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// The server-minted identity of one billing operation.
///
/// **This is the money key.** Hold, settle, release, reconcile and
/// `usage_logs.event_key` all key on it, and nothing a client sends can
/// influence it. Construct one with [`BillingOperationId::mint`]; parse one
/// back from storage with [`BillingOperationId::from_storage`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BillingOperationId(String);

/// Process-wide random prefix for minted operation ids.
///
/// One entropy draw for the whole process, not one per request: the id must be
/// unforgeable-by-the-client and unique, not cryptographically random. The
/// prefix separates processes and replicas; the counter separates requests
/// within one. Deliberately a *different* draw from the trace-id prefix, so a
/// leaked trace id says nothing about an operation id.
static OPERATION_PREFIX: LazyLock<u128> = LazyLock::new(|| uuid::Uuid::new_v4().as_u128());

static OPERATION_COUNTER: AtomicU64 = AtomicU64::new(0);

impl BillingOperationId {
    /// Mints a fresh operation id. **Never derived from request input.**
    ///
    /// Two requests arriving with the same `X-Trace-ID` — same tenant or not —
    /// get two different operation ids, which is the whole point.
    #[must_use]
    pub fn mint() -> Self {
        let n = OPERATION_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(format!("{:032x}{n:016x}", *OPERATION_PREFIX))
    }

    /// Rebuilds an id read back from Postgres or Redis.
    ///
    /// Returns `None` for an empty string: an operation with no id is not an
    /// operation, and silently accepting `""` is how every row ends up sharing
    /// one money key.
    #[must_use]
    pub fn from_storage(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(Self(trimmed.to_owned()))
    }

    /// The text form written to `billing_operations.billing_operation_id`,
    /// `balance_logs.reference` and `usage_logs.event_key`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BillingOperationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Declares one plain observability/scoping id: a checked-non-empty string
/// that is *not* interchangeable with the others.
macro_rules! opaque_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Wraps a value, trimming surrounding whitespace.
            #[must_use]
            pub fn new(raw: impl Into<String>) -> Self {
                Self(raw.into().trim().to_owned())
            }

            /// The text form.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Whether no value was supplied.
            #[must_use]
            pub fn is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

opaque_id!(
    /// The inbound `X-Trace-ID`, or a process-local id when the client sent
    /// none.
    ///
    /// **Observability only.** It appears in logs, in the response header and
    /// in `usage_logs.request_id`. It must never reach a hold, settle,
    /// release, reconcile or `event_key` argument — that is what
    /// [`BillingOperationId`] is for, and why these are different types.
    ClientTraceId
);

opaque_id!(
    /// The scope a client `Idempotency-Key` de-duplicates within.
    ///
    /// It bounds *client retries* (same key, same tenant, same route → replay
    /// the stored response). It is **not** a money key: two different
    /// operations may share a scope, and one operation is never identified by
    /// it.
    IdempotencyScope
);

opaque_id!(
    /// One HTTP attempt against one upstream account, under a route plan.
    ///
    /// Failover produces several of these per [`BillingOperationId`]; the
    /// billing side settles once, per operation, never per attempt.
    UpstreamAttemptId
);

impl UpstreamAttemptId {
    /// Names the attempt after the operation it serves plus the account and
    /// zero-based attempt index, so an upstream log line joins back to the
    /// billing row without a second lookup.
    #[must_use]
    pub fn for_attempt(operation: &BillingOperationId, auth_id: &str, index: usize) -> Self {
        Self(format!("{}:{auth_id}:{index}", operation.as_str()))
    }
}

#[cfg(test)]
mod tests;
