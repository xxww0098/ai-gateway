//! Streamed relay + the shared off-path settler used by unary too.
//!
//! Split out of `routes.rs` so the dispatch file stays under the 1,000-line
//! ratchet. Stream payload bytes go to the client; usage / error chunks stay
//! on [`StreamSettler`]. A client hang-up still settles through
//! [`ProxyState::drain`].
//!
//! Unary reuses the same settler: the HTTP response is built as soon as the
//! upstream body and usage (or the fallback / strict / release decision
//! inputs) are in hand, and the ledger write is spawned onto
//! [`ProxyState::drain`] via [`StreamSettler`]'s `Drop`. That is the same
//! path a hung-up stream already uses — not a second queue.
//!
//! `claim_finalize` 四入口（每请求恰一次结算权）：
//! | 入口 | 语义 |
//! | [`stream_response`] | 流式：settler 持票，中间件 finalize 让位 |
//! | [`unary_response`] | 一元：同上，Drop 时结算 |
//! | [`schedule_release`] | 上游未出 body：Release |
//! | [`crate::hold::HoldMiddleware::finalize`] | 兜底：handler 未结算时 settle_missing 或 Release |

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use futures_util::Stream;
use gw_provider::types::{ProviderResponse, StreamChunk, StreamResponse, UsageRecord};
use tokio_util::task::TaskTracker;

use crate::ProxyState;
use crate::settlectx::{BillingHandle, SettleCtx};
use crate::usage::{Settlement, UsageOutcome};

/// Relays a streamed response, settling when the stream ends.
///
/// Settlement is claimed up-front so the hold middleware's finalizer stands
/// down; the actual charge is applied by [`StreamSettler`] once the last chunk
/// has been forwarded, or by its `Drop` if the client hangs up mid-stream.
pub(super) fn stream_response(
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

    let mut response = Response::new(Body::from_stream(RelayBody {
        chunks,
        settler,
        finishing: None,
    }));
    *response.status_mut() = status;
    apply_upstream_headers(response.headers_mut(), headers);
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

/// Relays a completed non-streaming response without waiting for the ledger.
///
/// The settle-vs-release-vs-strict decision is made from inputs we already
/// have (status, parsed usage, in-memory strict flag). The *write* is the
/// same detached [`StreamSettler`] drop a hung-up stream uses, so the client
/// is not blocked on Redis / Postgres. [`claim_finalize`][BillingHandle::claim_finalize]
/// still runs here so the hold middleware's safety net stands down and
/// failover cannot bill twice.
pub(super) fn unary_response(
    state: &ProxyState,
    response: ProviderResponse,
    billing: Option<BillingHandle>,
    auth_id: String,
    provider: &str,
    started: Instant,
) -> Response {
    let status = StatusCode::from_u16(response.status).unwrap_or(StatusCode::OK);
    // Bind the settler so it drops *after* the response is built and
    // *before* we return: Drop spawns onto `state.drain`. Marking `done`
    // would skip the spawn and leak the hold.
    let _settler = billing.and_then(|b| {
        b.claim_finalize().then(|| StreamSettler {
            settlement: state.dispatch.settlement.clone(),
            drain: state.drain.clone(),
            ctx: b.ctx.clone(),
            usage: response.usage,
            failed: !status.is_success(),
            auth_id,
            provider: provider.to_owned(),
            started,
            done: false,
        })
    });
    relay(status, response.headers, response.body)
}

/// Releases (never charges) a request that never produced an upstream body.
///
/// Same drain path as [`unary_response`]: claim here, ledger I/O off the
/// client wait. Matches [`UsageOutcome::failed`]: no usage envelope, `failed`.
pub(super) fn schedule_release(state: &ProxyState, billing: Option<&BillingHandle>) {
    let Some(billing) = billing else {
        return;
    };
    if !billing.claim_finalize() {
        return;
    }
    let _settler = StreamSettler {
        settlement: state.dispatch.settlement.clone(),
        drain: state.drain.clone(),
        ctx: billing.ctx.clone(),
        usage: None,
        failed: true,
        auth_id: String::new(),
        provider: String::new(),
        started: Instant::now(),
        done: false,
    };
}

/// Client-facing body of a streamed relay.
///
/// Filters [`StreamChunk::Usage`] / [`StreamChunk::Error`] out of the byte
/// stream (they are billing metadata, not SSE) and settles exactly once when
/// the upstream ends. A `Stream` impl rather than `unfold` so each chunk is a
/// poll, not a new future that moves the settler and the boxed upstream.
struct RelayBody {
    chunks: Pin<Box<dyn Stream<Item = StreamChunk> + Send>>,
    settler: Option<StreamSettler>,
    finishing: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl Stream for RelayBody {
    type Item = Result<Bytes, std::convert::Infallible>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(fut) = this.finishing.as_mut() {
                ready!(fut.as_mut().poll(cx));
                this.finishing = None;
                return Poll::Ready(None);
            }
            match ready!(this.chunks.as_mut().poll_next(cx)) {
                Some(StreamChunk::Payload(bytes)) => {
                    return Poll::Ready(Some(Ok(bytes)));
                }
                Some(StreamChunk::Usage(record)) => {
                    if let Some(s) = this.settler.as_mut() {
                        s.usage = Some(record);
                    }
                }
                Some(StreamChunk::Error { status, message }) => {
                    tracing::warn!(?status, %message, "upstream stream error");
                    if let Some(s) = this.settler.as_mut() {
                        s.failed = true;
                    }
                }
                None => match this.settler.take() {
                    Some(mut s) => {
                        // Same order as the old unfold: mark done first so
                        // Drop does not also spawn, then await settle inline.
                        s.done = true;
                        let settlement = s.settlement.clone();
                        let ctx = s.ctx.clone();
                        let outcome = s.into_outcome();
                        this.finishing = Some(Box::pin(async move {
                            settlement.settle(&ctx, outcome).await;
                        }));
                    }
                    None => return Poll::Ready(None),
                },
            }
        }
    }
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
    fn into_outcome(mut self) -> UsageOutcome {
        UsageOutcome {
            usage: self.usage.take(),
            failed: self.failed,
            auth_id: std::mem::take(&mut self.auth_id),
            provider: std::mem::take(&mut self.provider),
            duration_ms: self.started.elapsed().as_millis() as i64,
        }
    }

    fn outcome(&self) -> UsageOutcome {
        UsageOutcome {
            usage: self.usage.clone(),
            failed: self.failed,
            auth_id: self.auth_id.clone(),
            provider: self.provider.clone(),
            duration_ms: self.started.elapsed().as_millis() as i64,
        }
    }
}

impl Drop for StreamSettler {
    /// Detaches the ledger write onto [`ProxyState::drain`].
    ///
    /// Two callers land here: a stream body dropped mid-flight, and a unary
    /// response that already has its usage (or the release / strict inputs)
    /// and must not make the client wait for Redis / Postgres. Either way
    /// the hold must not be left to its TTL — that is free upstream output.
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

/// Copies an upstream response through, minus hop-by-hop headers.
pub(super) fn relay(status: StatusCode, headers: HeaderMap, body: Bytes) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    apply_upstream_headers(response.headers_mut(), headers);
    if !response.headers().contains_key(header::CONTENT_TYPE) {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
    }
    response
}

/// Moves upstream headers onto the client response, dropping hop-by-hop names.
///
/// A move, not a clone: the provider is done with the map. Replacing the
/// destination keeps repeated values (`set-cookie`) that a per-name `insert`
/// would collapse.
fn apply_upstream_headers(dst: &mut HeaderMap, mut src: HeaderMap) {
    for name in HOP_BY_HOP {
        src.remove(*name);
    }
    *dst = src;
}

/// Headers that describe one hop and must not be forwarded.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "content-length",
];

#[cfg(test)]
pub(crate) fn is_hop_by_hop(name: &str) -> bool {
    HOP_BY_HOP.contains(&name)
}
