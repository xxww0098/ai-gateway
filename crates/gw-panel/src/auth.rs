//! Panel authentication extractors.
//!
//! OWNER: worker `panel-upstream`. This is shared ground by *use* — every domain
//! writes `AuthUser`/`AdminUser` in its handler signatures — but it has a single
//! owner so the two credential paths cannot drift. `panel-identity` consumes it
//! and must not fork it (rule 1.9).
//!
//! The types, error bodies and codes are fixed in `crate` root
//! (`AuthUser`, `AdminUser`, `AuthRejection`, `codes`, `bearer_token`); what
//! belongs here is only the `FromRequestParts` wiring.
//!
//! 对应 `AuthMiddleware` 与 `requireAdmin`。Required behaviour:
//!
//! 1. No/blank bearer  -> [`AuthRejection::missing_bearer`].
//! 2. Token starts `cpa-` -> API-key path：
//!    `hash_api_key` -> `ApiKeyCache` -> DB. A cached key whose `Status` is
//!    already non-active is rejected **before** any user lookup —— 已经被判定
//!    失效的 key 不需要再查 DB 就能知道它失效。
//! 3. Otherwise -> `validate_jwt`，然后 recheck
//!    `token_version` so a global logout invalidates already-issued tokens.
//! 4. Both paths -> user-status recheck through the **shared**
//!    `UserStatusCache`, so a suspension observed on `/v1/*` is honored here
//!    immediately and vice versa.
//! 5. Every failure returns the same opaque [`AuthRejection::invalid_credentials`];
//!    never reveal which check failed.
//! 6. `AdminUser` additionally requires `role = 'admin'` AND `status = 'active'`,
//!    rejecting with [`AuthRejection::not_admin`] (403) or
//!    [`AuthRejection::admin_check_failed`] (500) when the lookup itself errors.
//!
//! # Two departures from a literal transcription, both deliberate
//!
//! * 旧中间件只在 API-key 路径填 `BillingCtx.Email`，JWT 路径留空（JWT 的
//!   email claim 是「展示数据」，不是授权来源）。这个 extractor 从 `users` 行
//!   读 `email` 和 `role`（反正 status recheck 本来就要触达这行），所以
//!   `AuthUser.email` 永远是可信的 DB 值。Nothing authorizes on it; `role` is
//!   what `AdminUser` gates on, and it comes from the same row `requireAdmin`
//!   used to re-query.
//! * 旧实现在一个 detached goroutine 里更新 `api_keys.last_used_at`。That write
//!   belongs to whoever owns key lifecycle, not to an authentication extractor
//!   that runs on every panel request; the `/v1/*` access provider in `gw-proxy`
//!   already maintains the column.

use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::Utc;
use gw_infra::CachedKey;

use crate::{AdminUser, AuthRejection, AuthUser, PanelState, bearer_token};

#[cfg(test)]
mod tests;

/// `users.status` a caller must be in. 对应 `userStatusActive`。
const STATUS_ACTIVE: &str = "active";

/// How long a `users.status` lookup stays cached（对应 `userIsActive` 传给
/// `UserStatusCache.Set` 的 `5*time.Minute`）。
const USER_STATUS_TTL: Duration = Duration::from_secs(5 * 60);

/// How long a resolved API key stays cached（对应 `validateAPIKey` 构造的
/// `infra.CachedKey` 上的 `time.Now().Add(5*time.Minute)`）。
const API_KEY_TTL: Duration = Duration::from_secs(5 * 60);

/// Cache placeholder for "no such user". 缓存同一个哨兵，让对同一伪造 id 的
/// 凭证猜测洪峰不会打到 DB。
const STATUS_MISSING: &str = "missing";

/// The identity columns every authenticated request needs.
///
/// One query serves three 调用点：the status recheck (`userIsActive`),
/// the admin gate (`userHasAdminRole`) and the email the audit log records.
/// The columns are nullable in the既有 schema，旧实现把 NULL 填成
/// the zero value; `Option<String>` + the accessors below reproduce that
/// without letting a NULL fail the whole decode (CONTRACT §3.5).
#[derive(Debug, Default, sqlx::FromRow)]
struct Identity {
    email: Option<String>,
    role: Option<String>,
    status: Option<String>,
}

impl Identity {
    fn status(&self) -> &str {
        self.status.as_deref().unwrap_or_default()
    }

    fn email(self) -> String {
        self.email.unwrap_or_default()
    }

    /// Trimmed and lowercased, so the `AuthUser::is_admin` equality test is the
    /// case-insensitive comparison（对应 `strings.EqualFold`）。
    fn role(&self) -> String {
        self.role
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_lowercase()
    }
}

impl FromRequestParts<PanelState> for AuthUser {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &PanelState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers).ok_or_else(AuthRejection::missing_bearer)?;

        // Two credential kinds share this surface. The `cpa-` prefix is the
        // discriminator —— an API key is never fed to the JWT validator and vice
        // versa.
        let mut user = if is_api_key_token(token) {
            authenticate_api_key(state, token).await?
        } else {
            authenticate_jwt(state, token).await?
        };

        // Status recheck (both paths). The primary credential check may have
        // succeeded against a stale cache entry or against a JWT minted before
        // the owning user was suspended.
        let identity = load_identity(state, user.user_id).await?;
        user.role = identity.role();
        user.email = identity.email();
        Ok(user)
    }
}

impl FromRequestParts<PanelState> for AdminUser {
    type Rejection = AuthRejection;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &PanelState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        // `AuthUser` already proved the row is active; the role is the only
        // thing left to check. 旧实现在这里再查一次，所以 `requireAdmin` 有自己
        // 的 500 分支 —— kept, because a caller that reaches this
        // extractor with a torn `role` must not be silently treated as a
        // non-admin.
        if !user.is_admin() {
            return Err(AuthRejection::not_admin());
        }
        Ok(Self(user))
    }
}

/// Which of the two credential kinds a bearer token is.
///
/// 对应 `strings.HasPrefix(token, "cpa-")`。The prefix itself is owned by
/// `gw_authcore`, so minting and routing cannot drift.
#[must_use]
fn is_api_key_token(token: &str) -> bool {
    token.starts_with(gw_authcore::API_KEY_PREFIX)
}

/// The `cpa-`-prefixed path. 对应 `PanelRouter.validateAPIKey` 及其后的
/// `bc.Status != active` 短路。
async fn authenticate_api_key(state: &PanelState, token: &str) -> Result<AuthUser, AuthRejection> {
    let key_hash = gw_authcore::hash_api_key(token);

    let cached = match state.api_key_cache.get(&key_hash) {
        Some(cached) => cached,
        None => {
            let resolved = resolve_api_key(state, &key_hash).await?;
            state.api_key_cache.set(key_hash, resolved.clone());
            std::sync::Arc::new(resolved)
        }
    };

    // Rejected before any user lookup: a key the admin disabled between cache
    // populations is dead on its own terms.
    if cached.status != STATUS_ACTIVE {
        tracing::info!(
            event = "user_inactive",
            auth_type = "api_key",
            user_id = cached.user_id,
            api_key_status = %cached.status,
            "panel_auth_rejected_inactive_api_key"
        );
        return Err(AuthRejection::invalid_credentials());
    }

    Ok(AuthUser {
        user_id: cached.user_id,
        // Filled in by the caller from the `users` row.
        email: String::new(),
        role: String::new(),
        api_key_id: Some(cached.api_key_id),
        group_id: cached.group_id,
        rate_multiplier: cached.rate_mult,
    })
}

/// Cache miss: read the key row and its group multiplier.
///
/// 对应 `Where("key_hash = ? AND status = ?", keyHash, "active")` 的查询。
/// The status filter is part of the WHERE clause, so a disabled key is
/// indistinguishable from a nonexistent one —— that is intentional.
async fn resolve_api_key(state: &PanelState, key_hash: &str) -> Result<CachedKey, AuthRejection> {
    let row: Option<(i64, i64, Option<i64>, String)> = sqlx::query_as(
        "SELECT id, user_id, group_id, status FROM api_keys \
         WHERE key_hash = $1 AND status = 'active'",
    )
    .bind(key_hash)
    .fetch_optional(&state.pg)
    .await
    .map_err(|error| {
        tracing::error!(%error, "api key lookup failed");
        AuthRejection::invalid_credentials()
    })?;

    let (api_key_id, user_id, group_id, status) =
        row.ok_or_else(AuthRejection::invalid_credentials)?;

    // A missing or unreadable group leaves the multiplier at 1.0 rather than
    // failing the request（对标 `if err == nil { rateMult = ... }`）。
    let rate_mult = match group_id {
        Some(group_id) => {
            sqlx::query_scalar::<_, f64>("SELECT rate_multiplier::float8 FROM groups WHERE id = $1")
                .bind(group_id)
                .fetch_optional(&state.pg)
                .await
                .ok()
                .flatten()
                .unwrap_or(1.0)
        }
        None => 1.0,
    };

    Ok(CachedKey {
        user_id,
        api_key_id,
        group_id,
        rate_mult,
        status,
        expires_at: Utc::now()
            + chrono::TimeDelta::from_std(API_KEY_TTL).unwrap_or(chrono::TimeDelta::zero()),
    })
}

/// The JWT path. 对应 `authutil.ValidateJWT` 及 token-version recheck。
async fn authenticate_jwt(state: &PanelState, token: &str) -> Result<AuthUser, AuthRejection> {
    let claims = gw_authcore::validate_jwt(token, &state.cfg.auth.jwt.secret)
        .map_err(|_| AuthRejection::invalid_credentials())?;

    // Session revocation. Fail CLOSED on a DB error so a transient blip cannot
    // let a revoked token slip through（对标 `verErr != nil || jwtTokenVersion
    // < curVer`）。
    let current = gw_authcore::TokenVersionStore::new(state.pg.clone())
        .current(claims.user_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, "token version lookup failed");
            AuthRejection::invalid_credentials()
        })?;
    if claims.is_revoked(current) {
        tracing::info!(
            event = "token_revoked",
            user_id = claims.user_id,
            token_version = claims.token_version,
            current_version = current,
            "panel_auth_rejected_revoked_jwt"
        );
        return Err(AuthRejection::invalid_credentials());
    }

    Ok(AuthUser {
        user_id: claims.user_id,
        email: String::new(),
        role: String::new(),
        api_key_id: None,
        group_id: None,
        rate_multiplier: 1.0,
    })
}

/// Confirms the user row is active and returns the identity columns.
///
/// 对应 `PanelRouter.userIsActive`（cache，然后单行读，正负结果都缓存）
/// merged with `userHasAdminRole`'s `Select("role", "status")` —— 一个查询
/// where 旧实现 had two, because every caller of this needs both halves.
///
/// The shared [`UserStatusCache`](gw_infra::UserStatusCache) is used in both
/// directions: a cached *non-active* status rejects without touching the DB
/// (that is the path a suspension on `/v1/*` takes to reach the panel), while a
/// cached *active* status still falls through, because the role and email are
/// not cached and are needed regardless.
///
/// A transient DB error is **not** cached: a connectivity blip must not pin a
/// live user into "inactive" for the full TTL.
async fn load_identity(state: &PanelState, user_id: i64) -> Result<Identity, AuthRejection> {
    if user_id == 0 {
        return Err(AuthRejection::invalid_credentials());
    }

    if let Some(cached) = state.user_status_cache.get(user_id)
        && cached.status != STATUS_ACTIVE
    {
        tracing::info!(
            event = "user_inactive",
            user_id,
            source = "cache",
            "panel_auth_rejected_inactive_user"
        );
        return Err(AuthRejection::invalid_credentials());
    }

    let row: Option<Identity> =
        sqlx::query_as("SELECT email, role, status FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&state.pg)
            .await
            .map_err(|error| {
                tracing::error!(%error, "user identity lookup failed");
                AuthRejection::invalid_credentials()
            })?;

    // 旧实现把缺失行读成零值；the cache sentinel keeps
    // "no such user" distinguishable from "active" while still being cacheable,
    // so a burst against one bogus id does not hammer the DB.
    let identity = row.unwrap_or_default();
    let status = identity.status();
    state.user_status_cache.set(
        user_id,
        if status.is_empty() {
            STATUS_MISSING
        } else {
            status
        },
        USER_STATUS_TTL,
    );

    if status != STATUS_ACTIVE {
        tracing::info!(
            event = "user_inactive",
            user_id,
            source = "db",
            "panel_auth_rejected_inactive_user"
        );
        return Err(AuthRejection::invalid_credentials());
    }
    Ok(identity)
}
