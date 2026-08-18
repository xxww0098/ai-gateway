//! `/oauth-*` — starting, polling and completing an upstream OAuth flow.
//!
//! 对应 `SDKMgmtOAuthSessionsHandler`、`SDKMgmtOAuthCallbackHandler`、
//! `SDKMgmtSDKOAuthCallbackHandler`、`SDKMgmtOAuthStatusHandler` 和
//! `sdkMgmtOAuthAuthURLHandler`。两个纯函数半层放在旁边：[`flow`] 构造
//! authorize URL，[`exchange`] 把 code 换成凭证。
//!
//! # The flow
//!
//! 1. `GET|POST /{provider}-auth-url` mints a `state`, builds the provider's
//!    authorize URL, and stores a **pending** `o_auth_sessions` row holding the
//!    PKCE verifier. The operator opens the URL.
//! 2. The provider redirects back to `/oauth-callback/{provider}` with `code`
//!    and `state`. That row is claimed, the code is exchanged for tokens, and
//!    an `AuthRecord` is written.
//! 3. `GET /get-auth-status?state=` is polled by the console throughout.
//!
//! The `state` is the only thing tying the browser round-trip together, so it
//! is generated server-side, stored, matched exactly, and expired after
//! [`SESSION_TTL`].
//!
//! # Two endpoints that are permanently unavailable
//!
//! `POST /oauth-callback` (no provider) and the `antigravity-`/`kimi-auth-url`
//! endpoints have no backend anymore. 这里保留了「没接上」分支 —— `503` for the
//! callback, `404` for the two auth-url keys —— 而不是返回一个控制台从没见过的新错误。

pub mod device;
pub mod exchange;
pub mod flow;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use exchange::{TokenResponse, exchange as exchange_code, oauth_record};
pub use flow::{Provider, build_authorize_url, redirect_uri};

use crate::{AdminUser, PanelState, err, ok};

#[cfg(test)]
mod tests;

/// How long an operator has to finish the browser round-trip.
/// 对应 `sdkMgmtOAuthSessionTTL`。
pub const SESSION_TTL: Duration = Duration::minutes(10);

const ERR_BAD_REQUEST: i32 = 4001;
const ERR_SESSION: i32 = 4002;
const ERR_UNKNOWN_PROVIDER: i32 = 4040;
const ERR_REGISTER_FAILED: i32 = 5001;
const ERR_EXCHANGE_FAILED: i32 = 5021;
const ERR_LOAD_FAILED: i32 = 5003;
const ERR_CREATE_FAILED: i32 = 5004;
const ERR_UNAVAILABLE: i32 = 5031;

/// The `config_data` blob on a pending session.
/// 对应 `sdkMgmtOAuthSessionConfig`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionConfig {
    #[serde(default)]
    pub provider: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider_alias: String,
    #[serde(default)]
    pub endpoint_key: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub redirect_uri: String,
    /// PKCE secret. Never leaves the row — the callback needs it, nothing else
    /// does.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code_verifier: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code_challenge_method: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub expires_at: String,
    /// `device` | `authorization_code` | `idc` | `import`. Empty = PKCE.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub flow: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub device_code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user_code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub verification_uri: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub verification_uri_complete: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub interval: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token_endpoint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_secret: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub auth_method: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub start_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub region: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_poll_at: String,
}

fn is_zero_i64(value: &i64) -> bool {
    *value == 0
}

// ---------------------------------------------------------------- sessions

#[derive(Debug, sqlx::FromRow)]
struct SessionRow {
    id: i64,
    provider: String,
    status: Option<String>,
    auth_id: Option<String>,
    config_data: Option<Value>,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

/// Status of a session that has not been claimed yet.
const STATUS_PENDING: &str = "pending";
const STATUS_COMPLETED: &str = "completed";
const STATUS_FAILED: &str = "failed";

impl SessionRow {
    fn status(&self) -> &str {
        self.status.as_deref().unwrap_or_default()
    }

    /// 对应 `sdkMgmtSerializeOAuthSession` —— a pending row past its expiry is
    /// *displayed* as failed even before the sweeper has rewritten it, so the
    /// console never shows a session the callback would reject.
    fn to_json(&self) -> Value {
        let status = if self.status() == STATUS_PENDING && Utc::now() > self.expires_at {
            STATUS_FAILED
        } else {
            self.status()
        };
        json!({
            "id": self.id,
            "provider": self.provider,
            "status": status,
            "auth_id": self.auth_id,
            "created_at": rfc3339(self.created_at),
            "expires_at": rfc3339(self.expires_at),
        })
    }
}

/// RFC3339 in UTC, second precision — the format every timestamp in this
/// domain's payloads uses.
pub(super) fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

const SESSION_COLUMNS: &str = "id, provider, status, auth_id, config_data, created_at, expires_at";

/// `GET /oauth-sessions`. 对应 `SDKMgmtOAuthSessionsHandler`。
///
/// Sweeps expired pending rows first, so the list and the callback agree about
/// what is still claimable.
pub async fn list_sessions(State(state): State<PanelState>, _admin: AdminUser) -> Response {
    sweep_expired(&state).await;

    let rows: Result<Vec<SessionRow>, _> = sqlx::query_as(&format!(
        "SELECT {SESSION_COLUMNS} FROM o_auth_sessions \
         WHERE expires_at > $1 OR status IN ('completed', 'failed') \
         ORDER BY created_at DESC"
    ))
    .bind(Utc::now())
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => ok(json!({
            "sessions": rows.iter().map(SessionRow::to_json).collect::<Vec<_>>(),
        })),
        Err(error) => {
            tracing::error!(%error, "failed to list OAuth sessions");
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_LOAD_FAILED,
                "failed to list OAuth sessions",
            )
        }
    }
}

/// Marks every pending session past its expiry as failed.
/// 对应 `sdkMgmtCleanupExpiredOAuthSessions` —— best-effort.
async fn sweep_expired(state: &PanelState) {
    let result = sqlx::query(
        "UPDATE o_auth_sessions SET status = $1 WHERE status = $2 AND expires_at <= $3",
    )
    .bind(STATUS_FAILED)
    .bind(STATUS_PENDING)
    .bind(Utc::now())
    .execute(&state.pg)
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, "failed to expire stale OAuth sessions");
    }
}

/// Query string of `GET /get-auth-status`.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct StatusQuery {
    pub state: Option<String>,
}

/// `GET /get-auth-status`. 对应 `SDKMgmtOAuthStatusHandler`。
///
/// The console polls this while the operator is in the provider's browser tab.
/// Every answer is a `200` with a `status` — `wait`, `success`, `error` or
/// `missing` — because a poll that 404s would look like a broken endpoint.
pub async fn auth_status(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Query(query): Query<StatusQuery>,
) -> Response {
    let Some(oauth_state) = query
        .state
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "state is required",
        );
    };

    let row: Result<Option<SessionRow>, _> = sqlx::query_as(&format!(
        "SELECT {SESSION_COLUMNS} FROM o_auth_sessions WHERE state = $1"
    ))
    .bind(oauth_state)
    .fetch_optional(&state.pg)
    .await;

    let row = match row {
        Ok(Some(row)) => row,
        // 这里查不到会话时返回 `missing`（不存在与查询失败都落到同一分支）。
        Ok(None) => return ok(json!({"status": "missing"})),
        Err(error) => {
            tracing::error!(%error, "failed to load OAuth session");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_LOAD_FAILED,
                "failed to load OAuth session",
            );
        }
    };

    if row.status() == STATUS_PENDING && Utc::now() > row.expires_at {
        mark_session(&state, row.id, STATUS_FAILED, None).await;
        return ok(json!({"status": "error", "message": "OAuth session expired"}));
    }
    match row.status() {
        STATUS_COMPLETED => ok(json!({
            "status": "success",
            "provider": row.provider,
            "auth_id": row.auth_id,
        })),
        STATUS_FAILED => ok(json!({"status": "error", "provider": row.provider})),
        _ => {
            let config = session_config_of(&row);
            if device::is_device_flow(&config)
                && let Some(provider) = Provider::parse(&row.provider)
            {
                return finish_device_poll(&state, provider, &row, config, true).await;
            }
            ok(device_wait_payload(&row.provider, &config))
        }
    }
}

/// `POST /oauth-callback` — the SDK-delegated manual callback.
///
/// 原实现把它转发给 `sdkapi.ManagementTokenRequester`，缺少该协作者时返回
/// `503`。它现在已经永久缺位。
pub async fn sdk_callback(_admin: AdminUser) -> Response {
    err(
        StatusCode::SERVICE_UNAVAILABLE,
        ERR_UNAVAILABLE,
        "SDK OAuth callback is not available",
    )
}

// ---------------------------------------------------------------- start

/// `GET|POST /{provider}-auth-url`, dispatched from [`super::providers`].
///
/// 对应 `sdkMgmtHandleAuthURLEndpoint` → `sdkMgmtOAuthAuthURLHandler`。
///
/// `body` is ignored for Gemini / Claude / Codex. xAI always starts a device
/// flow. Kiro reads `method` (`device` / `authcode` / `idc` / `import`).
pub async fn auth_url(
    state: &PanelState,
    headers: &HeaderMap,
    endpoint: &str,
    body: Option<&Value>,
) -> Response {
    let Some(provider) = Provider::from_auth_url_key(endpoint) else {
        return err(
            StatusCode::NOT_FOUND,
            ERR_UNKNOWN_PROVIDER,
            "unknown auth-url provider",
        );
    };

    let oauth_state = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let mut expires_at = now + SESSION_TTL;
    let mut config = SessionConfig {
        provider: provider.as_str().to_owned(),
        endpoint_key: endpoint.trim().to_owned(),
        state: oauth_state.clone(),
        redirect_uri: redirect_uri(headers, provider),
        created_at: rfc3339(now),
        expires_at: rfc3339(expires_at),
        ..SessionConfig::default()
    };
    if endpoint.trim() == "anthropic-auth-url" {
        // Recorded so the row still says which key the operator used, even
        // though the credential is stored under `claude`.
        config.provider_alias = "anthropic".to_owned();
    }

    match provider {
        Provider::Xai => {
            let started = match device::start_xai_device(&mut config).await {
                Ok(started) => started,
                Err(error) => {
                    tracing::warn!(%error, "xAI device authorization failed");
                    return err(
                        StatusCode::BAD_GATEWAY,
                        ERR_EXCHANGE_FAILED,
                        "failed to start xAI device login",
                    );
                }
            };
            expires_at = now + device::device_session_ttl(started.expires_in);
            config.expires_at = rfc3339(expires_at);
            if let Err(response) = persist_session(state, provider, &oauth_state, &started.verification_uri_complete, &config, now, expires_at).await {
                return response;
            }
            return ok(device::device_start_payload(&oauth_state, &started));
        }
        Provider::Kiro => {
            let start = device::KiroStartBody::from_value(body);
            match start.method_key() {
                "import" => {
                    let tokens = match device::parse_kiro_import(&start.token) {
                        Ok(tokens) => tokens,
                        Err(error) => {
                            return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, error.to_string());
                        }
                    };
                    return persist_imported(state, provider, tokens).await;
                }
                "authcode" => {
                    let authorize_url = match device::start_kiro_authcode(&mut config, &start).await {
                        Ok(url) => url,
                        Err(error) => {
                            tracing::warn!(%error, "Kiro authorization-code start failed");
                            return err(
                                StatusCode::BAD_GATEWAY,
                                ERR_EXCHANGE_FAILED,
                                "failed to start Kiro login",
                            );
                        }
                    };
                    if let Err(response) = persist_session(state, provider, &oauth_state, &authorize_url, &config, now, expires_at).await {
                        return response;
                    }
                    return ok(json!({
                        "auth_url": authorize_url,
                        "url": authorize_url,
                        "state": oauth_state,
                        "flow": "authorization_code",
                    }));
                }
                _ => {
                    let started = match device::start_kiro_device(&mut config, &start).await {
                        Ok(started) => started,
                        Err(error) => {
                            tracing::warn!(%error, "Kiro device authorization failed");
                            return err(
                                StatusCode::BAD_GATEWAY,
                                ERR_EXCHANGE_FAILED,
                                "failed to start Kiro device login",
                            );
                        }
                    };
                    expires_at = now + device::device_session_ttl(started.expires_in);
                    config.expires_at = rfc3339(expires_at);
                    if let Err(response) = persist_session(state, provider, &oauth_state, &started.verification_uri_complete, &config, now, expires_at).await {
                        return response;
                    }
                    return ok(device::device_start_payload(&oauth_state, &started));
                }
            }
        }
        _ => {}
    }

    let Ok(authorize_url) = build_authorize_url(provider, &oauth_state, &mut config) else {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            ERR_CREATE_FAILED,
            "failed to create OAuth URL",
        );
    };
    if let Err(response) = persist_session(state, provider, &oauth_state, &authorize_url, &config, now, expires_at).await {
        return response;
    }
    // Both key spellings: the console reads `auth_url`, older callers `url`.
    ok(json!({"auth_url": authorize_url, "url": authorize_url, "state": oauth_state}))
}

/// `POST /oauth-device-poll/{provider}` — one token poll for a device session.
pub async fn device_poll(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(provider): Path<String>,
    body: Option<axum::Json<Value>>,
) -> Response {
    let Some(provider) = Provider::parse(&provider) else {
        return err(
            StatusCode::NOT_FOUND,
            ERR_UNKNOWN_PROVIDER,
            "unsupported OAuth provider",
        );
    };
    if !matches!(provider, Provider::Xai | Provider::Kiro) {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "provider does not use the device-code flow",
        );
    }
    let oauth_state = body
        .as_ref()
        .and_then(|axum::Json(value)| value.get("state"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
        .to_owned();
    if oauth_state.is_empty() {
        return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, "state is required");
    }

    let row = match load_session_by_state(&state, &oauth_state).await {
        Ok(row) => row,
        Err(response) => return response,
    };
    if row.provider != provider.as_str() {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_SESSION,
            "OAuth session provider mismatch",
        );
    }
    if row.status() == STATUS_COMPLETED {
        return ok(json!({
            "status": "success",
            "provider": row.provider,
            "auth_id": row.auth_id,
        }));
    }
    if row.status() == STATUS_FAILED || (row.status() == STATUS_PENDING && Utc::now() > row.expires_at) {
        if row.status() == STATUS_PENDING {
            mark_session(&state, row.id, STATUS_FAILED, None).await;
        }
        return ok(json!({"status": "error", "provider": row.provider, "message": "OAuth session expired"}));
    }
    let config = session_config_of(&row);
    finish_device_poll(&state, provider, &row, config, false).await
}

async fn persist_session(
    state: &PanelState,
    provider: Provider,
    oauth_state: &str,
    auth_url: &str,
    config: &SessionConfig,
    now: chrono::DateTime<Utc>,
    expires_at: chrono::DateTime<Utc>,
) -> Result<(), Response> {
    let encoded = serde_json::to_value(config).map_err(|_| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            ERR_CREATE_FAILED,
            "failed to create OAuth session",
        )
    })?;
    let inserted = sqlx::query(
        "INSERT INTO o_auth_sessions \
           (provider, state, auth_url, status, config_data, created_at, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(provider.as_str())
    .bind(oauth_state)
    .bind(auth_url)
    .bind(STATUS_PENDING)
    .bind(&encoded)
    .bind(now)
    .bind(expires_at)
    .execute(&state.pg)
    .await;
    if let Err(error) = inserted {
        tracing::error!(%error, "failed to store OAuth session");
        return Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            ERR_CREATE_FAILED,
            "failed to store OAuth session",
        ));
    }
    Ok(())
}

async fn persist_imported(
    state: &PanelState,
    provider: Provider,
    tokens: exchange::TokenResponse,
) -> Response {
    let record = oauth_record(provider, &tokens, Utc::now());
    if let Err(error) = state.auth_store.save(&record).await {
        tracing::error!(%error, "failed to persist imported OAuth credential");
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            ERR_REGISTER_FAILED,
            "failed to register OAuth auth",
        );
    }
    ok(json!({
        "status": "success",
        "message": "OAuth completed",
        "provider": provider.as_str(),
        "auth_id": record.id,
        "flow": "import",
    }))
}

fn session_config_of(row: &SessionRow) -> SessionConfig {
    row.config_data
        .clone()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn device_wait_payload(provider: &str, config: &SessionConfig) -> Value {
    json!({
        "status": "wait",
        "provider": provider,
        "user_code": config.user_code,
        "verification_uri": config.verification_uri,
        "verification_uri_complete": config.verification_uri_complete,
        "interval": config.interval,
        "flow": if config.flow.is_empty() { "device" } else { config.flow.as_str() },
    })
}

async fn load_session_by_state(
    state: &PanelState,
    oauth_state: &str,
) -> Result<SessionRow, Response> {
    let row: Option<SessionRow> = sqlx::query_as(&format!(
        "SELECT {SESSION_COLUMNS} FROM o_auth_sessions WHERE state = $1"
    ))
    .bind(oauth_state)
    .fetch_optional(&state.pg)
    .await
    .map_err(|error| {
        tracing::error!(%error, "failed to load OAuth session");
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            ERR_LOAD_FAILED,
            "failed to load OAuth session",
        )
    })?;
    row.ok_or_else(|| {
        err(
            StatusCode::BAD_REQUEST,
            ERR_SESSION,
            "OAuth session not found",
        )
    })
}

async fn finish_device_poll(
    state: &PanelState,
    provider: Provider,
    row: &SessionRow,
    mut config: SessionConfig,
    skip_if_early: bool,
) -> Response {
    let now = Utc::now();
    if skip_if_early && !device::interval_elapsed(&config, now) {
        return ok(device_wait_payload(provider.as_str(), &config));
    }

    let outcome = match device::poll_provider_token(provider, &config).await {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::warn!(%error, provider = provider.as_str(), "device token poll failed");
            return ok(device_wait_payload(provider.as_str(), &config));
        }
    };

    match outcome {
        device::DevicePollOutcome::Pending { interval } => {
            device::mark_polled(&mut config, now, interval);
            let _ = store_config(state, row.id, &config).await;
            ok(device_wait_payload(provider.as_str(), &config))
        }
        device::DevicePollOutcome::SlowDown { interval } => {
            device::mark_polled(&mut config, now, interval);
            let _ = store_config(state, row.id, &config).await;
            ok(device_wait_payload(provider.as_str(), &config))
        }
        device::DevicePollOutcome::Completed(tokens) => {
            let record = oauth_record(provider, &tokens, now);
            if let Err(error) = state.auth_store.save(&record).await {
                tracing::error!(%error, "failed to persist OAuth credential");
                mark_session(state, row.id, STATUS_FAILED, None).await;
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ERR_REGISTER_FAILED,
                    "failed to register OAuth auth",
                );
            }
            mark_session(state, row.id, STATUS_COMPLETED, Some(&record.id)).await;
            ok(json!({
                "status": "success",
                "message": "OAuth completed",
                "provider": provider.as_str(),
                "auth_id": record.id,
            }))
        }
        device::DevicePollOutcome::Failed { error, description } => {
            mark_session(state, row.id, STATUS_FAILED, None).await;
            ok(json!({
                "status": "error",
                "provider": provider.as_str(),
                "message": if description.is_empty() { error } else { description },
            }))
        }
    }
}

async fn store_config(state: &PanelState, id: i64, config: &SessionConfig) -> Result<(), ()> {
    let Ok(encoded) = serde_json::to_value(config) else {
        return Err(());
    };
    let result = sqlx::query("UPDATE o_auth_sessions SET config_data = $1 WHERE id = $2")
        .bind(&encoded)
        .bind(id)
        .execute(&state.pg)
        .await;
    if let Err(error) = result {
        tracing::warn!(%error, id, "failed to update OAuth session config");
        return Err(());
    }
    Ok(())
}

// ---------------------------------------------------------------- callback

/// Body of `POST /oauth-callback/{provider}`, when the console posts JSON.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct CallbackBody {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub state: String,
}

/// Query string of the same route, when the provider redirects.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
}

/// `POST /oauth-callback/{provider}`. 对应 `SDKMgmtOAuthCallbackHandler`。
///
/// Claims the pending session, exchanges the code, and stores the credential.
/// A failed exchange marks the session failed rather than leaving it pending,
/// so the console's poll resolves instead of spinning until the TTL.
pub async fn callback(
    State(state): State<PanelState>,
    _admin: AdminUser,
    Path(provider): Path<String>,
    Query(query): Query<CallbackQuery>,
    body: Option<axum::Json<CallbackBody>>,
) -> Response {
    let Some(provider) = Provider::parse(&provider) else {
        return err(
            StatusCode::NOT_FOUND,
            ERR_UNKNOWN_PROVIDER,
            "unsupported OAuth provider",
        );
    };

    // Query string first, then the JSON body. 原实现还读 form body；axum
    // 一个 handler 不能同时有两个 extractor，而控制台发的是 JSON。
    let body = body.map(|axum::Json(body)| body).unwrap_or_default();
    let code = first_non_empty(query.code.as_deref(), &body.code);
    let oauth_state = first_non_empty(query.state.as_deref(), &body.state);
    if code.is_empty() || oauth_state.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "OAuth code and state are required",
        );
    }

    let (session, config) = match load_pending(&state, provider, &oauth_state).await {
        Ok(pair) => pair,
        Err(response) => return response,
    };

    let tokens = match exchange_code(provider, &code, &config).await {
        Ok(tokens) => tokens,
        Err(error) => {
            tracing::warn!(%error, provider = provider.as_str(), "OAuth token exchange failed");
            mark_session(&state, session.id, STATUS_FAILED, None).await;
            return err(
                StatusCode::BAD_GATEWAY,
                ERR_EXCHANGE_FAILED,
                "OAuth token exchange failed",
            );
        }
    };

    let record = oauth_record(provider, &tokens, Utc::now());
    if let Err(error) = state.auth_store.save(&record).await {
        tracing::error!(%error, "failed to persist OAuth credential");
        mark_session(&state, session.id, STATUS_FAILED, None).await;
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            ERR_REGISTER_FAILED,
            "failed to register OAuth auth",
        );
    }
    mark_session(&state, session.id, STATUS_COMPLETED, Some(&record.id)).await;

    ok(json!({
        "message": "OAuth completed",
        "provider": provider.as_str(),
        "auth_id": record.id,
    }))
}

fn first_non_empty(query: Option<&str>, body: &str) -> String {
    let query = query.map(str::trim).unwrap_or_default();
    if query.is_empty() {
        body.trim().to_owned()
    } else {
        query.to_owned()
    }
}

/// Loads and validates the session a callback claims.
/// 对应 `sdkMgmtLoadPendingOAuthSession`。
///
/// Every rejection is a `400`: a callback that does not match a live session is
/// a client-side problem, whether the state is unknown, already used, expired,
/// or for a different provider.
async fn load_pending(
    state: &PanelState,
    provider: Provider,
    oauth_state: &str,
) -> Result<(SessionRow, SessionConfig), Response> {
    let row: Option<SessionRow> = sqlx::query_as(&format!(
        "SELECT {SESSION_COLUMNS} FROM o_auth_sessions WHERE state = $1"
    ))
    .bind(oauth_state)
    .fetch_optional(&state.pg)
    .await
    .map_err(|error| {
        tracing::error!(%error, "failed to load OAuth session");
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            ERR_LOAD_FAILED,
            "failed to load OAuth session",
        )
    })?;

    let row = row.ok_or_else(|| {
        err(
            StatusCode::BAD_REQUEST,
            ERR_SESSION,
            "OAuth session not found",
        )
    })?;

    let config: SessionConfig = row
        .config_data
        .clone()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();

    if row.provider != provider.as_str() || config.provider != provider.as_str() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            ERR_SESSION,
            "OAuth session provider mismatch",
        ));
    }
    if row.status() != STATUS_PENDING {
        return Err(err(
            StatusCode::BAD_REQUEST,
            ERR_SESSION,
            "OAuth session is not pending",
        ));
    }
    if Utc::now() > row.expires_at {
        mark_session(state, row.id, STATUS_FAILED, None).await;
        return Err(err(
            StatusCode::BAD_REQUEST,
            ERR_SESSION,
            "OAuth session expired",
        ));
    }
    if config.redirect_uri.trim().is_empty() && !device::is_device_flow(&config) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            ERR_SESSION,
            "OAuth session is incomplete",
        ));
    }
    Ok((row, config))
}

/// 对应 `sdkMgmtMarkOAuthSession` —— best-effort, never fails the request.
async fn mark_session(state: &PanelState, id: i64, status: &str, auth_id: Option<&str>) {
    let result = sqlx::query(
        "UPDATE o_auth_sessions SET status = $1, \
           auth_id = COALESCE($2::text, auth_id) WHERE id = $3",
    )
    .bind(status)
    .bind(auth_id)
    .bind(id)
    .execute(&state.pg)
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, id, "failed to update OAuth session status");
    }
}
