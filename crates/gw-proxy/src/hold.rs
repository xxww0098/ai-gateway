//! Pre-flight balance reservation and quota enforcement for `/v1/*`.
//!
//! The ordering below is load-bearing: every step that can reject the request
//! runs BEFORE any Redis hold is created, so a rejected request never leaves a
//! reservation behind:
//!
//! 1. skip non-billable paths
//! 2. read the [`AccessMetadata`] the access layer published (401 if absent)
//! 3. peek the body for `model` / `stream` / `max_tokens` / input size
//! 4. rate limiter (fail-open on infrastructure error)
//! 5. idempotency check — replay a completed duplicate, reject an in-flight one
//! 6. circuit breaker (503 when the provider is broken)
//! 7. **outstanding-debt pre-flight** -> 402 `outstanding_debt`
//! 8. subscription quota (lock, rotate stale counters, compare against estimate)
//! 9. **upper-bound pre-flight**: `max(hold, EstimateWithMaxTokens, Estimate(stream))`
//!    vs available balance -> 402 `insufficient_balance`, no hold created
//! 10. budget token -> else `ledger.Hold`
//! 11. idempotency claim (only now that funds are reserved)
//! 12. run downstream, then settle-or-release exactly once

use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes, HttpBody as _};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Datelike, NaiveDate, Utc};

use crate::ProxyState;
use crate::access::is_proxy_path;
use crate::error::HoldRejection;
use crate::idempotency::{CachedResponse, IdempotencyManager};
use crate::ports::{
    AccessMetadata, BillingError, BillingLedger, CircuitBreaker, Id, PricingCalculator,
    RateLimiter, SubscriptionQuota, SubscriptionQuotaStore,
};
use crate::settlectx::{BillingHandle, RequestBilling, SettleCtx};
use crate::usage::Settlement;

/// Caps how many bytes of the request body are read during pre-flight.
pub const HOLD_REQUEST_BODY_LIMIT: usize = 1 << 20;

/// Caps how many response bytes are buffered for idempotent replay.
pub const IDEMPOTENCY_BODY_CAPTURE_LIMIT: usize = 10 << 20;

/// Canonical trace header. An inbound value is honored so a caller can
/// correlate the Hold with its own request id.
pub const TRACE_HEADER: &str = "x-trace-id";

/// Default hold TTL when none is configured. Holds must always expire so a
/// stuck request cannot starve a balance indefinitely.
pub const DEFAULT_HOLD_TTL: Duration = Duration::from_secs(300);

/// Detached-operation budget for release / rate-limiter cleanup (2 seconds).
const DETACHED_TIMEOUT: Duration = Duration::from_secs(2);

/// The request body peeked for billing inputs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BodyPeek {
    pub model: String,
    pub stream: bool,
    /// `max_tokens` -> `max_completion_tokens` -> 0. Zero means "no cap".
    pub max_tokens: i64,
    /// Approximated from the raw body size; over-counts slightly, which is the
    /// safe direction for a reservation.
    pub input_tokens: i64,
    /// True when a JSON payload was parsed successfully.
    pub parsed: bool,
}

/// The peeked request body, republished as an extension so the dispatcher does
/// not read the stream twice.
#[derive(Debug, Clone)]
pub struct PeekedBody(pub Bytes);

/// Pre-flight balance reservation and quota gate.
///
/// Optional collaborators are `Option`al because a deployment that has not
/// wired Redis rate limiting or idempotency still bills correctly.
pub struct HoldMiddleware {
    ledger: Arc<dyn BillingLedger>,
    calc: Arc<dyn PricingCalculator>,
    settlement: Arc<Settlement>,
    ttl: Duration,

    quota_store: Option<Arc<dyn SubscriptionQuotaStore>>,
    rate_limiter: Option<Arc<dyn RateLimiter>>,
    circuit_breaker: Option<Arc<dyn CircuitBreaker>>,
    idempotency: Option<Arc<IdempotencyManager>>,
    budget_tokens: Option<Arc<crate::budget_token::BudgetTokenStore>>,
}

impl HoldMiddleware {
    /// A non-positive `ttl` falls back to [`DEFAULT_HOLD_TTL`].
    pub fn new(
        ledger: Arc<dyn BillingLedger>,
        calc: Arc<dyn PricingCalculator>,
        settlement: Arc<Settlement>,
        ttl: Duration,
    ) -> Self {
        Self {
            ledger,
            calc,
            settlement,
            ttl: if ttl.is_zero() { DEFAULT_HOLD_TTL } else { ttl },
            quota_store: None,
            rate_limiter: None,
            circuit_breaker: None,
            idempotency: None,
            budget_tokens: None,
        }
    }

    /// Attach the subscription-quota store.
    #[must_use]
    pub fn with_quota_store(mut self, store: Arc<dyn SubscriptionQuotaStore>) -> Self {
        self.quota_store = Some(store);
        self
    }

    /// Attach the rate limiter.
    #[must_use]
    pub fn with_rate_limiter(mut self, rl: Arc<dyn RateLimiter>) -> Self {
        self.rate_limiter = Some(rl);
        self
    }

    /// Attach the circuit breaker.
    #[must_use]
    pub fn with_circuit_breaker(mut self, cb: Arc<dyn CircuitBreaker>) -> Self {
        self.circuit_breaker = Some(cb);
        self
    }

    /// Attach the idempotency manager.
    #[must_use]
    pub fn with_idempotency(mut self, im: Arc<IdempotencyManager>) -> Self {
        self.idempotency = Some(im);
        self
    }

    /// Attach the budget-token store.
    #[must_use]
    pub fn with_budget_tokens(mut self, bts: Arc<crate::budget_token::BudgetTokenStore>) -> Self {
        self.budget_tokens = Some(bts);
        self
    }

    /// Configured hold TTL.
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// The full billable-request flow.
    pub async fn handle(&self, mut req: Request, next: Next) -> Response {
        if !is_billable(req.method(), req.uri().path()) {
            return next.run(req).await;
        }

        let Some(meta) = req.extensions().get::<AccessMetadata>().cloned() else {
            // The access layer runs first; missing metadata means auth was
            // skipped or failed in an unexpected way. Fail closed so an
            // unauthenticated request is never billed.
            return HoldRejection::MissingAccessContext.into_response();
        };
        if meta.user_id == 0 {
            return HoldRejection::InvalidUserId.into_response();
        }
        let rate_mult = if meta.rate_mult > 0.0 {
            meta.rate_mult
        } else {
            1.0
        };

        let request_id = trace_id_from(req.headers());
        let ip_address = extract_ip_address(req.headers());
        let idempotency_key = extract_idempotency_key(req.headers());
        let method = req.method().clone();
        let path = req.uri().path().to_owned();

        // --- body peek (body is restored so the handler sees it unchanged) ---
        let (peek, body_bytes) = match peek_request_body(&mut req).await {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        req.extensions_mut().insert(PeekedBody(body_bytes));

        // --- rate limiter (fail-open on infrastructure error) ---
        let identity = meta.user_id.to_string();
        let mut conc_release: Option<String> = None;
        if let Some(rl) = &self.rate_limiter {
            match rl.allow(&identity, 1, &peek.model, meta.group_id).await {
                Ok((false, _)) => return HoldRejection::RateLimited.into_response(),
                Ok((true, release_id)) => conc_release = release_id,
                Err(err) => tracing::warn!(%err, "rate limiter unavailable; failing open"),
            }
        }

        let response = self
            .handle_reserved(
                req,
                next,
                ReservationInput {
                    meta,
                    rate_mult,
                    peek,
                    request_id,
                    ip_address,
                    idempotency_key,
                    method,
                    path,
                },
            )
            .await;

        // Free the concurrency slot on every exit path — including aborts and
        // panicking handlers — or MaxConcurrent degrades into a TTL-length cap.
        if let (Some(rl), Some(release_id)) = (&self.rate_limiter, conc_release) {
            let _ = with_timeout(rl.release_concurrency(&identity, &release_id)).await;
        }
        response
    }

    /// Everything from the idempotency check through settlement.
    async fn handle_reserved(
        &self,
        mut req: Request,
        next: Next,
        input: ReservationInput,
    ) -> Response {
        let ReservationInput {
            meta,
            rate_mult,
            peek,
            request_id,
            ip_address,
            idempotency_key,
            method,
            path,
        } = input;

        // Scoping by user + method + path is what prevents cross-tenant replay
        // (blocker B6): two tenants picking the same Idempotency-Key value must
        // never share a cache entry.
        let idem_store_key = self
            .idempotency
            .as_ref()
            .map(|im| im.scoped_key(meta.user_id, method.as_str(), &path, &idempotency_key))
            .unwrap_or_default();

        // --- idempotency check: replay a completed duplicate, reject an
        // in-flight one. The claim that backs this is taken later, only after a
        // successful Hold, so a request rejected at pre-flight never locks its key.
        if let Some(im) = &self.idempotency
            && !idem_store_key.is_empty()
        {
            match im.check(&idem_store_key).await {
                Err(err) => tracing::warn!(%err, "idempotency check failed; continuing"),
                Ok(Some(cached)) if cached.processing => {
                    return HoldRejection::IdempotencyConflict.into_response();
                }
                Ok(Some(cached)) if cached.truncated => {
                    return HoldRejection::IdempotencyReplayUnavailable.into_response();
                }
                Ok(Some(cached)) => return cached.into_response(),
                Ok(None) => {}
            }
        }

        // --- circuit breaker ---
        if let Some(cb) = &self.circuit_breaker
            && let Some(provider) = infer_provider(&peek.model)
            && matches!(cb.allow(provider).await, Ok(false))
        {
            return HoldRejection::CircuitOpen.into_response();
        }

        // --- outstanding-debt pre-flight ---
        // A tenant carrying an unresolved shortfall must not accumulate more
        // billable work. A lookup error fails closed so a transient DB hiccup
        // cannot let a debtor slip through. No Redis hold is created either way.
        match self.ledger.has_unresolved_shortfall(meta.user_id).await {
            Ok(false) => {}
            Ok(true) => {
                tracing::warn!(
                    event = "outstanding_debt_block",
                    user_id = meta.user_id,
                    path = %path,
                );
                return HoldRejection::OutstandingDebt.into_response();
            }
            Err(err) => {
                tracing::warn!(
                    event = "shortfall_lookup_failed",
                    user_id = meta.user_id,
                    path = %path,
                    %err,
                );
                return HoldRejection::OutstandingDebt.into_response();
            }
        }

        // The reservation scales with the real prompt size so a large request
        // reserves proportional funds instead of under-holding on a flat
        // nominal input assumption.
        let hold_amount = self.calc.estimate_with_tokens(
            &peek.model,
            peek.input_tokens,
            peek.max_tokens,
            peek.stream,
            rate_mult,
        );

        // --- subscription quota ---
        if let (Some(sub), Some(store)) = (&meta.subscription, &self.quota_store) {
            match store.lock_and_rotate(sub.id, Utc::now()).await {
                // A missing subscription row is permissive: the quota system is
                // opt-in and such a user is billed purely from their balance.
                Ok(None) => {}
                Ok(Some(rotated)) => {
                    if let Some(reason) = evaluate_quota(&rotated, hold_amount) {
                        return HoldRejection::QuotaExceeded(reason.to_owned()).into_response();
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, subscription_id = sub.id, "quota check failed");
                    return HoldRejection::QuotaExceeded(
                        "subscription quota check failed".to_owned(),
                    )
                    .into_response();
                }
            }
        }

        // --- upper-bound pre-flight ---
        // Reject before creating a hold when the balance cannot cover even the
        // worst case. The reserved amount stays `hold_amount`; the upper bound
        // only gates the balance comparison.
        let upper_bound = preflight_upper_bound(
            self.calc.as_ref(),
            &peek.model,
            peek.max_tokens,
            peek.stream,
            rate_mult,
            hold_amount,
        );
        // A lookup failure reads as zero available, i.e. it rejects. This is
        // the right posture: letting spend through during a balance-store
        // outage is how a tenant ends up owing money the ledger cannot claw
        // back.
        let available = self
            .ledger
            .available_balance(meta.user_id)
            .await
            .unwrap_or(0.0);
        if available < upper_bound {
            tracing::warn!(
                event = "preflight_insufficient_balance",
                user_id = meta.user_id,
                model = %peek.model,
                upper_bound,
                available,
            );
            return HoldRejection::InsufficientBalance {
                current_balance: available,
                required_amount: upper_bound,
            }
            .into_response();
        }

        // --- budget token, else Redis hold ---
        let used_budget_token = self
            .budget_tokens
            .as_ref()
            .is_some_and(|bts| bts.try_deduct(meta.user_id, hold_amount));

        if !used_budget_token
            && let Err(err) = self
                .ledger
                .hold(meta.user_id, hold_amount, &request_id, self.ttl)
                .await
        {
            return self.reject_hold_error(err, meta.user_id, hold_amount).await;
        }

        let billing: BillingHandle = Arc::new(RequestBilling::new(
            SettleCtx {
                request_id: request_id.clone(),
                user_id: meta.user_id,
                api_key_id: meta.api_key_id,
                group_id: meta.group_id,
                rate_mult,
                subscription_id: meta.subscription.as_ref().map(|s| s.id),
                model: peek.model.clone(),
                stream: peek.stream,
                ip_address,
                idempotency_key,
            },
            hold_amount,
            used_budget_token,
        ));
        req.extensions_mut().insert(billing.clone());

        // --- idempotency claim, now that funds are reserved ---
        let mut idem_owned = false;
        if let Some(im) = &self.idempotency
            && !idem_store_key.is_empty()
        {
            match im.claim(&idem_store_key).await {
                Err(err) => tracing::warn!(%err, "idempotency claim failed; continuing"),
                Ok(true) => idem_owned = true,
                Ok(false) => {
                    // Lost the race: give the reservation back, then either
                    // replay the winner's response or report the conflict.
                    if !used_budget_token {
                        let _ = with_timeout(self.ledger.release(meta.user_id, &request_id)).await;
                    }
                    if let Ok(Some(other)) = im.check(&idem_store_key).await
                        && !other.processing
                        && !other.truncated
                    {
                        return other.into_response();
                    }
                    return HoldRejection::IdempotencyConflict.into_response();
                }
            }
        }

        let response = next.run(req).await;
        let (response, captured) = if idem_owned {
            capture_body(response).await
        } else {
            (response, None)
        };

        let status = response.status();
        self.finalize(&billing, status).await;

        if idem_owned && let Some(im) = &self.idempotency {
            // Pass the pieces by value: `&Response<Body>` is not `Send`, so
            // holding one across an await would make this whole layer's future
            // non-Send and axum could not mount it.
            let content_type = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            self.finalize_idempotency(
                im,
                &idem_store_key,
                &request_id,
                status,
                content_type,
                captured,
            )
            .await;
        }
        response
    }

    /// Terminates billing for a request that the dispatcher did not settle.
    ///
    /// The dispatcher normally claims the settlement itself (it holds the exact
    /// usage record);
    /// this is the safety net for responses that never reached it — a 4xx from
    /// a downstream guard, a panic-turned-500, or a handler that returned
    /// without publishing usage.
    async fn finalize(&self, billing: &BillingHandle, status: StatusCode) {
        if !billing.claim_finalize() {
            return; // the dispatcher already settled or released
        }
        if status.is_success() {
            // Success without a usage record: bill the conservative hold
            // estimate. Never free, never above what the estimate allowed.
            self.settlement.settle_missing_usage(&billing.ctx).await;
            return;
        }
        if !billing.used_budget_token {
            let _ = with_timeout(
                self.ledger
                    .release(billing.ctx.user_id, &billing.ctx.request_id),
            )
            .await;
        }
    }

    /// Stores a replayable 2xx response, or drops the claim so a retry can
    /// proceed.
    async fn finalize_idempotency(
        &self,
        im: &IdempotencyManager,
        store_key: &str,
        request_id: &str,
        status: StatusCode,
        content_type: Option<String>,
        captured: Option<Bytes>,
    ) {
        if !status.is_success() {
            let _ = im.release(store_key).await;
            return;
        }
        let mut cached = CachedResponse {
            status_code: status.as_u16(),
            request_id: request_id.to_owned(),
            ..CachedResponse::default()
        };
        if let Some(ct) = content_type {
            cached.headers.insert("Content-Type".to_owned(), ct);
        }
        match captured {
            // Body too large (or streamed): keep the entry so a retry is not
            // re-billed, but mark it un-replayable.
            None => cached.truncated = true,
            Some(body) => cached.body = body.to_vec(),
        }
        if let Err(err) = im.store(store_key, &cached).await {
            tracing::warn!(%err, "idempotency store failed");
        }
    }

    /// Maps a `Hold` failure onto the structured 402 body.
    async fn reject_hold_error(&self, err: BillingError, user_id: Id, required: f64) -> Response {
        if matches!(err, BillingError::InsufficientBalance) {
            let current_balance = self.ledger.available_balance(user_id).await.unwrap_or(0.0);
            return HoldRejection::InsufficientBalance {
                current_balance,
                required_amount: required,
            }
            .into_response();
        }
        if matches!(err, BillingError::OutstandingDebt) {
            return HoldRejection::OutstandingDebt.into_response();
        }
        tracing::warn!(%err, user_id, "hold failed");
        HoldRejection::PaymentRequired.into_response()
    }
}

/// Everything `handle` resolved before the rate-limiter slot was taken.
struct ReservationInput {
    meta: AccessMetadata,
    rate_mult: f64,
    peek: BodyPeek,
    request_id: String,
    ip_address: String,
    idempotency_key: String,
    method: Method,
    path: String,
}

/// axum entry point. Registered AFTER [`crate::access::layer`] so it observes
/// the access metadata (blocker B1).
pub async fn layer(State(state): State<ProxyState>, req: Request, next: Next) -> Response {
    state.hold.clone().handle(req, next).await
}

// ---------------------------------------------------------------- pure logic

/// Whether a request must go through billing pre-flight.
///
/// The whole proxy surface is — the prefix is the only test applied. The set
/// of prefixes is [`crate::access::is_proxy_path`], shared with the auth
/// layer so a route can never be billed without being authenticated first. The
/// `method` argument is unused for that decision and kept so the seam below
/// stays a one-line change.
///
/// # This charges for two endpoints that cost the upstream nothing
///
/// `GET /v1/models` (and its Gemini twin `GET /v1beta/models`) and
/// `POST /v1/messages/count_tokens` are billed here, and that is a faithful
/// reproduction of a historical behaviour that looks like a product defect.
/// All reach the settlement with no usage envelope:
///
/// * the catalogue reads fall to the default arm of usage parsing;
/// * `/v1/messages/count_tokens` matches its `/messages` arm, but Anthropic's
///   reply is a bare `{"input_tokens": N}` with no `usage` wrapper, so the
///   usage parser finds nothing and reports `present = false`.
///
/// Absent usage plus non-strict mode is the fallback settle, so each call is
/// charged `max(ActiveHoldAmount, Estimate(model, stream = true, rate_mult))`.
/// With the shipped config (`default_price_per_1k_tokens: 0.001`,
/// `estimatedTokens = 1000`) a catalogue read costs the tenant about $0.004;
/// `count_tokens` carries a real model name, so it is priced at that model's
/// rate and can cost considerably more. Neither endpoint bills anything
/// upstream — Anthropic charges nothing for token counting — and the amount
/// tracks a knob meant for *unknown models*, not a tariff anyone chose.
///
/// It is reproduced anyway because billing semantics are a hard constraint
/// (`AGENTS.md`) and a divergence here would surface as drift that nobody
/// could distinguish from a porting bug. Changing it is a product decision,
/// not a porting one.
///
/// **To adopt the fix**, once that decision is made, this becomes:
///
/// ```ignore
/// is_proxy_path(path)
///     && !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
///     && !path.ends_with("/count_tokens")
/// ```
pub fn is_billable(_method: &Method, path: &str) -> bool {
    is_proxy_path(path)
}

/// Conservative worst case used by the balance gate:
/// `max(hold, EstimateWithMaxTokens, Estimate(stream = true))`.
///
/// `EstimateWithMaxTokens` tightens the bound when the client supplied a cap;
/// the streaming estimate guards the case where the cap is absent or absurd.
/// The upper-bound computation for the balance gate.
pub fn preflight_upper_bound(
    calc: &dyn PricingCalculator,
    model: &str,
    max_tokens: i64,
    stream: bool,
    rate_mult: f64,
    hold_amount: f64,
) -> f64 {
    let with_max = calc.estimate_with_max_tokens(model, max_tokens, stream, rate_mult);
    let streaming = calc.estimate(model, true, rate_mult);
    hold_amount.max(with_max).max(streaming)
}

/// Returns the rejection reason when `estimated` would push any period over its
/// limit.
pub fn evaluate_quota(quota: &SubscriptionQuota, estimated: f64) -> Option<&'static str> {
    let periods = [
        (
            quota.daily_limit_usd,
            quota.daily_usage_usd,
            "subscription daily quota exceeded",
        ),
        (
            quota.weekly_limit_usd,
            quota.weekly_usage_usd,
            "subscription weekly quota exceeded",
        ),
        (
            quota.monthly_limit_usd,
            quota.monthly_usage_usd,
            "subscription monthly quota exceeded",
        ),
    ];
    for (limit, used, reason) in periods {
        if let Some(limit) = limit
            && used + estimated > limit
        {
            return Some(reason);
        }
    }
    None
}

/// Zeroes any period counter whose reset boundary has passed and advances that
/// boundary. Returns whether anything changed.
///
/// `pub` so a [`SubscriptionQuotaStore`] implementation can apply the identical
/// rotation inside its `SELECT ... FOR UPDATE` transaction instead of
/// re-deriving the rule in SQL.
pub fn rotate_counters(quota: &mut SubscriptionQuota, now: DateTime<Utc>) -> bool {
    let mut dirty = false;
    if let Some(at) = quota.daily_reset_at
        && now > at
    {
        quota.daily_usage_usd = 0.0;
        quota.daily_reset_at = Some(next_daily_reset_after(now));
        dirty = true;
    }
    if let Some(at) = quota.weekly_reset_at
        && now > at
    {
        quota.weekly_usage_usd = 0.0;
        quota.weekly_reset_at = Some(next_weekly_reset_after(now));
        dirty = true;
    }
    if let Some(at) = quota.monthly_reset_at
        && now > at
    {
        quota.monthly_usage_usd = 0.0;
        quota.monthly_reset_at = Some(next_monthly_reset_after(now));
        dirty = true;
    }
    dirty
}

fn midnight(date: NaiveDate) -> DateTime<Utc> {
    date.and_hms_opt(0, 0, 0)
        .expect("midnight is always a valid time")
        .and_utc()
}

/// Next UTC midnight strictly after `t`.
pub fn next_daily_reset_after(t: DateTime<Utc>) -> DateTime<Utc> {
    midnight(t.date_naive() + chrono::Duration::days(1))
}

/// Next UTC Monday 00:00 strictly after `t` (ISO weeks start on Monday),
/// including the "today is Monday midnight counts as past" rule.
pub fn next_weekly_reset_after(t: DateTime<Utc>) -> DateTime<Utc> {
    let day = t.date_naive();
    let iso = i64::from(day.weekday().number_from_monday()); // Mon=1 .. Sun=7
    midnight(day + chrono::Duration::days(8 - iso))
}

/// First day of the next UTC month at 00:00.
pub fn next_monthly_reset_after(t: DateTime<Utc>) -> DateTime<Utc> {
    let d = t.date_naive();
    let (year, month) = if d.month() == 12 {
        (d.year() + 1, 1)
    } else {
        (d.year(), d.month() + 1)
    };
    midnight(NaiveDate::from_ymd_opt(year, month, 1).expect("first of month is always valid"))
}

/// Maps a model name onto the circuit-breaker key (NOT the dispatch registry —
/// see [`crate::routes::route_provider`] for that).
pub fn infer_provider(model: &str) -> Option<&'static str> {
    let lower = model.to_ascii_lowercase();
    if lower.starts_with("gpt-")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
    {
        Some("openai")
    } else if lower.starts_with("claude-") {
        Some("anthropic")
    } else if lower.starts_with("gemini-") {
        Some("google")
    } else if lower.contains("codex") {
        Some("codex")
    } else {
        None
    }
}

/// Approximates a token count from a byte length (`ceil(size / 4)`).
pub fn approximate_tokens_from_bytes(size: usize) -> i64 {
    if size == 0 {
        return 0;
    }
    size.div_ceil(4) as i64
}

/// Extracts `model` / `stream` / output cap from a JSON payload.
///
/// Permissive failure semantics: the billing layer must never reject a request
/// merely because the payload could not be peeked.
pub fn parse_body_peek(content_type: Option<&str>, body: &[u8]) -> BodyPeek {
    let mut peek = BodyPeek::default();
    if body.is_empty() {
        return peek;
    }
    if let Some(ct) = content_type
        && !ct.is_empty()
        && !ct.to_ascii_lowercase().contains("json")
    {
        return peek;
    }
    peek.input_tokens = approximate_tokens_from_bytes(body.len());

    #[derive(serde::Deserialize)]
    struct Payload {
        #[serde(default)]
        model: String,
        #[serde(default)]
        stream: bool,
        #[serde(default)]
        max_tokens: i64,
        #[serde(default)]
        max_completion_tokens: i64,
    }
    let Ok(payload) = serde_json::from_slice::<Payload>(body) else {
        return peek;
    };

    // max_tokens -> max_completion_tokens -> 0. A non-positive value means
    // "unset", so callers fall back to the default streaming estimate.
    let mut resolved = payload.max_tokens;
    if resolved <= 0 {
        resolved = payload.max_completion_tokens;
    }
    peek.model = payload.model.trim().to_owned();
    peek.stream = payload.stream;
    peek.max_tokens = resolved.max(0);
    peek.parsed = true;
    peek
}

/// Stable request id used as the ledger hold key: an inbound `X-Trace-ID`, else
/// a fresh UUID.
pub fn trace_id_from(headers: &HeaderMap) -> String {
    headers
        .get(TRACE_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

/// Client IP: `X-Forwarded-For` (first entry) -> `X-Real-IP` -> nothing.
///
/// The `RemoteAddr` fallback is the caller's job here because axum surfaces the
/// peer address as a `ConnectInfo` extension rather than on the request itself.
pub fn extract_ip_address(headers: &HeaderMap) -> String {
    if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let first = xff.split(',').next().unwrap_or("").trim();
        if !first.is_empty() {
            return first.to_owned();
        }
    }
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_owned())
        .unwrap_or_default()
}

/// Reads `Idempotency-Key`, then `X-Idempotency-Key`.
pub fn extract_idempotency_key(headers: &HeaderMap) -> String {
    for name in ["idempotency-key", "x-idempotency-key"] {
        if let Some(v) = headers.get(name).and_then(|v| v.to_str().ok()) {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_owned();
            }
        }
    }
    String::new()
}

// ---------------------------------------------------------------- plumbing

/// Buffers the body for peeking and puts it back so the handler sees it
/// unchanged. A body over [`HOLD_REQUEST_BODY_LIMIT`] is rejected with 413
/// rather than silently corrupting the payload forwarded upstream.
async fn peek_request_body(req: &mut Request) -> Result<(BodyPeek, Bytes), Response> {
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let body = std::mem::replace(req.body_mut(), Body::empty());
    let bytes = match axum::body::to_bytes(body, HOLD_REQUEST_BODY_LIMIT).await {
        Ok(b) => b,
        Err(_) => {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                axum::Json(serde_json::json!({
                    "error": "Payload Too Large",
                    "message": "request body exceeds the billing pre-flight limit",
                })),
            )
                .into_response());
        }
    };

    let peek = parse_body_peek(content_type.as_deref(), &bytes);
    *req.body_mut() = Body::from(bytes.clone());
    Ok((peek, bytes))
}

/// Buffers a response body for idempotent replay when it is small enough and
/// not a live stream, returning the response with an equivalent body.
///
/// A response over the cap is still recorded (so the retry is not re-billed)
/// but flagged un-replayable, which is what `None` means here.
async fn capture_body(response: Response) -> (Response, Option<Bytes>) {
    let is_stream = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("event-stream"));
    if is_stream {
        return (response, None);
    }

    let (parts, body) = response.into_parts();
    // Only buffer a body whose size is known to fit. Buffering one that turns
    // out to be larger consumes it and leaves nothing to send the client, so an
    // unknown or oversized length is passed straight through and the entry is
    // recorded un-replayable instead.
    let fits = body
        .size_hint()
        .upper()
        .is_some_and(|upper| upper <= IDEMPOTENCY_BODY_CAPTURE_LIMIT as u64);
    if !fits {
        return (Response::from_parts(parts, body), None);
    }

    match axum::body::to_bytes(body, IDEMPOTENCY_BODY_CAPTURE_LIMIT).await {
        Ok(bytes) => (
            Response::from_parts(parts, Body::from(bytes.clone())),
            Some(bytes),
        ),
        Err(err) => {
            tracing::warn!(%err, "response capture failed; entry kept but unreplayable");
            (Response::from_parts(parts, Body::empty()), None)
        }
    }
}

/// Bounds a detached cleanup operation to [`DETACHED_TIMEOUT`].
async fn with_timeout<F, T>(fut: F) -> Option<T>
where
    F: std::future::Future<Output = T>,
{
    tokio::time::timeout(DETACHED_TIMEOUT, fut).await.ok()
}

#[cfg(test)]
mod tests;
