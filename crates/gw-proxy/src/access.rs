//! 代理面的租户鉴权。
//!
//! provider 与它的 HTTP 桥接中间件合并成一个 axum layer。
//!
//! 只接受两种凭据形态：
//!
//! ```text
//! cpa-<hex>   -> api_keys 查表（L1 缓存 -> DB）
//! <jwt>       -> 面板密钥签的 HS256 JWT
//! ```
//!
//! **载体只有一个**：`Authorization: Bearer <token>`（见 [`credential_from`]）。
//! 三面收敛（`docs/relay-surface-plan.md` §2）删掉 `/v1beta/**` 之后，
//! Google SDK 用的 `x-goog-api-key` / `x-api-key` / `?key=` 三种载体一并下线 ——
//! 它们只在 Gemini 原生面上有意义，而那个面已经不存在了。
//!
//! ⚠️ **`/v1` 上的 `x-api-key` 不是租户凭据**，它是 Anthropic 自己的上游头，
//! 必须原样透传给 claude executor。历史上 `strip_consumed_credentials` 用
//! `if !path.starts_with("/v1beta/") { return; }` 守住这个区别；`/v1beta` 消失后
//! 整个剥离函数变成死代码被删掉了，但**这条语义必须保留** —— 否则下一个人会
//! 「顺手」把 `/v1` 上的 `x-api-key` 也剥掉，Anthropic 直连立刻全线 401。
//! 见 [`crate::routes::inbound`] 附近的同一条注释。
//!
//! 成功后挂一个 [`AccessMetadata`] 扩展，带上 [`crate::hold`] 预扣需要的全部计费状态，
//! 所以 hold 的 preflight 不必重查。
//!
//! 本层不做任何 Hold/Settle/Release 副作用，因此保持幂等、可安全重试。

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::ProxyState;
use crate::error::AuthError;
use crate::ports::{AccessMetadata, AuthCrypto, Id, TenantDirectory};

/// Prefix that marks a CPA-issued API key (as opposed to a JWT).
const API_KEY_PREFIX: &str = "cpa-";

/// 代理面的**唯一**前缀。收敛后 `/v1beta/` 不再是兄弟前缀，它根本不存在。
pub const V1_PATH_PREFIX: &str = "/v1/";

/// `path` 是否属于计量代理面。
///
/// 一个判据同时服务两道门是有意的：[`layer`] 鉴权的路径集合与
/// [`crate::hold::is_billable`] 预扣的路径集合必须同源，否则会出现
/// 「计费但匿名」或「鉴权但免费」的路由。
///
/// **收敛的连带收益**：这里只剩 `/v1/` 之后，它与
/// `gw-server/src/metrics.rs` 的 `path.starts_with("/v1/")` 口径自动一致 ——
/// 历史上 `/v1beta` 流量被鉴权、被计费，却不进 `cpa_v1_requests_total`。
#[must_use]
pub fn is_proxy_path(path: &str) -> bool {
    path.starts_with(V1_PATH_PREFIX)
}

/// The `users.status` value that grants access.
const STATUS_ACTIVE: &str = "active";

/// 解析请求携带的租户凭据。**载体只有 `Authorization: Bearer`**。
///
/// 收敛前还接受 `x-goog-api-key` / `x-api-key` / `?key=` 三种载体，那是为了让
/// Gemini 客户端能在 `/v1beta` 上认证。`/v1beta` 已硬删，三种载体随之下线。
///
/// 特别注意 `x-api-key`：它**没有**变成「不再接受的租户凭据」，而是回到了它在
/// `/v1` 上一直以来的身份 —— **Anthropic 自己的上游头**，网关原样透传给 claude。
/// 两者只是碰巧同名。
#[must_use]
pub fn credential_from(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(bearer_token)
}

/// Resolves credentials to a CPA tenant.
///
/// Resolves credentials to a CPA tenant. The `UserStatusCache` / `APIKeyCache`
/// that were originally held as struct fields live behind [`TenantDirectory`]
/// here, because caching is an infrastructure concern and this type only owns
/// policy.
pub struct AccessProvider {
    directory: Arc<dyn TenantDirectory>,
    crypto: Arc<dyn AuthCrypto>,
}

impl AccessProvider {
    /// Builds the provider over a directory and crypto backend.
    pub fn new(directory: Arc<dyn TenantDirectory>, crypto: Arc<dyn AuthCrypto>) -> Self {
        Self { directory, crypto }
    }

    /// Registry key the access manager routes on.
    pub fn identifier(&self) -> &'static str {
        "cpa-tenant"
    }

    /// Parses the `Authorization` header and resolves it to billing metadata.
    pub async fn authenticate(
        &self,
        authorization: Option<&str>,
    ) -> Result<AccessMetadata, AuthError> {
        let token = authorization
            .and_then(bearer_token)
            .ok_or(AuthError::NoCredentials)?;
        self.authenticate_token(token).await
    }

    /// Resolves an already-extracted credential to billing metadata.
    ///
    /// 与 [`Self::authenticate`] 分开，是因为 [`layer`] 已经用
    /// [`credential_from`] 取过 token（它要先拿到所有权才能把结果插回请求扩展）。
    /// 提取之后的一切 —— API key 还是 JWT、status 复查、entitlement 过滤 ——
    /// 三个入口完全一致。
    pub async fn authenticate_token(&self, token: &str) -> Result<AccessMetadata, AuthError> {
        if token.is_empty() {
            return Err(AuthError::NoCredentials);
        }
        if let Some(rest) = token.strip_prefix(API_KEY_PREFIX) {
            // Reject "cpa-" with nothing after it before paying for a hash.
            if rest.is_empty() {
                return Err(AuthError::InvalidCredential);
            }
            self.authenticate_api_key(token).await
        } else {
            self.authenticate_jwt(token).await
        }
    }

    /// Resolve a `cpa-` API key to billing metadata.
    async fn authenticate_api_key(&self, plaintext: &str) -> Result<AccessMetadata, AuthError> {
        let key_hash = self.crypto.hash_api_key(plaintext);

        let row = self
            .directory
            .api_key_by_hash(&key_hash)
            .await
            .map_err(|e| AuthError::Internal(format!("api key lookup failed: {e}")))?
            .ok_or(AuthError::InvalidCredential)?;

        // A cached entry can outlive a DB-side deactivation, so the status gate
        // fires on the cache-hit path too — the DB query filters on
        // `status = 'active'` and the cache branch checks the cached status for
        // the very same reason.
        if row.status != STATUS_ACTIVE {
            return Err(AuthError::InvalidCredential);
        }

        // last_used_at 是展示字段：实现里已经 detach，这里 await 只等到 spawn。
        self.directory.touch_api_key(row.id).await;

        // 状态 / 订阅 / 倍率互不依赖，并行拿。拒绝时仍然不回订阅内容，
        // 所以和「先查 status 再查订阅」一样不会变成用户状态神谕。
        let group_id = row.group_id;
        let (active, subscription, multiplier) = tokio::join!(
            self.user_is_active(row.user_id),
            self.active_subscription(row.user_id),
            async {
                match group_id {
                    Some(gid) => self
                        .directory
                        .group_rate_multiplier(gid)
                        .await
                        .ok()
                        .flatten()
                        .filter(|m| *m > 0.0),
                    None => None,
                }
            },
        );
        if !active {
            return Err(AuthError::InvalidCredential);
        }

        // 倍率只查一次。1.0 = 基线组，人人有；其它值要现场核对订阅。
        let (group_id, rate_mult) = match (group_id, multiplier) {
            (Some(gid), Some(1.0)) => (Some(gid), 1.0),
            (Some(gid), Some(mult)) => {
                if self.group_entitled(row.user_id, gid).await {
                    (Some(gid), mult)
                } else {
                    (None, 1.0)
                }
            }
            _ => (None, 1.0),
        };

        Ok(AccessMetadata {
            user_id: row.user_id,
            api_key_id: row.id,
            group_id,
            rate_mult,
            subscription,
        })
    }

    /// Resolve a JWT to billing metadata.
    async fn authenticate_jwt(&self, token: &str) -> Result<AccessMetadata, AuthError> {
        let claims = self
            .crypto
            .verify_jwt(token)
            .ok_or(AuthError::InvalidCredential)?;
        if claims.user_id == 0 {
            return Err(AuthError::InvalidCredential);
        }

        // The JWT path has no API-key cache entry to gate on, so every request
        // re-confirms the user. Checking BEFORE loading the subscription keeps
        // quota values from leaking for a suspended user — the rejection must
        // be indistinguishable from any other invalid credential.
        let (active, subscription) = tokio::join!(
            self.user_is_active(claims.user_id),
            self.active_subscription(claims.user_id),
        );
        if !active {
            return Err(AuthError::InvalidCredential);
        }

        Ok(AccessMetadata {
            user_id: claims.user_id,
            api_key_id: 0,
            group_id: None,
            rate_mult: 1.0,
            subscription,
        })
    }

    /// Whether `users.status` is currently `active`.
    ///
    /// Fail-closed: a zero id, a missing row, or a DB error all deny. Denying
    /// on a
    /// transient error is deliberate — the alternative is letting a suspended
    /// tenant spend during an outage.
    async fn user_is_active(&self, user_id: Id) -> bool {
        if user_id == 0 {
            return false;
        }
        matches!(
            self.directory.user_status(user_id).await,
            Ok(Some(ref s)) if s == STATUS_ACTIVE
        )
    }

    /// Whether the user still holds an entitlement for `group_id`.
    ///
    /// A baseline group
    /// (multiplier 1.0) is implicitly held by every active user, anything else
    /// needs a live subscription bound to that group. Transient errors deny the
    /// multiplier rather than 5xx-ing a valid request.
    /// 非基线组是否仍有有效订阅。倍率已经在上面查过一次，这里不再重查。
    async fn group_entitled(&self, user_id: Id, group_id: Id) -> bool {
        if user_id == 0 || group_id == 0 {
            return false;
        }
        self.directory
            .holds_group_entitlement(user_id, group_id)
            .await
            .unwrap_or(false)
    }

    /// A missing or stale subscription is silently skipped, never an error.
    async fn active_subscription(&self, user_id: Id) -> Option<crate::ports::SubscriptionQuota> {
        self.directory
            .active_subscription(user_id)
            .await
            .ok()
            .flatten()
    }
}

/// axum layer that authenticates the proxy surface and publishes
/// [`AccessMetadata`].
///
/// axum layer that authenticates the proxy surface. **This must run before
/// [`crate::hold::layer`]** — that ordering is the entire point of blocker B1:
/// with Hold first, every `/v1` request aborts with a pre-auth 401
/// and the billing hot path never executes. [`crate::router`] wires the two in
/// the correct order, and `access::tests` pins it.
pub async fn layer(State(state): State<ProxyState>, mut req: Request, next: Next) -> Response {
    if !is_proxy_path(req.uri().path()) {
        return next.run(req).await;
    }

    // Owned, because resolving it borrows the request and inserting the result
    // needs it back mutably.
    let credential = credential_from(req.headers()).map(str::to_owned);

    let outcome = match &credential {
        Some(token) => state.access.authenticate_token(token).await,
        None => Err(AuthError::NoCredentials),
    };

    match outcome {
        Ok(meta) => {
            req.extensions_mut().insert(meta);
            next.run(req).await
        }
        Err(err) => err.into_response(),
    }
}

/// Returns the token from `Bearer <token>` (scheme match is
/// case-insensitive), rejecting malformed headers.
pub fn bearer_token(header: &str) -> Option<&str> {
    let mut parts = header.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    if parts.next().is_some() || !scheme.eq_ignore_ascii_case("Bearer") || token.is_empty() {
        return None;
    }
    Some(token)
}

#[cfg(test)]
mod tests;
