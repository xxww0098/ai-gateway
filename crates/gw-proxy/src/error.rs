//! Wire-compatible rejection bodies for the `/v1/*` surface.
//!
//! The status code *and* the machine-readable `error` field are the contract
//! that client SDKs branch on, so both are reproduced exactly.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Top-up hint returned in structured 402 bodies.
pub const TOP_UP_URL: &str = "/api/panel/billing/topup";

/// Authentication failure shapes produced by [`crate::access`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// No `Authorization: Bearer <token>` header at all.
    #[error("missing credentials")]
    NoCredentials,
    /// Header present but the key/JWT did not resolve to an active tenant.
    /// Deliberately indistinguishable from a suspended-user rejection so the
    /// endpoint cannot be used as a user-status oracle.
    #[error("invalid credentials")]
    InvalidCredential,
    /// Infrastructure failure while authenticating.
    #[error("{0}")]
    Internal(String),
}

impl AuthError {
    /// The HTTP status for this failure.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::NoCredentials | Self::InvalidCredential => StatusCode::UNAUTHORIZED,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        (self.status(), Json(json!({ "error": self.to_string() }))).into_response()
    }
}

/// Pre-flight rejections produced by [`crate::hold`].
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HoldRejection {
    /// `abortJSON(c, 401, "authentication context required")` — the access
    /// layer did not run or did not publish metadata. Fail closed: an
    /// unauthenticated request must never be billed.
    #[error("authentication context required")]
    MissingAccessContext,

    /// `abortJSON(c, 401, "invalid user_id in access metadata")`.
    #[error("invalid user_id in access metadata")]
    InvalidUserId,

    /// `429 {"error":"Too Many Requests","message":"rate limit exceeded"}`.
    #[error("rate limit exceeded")]
    RateLimited,

    /// `503 {"error":"Service Unavailable","message":"provider temporarily unavailable"}`.
    #[error("provider temporarily unavailable")]
    CircuitOpen,

    /// `402 {"error":"outstanding_debt"}` — an unresolved shortfall blocks all
    /// further billable work (AGENTS.md, Requirements 2.5 / 2.6). A ledger
    /// lookup error maps here too: fail closed so a transient DB hiccup cannot
    /// let a debtor through.
    #[error("outstanding_debt")]
    OutstandingDebt,

    /// Structured 402 carrying the balance gap. Emitted both by the
    /// upper-bound pre-flight (no Redis hold created) and by a `Hold` that
    /// itself reported insufficient funds, so clients see one shape.
    #[error("insufficient balance")]
    InsufficientBalance {
        current_balance: f64,
        required_amount: f64,
    },

    /// `abortJSON(c, 402, reason)` from the subscription quota gate.
    #[error("{0}")]
    QuotaExceeded(String),

    /// `abortJSON(c, 402, paymentErrorMessage(err))` — a Hold error that is not
    /// an insufficient-balance condition.
    #[error("payment required")]
    PaymentRequired,

    /// `409 {"error":"idempotency_conflict", ...}`.
    #[error("a request with this Idempotency-Key is already in progress")]
    IdempotencyConflict,

    /// `409 {"error":"idempotency_replay_unavailable", ...}`.
    #[error("request already processed; cached response was too large to replay")]
    IdempotencyReplayUnavailable,
}

impl HoldRejection {
    /// HTTP status for this rejection.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::MissingAccessContext | Self::InvalidUserId => StatusCode::UNAUTHORIZED,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::CircuitOpen => StatusCode::SERVICE_UNAVAILABLE,
            Self::OutstandingDebt
            | Self::InsufficientBalance { .. }
            | Self::QuotaExceeded(_)
            | Self::PaymentRequired => StatusCode::PAYMENT_REQUIRED,
            Self::IdempotencyConflict | Self::IdempotencyReplayUnavailable => StatusCode::CONFLICT,
        }
    }

    /// Machine-readable `error` field. Either a snake_case code or the HTTP
    /// status text; both shapes are reproduced.
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingAccessContext | Self::InvalidUserId => "Unauthorized",
            Self::RateLimited => "Too Many Requests",
            Self::CircuitOpen => "Service Unavailable",
            Self::OutstandingDebt => "outstanding_debt",
            Self::InsufficientBalance { .. } => "insufficient_balance",
            Self::QuotaExceeded(_) | Self::PaymentRequired => "Payment Required",
            Self::IdempotencyConflict => "idempotency_conflict",
            Self::IdempotencyReplayUnavailable => "idempotency_replay_unavailable",
        }
    }
}

impl IntoResponse for HoldRejection {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = match &self {
            // `outstanding_debt` is the one rejection emitted with no
            // `message` field; keep it byte-identical.
            Self::OutstandingDebt => json!({ "error": self.code() }),
            Self::InsufficientBalance {
                current_balance,
                required_amount,
            } => json!({
                "error": self.code(),
                "message": self.to_string(),
                "current_balance": current_balance,
                "required_amount": required_amount,
                "top_up_url": TOP_UP_URL,
            }),
            _ => json!({ "error": self.code(), "message": self.to_string() }),
        };
        (status, Json(body)).into_response()
    }
}

/// Failures of the upstream dispatch itself (after billing pre-flight passed).
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    /// No upstream account is registered / enabled for the resolved provider.
    #[error("no upstream credential available for provider {0}")]
    NoUpstream(String),
    /// The model name did not resolve to any known provider.
    #[error("unsupported model: {0}")]
    UnknownModel(String),
    /// 客户端写了 `<渠道>/<模型>`，而网关看不见（或大到不该整体重序列化）
    /// 这份 body，没法把顶层 `model` 改成上游认识的名字。
    ///
    /// **明确 400，不原样转发**：带着网关前缀的 `model` 送到上游只会换回一个
    /// 「模型不存在」，客户端从那个错误里读不出「前缀是给网关看的」。
    #[error(
        "cannot rewrite the channel-prefixed model {0}: request body is not readable as a whole"
    )]
    BodyNotRewritable(String),
    /// Upstream answered with a non-2xx status; relayed verbatim.
    #[error("upstream error {status}")]
    Upstream { status: StatusCode, body: String },
    /// Transport / translation failure.
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl DispatchError {
    /// HTTP status this failure surfaces to the client.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::NoUpstream(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::UnknownModel(_) | Self::BodyNotRewritable(_) => StatusCode::BAD_REQUEST,
            Self::Upstream { status, .. } => *status,
            Self::Internal(_) => StatusCode::BAD_GATEWAY,
        }
    }
}

impl IntoResponse for DispatchError {
    fn into_response(self) -> Response {
        let status = self.status();
        // An upstream body is already in the caller's dialect — relay it
        // unchanged rather than re-wrapping, matching the SDK's passthrough.
        if let Self::Upstream { body, .. } = &self
            && !body.is_empty()
        {
            return (
                status,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                body.clone(),
            )
                .into_response();
        }
        (
            status,
            Json(json!({
                "error": { "message": self.to_string(), "type": "upstream_error" }
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests;
