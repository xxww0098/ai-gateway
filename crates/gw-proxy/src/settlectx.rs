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

use gw_ledger::{BillingOperationId, ClientTraceId};
use gw_pricing::PricingQuote;

use crate::ports::Id;

/// Immutable billing facts resolved before the upstream call.
#[derive(Debug, Clone, PartialEq)]
pub struct SettleCtx {
    /// The server-minted money key: Hold / Settle / Release / reconcile /
    /// `usage_logs.event_key` all use this and nothing else.
    pub operation: BillingOperationId,
    /// The inbound `X-Trace-ID` (or a process-local id). **Observability
    /// only** — it reaches logs, the response header and
    /// `usage_logs.request_id`, and never a ledger call.
    pub client_trace: ClientTraceId,
    pub user_id: Id,
    /// Zero when the request authenticated via JWT without an API key.
    pub api_key_id: Id,
    pub group_id: Option<Id>,
    /// 这次请求**冻结**下来的价格：四列单价 + 分组倍率 + 价目表代次。
    ///
    /// Hold 处铸造一次，结算只读它。这是「在途请求不会被改价追上、
    /// 也不会被上游回的模型名换掉价格键」的落地位置 —— 结算侧因此
    /// 根本没有第二次查价目表的入口。
    pub quote: PricingQuote,
    /// Active subscription whose quota counters accumulate on Settle.
    pub subscription_id: Option<Id>,
    /// 请求里那个模型名（保留 peek 时的大小写），**只供日志与目录**。
    /// 计价用的键在 [`SettleCtx::quote`] 里。
    pub model: String,
    /// Client asked for SSE (or any streaming transport).
    pub stream: bool,
    pub ip_address: String,
    /// Raw client `Idempotency-Key`, recorded on the usage log.
    pub idempotency_key: String,
}

/// A context with a freshly-minted operation and every other field empty.
///
/// Hand-written rather than derived because [`BillingOperationId`] has no
/// `Default` — an operation id that is not minted is not an operation id, and
/// a defaulted empty one would be a money key shared by every such value.
/// 报价同理没有 `Default`：默认价必须是**零价**，因为一个没被铸造过的报价
/// 唯一安全的取值就是「什么也不收」。
impl Default for SettleCtx {
    fn default() -> Self {
        Self {
            operation: BillingOperationId::mint(),
            client_trace: ClientTraceId::default(),
            user_id: 0,
            api_key_id: 0,
            group_id: None,
            quote: PricingQuote::flat("", 0.0, 1.0, 0),
            subscription_id: None,
            model: String::new(),
            stream: false,
            ip_address: String::new(),
            idempotency_key: String::new(),
        }
    }
}

impl SettleCtx {
    /// 写进 `usage_logs.rate_multiplier` 的那个数。它就是报价里冻住的倍率
    /// —— 不是第二个字段，否则日志和实际收费可能各说各话。
    #[must_use]
    pub fn rate_mult(&self) -> f64 {
        self.quote.multiplier().get()
    }
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
