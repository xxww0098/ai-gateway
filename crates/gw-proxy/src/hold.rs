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
//! 10. **mint the server [`BillingOperationId`]**, then budget token ->
//!     else `ledger.admit_operation`
//! 11. idempotency claim (only now that funds are reserved)
//! 12. run downstream, then settle-or-release exactly once
//!
//! Step 10 is where the money key comes from, and it comes from the *server*.
//! The inbound `X-Trace-ID` never reaches it — see [`client_trace_from`].

use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes, HttpBody as _};
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use gw_ledger::{BillingOperationId, ClientTraceId, IdempotencyScope, NewOperation};
use gw_relay::endpoint::spec::{RequestSpec, SurfaceError, validate};

use crate::ProxyState;
use crate::error::HoldRejection;
use crate::idempotency::{CachedResponse, IdempotencyManager};
use crate::kernel::{self, Phase, RelayCtx};
use crate::ports::{
    AccessMetadata, BillingError, BillingLedger, CircuitBreaker, HoldAdmit, Id, PricingCalculator,
    RateLimiter, SubscriptionQuotaStore,
};
use crate::settlectx::{BillingHandle, RequestBilling, SettleCtx};
use crate::usage::Settlement;

mod preflight;

use preflight::request_fingerprint;
pub use preflight::{
    approximate_tokens_from_bytes, client_trace_from, compute_reservation, evaluate_quota,
    extract_idempotency_key, extract_ip_address, infer_provider, is_billable,
    next_daily_reset_after, next_monthly_reset_after, next_weekly_reset_after,
    preflight_upper_bound, rotate_counters,
};

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

/// 计费侧从 [`RequestSpec`] 派生出来的两个量。
///
/// [`RequestSpec`] 是 `gw-relay` 那一次**全链路唯一**的 body 解析的产物
/// （根除审计缺陷 #15：今天同一个 body 被 `hold.rs` 与 `routes.rs` 各解析一遍，
/// 流式还有第三遍）。这个结构只补上中继层不关心、而计费必需的东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillingPeek {
    /// 已 trim 的模型名。`RequestSpec::model` 按合同保留原样字节；
    /// 目录 / 熔断 / 日志仍用这份，大小写保持 peek 时的样子。
    pub model: String,
    /// [`gw_pricing::normalize_model_key`] 的结果。三次 estimator 共用，
    /// 价目表 lookup 不再对同一请求重复 `trim` + `to_lowercase`。
    pub price_key: String,
    /// 由原始 body 长度近似而来；略微高估，对预扣而言是安全方向。
    pub input_tokens: i64,
    /// 输出上限。`None`（客户端没说）在这里塌缩成 0 = 「无上限」，
    /// 这是 [`PricingCalculator::estimate_with_max_tokens`] 一直以来的约定。
    pub max_tokens: i64,
    pub stream: bool,
}

impl BillingPeek {
    /// 从唯一一次解析的结果派生。**不再解析 body**。
    #[must_use]
    pub fn from_spec(spec: &RequestSpec, body_len: usize) -> Self {
        let model = spec.model().unwrap_or_default().trim().to_owned();
        Self {
            price_key: gw_pricing::normalize_model_key(&model),
            model,
            input_tokens: approximate_tokens_from_bytes(body_len),
            // 负数与缺失同义：都表示「客户端没有给出可用的上限」。
            max_tokens: spec.max_tokens.unwrap_or(0).max(0),
            stream: spec.stream,
        }
    }
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

    /// 只读账本。`GET /v1/usage` 查余额用，不把 ledger 再克隆进 [`crate::ProxyState`]。
    pub fn ledger(&self) -> &Arc<dyn BillingLedger> {
        &self.ledger
    }

    /// 只读用量存储。`GET /v1/usage` 查今日模型 token 用。
    pub fn usage_store(&self) -> &Arc<dyn crate::ports::UsageStore> {
        self.settlement.store()
    }

    /// The full billable-request flow.
    pub async fn handle(&self, mut req: Request, next: Next) -> Response {
        if !is_billable(req.method(), req.uri().path()) {
            kernel::advance_ext(&mut req, Phase::Skipped);
            return next.run(req).await;
        }

        // 内核路径上 AccessMetadata 只住在 RelayCtx 里，不再单独插一份。
        // hold::layer 单测仍会直接挂 AccessMetadata，所以两处都认。
        let Some(meta) = req
            .extensions()
            .get::<crate::kernel::RelayCtx>()
            .map(|ctx| ctx.access.clone())
            .or_else(|| req.extensions().get::<AccessMetadata>().cloned())
        else {
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

        let client_trace = client_trace_from(req.headers());
        let ip_address = extract_ip_address(req.headers());
        let idempotency_key = extract_idempotency_key(req.headers());
        let method = req.method().clone();
        let path = req.uri().path().to_owned();

        // --- 入口校验（`gw-relay` 的三面规范）---
        //
        // `UnknownPath` / `MethodNotAllowed` 一律放行给 axum 自己回 404 / 405：
        // 路由表是 `crate::router` 的，这里不复制第二份。**连带收益**：
        // `/v1/` 下的未注册路径不再先创建一个 Redis hold 再被 404 掉。
        //
        // `UnsupportedMediaType` 则必须在这里 400。今天 `parse_body_peek` 在
        // content-type 不含 `json` 时**静默返回全零 peek**，于是请求带着
        // `model=""` / `stream=false` / `max_tokens=0` 走完整个计费与派发链
        // （`docs/relay-surface-plan.md` §3.0）。
        let surface = match validate(&method, &path, req.headers()) {
            Ok(surface) => surface,
            Err(SurfaceError::UnknownPath | SurfaceError::MethodNotAllowed) => {
                return next.run(req).await;
            }
            Err(err @ SurfaceError::UnsupportedMediaType) => {
                return (
                    err.status(),
                    axum::Json(serde_json::json!({
                        "error": "Bad Request",
                        "message": "content-type must be application/json",
                    })),
                )
                    .into_response();
            }
        };

        // --- body peek (body is restored so the handler sees it unchanged) ---
        let (spec, body_bytes) = match peek_request_body(&mut req, surface).await {
            Ok(v) => v,
            Err(resp) => return resp,
        };
        let peek = BillingPeek::from_spec(&spec, body_bytes.len());
        req.extensions_mut().insert(PeekedBody(body_bytes));
        // 唯一一次解析的结果原样交给 handler，`routes::inbound` 直接复用。
        req.extensions_mut().insert(spec);
        kernel::advance_ext(&mut req, Phase::Inspected);

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
                    client_trace,
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
            client_trace,
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

        // --- outstanding-debt（必须在任何 Redis hold 之前）---
        // 拒绝优先级不变：欠款 402 先于余额 402。查失败仍然 fail-closed。
        // 余额 peek 不再单独打一趟 GET-balance：无预算代币时由
        // `hold_gated` 在同一条 Lua 里做 floor 检查 + 预扣 + EXPIRE。
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
        let (hold_amount, upper_bound) = compute_reservation(&peek, rate_mult, self.calc.as_ref());

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

        // --- mint the money key ---
        //
        // **The server mints it.** Not the inbound `X-Trace-ID`, not the
        // client `Idempotency-Key`, not a hash of either. A caller can replay
        // or collide a header it controls; it cannot reach this.
        let operation_id = BillingOperationId::mint();

        // --- upper-bound pre-flight ---
        //
        // Reject before reserving when the balance cannot cover the worst
        // case. **Prepaid reserves the bound it checked**: `reserved_amount`
        // is `upper_bound`, not the smaller `hold_amount` — reserving less
        // than the liability that was admitted is exactly the under-hold that
        // lets a large request settle into debt.
        let operation = NewOperation {
            operation_id: operation_id.clone(),
            user_id: meta.user_id,
            reserved_amount: upper_bound,
            admitted_liability: upper_bound,
            request_fingerprint: request_fingerprint(meta.user_id, &method, &path, &peek),
            client_trace_id: client_trace.as_str().to_owned(),
        };

        // 预算代币：仅 try_deduct 命中时跳过 Redis 预留；无 token / 不足 /
        // 过期走 admit_operation 的单趟 Lua（floor + 预扣 + EXPIRE）。
        // **持久化那一行两条路都写** —— 它是操作的身份，不是它的缓存。
        let used_budget_token = self
            .budget_tokens
            .as_ref()
            .is_some_and(|bts| bts.try_deduct(meta.user_id, upper_bound));
        let redis_ttl = (!used_budget_token).then_some(self.ttl);

        match self.ledger.admit_operation(&operation, redis_ttl).await {
            Ok(HoldAdmit::Reserved) => {}
            Ok(HoldAdmit::Insufficient { available }) => {
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
            Err(err) => {
                return self.reject_hold_error(err, meta.user_id, upper_bound).await;
            }
        }

        kernel::advance_ext(&mut req, Phase::Gated);

        let billing: BillingHandle = Arc::new(RequestBilling::new(
            SettleCtx {
                operation: operation_id.clone(),
                client_trace: client_trace.clone(),
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
            used_budget_token,
        ));
        req.extensions_mut().insert(billing.clone());
        kernel::advance_ext(&mut req, Phase::Reserved);
        if let Some(ctx) = req.extensions_mut().get_mut::<RelayCtx>() {
            ctx.peek = Some(peek);
            ctx.client_trace = billing.ctx.client_trace.clone();
            ctx.operation = Some(billing.ctx.operation.clone());
            ctx.ip_address.clone_from(&billing.ctx.ip_address);
            ctx.idempotency_key.clone_from(&billing.ctx.idempotency_key);
            ctx.billing = Some(billing.clone());
        }

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
                    let _ =
                        with_timeout(self.ledger.release_once(meta.user_id, &operation_id)).await;
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
                client_trace.as_str(),
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
        // The durable operation is terminated on **both** reservation paths:
        // a budget-token request has no Redis hold to give back, but it still
        // owns a `held` row that reconciliation would otherwise charge.
        let _ = with_timeout(
            self.ledger
                .release_once(billing.ctx.user_id, &billing.ctx.operation),
        )
        .await;
    }

    /// Stores a replayable 2xx response, or drops the claim so a retry can
    /// proceed.
    async fn finalize_idempotency(
        &self,
        im: &IdempotencyManager,
        store_key: &IdempotencyScope,
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
        if let BillingError::OperationConflict(conflict) = &err {
            // A minted id collided with a live or finished operation. That is
            // a gateway bug, not a tenant error — refuse rather than reuse the
            // row, and say so loudly enough to be found.
            tracing::error!(%conflict, user_id, "billing operation id conflict");
            return HoldRejection::PaymentRequired.into_response();
        }
        tracing::warn!(%err, user_id, "hold failed");
        HoldRejection::PaymentRequired.into_response()
    }
}

/// Everything `handle` resolved before the rate-limiter slot was taken.
struct ReservationInput {
    meta: AccessMetadata,
    rate_mult: f64,
    peek: BillingPeek,
    /// Observability only. The money key is minted later, in `handle_reserved`.
    client_trace: ClientTraceId,
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

// ---------------------------------------------------------------- plumbing

/// 把入站 body 收成一块 [`Bytes`] 供 peek 与转发共用。
///
/// 超 [`HOLD_REQUEST_BODY_LIMIT`] 回 413。**不再写回 `Request`**：
/// handler 走 [`PeekedBody`]，再 `Body::from(bytes.clone())` 只是多挂一个
/// `Full`，火焰图上 `peek_request_body` 的一部分就是这个。
async fn peek_request_body(
    req: &mut Request,
    surface: gw_relay::Surface,
) -> Result<(RequestSpec, Bytes), Response> {
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

    // **全链路唯一一次** JSON 解析（根除缺陷 #15）。结果作为扩展下发，
    // `routes::inbound` 直接复用，不再解第二遍。
    let spec = RequestSpec::parse(surface, Some(&bytes));
    // 规则 S3：`Accept` 与 body 的 `stream` 冲突时**以 body 为准**，只告警不改行为
    // （告警由被调用方自己打，返回值这里不需要）。
    let _conflicted =
        gw_relay::endpoint::accept_conflicts_with_body(&spec, req.headers(), req.uri().path());
    Ok((spec, bytes))
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
