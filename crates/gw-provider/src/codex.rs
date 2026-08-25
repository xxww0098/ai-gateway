//! Codex / OpenAI-OAuth upstream.
//!
//! OWNER: worker `provider-openai`.
//!
//! Codex speaks the OpenAI chat-completions wire format but authenticates with
//! a rotating OAuth access token instead of a static API key. This module owns
//! the whole HTTP lifecycle and treats persisted credential records as data
//! only, with [`gw_authcore::AuthRecord`] as the record.

use crate::common::{
    PROVIDER_CODEX, ProviderConfig, chat_completions_endpoint, ensure_include_usage, nested_string,
    relay_timeouts, request_surface, resolve_timeout, responses_endpoint, string_from_map,
    upstream_dialect,
};
use crate::route::{RoutePlan, RoutePlanner};
use crate::types::{ProviderError, ProviderRequest};
use chrono::{SecondsFormat, Utc};
use gw_authcore::{AuthRecord, AuthStatus};
use gw_relay::{Credential, Surface};
use http::header::{ACCEPT, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::time::Duration;

pub const CODEX_DEFAULT_BASE_URL: &str = "https://api.openai.com";
pub const CODEX_OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const CODEX_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Credential-metadata key holding the bearer token. Exported because the auth
/// store seeds it.
pub const CODEX_METADATA_ACCESS_TOKEN: &str = "access_token";

const CODEX_METADATA_API_KEY: &str = "api_key";
const CODEX_METADATA_REFRESH_TOKEN: &str = "refresh_token";
const CODEX_METADATA_TOKEN_DATA: &str = "token_data";
const CODEX_METADATA_EXPIRES_AT: &str = "expires_at";
const CODEX_METADATA_EXPIRED: &str = "expired";
const CODEX_METADATA_LAST_REFRESH: &str = "last_refresh";
const CODEX_METADATA_ID_TOKEN: &str = "id_token";

#[derive(Debug, Default, Deserialize)]
struct CodexRefreshResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    id_token: String,
    #[serde(default)]
    expires_in: i64,
}

/// Executor for Codex / OpenAI OAuth credentials.
#[derive(Debug, Clone)]
pub struct CodexProvider {
    base_url: String,
    access_token: String,
    timeout: Duration,
}

impl CodexProvider {
    /// Builds an executor from provider config.
    ///
    /// Unlike the OpenAI executor an empty base URL is not an error — it falls
    /// back to [`CODEX_DEFAULT_BASE_URL`] — and an empty token is tolerated at
    /// construction because the real credential usually arrives per-request on
    /// the auth record.
    pub fn new(cfg: &ProviderConfig, timeout_seconds: i64) -> Result<Self, ProviderError> {
        let mut base_url = cfg.base_url.trim().trim_end_matches('/').to_owned();
        if base_url.is_empty() {
            base_url = CODEX_DEFAULT_BASE_URL.to_owned();
        }
        let parsed = url::Url::parse(&base_url)
            .map_err(|_| ProviderError::Other(anyhow::anyhow!("invalid codex base_url")))?;
        if parsed.host_str().unwrap_or_default().is_empty() {
            return Err(ProviderError::Other(anyhow::anyhow!(
                "invalid codex base_url"
            )));
        }
        Ok(Self {
            base_url,
            access_token: cfg.api_key.trim().to_owned(),
            timeout: resolve_timeout(timeout_seconds),
        })
    }

    /// The configured upstream base URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The config-level access token used to seed auth records.
    #[must_use]
    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// Walks the credential cascade for a usable bearer token and base URL.
    ///
    /// The token is looked up in four places, in priority order, because
    /// credentials imported from different Codex CLI versions nest it
    /// differently:
    ///
    /// 1. `metadata.access_token`
    /// 2. `metadata.token_data.access_token`
    /// 3. `metadata.api_key`
    /// 4. `metadata.token_data.api_key`
    ///
    /// [`AuthRecord`] has no separate storage field — everything lives in
    /// `metadata` — so the cascade ends at four.
    fn resolve_credentials(&self, auth: &AuthRecord) -> (String, String) {
        let mut base_url = self.base_url.trim().to_owned();
        if base_url.is_empty() {
            base_url = CODEX_DEFAULT_BASE_URL.to_owned();
        }
        for key in ["base_url", "base-url"] {
            if let Some(value) = auth.attributes.get(key) {
                let value = value.trim().trim_end_matches('/');
                if !value.is_empty() {
                    base_url = value.to_owned();
                    break;
                }
            }
        }

        let token = string_from_map(&auth.metadata, CODEX_METADATA_ACCESS_TOKEN)
            .or_else(|| {
                nested_string(
                    &auth.metadata,
                    CODEX_METADATA_TOKEN_DATA,
                    CODEX_METADATA_ACCESS_TOKEN,
                )
            })
            .or_else(|| string_from_map(&auth.metadata, CODEX_METADATA_API_KEY))
            .or_else(|| {
                nested_string(
                    &auth.metadata,
                    CODEX_METADATA_TOKEN_DATA,
                    CODEX_METADATA_API_KEY,
                )
            })
            .unwrap_or_else(|| self.access_token.trim().to_owned());
        (token, base_url)
    }

    /// Reads the refresh token from either nesting level.
    fn resolve_refresh_token(auth: &AuthRecord) -> Option<String> {
        string_from_map(&auth.metadata, CODEX_METADATA_REFRESH_TOKEN).or_else(|| {
            nested_string(
                &auth.metadata,
                CODEX_METADATA_TOKEN_DATA,
                CODEX_METADATA_REFRESH_TOKEN,
            )
        })
    }

    /// Plans an outbound chat-completions / responses request.
    ///
    /// 端点由**入口**决定（缺陷 #1），不由 provider 名或 model 名猜。
    fn plan_request(
        &self,
        req: &ProviderRequest,
        access_token: &str,
        base_url: &str,
    ) -> Result<RoutePlan, ProviderError> {
        if access_token.is_empty() {
            return Err(ProviderError::Credential(
                "codex access token is required".to_owned(),
            ));
        }
        let surface = request_surface(req);
        let endpoint = match surface {
            Surface::OpenAiResponses => responses_endpoint(base_url, &req.query)?,
            Surface::OpenAiCompletions | Surface::AnthropicMessages => {
                chat_completions_endpoint(base_url, &req.query)?
            }
        };
        let endpoint = url::Url::parse(&endpoint).map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("invalid codex endpoint: {err}"))
        })?;
        // Force the terminal usage envelope on streams, but only after
        // re-verifying `stream: true` in the body itself. `None` 表示一个字节都不动。
        let body = if req.stream {
            RoutePlan::splice(ensure_include_usage(&req.payload, surface))
        } else {
            None
        };

        let mut headers = HeaderMap::new();
        if !req.headers.contains_key(CONTENT_TYPE) {
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        }
        if req.stream {
            headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
        } else if !req.headers.contains_key(ACCEPT) {
            headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        }

        Ok(RoutePlan {
            provider: PROVIDER_CODEX,
            endpoint,
            credential: Credential::Bearer(access_token.to_owned()),
            headers,
            body,
            timeouts: relay_timeouts(self.timeout),
            dialect: upstream_dialect(surface),
        })
    }

    /// Exchanges a refresh token for a fresh access token.
    async fn refresh_oauth_token(
        &self,
        refresh_token: &str,
    ) -> Result<CodexRefreshResponse, ProviderError> {
        let payload = crate::oauth::post_form(
            CODEX_OAUTH_TOKEN_URL,
            self.timeout,
            "codex",
            &[
                ("client_id", CODEX_OAUTH_CLIENT_ID),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("scope", "openid profile email"),
            ],
        )
        .await?;
        serde_json::from_slice(&payload).map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("parsing codex refresh response: {err}"))
        })
    }
}

/// Extracts the `model` field from a JSON request body.
///
/// Returns an empty string when the body is empty, not valid JSON, or lacks a
/// *string* `model` field — a numeric, null or array `model` is treated as
/// absent.
#[must_use]
pub fn codex_model_from_body(body: &[u8]) -> String {
    if body.is_empty() {
        return String::new();
    }
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return String::new();
    };
    match value.get("model") {
        Some(Value::String(model)) => model.trim().to_owned(),
        _ => String::new(),
    }
}

/// Merges a refresh response into the stored `token_data` blob.
///
/// The previous blob may be a nested object or a JSON document embedded in a
/// string, and unknown keys inside it are preserved untouched.
fn updated_token_data(
    raw: Option<&Value>,
    token: &CodexRefreshResponse,
    previous_refresh_token: &str,
    now_rfc3339: &str,
    expires_at: Option<&str>,
) -> Map<String, Value> {
    let mut data = match raw {
        Some(Value::Object(map)) => map.clone(),
        Some(Value::String(text)) => {
            serde_json::from_str::<Map<String, Value>>(text.trim()).unwrap_or_else(|_| Map::new())
        }
        _ => Map::new(),
    };
    if !token.access_token.is_empty() {
        data.insert(
            CODEX_METADATA_ACCESS_TOKEN.to_owned(),
            Value::String(token.access_token.clone()),
        );
    }
    let refresh_token = if token.refresh_token.is_empty() {
        previous_refresh_token
    } else {
        &token.refresh_token
    };
    if !refresh_token.is_empty() {
        data.insert(
            CODEX_METADATA_REFRESH_TOKEN.to_owned(),
            Value::String(refresh_token.to_owned()),
        );
    }
    if !token.id_token.is_empty() {
        data.insert(
            CODEX_METADATA_ID_TOKEN.to_owned(),
            Value::String(token.id_token.clone()),
        );
    }
    if let Some(expires_at) = expires_at {
        data.insert(
            CODEX_METADATA_EXPIRED.to_owned(),
            Value::String(expires_at.to_owned()),
        );
    }
    data.insert(
        CODEX_METADATA_LAST_REFRESH.to_owned(),
        Value::String(now_rfc3339.to_owned()),
    );
    data
}

#[async_trait::async_trait]
impl RoutePlanner for CodexProvider {
    fn name(&self) -> &'static str {
        PROVIDER_CODEX
    }

    async fn plan(
        &self,
        auth: &AuthRecord,
        req: &ProviderRequest,
    ) -> Result<RoutePlan, ProviderError> {
        let (access_token, base_url) = self.resolve_credentials(auth);
        self.plan_request(req, &access_token, &base_url)
    }

    /// Rotates the OAuth credential.
    ///
    /// A record with no refresh token is not an error — a config-seeded static
    /// token has nothing to rotate — so it is only re-marked healthy.
    async fn refresh(&self, auth: &AuthRecord) -> Result<AuthRecord, ProviderError> {
        let mut refreshed = auth.clone();
        let now = Utc::now();
        let Some(previous_refresh_token) = Self::resolve_refresh_token(auth) else {
            refreshed.status = AuthStatus::Active;
            refreshed.updated_at = now;
            return Ok(refreshed);
        };

        let token = self.refresh_oauth_token(&previous_refresh_token).await?;
        let now_rfc3339 = now.to_rfc3339_opts(SecondsFormat::Secs, true);
        let expires_at = (token.expires_in > 0).then(|| {
            (now + chrono::Duration::seconds(token.expires_in))
                .to_rfc3339_opts(SecondsFormat::Secs, true)
        });

        let token_data = updated_token_data(
            refreshed.metadata.get(CODEX_METADATA_TOKEN_DATA),
            &token,
            &previous_refresh_token,
            &now_rfc3339,
            expires_at.as_deref(),
        );

        // Coerce a non-object metadata blob into an empty map before writing.
        let mut metadata = match refreshed.metadata {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        if !token.access_token.is_empty() {
            metadata.insert(
                CODEX_METADATA_ACCESS_TOKEN.to_owned(),
                Value::String(token.access_token.clone()),
            );
        }
        let refresh_token = if token.refresh_token.is_empty() {
            previous_refresh_token.clone()
        } else {
            token.refresh_token.clone()
        };
        metadata.insert(
            CODEX_METADATA_REFRESH_TOKEN.to_owned(),
            Value::String(refresh_token),
        );
        if !token.id_token.is_empty() {
            metadata.insert(
                CODEX_METADATA_ID_TOKEN.to_owned(),
                Value::String(token.id_token.clone()),
            );
        }
        if let Some(expires_at) = expires_at {
            // Write both spellings: `expires_at` is what this gateway reads,
            // `expired` is what the Codex CLI's own credential file uses.
            metadata.insert(
                CODEX_METADATA_EXPIRES_AT.to_owned(),
                Value::String(expires_at.clone()),
            );
            metadata.insert(CODEX_METADATA_EXPIRED.to_owned(), Value::String(expires_at));
        }
        metadata.insert(
            CODEX_METADATA_LAST_REFRESH.to_owned(),
            Value::String(now_rfc3339),
        );
        metadata.insert(
            CODEX_METADATA_TOKEN_DATA.to_owned(),
            Value::Object(token_data),
        );

        refreshed.metadata = Value::Object(metadata);
        refreshed.status = AuthStatus::Active;
        refreshed.updated_at = now;
        refreshed.last_refreshed_at = Some(now);
        Ok(refreshed)
    }
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
