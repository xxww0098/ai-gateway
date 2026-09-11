//! 代理内核的热路径中间件。
//!
//! NewAPI 把鉴权、渠道、预扣、重试的生命周期写在 `Relay()` 里而不是散落在
//! Gin 中间件；CLIProxyAPI 则把内核收成「薄 handler + 账号选择 + executor」。
//! 这里只借一件事：axum 热路径只挂 **一层** [`layer`]，access → hold 的顺序
//! 写在函数体里（先鉴权拿到 [`crate::ports::AccessMetadata`]，再进
//! `hold.handle`），B1（先鉴权再预扣）因此是控制流上的不变量，
//! 不再靠「两个 `.layer()` 谁先谁后」维持。
//!
//! Hold / Settle / Release 的签名与语义一行未动。停机排空仍走
//! [`crate::ProxyState::drain`]（`StreamSettler` / `TaskTracker`）。

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::ProxyState;
use crate::access::{credential_from, is_proxy_path};
use crate::error::AuthError;

/// 热路径上**唯一**的 axum 中间件。
///
/// 先鉴权、再预扣，顺序写在函数体里，不再依赖 `.layer()` 的挂载顺序。
/// `access::layer` / `hold::layer` 仍保留，给只想测其中一层的用例用。
pub async fn layer(State(state): State<ProxyState>, mut req: Request, next: Next) -> Response {
    if !is_proxy_path(req.uri().path()) {
        return next.run(req).await;
    }

    // 借用直接跨过 await：token 只在鉴权调用期间存活，不拷贝整个 Bearer。
    let outcome = match credential_from(req.headers()) {
        Some(token) => state.access.authenticate_token(token).await,
        None => Err(AuthError::NoCredentials),
    };

    match outcome {
        Ok(meta) => {
            req.extensions_mut().insert(meta);
            state.hold.clone().handle(req, next).await
        }
        Err(err) => err.into_response(),
    }
}
