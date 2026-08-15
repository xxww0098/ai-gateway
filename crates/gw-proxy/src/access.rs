//! Tenant authentication for the proxy surface.
//!
//! The provider and the middleware that used to bridge it into the HTTP layer
//! collapse into one axum layer.
//!
//! Two credential shapes are accepted:
//!
//! ```text
//! cpa-<hex>   -> api_keys lookup (L1 cache -> DB)
//! <jwt>       -> HS256 JWT signed with the panel secret
//! ```
//!
//! Where they may be presented depends on the dialect — see
//! [`credential_from`]. `/v1/*` is `Authorization: Bearer` only, byte-for-
//! byte. `/v1beta/*` also takes the header and query shapes Google's
//! own SDKs send, because that surface was the SDK's and no Gemini client sets
//! `Authorization`.
//!
//! On success an [`AccessMetadata`] extension carries every piece of billing
//! state [`crate::hold`] needs, so the hold pre-flight does not re-query.
//!
//! This layer performs no Hold/Settle/Release side effects, so it stays
//! idempotent and safely retryable.

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

/// The OpenAI/Anthropic dialect prefix.
pub const V1_PATH_PREFIX: &str = "/v1/";

/// The Gemini native dialect prefix — Google's own API version segment, which
/// is why it is `v1beta` and not another `/v1` sub-path.
///
/// This surface was formerly served upstream, but the
/// dashboard still hands it to tenants as their integration endpoint
/// (`QuickIntegrationPanel.tsx`, which is frozen), so a client that follows the
/// panel's instructions must land on a real route rather than a 404.
pub const V1BETA_PATH_PREFIX: &str = "/v1beta/";

/// Whether `path` belongs to the metered proxy surface.
///
/// One predicate serves both gates on purpose: [`layer`] authenticates exactly
/// the paths [`crate::hold::is_billable`] reserves against. Letting the two
/// drift is how a route ends up billed but anonymous, or authenticated but
/// free — and `/v1beta` is precisely where that would have happened, since
/// `"/v1beta/models"` does *not* start with `"/v1/"`.
#[must_use]
pub fn is_proxy_path(path: &str) -> bool {
    path.starts_with(V1_PATH_PREFIX) || path.starts_with(V1BETA_PATH_PREFIX)
}

/// The `users.status` value that grants access.
const STATUS_ACTIVE: &str = "active";

/// Header Google's own Gemini SDKs put the API key on. They never set
/// `Authorization`, so without this the `/v1beta` surface answers every stock
/// client with a 401 — which is only marginally better than the 404 it used to
/// answer with.
const GOOGLE_API_KEY_HEADER: &str = "x-goog-api-key";

/// The header name the dashboard's own integration panel prints for this
/// surface. Accepted for the same reason: the panel is frozen, so the backend
/// is the only side that can make its instructions true.
const API_KEY_HEADER: &str = "x-api-key";

/// Query parameter Google's SDKs fall back to when a header is inconvenient.
/// A credential in a query string is a credential in every access log that
/// records a URI — see [`redact_query`], which is not optional.
const KEY_QUERY_PARAM: &str = "key";

/// Resolves the tenant credential a request presents, honouring the dialect of
/// the surface it arrived on.
///
/// `/v1/*` accepts `Authorization: Bearer` and nothing else. That surface is
/// this repo's own, so it is real parity territory and stays byte-for-byte
/// identical.
///
/// `/v1beta/*` additionally accepts [`GOOGLE_API_KEY_HEADER`],
/// [`API_KEY_HEADER`] and `?key=`, so a Gemini client can present its
/// credential in any of those ways.
///
/// Priority is fixed rather than incidental, so a request carrying several has
/// one defined outcome: **Authorization > x-goog-api-key > x-api-key > ?key=**.
/// Whichever wins, it resolves through the same API-key lookup, the same
/// user-status recheck and the same billing pipeline.
#[must_use]
pub fn credential_from<'a>(
    path: &str,
    headers: &'a HeaderMap,
    query: Option<&'a str>,
) -> Option<&'a str> {
    if let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(bearer_token)
    {
        return Some(token);
    }
    if !path.starts_with(V1BETA_PATH_PREFIX) {
        return None;
    }
    for name in [GOOGLE_API_KEY_HEADER, API_KEY_HEADER] {
        if let Some(token) = headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            return Some(token);
        }
    }
    query.and_then(key_query_param)
}

/// Reads the raw `key` parameter out of a query string.
///
/// Not percent-decoded: both credential shapes this gateway issues (`cpa-` plus
/// hex, and a base64url JWT) are already URL-safe, and decoding would invent a
/// transformation that was never applied.
fn key_query_param(query: &str) -> Option<&str> {
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == KEY_QUERY_PARAM && !value.is_empty()).then_some(value)
    })
}

/// Removes the credential carriers [`credential_from`] consumed, so they are
/// not relayed upstream.
///
/// On `/v1beta` these carry a **tenant** credential; the upstream credential
/// comes from the account pool. Relaying them hands a CPA API key to Google:
/// `gw_provider::types::is_skipped_proxy_header` drops only `Authorization`,
/// and the Gemini executor overwrites `x-goog-api-key` only when an upstream
/// key is actually configured — with a pooled OAuth credential it is not, and
/// the tenant's own key would travel to Google untouched. The layer that
/// consumed a credential is the layer that has to drop it.
///
/// A no-op off the Gemini surface: `/v1` never reads a credential from these,
/// and `x-api-key` there is Anthropic's own header, which its executor needs.
pub fn strip_consumed_credentials(
    path: &str,
    headers: &mut HeaderMap,
    query: &mut Vec<(String, String)>,
) {
    if !path.starts_with(V1BETA_PATH_PREFIX) {
        return;
    }
    headers.remove(GOOGLE_API_KEY_HEADER);
    headers.remove(API_KEY_HEADER);
    query.retain(|(name, _)| name != KEY_QUERY_PARAM);
}

/// Rewrites a query string with the tenant credential masked.
///
/// **Anything that renders a proxy request's URI must render this instead** —
/// access logs, tracing spans, error bodies, metric labels. `?key=` is how a
/// Gemini client authenticates, so on this surface the query string is
/// credential material, and a query string is the one part of a URI that
/// everything logs by default (`tower_http::trace::DefaultMakeSpan` included).
///
/// Returns the query unchanged when it carries no credential, so callers can
/// use it unconditionally.
#[must_use]
pub fn redact_query(query: &str) -> String {
    query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((name, value)) if name == KEY_QUERY_PARAM && !value.is_empty() => {
                format!("{name}=REDACTED")
            }
            _ => pair.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("&")
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
    /// Split out of [`Self::authenticate`] because the Gemini surface presents
    /// the same token on extra carriers ([`credential_from`]).
    /// Everything downstream of the extraction — API key vs JWT, the status
    /// recheck, the entitlement filter — is deliberately identical whichever
    /// carrier it arrived on, so a Gemini client is exactly as authenticated,
    /// and exactly as billed, as an OpenAI one.
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

        let mut group_id = row.group_id;
        let mut rate_mult = 1.0_f64;
        if let Some(gid) = group_id
            && let Ok(Some(mult)) = self.directory.group_rate_multiplier(gid).await
            && mult > 0.0
        {
            rate_mult = mult;
        }

        // Fire-and-forget last_used_at bump; never blocks the request.
        self.directory.touch_api_key(row.id).await;

        // Second-stage gate: the key may be active while its owner was
        // suspended or deleted since the cache entry was written.
        if !self.user_is_active(row.user_id).await {
            return Err(AuthError::InvalidCredential);
        }

        // Entitlement filter: the key's bound group may point at a lapsed
        // subscription. Re-evaluating here is what makes an expired
        // subscription collapse the principal back to baseline immediately
        // instead of waiting for the API-key cache TTL.
        if let Some(gid) = group_id
            && !self.group_entitled(row.user_id, gid).await
        {
            group_id = None;
            rate_mult = 1.0;
        }

        Ok(AccessMetadata {
            user_id: row.user_id,
            api_key_id: row.id,
            group_id,
            rate_mult,
            subscription: self.active_subscription(row.user_id).await,
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
        if !self.user_is_active(claims.user_id).await {
            return Err(AuthError::InvalidCredential);
        }

        Ok(AccessMetadata {
            user_id: claims.user_id,
            api_key_id: 0,
            group_id: None,
            rate_mult: 1.0,
            subscription: self.active_subscription(claims.user_id).await,
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
    async fn group_entitled(&self, user_id: Id, group_id: Id) -> bool {
        if user_id == 0 || group_id == 0 {
            return false;
        }
        match self.directory.group_rate_multiplier(group_id).await {
            // The baseline group is implicitly held by every active user, so
            // there is no subscription to consult.
            Ok(Some(1.0)) => true,
            Ok(Some(_)) => self
                .directory
                .holds_group_entitlement(user_id, group_id)
                .await
                .unwrap_or(false),
            // Missing group row or transient error: baseline wins.
            _ => false,
        }
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
    let path = req.uri().path().to_owned();
    if !is_proxy_path(&path) {
        return next.run(req).await;
    }

    // Owned, because resolving it borrows the request and inserting the result
    // needs it back mutably.
    let credential = credential_from(&path, req.headers(), req.uri().query()).map(str::to_owned);

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
