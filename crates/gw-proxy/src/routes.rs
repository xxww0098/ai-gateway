//! The `/v1/*` and `/v1beta/*` endpoints and upstream dispatch.
//!
//! Owns the gateway's `/v1` handlers. The reference for *behaviour* is the
//! billing pipeline these routes sit inside: pre-flight hold, dispatch, settle.
//!
//! Dispatch resolves a provider from the model name (falling back to the
//! endpoint family), picks an upstream account through [`crate::channel`], and
//! retries on a *different* account when one fails. Settlement happens **once**,
//! after the final attempt — cross-auth retry settles once, on the final
//! response, so failover cannot double-bill.

use std::sync::Arc;
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use gw_authcore::{AuthRecord, AuthStore};
use gw_provider::types::{
    Provider, ProviderError, ProviderRequest, ProviderResponse, StreamChunk, StreamResponse,
    UsageRecord,
};
use tokio_util::task::TaskTracker;

use crate::ProxyState;
use crate::channel::ChannelPool;
use crate::error::DispatchError;
use crate::hold::PeekedBody;
use crate::ports::{CircuitBreaker, ModelCatalog};
use crate::settlectx::{BillingHandle, SettleCtx};
use crate::usage::{Settlement, UsageOutcome};

/// How many upstream accounts one client request may burn through.
/// Bounded so a fully-broken pool fails fast instead of walking every account.
pub const MAX_UPSTREAM_ATTEMPTS: usize = 3;

/// Which client-facing dialect an endpoint speaks. Selects the provider when
/// the model name alone is ambiguous, and picks the usage envelope shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiFamily {
    /// `/v1/chat/completions`, `/v1/completions`, `/v1/responses`
    OpenAi,
    /// `/v1/embeddings`
    Embeddings,
    /// `/v1/messages`
    Claude,
    /// `/v1beta/models/{model}:generateContent`
    Gemini,
}

impl ApiFamily {
    /// Provider to use when the model name says nothing useful.
    fn default_provider(self) -> Option<&'static str> {
        match self {
            Self::OpenAi | Self::Embeddings => Some("openai"),
            Self::Claude => Some("claude"),
            Self::Gemini => Some("gemini"),
        }
    }
}

/// Providers to try, most specific first.
///
/// The model name wins when it is recognisable, because a caller may reach any
/// dialect endpoint with any model; the endpoint family is the tiebreak. Gemini
/// models list `vertex` as an alternate since the same model id is served by
/// both, and only the configured credential pool can tell them apart.
pub fn provider_candidates(model: &str, family: ApiFamily) -> Vec<&'static str> {
    let lower = model.to_ascii_lowercase();
    let mut candidates: Vec<&'static str> = if lower.contains("codex") {
        vec!["codex", "openai"]
    } else if lower.starts_with("claude-") {
        vec!["claude"]
    } else if lower.starts_with("gemini-") {
        vec!["gemini", "vertex"]
    } else if lower.starts_with("gpt-")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
        || lower.starts_with("text-embedding")
    {
        vec!["openai"]
    } else {
        Vec::new()
    };
    if let Some(default) = family.default_provider()
        && !candidates.contains(&default)
    {
        candidates.push(default);
    }
    candidates
}

/// Upstream execution: account selection, retry, and the single settlement.
///
/// Collectively implements the auth manager + the conductor's retry
/// loop.
pub struct Dispatcher {
    providers: Vec<Arc<dyn Provider>>,
    auth_store: Arc<dyn AuthStore>,
    channels: Arc<ChannelPool>,
    settlement: Arc<Settlement>,
    circuit_breaker: Option<Arc<dyn CircuitBreaker>>,
    catalog: Option<Arc<dyn ModelCatalog>>,
}

impl Dispatcher {
    /// Wires the upstream side of the pipeline.
    ///
    /// The manager plus the executor registration loop.
    pub fn new(
        providers: Vec<Arc<dyn Provider>>,
        auth_store: Arc<dyn AuthStore>,
        channels: Arc<ChannelPool>,
        settlement: Arc<Settlement>,
    ) -> Self {
        Self {
            providers,
            auth_store,
            channels,
            settlement,
            circuit_breaker: None,
            catalog: None,
        }
    }

    /// Hands the same breaker to hold and dispatch.
    #[must_use]
    pub fn with_circuit_breaker(mut self, cb: Arc<dyn CircuitBreaker>) -> Self {
        self.circuit_breaker = Some(cb);
        self
    }

    /// Source for `GET /v1/models`.
    #[must_use]
    pub fn with_catalog(mut self, catalog: Arc<dyn ModelCatalog>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// The account pool, for the metrics exporter.
    pub fn channels(&self) -> &Arc<ChannelPool> {
        &self.channels
    }

    fn provider(&self, name: &str) -> Option<&Arc<dyn Provider>> {
        self.providers.iter().find(|p| p.name() == name)
    }

    /// Live credentials for one provider, newest state first.
    async fn auths_for(&self, provider: &str) -> Vec<AuthRecord> {
        match self.auth_store.list().await {
            Ok(records) => records
                .into_iter()
                .filter(|r| r.provider == provider && r.is_usable())
                .collect(),
            Err(err) => {
                tracing::warn!(%err, provider, "listing upstream credentials failed");
                Vec::new()
            }
        }
    }
}

/// Runs one client request against the upstream pool and settles it once.
///
/// `billing` is `None` for the endpoints [`crate::hold::is_billable`] excludes
/// (token counting, catalogue reads); those never reserve, never settle.
async fn dispatch(
    state: &ProxyState,
    family: ApiFamily,
    model: String,
    inbound: Inbound,
    stream: bool,
    billing: Option<BillingHandle>,
) -> Response {
    let dispatcher = &state.dispatch;
    let candidates = provider_candidates(&model, family);
    if candidates.is_empty() {
        return finish_error(
            state,
            billing.as_ref(),
            DispatchError::UnknownModel(model.clone()),
        )
        .await;
    }

    let mut tried: Vec<String> = Vec::new();
    let mut last_error: Option<DispatchError> = None;

    for provider_name in candidates {
        let Some(provider) = dispatcher.provider(provider_name) else {
            continue;
        };
        let auths = dispatcher.auths_for(provider_name).await;
        if auths.is_empty() {
            last_error.get_or_insert_with(|| DispatchError::NoUpstream(provider_name.to_owned()));
            continue;
        }

        while tried.len() < MAX_UPSTREAM_ATTEMPTS {
            let Some(auth) = dispatcher.channels.pick_excluding(&auths, &tried) else {
                break; // this provider's accounts are exhausted for this request
            };
            let auth = auth.clone();
            tried.push(auth.id.clone());

            let request = ProviderRequest {
                model: model.clone(),
                payload: inbound.body.to_vec(),
                stream,
                metadata: request_metadata(billing.as_ref()),
                headers: inbound.headers.clone(),
                query: inbound.query.clone(),
            };
            let started = Instant::now();

            if stream {
                match provider.execute_stream(&auth, request).await {
                    Ok(upstream) => {
                        // The connection stood up, so the account is working;
                        // a mid-stream failure is reported through the usage
                        // outcome instead.
                        record_success(state, provider_name, &auth.id).await;
                        return stream_response(
                            state,
                            upstream,
                            billing,
                            auth.id,
                            provider_name,
                            started,
                        );
                    }
                    Err(err) => {
                        record_failure(state, provider_name, &auth.id).await;
                        if !is_retryable(&err) {
                            return finish_error(state, billing.as_ref(), map_error(err)).await;
                        }
                        last_error = Some(map_error(err));
                    }
                }
            } else {
                match provider.execute(&auth, request).await {
                    // An error status the provider relayed rather than raised
                    // is still that account failing: fail over to the next one
                    // instead of handing the client a 503 another credential
                    // would have served.
                    Ok(response) if is_retryable_status(response.status) => {
                        record_failure(state, provider_name, &auth.id).await;
                        last_error = Some(DispatchError::Upstream {
                            status: StatusCode::from_u16(response.status)
                                .unwrap_or(StatusCode::BAD_GATEWAY),
                            body: String::from_utf8_lossy(&response.body).into_owned(),
                        });
                    }
                    Ok(response) => {
                        record_success(state, provider_name, &auth.id).await;
                        return unary_response(
                            state,
                            response,
                            billing,
                            auth.id,
                            provider_name,
                            started,
                        )
                        .await;
                    }
                    Err(err) => {
                        record_failure(state, provider_name, &auth.id).await;
                        if !is_retryable(&err) {
                            return finish_error(state, billing.as_ref(), map_error(err)).await;
                        }
                        last_error = Some(map_error(err));
                    }
                }
            }
        }
    }

    let err = last_error.unwrap_or_else(|| DispatchError::NoUpstream(model));
    finish_error(state, billing.as_ref(), err).await
}

/// Whether a failure is worth trying on a different account.
///
/// A 4xx other than 429 is the caller's fault and will fail identically
/// everywhere, so it is surfaced immediately instead of burning the pool.
fn is_retryable(err: &ProviderError) -> bool {
    match err {
        ProviderError::Upstream { status, .. } => is_retryable_status(*status),
        ProviderError::Credential(_) | ProviderError::Transport(_) => true,
        ProviderError::Other(_) => false,
    }
}

/// Statuses that say "this account, right now" rather than "this request".
fn is_retryable_status(status: u16) -> bool {
    status >= 500 || status == 429
}

fn map_error(err: ProviderError) -> DispatchError {
    match err {
        ProviderError::Upstream { status, body } => DispatchError::Upstream {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
            body,
        },
        other => DispatchError::Internal(anyhow::Error::new(other)),
    }
}

/// Metadata handed to the provider, mirroring what the SDK put on its request
/// options so an executor can log or shard by tenant.
fn request_metadata(billing: Option<&BillingHandle>) -> std::collections::HashMap<String, String> {
    let mut meta = std::collections::HashMap::new();
    if let Some(b) = billing {
        meta.insert("request_id".to_owned(), b.ctx.request_id.clone());
        meta.insert("user_id".to_owned(), b.ctx.user_id.to_string());
    }
    meta
}

async fn record_success(state: &ProxyState, provider: &str, auth_id: &str) {
    state
        .dispatch
        .channels
        .health()
        .record_result(auth_id, true, None);
    if let Some(cb) = &state.dispatch.circuit_breaker {
        cb.record(provider, true).await;
    }
}

async fn record_failure(state: &ProxyState, provider: &str, auth_id: &str) {
    state
        .dispatch
        .channels
        .health()
        .record_result(auth_id, false, None);
    if let Some(cb) = &state.dispatch.circuit_breaker {
        cb.record(provider, false).await;
    }
}

/// Settles a completed non-streaming request and relays the upstream response.
async fn unary_response(
    state: &ProxyState,
    response: ProviderResponse,
    billing: Option<BillingHandle>,
    auth_id: String,
    provider: &str,
    started: Instant,
) -> Response {
    let status = StatusCode::from_u16(response.status).unwrap_or(StatusCode::OK);
    if let Some(billing) = billing
        && billing.claim_finalize()
    {
        let outcome = UsageOutcome {
            usage: response.usage.clone(),
            failed: !status.is_success(),
            auth_id,
            provider: provider.to_owned(),
            duration_ms: started.elapsed().as_millis() as i64,
        };
        state
            .dispatch
            .settlement
            .settle(&billing.ctx, outcome)
            .await;
    }
    relay(status, response.headers, response.body)
}

/// Relays a streamed response, settling when the stream ends.
///
/// Settlement is claimed up-front so the hold middleware's finalizer stands
/// down; the actual charge is applied by [`StreamSettler`] once the last chunk
/// has been forwarded, or by its `Drop` if the client hangs up mid-stream.
fn stream_response(
    state: &ProxyState,
    upstream: StreamResponse,
    billing: Option<BillingHandle>,
    auth_id: String,
    provider: &str,
    started: Instant,
) -> Response {
    let StreamResponse {
        status,
        headers,
        chunks,
    } = upstream;
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    let settler = billing.and_then(|b| {
        b.claim_finalize().then(|| StreamSettler {
            settlement: state.dispatch.settlement.clone(),
            drain: state.drain.clone(),
            ctx: b.ctx.clone(),
            usage: None,
            failed: false,
            auth_id,
            provider: provider.to_owned(),
            started,
            done: false,
        })
    });

    let body = Body::from_stream(futures_util::stream::unfold(
        Some((chunks, settler)),
        |carrier| async move {
            let (mut chunks, mut settler) = carrier?;
            loop {
                match chunks.next().await {
                    Some(StreamChunk::Payload(bytes)) => {
                        return Some((
                            Ok::<Bytes, std::convert::Infallible>(bytes),
                            Some((chunks, settler)),
                        ));
                    }
                    Some(StreamChunk::Usage(record)) => {
                        if let Some(s) = settler.as_mut() {
                            s.usage = Some(record);
                        }
                    }
                    Some(StreamChunk::Error { status, message }) => {
                        tracing::warn!(?status, %message, "upstream stream error");
                        if let Some(s) = settler.as_mut() {
                            s.failed = true;
                        }
                    }
                    None => {
                        if let Some(s) = settler.as_mut() {
                            s.finish().await;
                        }
                        return None;
                    }
                }
            }
        },
    ));

    // Relay the upstream's own status and headers. The provider carries them
    // explicitly (`StreamResponse`) precisely so a relay does not have to
    // hardcode 200 and guess the content type.
    let mut response = Response::new(body);
    *response.status_mut() = status;
    for (name, value) in headers.iter() {
        if is_hop_by_hop(name.as_str()) {
            continue;
        }
        response.headers_mut().insert(name, value.clone());
    }
    let response_headers = response.headers_mut();
    if !response_headers.contains_key(header::CONTENT_TYPE) {
        response_headers.insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("text/event-stream"),
        );
    }
    if !response_headers.contains_key(header::CACHE_CONTROL) {
        response_headers.insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-cache"),
        );
    }
    response
}

/// Carries the settlement obligation for a streamed response.
struct StreamSettler {
    settlement: Arc<Settlement>,
    /// The composition root's tracker, so a settlement spawned from `drop`
    /// survives shutdown. Cloned from [`ProxyState::drain`] — never created
    /// here, or the drain would wait on a tracker nobody else can see.
    drain: TaskTracker,
    ctx: SettleCtx,
    usage: Option<UsageRecord>,
    failed: bool,
    auth_id: String,
    provider: String,
    started: Instant,
    done: bool,
}

impl StreamSettler {
    fn outcome(&self) -> UsageOutcome {
        UsageOutcome {
            usage: self.usage.clone(),
            failed: self.failed,
            auth_id: self.auth_id.clone(),
            provider: self.provider.clone(),
            duration_ms: self.started.elapsed().as_millis() as i64,
        }
    }

    async fn finish(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        self.settlement.settle(&self.ctx, self.outcome()).await;
    }
}

impl Drop for StreamSettler {
    /// A client that hangs up mid-stream drops the body without the stream ever
    /// ending, so settle from a detached task rather than leaving the hold to
    /// its TTL. The detached finalizer runs regardless of the client.
    ///
    /// The task goes to [`ProxyState::drain`], **not** to `tokio::spawn`. A
    /// bare spawn is aborted the instant the runtime is dropped, which for a
    /// disconnect that lands during shutdown means the charge is lost and the
    /// hold leaks until its TTL — free upstream output, and a breach of the
    /// billing invariants in `AGENTS.md`.
    ///
    /// No "is the tracker closed?" check is needed or wanted:
    /// `TaskTracker::close` does not block later spawns, so a `drop` that lands
    /// mid-drain is still tracked and still waited on.
    fn drop(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        let settlement = self.settlement.clone();
        let ctx = self.ctx.clone();
        let outcome = self.outcome();
        // `TaskTracker::spawn` panics outside a runtime, exactly like
        // `tokio::spawn`; a body can only be dropped on one, but tests may
        // construct a settler off-runtime.
        if tokio::runtime::Handle::try_current().is_ok() {
            self.drain
                .spawn(async move { settlement.settle(&ctx, outcome).await });
        }
    }
}

/// Terminates billing for a request that never reached an upstream.
async fn finish_error(
    state: &ProxyState,
    billing: Option<&BillingHandle>,
    err: DispatchError,
) -> Response {
    if let Some(billing) = billing
        && billing.claim_finalize()
    {
        state
            .dispatch
            .settlement
            .settle(&billing.ctx, UsageOutcome::failed())
            .await;
    }
    err.into_response()
}

/// Copies an upstream response through, minus hop-by-hop headers.
fn relay(status: StatusCode, headers: HeaderMap, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    for (name, value) in headers.iter() {
        if is_hop_by_hop(name.as_str()) {
            continue;
        }
        response.headers_mut().insert(name, value.clone());
    }
    if !response.headers().contains_key(header::CONTENT_TYPE) {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
    }
    response
}

/// Headers that describe one hop and must not be forwarded.
fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
    )
}

// ---------------------------------------------------------------- handlers

/// Body + model resolved from an inbound request.
struct Inbound {
    model: String,
    stream: bool,
    body: Bytes,
    /// Inbound headers, forwarded upstream through
    /// `gw_provider::types::copy_outbound_headers` by each provider.
    headers: HeaderMap,
    /// Inbound query pairs, appended to the provider endpoint. Order and
    /// duplicates are both significant, hence a `Vec`.
    query: Vec<(String, String)>,
    billing: Option<BillingHandle>,
}

/// Reuses the body the hold layer already buffered, or reads it directly when
/// no pre-flight ran (the layer skips everything outside `/v1/`).
async fn inbound(req: Request) -> Result<Inbound, Response> {
    let billing = req.extensions().get::<BillingHandle>().cloned();
    let peeked = req.extensions().get::<PeekedBody>().cloned();
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let path = req.uri().path().to_owned();
    let mut headers = req.headers().clone();
    let mut query = parse_query(req.uri().query().unwrap_or_default());
    // The tenant credential stops here. On the Gemini surface it rides on
    // headers and a query parameter that would otherwise be forwarded straight
    // to Google.
    crate::access::strip_consumed_credentials(&path, &mut headers, &mut query);

    let body = match peeked {
        Some(PeekedBody(bytes)) => bytes,
        None => match axum::body::to_bytes(req.into_body(), crate::hold::HOLD_REQUEST_BODY_LIMIT)
            .await
        {
            Ok(bytes) => bytes,
            Err(_) => return Err(StatusCode::PAYLOAD_TOO_LARGE.into_response()),
        },
    };

    let peek = crate::hold::parse_body_peek(content_type.as_deref(), &body);
    Ok(Inbound {
        model: billing
            .as_ref()
            .map(|b| b.ctx.model.clone())
            .filter(|m| !m.is_empty())
            .unwrap_or(peek.model),
        stream: peek.stream,
        body,
        headers,
        query,
        billing,
    })
}

/// Splits a raw query string into ordered `(key, value)` pairs, percent-decoding
/// nothing: the providers re-encode when they build the outbound URL.
fn parse_query(raw: &str) -> Vec<(String, String)> {
    raw.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (k.to_owned(), v.to_owned()),
            None => (pair.to_owned(), String::new()),
        })
        .collect()
}

macro_rules! endpoint {
    ($(#[$meta:meta])* $name:ident, $family:expr) => {
        $(#[$meta])*
        pub async fn $name(State(state): State<ProxyState>, req: Request) -> Response {
            match inbound(req).await {
                Ok(i) => {
                    let (model, stream, billing) = (i.model.clone(), i.stream, i.billing.clone());
                    dispatch(&state, $family, model, i, stream, billing).await
                }
                Err(response) => response,
            }
        }
    };
}

endpoint!(
    /// `POST /v1/chat/completions` — the OpenAI chat dialect.
    chat_completions,
    ApiFamily::OpenAi
);
endpoint!(
    /// `POST /v1/completions` — the OpenAI legacy completion dialect.
    completions,
    ApiFamily::OpenAi
);
endpoint!(
    /// `POST /v1/responses` — the OpenAI Responses dialect.
    responses,
    ApiFamily::OpenAi
);
endpoint!(
    /// `POST /v1/embeddings`.
    embeddings,
    ApiFamily::Embeddings
);
endpoint!(
    /// `POST /v1/messages` — the Anthropic Messages dialect.
    messages,
    ApiFamily::Claude
);

/// `POST /v1beta/models/{model_action}` (and its `/v1` alias) — the Gemini
/// `{model}:generateContent` / `:streamGenerateContent` dialect.
///
/// The action suffix, not the JSON body, decides whether this streams: a Gemini
/// request body carries neither `model` nor `stream`, so the URL is the only
/// source for both. That also means the pre-flight estimate this request was
/// held against was computed from an empty model name and `stream = false` —
/// the pre-flight peeks the body and nothing else — and the *settlement* uses
/// the model and token counts the upstream actually reported, so only the
/// reservation size is affected.
///
/// Any action other than the two above is dispatched as `generateContent`,
/// which is what the pre-existing `/v1` route did and what the Gemini provider
/// can build an endpoint for. `:countTokens` and `:embedContent` are therefore
/// *not* served correctly — see the crate-level note.
pub async fn gemini_generate(
    State(state): State<ProxyState>,
    Path(model_action): Path<String>,
    req: Request,
) -> Response {
    let (model, action) = split_model_action(&model_action);
    let stream = action.starts_with("stream");
    match inbound(req).await {
        Ok(i) => {
            let model = if model.is_empty() {
                i.model.clone()
            } else {
                model
            };
            let billing = i.billing.clone();
            dispatch(&state, ApiFamily::Gemini, model, i, stream, billing).await
        }
        Err(response) => response,
    }
}

/// Splits `gemini-2.5-pro:streamGenerateContent` into model and action.
pub fn split_model_action(raw: &str) -> (String, String) {
    match raw.split_once(':') {
        Some((model, action)) => (model.to_owned(), action.to_owned()),
        None => (raw.to_owned(), String::new()),
    }
}

/// `POST /v1/messages/count_tokens` — Anthropic token counting.
///
/// This handler publishes no usage record, so the hold middleware's finalizer
/// settles the fallback estimate, charging the tenant for a call Anthropic
/// itself bills at zero. Deliberate parity; see
/// [`crate::hold::is_billable`] for the mechanism and the one-line seam.
pub async fn count_tokens(State(state): State<ProxyState>, req: Request) -> Response {
    let inbound = match inbound(req).await {
        Ok(i) => i,
        Err(response) => return response,
    };
    for name in provider_candidates(&inbound.model, ApiFamily::Claude) {
        let Some(provider) = state.dispatch.provider(name) else {
            continue;
        };
        let auths = state.dispatch.auths_for(name).await;
        let Some(auth) = state.dispatch.channels.pick(&auths) else {
            continue;
        };
        let request = ProviderRequest {
            model: inbound.model.clone(),
            payload: inbound.body.to_vec(),
            stream: false,
            metadata: std::collections::HashMap::new(),
            headers: inbound.headers.clone(),
            query: inbound.query.clone(),
        };
        return match provider.count_tokens(auth, request).await {
            Ok(count) => axum::Json(serde_json::json!({ "input_tokens": count })).into_response(),
            Err(err) => map_error(err).into_response(),
        };
    }
    DispatchError::NoUpstream(inbound.model).into_response()
}

/// `GET /v1/models` — the OpenAI-shaped catalogue.
///
/// Reserves and settles like every other `/v1/` path, because the middleware
/// keys on the prefix alone and the catalogue reply carries no usage envelope.
/// A read that costs the upstream nothing therefore costs the tenant the
/// fallback estimate. Deliberate parity; see [`crate::hold::is_billable`].
pub async fn models(State(state): State<ProxyState>) -> Response {
    let Some(catalog) = &state.dispatch.catalog else {
        return axum::Json(serde_json::json!({ "object": "list", "data": [] })).into_response();
    };
    match catalog.list_models().await {
        Ok(models) => axum::Json(serde_json::json!({
            "object": "list",
            "data": models.iter().map(|m| serde_json::json!({
                "id": m.id,
                "object": "model",
                "created": m.created,
                "owned_by": m.owned_by,
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(err) => {
            tracing::warn!(%err, "model catalog lookup failed");
            DispatchError::Internal(err).into_response()
        }
    }
}

/// `GET /v1/models/{model}` — one catalogue entry.
pub async fn model_detail(State(state): State<ProxyState>, Path(model): Path<String>) -> Response {
    let Some(catalog) = &state.dispatch.catalog else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match catalog.list_models().await {
        Ok(models) => match models.iter().find(|m| m.id == model) {
            Some(m) => axum::Json(serde_json::json!({
                "id": m.id,
                "object": "model",
                "created": m.created,
                "owned_by": m.owned_by,
            }))
            .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
        Err(err) => DispatchError::Internal(err).into_response(),
    }
}

// ------------------------------------------------- gemini native catalogue

/// The two actions [`gemini_generate`] can dispatch, which is what the
/// catalogue may honestly claim to support.
const GEMINI_GENERATION_METHODS: [&str; 2] = ["generateContent", "streamGenerateContent"];

/// Renders a catalogue entry as a Google `Model` resource.
///
/// Google keys a model by its *resource name* — `models/<id>`, not `<id>` — and
/// every client strips that prefix back off (the frontend's own
/// `stripGeminiModelResourceName` does exactly this against upstream Google).
/// Fields the catalogue cannot know (`description`, `inputTokenLimit`,
/// `temperature`, ...) are omitted rather than invented: they are optional in
/// Google's schema, and a fabricated token limit would be worse than an absent
/// one for any client that trusts it.
fn gemini_model_json(entry: &crate::ports::ModelEntry) -> serde_json::Value {
    serde_json::json!({
        "name": format!("models/{}", entry.id),
        "baseModelId": entry.id,
        "displayName": entry.id,
        "supportedGenerationMethods": GEMINI_GENERATION_METHODS,
    })
}

/// `GET /v1beta/models` — the Gemini-shaped catalogue.
///
/// The same catalogue [`models`] serves, in Google's envelope (`{"models": …}`)
/// rather than OpenAI's (`{"object": "list", "data": …}`). Reserves and settles
/// like every other metered path, for the reason spelled out on
/// [`crate::hold::is_billable`].
pub async fn gemini_models(State(state): State<ProxyState>) -> Response {
    let Some(catalog) = &state.dispatch.catalog else {
        return axum::Json(serde_json::json!({ "models": [] })).into_response();
    };
    match catalog.list_models().await {
        Ok(models) => axum::Json(serde_json::json!({
            "models": models.iter().map(gemini_model_json).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(err) => {
            tracing::warn!(%err, "model catalog lookup failed");
            DispatchError::Internal(err).into_response()
        }
    }
}

/// `GET /v1beta/models/{model}` — one Gemini-shaped catalogue entry.
///
/// Google accepts the bare id in the path even though it reports the resource
/// name in the body, so the lookup is by id — the same key [`model_detail`]
/// uses.
pub async fn gemini_model_detail(
    State(state): State<ProxyState>,
    Path(model): Path<String>,
) -> Response {
    let Some(catalog) = &state.dispatch.catalog else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match catalog.list_models().await {
        Ok(models) => match models.iter().find(|m| m.id == model) {
            Some(m) => axum::Json(gemini_model_json(m)).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
        Err(err) => DispatchError::Internal(err).into_response(),
    }
}

#[cfg(test)]
mod tests;
