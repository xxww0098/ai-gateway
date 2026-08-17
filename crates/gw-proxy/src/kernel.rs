//! 代理内核的请求状态机与单一请求上下文。
//!
//! NewAPI 用 `RelayInfo` 把鉴权、渠道、预扣、重试拧成一份请求上下文，
//! 生命周期写在 `Relay()` 里而不是散落在 Gin 中间件。CLIProxyAPI 则把
//! 内核收成「薄 handler + 账号选择 + executor」，协议翻译不进热路径决策。
//!
//! 这里只借这两件事，不搬它们的产品：
//!
//! 1. **一份 [`RelayCtx`]**（对应 NewAPI 的 `RelayInfo`）：租户、预扣、
//!    模型、trace 都在同一个对象上，handler 不再东拼西凑扩展。
//! 2. **具名状态 + 显式转移**（对应 NewAPI 的阶段表）：axum 热路径只挂
//!    **一层** [`layer`]，access → hold 的顺序变成状态机边，而不是
//!    「两个 `.layer()` 谁先谁后」。B1（先鉴权再预扣）因此是类型上的
//!    不变量，不再靠注释维持。
//!
//! Hold / Settle / Release 的签名与语义一行未动。停机排空仍走
//! [`crate::ProxyState::drain`]（`StreamSettler` / `TaskTracker`）。

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::ProxyState;
use crate::access::{credential_from, is_proxy_path};
use crate::error::AuthError;
use crate::hold::BillingPeek;
use crate::ports::AccessMetadata;
use crate::settlectx::BillingHandle;

/// 一条 `/v1` 请求在内核里所处的阶段。
///
/// 转移是单向的；终态不再走出。非法跳转在生产里被忽略并打 debug，
/// 测试里用 [`Phase::can_transition_to`] 钉死合法边。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// 刚进内核，还没有租户。
    Received,
    /// 鉴权通过，[`AccessMetadata`] 已就位。
    Authenticated,
    /// 请求体已 peek，计费侧的 model / stream / max_tokens 已知。
    Inspected,
    /// 限流 / 熔断 / 欠款 / 配额 / 余额门都过了，尚未预扣。
    Gated,
    /// Redis hold（或预算代币）已落下。
    Reserved,
    /// 上游候选已选出。
    Routed,
    /// 正在打某一个上游账号。
    Attempting,
    /// 上游已经回了，正在往客户端中继。
    Relaying,
    /// 不计费路径（catalogue / count_tokens）或幂等重放：鉴权过、不预扣。
    Skipped,
    /// 按真实 usage 或 fallback 精算入账。
    Settled,
    /// 上游失败或网关拒单，hold 已释放。
    Released,
    /// strict-usage-metadata：不结算不释放，hold 等 TTL。
    StrictHeld,
    /// 在预扣之前就被拒（401 / 402 / 429 / 503），账本未被触碰。
    Failed,
}

impl Phase {
    /// 终态：不再有出边。
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Skipped | Self::Settled | Self::Released | Self::StrictHeld | Self::Failed
        )
    }

    /// 这一跳是否在状态机的合法边上。
    ///
    /// 合法图（与 NewAPI 的阶段顺序对齐，计费语义仍是我们的）：
    ///
    /// ```text
    /// Received → Authenticated | Failed
    /// Authenticated → Inspected | Skipped | Failed
    /// Inspected → Gated | Failed
    /// Gated → Reserved | Skipped | Failed
    /// Reserved → Routed | Released | Failed
    /// Routed → Attempting | Failed
    /// Attempting → Relaying | Attempting | Failed
    /// Relaying → Settled | Released | StrictHeld | Failed
    /// ```
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        use Phase::*;
        match (self, next) {
            (Received, Authenticated | Failed) => true,
            (Authenticated, Inspected | Skipped | Failed) => true,
            (Inspected, Gated | Failed) => true,
            (Gated, Reserved | Skipped | Failed) => true,
            (Reserved, Routed | Released | Failed) => true,
            (Routed, Attempting | Failed) => true,
            (Attempting, Relaying | Attempting | Failed) => true,
            (Relaying, Settled | Released | StrictHeld | Failed) => true,
            _ => false,
        }
    }
}

/// 一条请求的内核上下文。对应 NewAPI 的 `RelayInfo`，但只装我们热路径
/// 真正用到的字段：租户、预扣、peek、trace。body 仍走 [`crate::hold::PeekedBody`]
/// （`Bytes` 引用计数，再拷一份是浪费）。
#[derive(Debug, Clone)]
pub struct RelayCtx {
    pub phase: Phase,
    pub access: AccessMetadata,
    pub request_id: String,
    pub ip_address: String,
    pub idempotency_key: String,
    pub peek: Option<BillingPeek>,
    pub billing: Option<BillingHandle>,
}

impl RelayCtx {
    /// 鉴权刚通过时的起点。
    #[must_use]
    pub fn authenticated(access: AccessMetadata) -> Self {
        Self {
            phase: Phase::Authenticated,
            access,
            request_id: String::new(),
            ip_address: String::new(),
            idempotency_key: String::new(),
            peek: None,
            billing: None,
        }
    }

    /// 推进到 `next`。非法跳转不改状态，避免一次写错把后续结算带偏。
    pub fn advance(&mut self, next: Phase) {
        if self.phase.can_transition_to(next) {
            self.phase = next;
            return;
        }
        tracing::debug!(from = ?self.phase, to = ?next, "relay phase jump ignored");
    }
}

/// 请求扩展上的状态机推进。扩展缺席（测试里只挂了 hold）时是空操作。
pub fn advance_ext(req: &mut Request, next: Phase) {
    if let Some(ctx) = req.extensions_mut().get_mut::<RelayCtx>() {
        ctx.advance(next);
    }
}

/// 热路径上**唯一**的 axum 中间件。
///
/// 先鉴权、再预扣，顺序写在函数体里，不再依赖 `.layer()` 的挂载顺序。
/// `access::layer` / `hold::layer` 仍保留，给只想测其中一层的用例用。
pub async fn layer(State(state): State<ProxyState>, mut req: Request, next: Next) -> Response {
    if !is_proxy_path(req.uri().path()) {
        return next.run(req).await;
    }

    let credential = credential_from(req.headers()).map(str::to_owned);
    let outcome = match credential.as_deref() {
        Some(token) => state.access.authenticate_token(token).await,
        None => Err(AuthError::NoCredentials),
    };

    match outcome {
        Ok(meta) => {
            req.extensions_mut()
                .insert(RelayCtx::authenticated(meta.clone()));
            req.extensions_mut().insert(meta);
            state.hold.clone().handle(req, next).await
        }
        Err(err) => err.into_response(),
    }
}

#[cfg(test)]
mod tests;
