//! Per-request billing state carried across the pipeline.
//!
//! The billing context threads a request-scoped record through the pipeline;
//! the axum equivalent is a request extension, which has the same "local to one
//! request, no shared mutable registry" property.
//!
//! [`RequestBilling::claim_finalize`] is what makes "who settles?" a dynamic
//! decision. Here the dispatcher owns the exact usage record, so whichever side
//! reaches the request's end first claims the single settlement — the atomic
//! flag is what keeps `failover` from billing twice (see the channel selector's
//! note on cross-auth retry).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::ports::Id;

/// Immutable billing facts resolved before the upstream call.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettleCtx {
    /// Matches the key used by Hold / Settle / Release.
    pub request_id: String,
    pub user_id: Id,
    /// Zero when the request authenticated via JWT without an API key.
    pub api_key_id: Id,
    pub group_id: Option<Id>,
    /// Group-configured rate multiplier (1.0 by default).
    pub rate_mult: f64,
    /// Active subscription whose quota counters accumulate on Settle.
    pub subscription_id: Option<Id>,
    /// Model identifier extracted from the request body.
    pub model: String,
    /// Client asked for SSE (or any streaming transport).
    pub stream: bool,
    pub ip_address: String,
    /// Raw client `Idempotency-Key`, recorded on the usage log.
    pub idempotency_key: String,
}

/// [`SettleCtx`] plus the mutable bookkeeping the finalizer needs.
///
/// Inserted into the request extensions as `Arc<RequestBilling>` by
/// [`crate::hold`] and read back by the dispatcher.
#[derive(Debug)]
pub struct RequestBilling {
    pub ctx: SettleCtx,
    /// True when the reservation came from the process-local budget token
    /// rather than a Redis hold, so the finalizer must not Release.
    pub used_budget_token: bool,
    finalized: AtomicBool,
}

impl RequestBilling {
    /// Creates the per-request record.
    pub fn new(ctx: SettleCtx, used_budget_token: bool) -> Self {
        Self {
            ctx,
            used_budget_token,
            finalized: AtomicBool::new(false),
        }
    }

    /// Takes exclusive responsibility for terminating this request's billing.
    ///
    /// Returns `true` exactly once per request: the winner must Settle or
    /// Release, every later caller must do nothing. This is what makes
    /// cross-account failover safe — the retry loop settles once, on the final
    /// response, never per attempt.
    pub fn claim_finalize(&self) -> bool {
        !self.finalized.swap(true, Ordering::SeqCst)
    }

    /// Whether billing for this request has already been terminated.
    pub fn is_finalized(&self) -> bool {
        self.finalized.load(Ordering::SeqCst)
    }
}

/// Convenience alias for the request-extension type.
pub type BillingHandle = Arc<RequestBilling>;

#[cfg(test)]
mod tests;
