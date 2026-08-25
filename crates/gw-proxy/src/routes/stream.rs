//! Relaying an upstream response back to the client, and the off-path settler.
//!
//! Split out of `routes.rs` so the dispatch file stays under the 1,000-line
//! ratchet.
//!
//! # One response path
//!
//! There is no streaming/unary fork here any more. `gw_relay::RelayEngine`
//! hands back frames for both, the client's body is those frames verbatim, and
//! usage is read on a **side band** — `gw-relay`'s [`UsageProbe`] sees a
//! read-only view of every frame and never sits in the write path. A 4xx from
//! the upstream travels this same way, with its headers intact.
//!
//! # Settling
//!
//! [`StreamSettler`] carries the obligation. It settles when the body ends
//! and, through `Drop`, when the client hangs up mid-stream — the case a plain
//! "settle after the handler returns" misses entirely. `Drop` spawns onto
//! [`ProxyState::drain`] so a settle straddling shutdown is waited out rather
//! than aborted with the runtime.
//!
//! # 长流的租约
//!
//! Redis 里的预留只有一片 TTL（[`gw_ledger::DEFAULT_HOLD_TTL`]，300 秒）。
//! 一条跑得比它久的健康流会在**自己还活着的时候**被过期清理掉，那之后余额
//! 闸门就看不见这笔在途负债了。所以 body 每动一次就检查一下租约，
//! 到点了就续一片 —— 见 [`LEASE_RENEW_PERIOD`]。
//!
//! 续租挂在这里（回写包装），**不在 `gw-relay::engine` 里**：中继层是计费盲的。
//!
//! `claim_finalize` 三入口（每请求恰一次结算权）：
//! | 入口 | 语义 |
//! | [`relay_response`] | 上游出了 body：settler 持票，body 结束或断开时结算 |
//! | [`schedule_release`] | 上游未出 body：Release |
//! | [`crate::hold::HoldMiddleware::finalize`] | 兜底：handler 未结算时 settle_missing 或 Release |

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::{StatusCode, header};
use axum::response::Response;
use futures_util::Stream;
use gw_provider::types::UsageRecord;
use gw_relay::probe::{SseUsageProbe, UsageHandle, UsageShape};
use gw_relay::{RelayError, RelayResponse, RelayResponseBody, RelayUsage, UpstreamDialect};
use http_body_util::BodyExt as _;
use tokio_util::task::TaskTracker;

use crate::ProxyState;
use crate::settlectx::{BillingHandle, SettleCtx};
use crate::usage::{Settlement, UsageOutcome};

/// 多久续一次租约：一片租约（[`gw_ledger::DEFAULT_HOLD_TTL`]）的**一半**。
///
/// 这个比例是有意义的：丢掉一次续租（Redis 抖一下、任务被调度晚了）之后
/// 还剩整整半片的余量，预留不会因为一次瞬时故障就消失。
///
/// 由那个 TTL **算出来**而不是抄一个数字，所以 TTL 改了这里跟着改；
/// 但它不读配置 —— 它是这条回路的阻尼系数，不是运维旋钮。
const LEASE_RENEW_PERIOD: Duration = Duration::from_secs(gw_ledger::DEFAULT_HOLD_TTL.as_secs() / 2);

/// The side-band probe for one attempt, plus the handle its result lands in.
///
/// Built before the relay call because the engine takes ownership of the probe
/// and guarantees exactly one `finish()` — including on a client hang-up.
pub(super) fn usage_probe(dialect: UpstreamDialect) -> (Box<SseUsageProbe>, UsageHandle) {
    let (probe, handle) = SseUsageProbe::new(usage_shape(dialect));
    (Box::new(probe), handle)
}

/// Upstream wire protocol → the shape its usage envelope takes.
fn usage_shape(dialect: UpstreamDialect) -> UsageShape {
    match dialect {
        UpstreamDialect::OpenAiChat | UpstreamDialect::OpenAiResponses => UsageShape::OpenAi,
        UpstreamDialect::AnthropicMessages => UsageShape::Anthropic,
        UpstreamDialect::GoogleGenerateContent => UsageShape::Google,
    }
}

/// Everything one relayed attempt needs to become a client response.
///
/// A struct rather than eight positional parameters: `auth_id`, `provider` and
/// `model` are all strings, and at that width the compiler stops helping.
pub(super) struct Relayed {
    pub(super) response: RelayResponse,
    pub(super) handle: UsageHandle,
    pub(super) billing: Option<BillingHandle>,
    pub(super) auth_id: String,
    pub(super) provider: &'static str,
    pub(super) model: String,
    pub(super) started: Instant,
}

/// Relays one upstream response to the client and settles it exactly once.
///
/// The status is whatever the upstream said — 200, 429 or 503 — and it travels
/// with its own headers. A non-2xx is still a *response*; the only thing that
/// differs is that it settles as a failure so the hold is released rather than
/// charged.
pub(super) fn relay_response(state: &ProxyState, relayed: Relayed) -> Response {
    let Relayed {
        response,
        handle,
        billing,
        auth_id,
        provider,
        model,
        started,
    } = relayed;
    let RelayResponse {
        status,
        headers,
        body,
    } = response;

    let settler = billing.and_then(|b| {
        b.claim_finalize().then(|| StreamSettler {
            settlement: state.dispatch.settlement.clone(),
            drain: state.drain.clone(),
            ctx: b.ctx.clone(),
            usage: None,
            // An upstream error status is a failed request for billing: it
            // released nothing upstream, so it must release here.
            failed: !status.is_success(),
            auth_id,
            provider: provider.to_owned(),
            model,
            started,
            // 预算代币付的账在 Redis 里根本没有预留，续租只会得到
            // HoldNotFound —— 不去问，省掉每分钟一次的噪音告警。
            renewable: !b.used_budget_token,
            last_renew: started,
            done: false,
        })
    });

    let stream = SettledBody {
        inner: Some(match body {
            RelayResponseBody::Stream(body) => Box::pin(body.into_data_stream()),
            RelayResponseBody::Buffered(bytes) => {
                Box::pin(futures_util::stream::once(async move { Ok(bytes) }))
            }
        }),
        handle,
        settler,
    };

    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = status;
    // 上游 header 回写：复制**只有一份实现**，在 `gw-relay`。它保住 `set-cookie`
    // 这类同名多值（逐个 `insert` 会折成最后一条），并按 RFC 7230 §6.1 剥掉这条
    // 消息自己 `Connection` 点名的逐跳头。这里曾经有第二张写死的 `HOP_BY_HOP`
    // 名单 —— 两张名单必然漂：那张漏了 `expect`，也从不看 `Connection` 的值，
    // 于是上游一句 `Connection: close, x-foo` 里的 `x-foo` 被原样发给客户端。
    gw_relay::copy_preserving_multivalue(response.headers_mut(), &headers);
    if !response.headers().contains_key(header::CONTENT_TYPE) {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );
    }
    response
}

/// Releases (never charges) a request that never produced an upstream body.
///
/// Claim here, ledger I/O off the client's wait. Matches
/// [`UsageOutcome::failed`]: no usage envelope, `failed`.
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
        model: String::new(),
        started: Instant::now(),
        // 这条路上没有 body，也就没有要续的租约。
        renewable: false,
        last_renew: Instant::now(),
        done: false,
    };
}

/// The client-facing body: the upstream's frames, plus the settlement.
///
/// The frames are forwarded byte-for-byte. The only thing this type adds is
/// *when* to settle — and it has to read the probe's result **after** the
/// relay's own body has been dropped, because that drop is what calls
/// `finish()`. Hence the explicit `inner.take()` in both paths.
/// The upstream's frames, boxed. `RelayResponseBody` has two shapes and this
/// erases the difference.
type UpstreamFrames = Pin<Box<dyn Stream<Item = Result<Bytes, RelayError>> + Send>>;

struct SettledBody {
    /// `Option` so it can be dropped early, releasing the relay's probe guard
    /// before the usage handle is read.
    inner: Option<UpstreamFrames>,
    handle: UsageHandle,
    settler: Option<StreamSettler>,
}

impl SettledBody {
    /// Drops the upstream body — which finishes the probe — then hands the
    /// resulting usage to the settler and lets it go.
    ///
    /// Dropping the settler is what schedules the ledger write, so this is
    /// also where the request stops being billable.
    fn finish(&mut self) {
        let Some(mut settler) = self.settler.take() else {
            return;
        };
        drop(self.inner.take());
        settler.usage = self
            .handle
            .get()
            .flatten()
            .map(|usage| to_record(usage, &settler.model, &settler.provider));
    }
}

impl Stream for SettledBody {
    type Item = Result<Bytes, RelayError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let Some(inner) = this.inner.as_mut() else {
            return Poll::Ready(None);
        };
        match ready!(inner.as_mut().poll_next(cx)) {
            Some(Ok(bytes)) => {
                // 客户端的 body 还在动 = 这个请求还活着 = 它的预留不该过期。
                if let Some(settler) = this.settler.as_mut() {
                    settler.maybe_renew_lease();
                }
                Poll::Ready(Some(Ok(bytes)))
            }
            Some(Err(err)) => {
                // A mid-stream failure is a real error, not a clean EOF: hyper
                // turns it into RST_STREAM so the client can tell it was cut
                // off. It also means the response is not billable as a success.
                tracing::warn!(%err, "upstream stream failed mid-response");
                if let Some(settler) = this.settler.as_mut() {
                    settler.failed = true;
                }
                this.finish();
                Poll::Ready(Some(Err(err)))
            }
            None => {
                this.finish();
                Poll::Ready(None)
            }
        }
    }
}

impl Drop for SettledBody {
    /// The client hung up. `finish` is idempotent (`settler.take()`), so a body
    /// that already ended does nothing here.
    fn drop(&mut self) {
        self.finish();
    }
}

/// `gw-relay`'s side-band usage → the record the settlement pipeline bills on.
fn to_record(usage: RelayUsage, model: &str, provider: &str) -> UsageRecord {
    UsageRecord {
        model: model.to_owned(),
        provider: provider.to_owned(),
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_tokens: usage.cached_tokens,
        reasoning_tokens: usage.reasoning_tokens,
    }
}

/// Carries the settlement obligation for one relayed response.
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
    /// The upstream model name, stamped onto the usage record the probe
    /// produces — the probe reads token counts, not model names.
    model: String,
    started: Instant,
    /// 这个请求在 Redis 里有没有预留可续。预算代币那条路没有。
    renewable: bool,
    /// 上一次续租（或开始）的时刻。
    last_renew: Instant,
    done: bool,
}

impl StreamSettler {
    /// 到点就续一片租约，**在结算之外**：它不动金额，也不重新检查余额。
    ///
    /// 失败只记日志，绝不中断这条流 —— 预留会在剩下的 TTL 里自然过期，
    /// 那一行留给对账。为了一次 Redis 抖动把客户端的回复切断是明显更坏的交易。
    ///
    /// 账本写在 [`ProxyState::drain`] 上，所以 poll 不会被 I/O 卡住。
    fn maybe_renew_lease(&mut self) {
        if !self.renewable || self.last_renew.elapsed() < LEASE_RENEW_PERIOD {
            return;
        }
        self.last_renew = Instant::now();
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let ledger = self.settlement.ledger().clone();
        let user_id = self.ctx.user_id;
        let operation = self.ctx.operation.clone();
        self.drain.spawn(async move {
            if let Err(err) = ledger.renew_lease(user_id, &operation).await {
                tracing::warn!(
                    user_id,
                    operation = %operation,
                    %err,
                    "hold lease renew failed; the reservation will expire on its TTL",
                );
            }
        });
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

/// Status codes that mean "this account, right now" rather than "this request".
pub(super) fn is_retryable_status(status: StatusCode) -> bool {
    status.as_u16() >= 500 || status == StatusCode::TOO_MANY_REQUESTS
}
