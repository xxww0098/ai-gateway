//! xAI Grok OAuth upstream.
//!
//! Speaks the OpenAI chat-completions wire. Authenticates with a rotating
//! OAuth access token from the public Grok CLI client (issuer `auth.x.ai`).
//! The 15-cell relay matrix has no xAI cell: L1 `xai/<model>` maps to
//! [`crate::openai::OpenAiCompatibleProvider`] via the channel table, and
//! `xai` auth records are also attached to the `openai` credential bucket.
//! This executor still exists so a stored `xai` row can be refreshed.

use crate::common::{
    PROVIDER_XAI, ProviderConfig, Redacted, chat_completions_endpoint, ensure_include_usage,
    nested_string, relay_timeouts, request_surface, resolve_timeout, responses_endpoint,
    string_from_map, upstream_dialect,
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

/// Default xAI OpenAI-compatible API root.
pub const XAI_DEFAULT_BASE_URL: &str = "https://api.x.ai/v1";
/// Public token endpoint on the xAI issuer. Source: xAI OIDC discovery
/// (`https://auth.x.ai/.well-known/openid-configuration`).
pub const XAI_OAUTH_TOKEN_URL: &str = "https://auth.x.ai/oauth/token";
/// Public Grok CLI OAuth client. Source: CLIProxyAPI `internal/auth/xai/types.go`.
pub const XAI_OAUTH_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";

const META_ACCESS: &str = "access_token";
const META_API_KEY: &str = "api_key";
const META_REFRESH: &str = "refresh_token";
const META_TOKEN_DATA: &str = "token_data";
const META_EXPIRES_AT: &str = "expires_at";
const META_EXPIRED: &str = "expired";
const META_LAST_REFRESH: &str = "last_refresh";
const META_ID_TOKEN: &str = "id_token";

#[derive(Default, Deserialize)]
struct XaiRefreshResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    id_token: String,
    #[serde(default)]
    expires_in: i64,
}

impl std::fmt::Debug for XaiRefreshResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XaiRefreshResponse")
            .field("access_token", &Redacted(&self.access_token))
            .field("refresh_token", &Redacted(&self.refresh_token))
            .field("id_token", &Redacted(&self.id_token))
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

/// Executor for xAI Grok OAuth credentials.
#[derive(Clone)]
pub struct XaiProvider {
    base_url: String,
    access_token: String,
    timeout: Duration,
}

impl std::fmt::Debug for XaiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XaiProvider")
            .field("base_url", &self.base_url)
            .field("access_token", &Redacted(&self.access_token))
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl XaiProvider {
    /// Builds an executor. An empty base URL falls back to
    /// [`XAI_DEFAULT_BASE_URL`]; an empty token is tolerated because the
    /// real credential usually arrives per-request on the auth record.
    pub fn new(cfg: &ProviderConfig, timeout_seconds: i64) -> Result<Self, ProviderError> {
        let mut base_url = cfg.base_url.trim().trim_end_matches('/').to_owned();
        if base_url.is_empty() {
            base_url = XAI_DEFAULT_BASE_URL.to_owned();
        }
        let parsed = url::Url::parse(&base_url)
            .map_err(|_| ProviderError::Other(anyhow::anyhow!("invalid xai base_url")))?;
        if parsed.host_str().unwrap_or_default().is_empty() {
            return Err(ProviderError::Other(anyhow::anyhow!(
                "invalid xai base_url"
            )));
        }
        Ok(Self {
            base_url,
            access_token: cfg.api_key.trim().to_owned(),
            timeout: resolve_timeout(timeout_seconds),
        })
    }

    fn resolve_credentials(&self, auth: &AuthRecord) -> (String, String) {
        let mut base_url = self.base_url.trim().to_owned();
        for key in ["base_url", "base-url"] {
            if let Some(value) = auth.attributes.get(key) {
                let value = value.trim().trim_end_matches('/');
                if !value.is_empty() {
                    base_url = value.to_owned();
                    break;
                }
            }
        }
        let token = string_from_map(&auth.metadata, META_ACCESS)
            .or_else(|| nested_string(&auth.metadata, META_TOKEN_DATA, META_ACCESS))
            .or_else(|| string_from_map(&auth.metadata, META_API_KEY))
            .or_else(|| nested_string(&auth.metadata, META_TOKEN_DATA, META_API_KEY))
            .unwrap_or_else(|| self.access_token.trim().to_owned());
        (token, base_url)
    }

    fn resolve_refresh_token(auth: &AuthRecord) -> Option<String> {
        string_from_map(&auth.metadata, META_REFRESH)
            .or_else(|| nested_string(&auth.metadata, META_TOKEN_DATA, META_REFRESH))
    }

    /// Plans an outbound chat-completions / responses request.
    ///
    /// 端点由**入口**决定（缺陷 #1），不由 provider 名或 model 名猜。
    fn plan_request(
        &self,
        req: &ProviderRequest,
        access_token: &str,
        base_url: &str,
        user_id: Option<&str>,
    ) -> Result<RoutePlan, ProviderError> {
        if access_token.is_empty() {
            return Err(ProviderError::Credential(
                "xai access token is required".to_owned(),
            ));
        }
        let surface = request_surface(req);
        let endpoint = match surface {
            Surface::OpenAiResponses => responses_endpoint(base_url, &req.query)?,
            Surface::OpenAiCompletions | Surface::AnthropicMessages => {
                chat_completions_endpoint(base_url, &req.query)?
            }
        };
        let endpoint = url::Url::parse(&endpoint)
            .map_err(|err| ProviderError::Other(anyhow::anyhow!("invalid xai endpoint: {err}")))?;

        let inbound_req_id = req
            .headers
            .get("x-grok-req-id")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned);
        let minted_req_id = inbound_req_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let hop = gw_oauth_hops::grok::plan(&gw_oauth_hops::HopInput {
            body: &req.payload,
            user_id,
            model: (!req.model.trim().is_empty()).then_some(req.model.as_str()),
            request_id: Some(minted_req_id.as_str()),
            ..gw_oauth_hops::HopInput::default()
        });
        let hop_headers = hop.headers;
        let hop_body = hop.body;
        // Force the terminal usage envelope on streams, but only after
        // re-verifying `stream: true` in the body itself. `None` 表示一个字节都不动。
        let body = if req.stream {
            let source = hop_body.as_ref().unwrap_or(&req.payload);
            match RoutePlan::splice(ensure_include_usage(source, surface)) {
                Some(bytes) => Some(bytes),
                None => hop_body,
            }
        } else {
            hop_body
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
        gw_oauth_hops::merge_headers(&mut headers, hop_headers);

        Ok(RoutePlan {
            provider: PROVIDER_XAI,
            endpoint,
            credential: Credential::Bearer(access_token.to_owned()),
            headers,
            body,
            timeouts: relay_timeouts(self.timeout),
            dialect: upstream_dialect(surface),
        })
    }

    async fn refresh_oauth_token(
        &self,
        refresh_token: &str,
        client_id: &str,
    ) -> Result<XaiRefreshResponse, ProviderError> {
        let payload = crate::oauth::post_form(
            XAI_OAUTH_TOKEN_URL,
            self.timeout,
            "xai",
            &[
                ("client_id", client_id),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ],
        )
        .await?;
        serde_json::from_slice(&payload).map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("parsing xai refresh response: {err}"))
        })
    }
}

#[async_trait::async_trait]
impl RoutePlanner for XaiProvider {
    fn name(&self) -> &'static str {
        PROVIDER_XAI
    }

    async fn plan(
        &self,
        auth: &AuthRecord,
        req: &ProviderRequest,
    ) -> Result<RoutePlan, ProviderError> {
        let (access_token, base_url) = self.resolve_credentials(auth);
        let user_id = string_from_map(&auth.metadata, "user_id")
            .or_else(|| nested_string(&auth.metadata, META_TOKEN_DATA, "user_id"));
        self.plan_request(req, &access_token, &base_url, user_id.as_deref())
    }

    async fn refresh(&self, auth: &AuthRecord) -> Result<AuthRecord, ProviderError> {
        let mut refreshed = auth.clone();
        let now = Utc::now();
        let Some(previous) = Self::resolve_refresh_token(auth) else {
            refreshed.status = AuthStatus::Active;
            refreshed.updated_at = now;
            return Ok(refreshed);
        };
        let client_id = string_from_map(&auth.metadata, "client_id")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| XAI_OAUTH_CLIENT_ID.to_owned());
        let token = self.refresh_oauth_token(&previous, &client_id).await?;
        let now_rfc3339 = now.to_rfc3339_opts(SecondsFormat::Secs, true);
        let expires_at = (token.expires_in > 0).then(|| {
            (now + chrono::Duration::seconds(token.expires_in))
                .to_rfc3339_opts(SecondsFormat::Secs, true)
        });

        let mut metadata = match refreshed.metadata {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        let mut token_data = match metadata.get(META_TOKEN_DATA) {
            Some(Value::Object(map)) => map.clone(),
            _ => Map::new(),
        };
        if !token.access_token.is_empty() {
            metadata.insert(
                META_ACCESS.to_owned(),
                Value::String(token.access_token.clone()),
            );
            metadata.insert(
                META_API_KEY.to_owned(),
                Value::String(token.access_token.clone()),
            );
            token_data.insert(
                META_ACCESS.to_owned(),
                Value::String(token.access_token.clone()),
            );
        }
        let refresh_token = if token.refresh_token.is_empty() {
            previous
        } else {
            token.refresh_token
        };
        metadata.insert(
            META_REFRESH.to_owned(),
            Value::String(refresh_token.clone()),
        );
        token_data.insert(META_REFRESH.to_owned(), Value::String(refresh_token));
        if !token.id_token.is_empty() {
            metadata.insert(
                META_ID_TOKEN.to_owned(),
                Value::String(token.id_token.clone()),
            );
            token_data.insert(META_ID_TOKEN.to_owned(), Value::String(token.id_token));
        }
        if let Some(expires_at) = &expires_at {
            metadata.insert(
                META_EXPIRES_AT.to_owned(),
                Value::String(expires_at.clone()),
            );
            metadata.insert(META_EXPIRED.to_owned(), Value::String(expires_at.clone()));
            token_data.insert(
                META_EXPIRES_AT.to_owned(),
                Value::String(expires_at.clone()),
            );
            token_data.insert(META_EXPIRED.to_owned(), Value::String(expires_at.clone()));
        }
        metadata.insert(META_LAST_REFRESH.to_owned(), Value::String(now_rfc3339));
        metadata.insert(META_TOKEN_DATA.to_owned(), Value::Object(token_data));
        refreshed.metadata = Value::Object(metadata);
        refreshed.status = AuthStatus::Active;
        refreshed.updated_at = now;
        refreshed.last_refreshed_at = Some(now);
        Ok(refreshed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(metadata: Value) -> AuthRecord {
        let mut record = AuthRecord::new("id-1", PROVIDER_XAI, Utc::now());
        record.metadata = metadata;
        record
    }

    #[test]
    fn the_token_cascade_prefers_access_token() {
        let provider = XaiProvider::new(
            &ProviderConfig {
                base_url: String::new(),
                api_key: "cfg".to_owned(),
                enabled: true,
            },
            30,
        )
        .expect("builds");
        let auth = record(json!({
            "access_token": "at",
            "api_key": "ak",
            "token_data": {"access_token": "nested"},
        }));
        let (token, base) = provider.resolve_credentials(&auth);
        assert_eq!(token, "at");
        assert_eq!(base, XAI_DEFAULT_BASE_URL);
    }

    #[test]
    fn a_record_base_url_overrides_the_default() {
        let provider = XaiProvider::new(
            &ProviderConfig {
                base_url: String::new(),
                api_key: String::new(),
                enabled: true,
            },
            30,
        )
        .expect("builds");
        let mut auth = record(json!({"access_token": "at"}));
        auth.set_attribute("base_url", "https://api.x.ai/v1");
        let (_, base) = provider.resolve_credentials(&auth);
        assert_eq!(base, "https://api.x.ai/v1");
    }
}
