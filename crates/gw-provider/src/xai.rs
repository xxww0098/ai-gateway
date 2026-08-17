//! xAI Grok OAuth upstream.
//!
//! Speaks the OpenAI chat-completions wire. Authenticates with a rotating
//! OAuth access token from the public Grok CLI client (issuer `auth.x.ai`).
//! The 15-cell relay matrix has no xAI cell: L1 `xai/<model>` maps to
//! [`crate::openai::OpenAiCompatibleProvider`] via the channel table, and
//! `xai` auth records are also attached to the `openai` credential bucket.
//! This executor still exists so a stored `xai` row can be refreshed.

use crate::common::{
    DEFAULT_STREAM_IDLE_TIMEOUT, PROVIDER_XAI, ProviderConfig, attach_body,
    chat_completions_endpoint, ensure_include_usage, nested_string, request_surface,
    requested_model, resolve_timeout, responses_endpoint, shared_client, stream_response,
    string_from_map, usage_stream,
};
use crate::openai::bearer;
use crate::types::{
    Provider, ProviderError, ProviderRequest, ProviderResponse, StreamResponse,
    copy_outbound_headers,
};
use crate::usage::{parse_openai_stream_usage, parse_openai_usage};
use chrono::{SecondsFormat, Utc};
use gw_authcore::{AuthRecord, AuthStatus};
use gw_relay::Surface;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
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

#[derive(Debug, Default, Deserialize)]
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

/// Executor for xAI Grok OAuth credentials.
#[derive(Debug, Clone)]
pub struct XaiProvider {
    base_url: String,
    access_token: String,
    timeout: Duration,
    client: reqwest::Client,
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
            return Err(ProviderError::Other(anyhow::anyhow!("invalid xai base_url")));
        }
        Ok(Self {
            base_url,
            access_token: cfg.api_key.trim().to_owned(),
            timeout: resolve_timeout(timeout_seconds),
            client: shared_client(),
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

    fn build_request(
        &self,
        req: &ProviderRequest,
        stream: bool,
        access_token: &str,
        base_url: &str,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
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
        let spliced = if stream {
            ensure_include_usage(&req.payload, surface)
        } else {
            None
        };
        let mut headers = HeaderMap::new();
        copy_outbound_headers(&mut headers, &req.headers);
        if !headers.contains_key(CONTENT_TYPE) {
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        }
        if stream {
            headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
        } else if !headers.contains_key(ACCEPT) {
            headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        }
        headers.insert(AUTHORIZATION, bearer(access_token)?);
        let mut builder = attach_body(
            self.client.post(endpoint).headers(headers),
            &req.payload,
            spliced,
        );
        if !stream {
            builder = builder.timeout(self.timeout);
        }
        Ok(builder)
    }

    async fn refresh_oauth_token(
        &self,
        refresh_token: &str,
        client_id: &str,
    ) -> Result<XaiRefreshResponse, ProviderError> {
        let response = self
            .client
            .post(XAI_OAUTH_TOKEN_URL)
            .timeout(self.timeout)
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .form(&[
                ("client_id", client_id),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await
            .map_err(|err| {
                ProviderError::Other(anyhow::anyhow!("xai token refresh request failed: {err}"))
            })?;
        let status = response.status().as_u16();
        let payload = response.bytes().await.map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("reading xai refresh response: {err}"))
        })?;
        if status >= 400 {
            return Err(ProviderError::Upstream {
                status,
                body: String::from_utf8_lossy(&payload).into_owned(),
            });
        }
        serde_json::from_slice(&payload).map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("parsing xai refresh response: {err}"))
        })
    }
}

#[async_trait::async_trait]
impl Provider for XaiProvider {
    fn name(&self) -> &'static str {
        PROVIDER_XAI
    }

    async fn execute(
        &self,
        auth: &AuthRecord,
        req: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        let (access_token, base_url) = self.resolve_credentials(auth);
        let model = requested_model(&req);
        let response = self
            .build_request(&req, false, &access_token, &base_url)?
            .send()
            .await?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.bytes().await?;
        if status >= 400 {
            return Err(ProviderError::Upstream {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }
        let usage = parse_openai_usage(&body).map(|t| t.to_record(model, PROVIDER_XAI));
        Ok(ProviderResponse {
            status,
            headers,
            body,
            usage,
        })
    }

    async fn execute_stream(
        &self,
        auth: &AuthRecord,
        req: ProviderRequest,
    ) -> Result<StreamResponse, ProviderError> {
        let (access_token, base_url) = self.resolve_credentials(auth);
        let model = requested_model(&req).to_owned();
        let response = self
            .build_request(&req, true, &access_token, &base_url)?
            .send()
            .await?;
        let status = response.status().as_u16();
        if status >= 400 {
            let body = response.bytes().await.unwrap_or_default();
            return Err(ProviderError::Upstream {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }
        Ok(stream_response(response, move |response, status| {
            usage_stream(
                response.bytes_stream(),
                DEFAULT_STREAM_IDLE_TIMEOUT,
                model,
                PROVIDER_XAI,
                parse_openai_stream_usage,
                status,
            )
        }))
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
            metadata.insert(META_ACCESS.to_owned(), Value::String(token.access_token.clone()));
            metadata.insert(META_API_KEY.to_owned(), Value::String(token.access_token.clone()));
            token_data.insert(META_ACCESS.to_owned(), Value::String(token.access_token.clone()));
        }
        let refresh_token = if token.refresh_token.is_empty() {
            previous
        } else {
            token.refresh_token
        };
        metadata.insert(META_REFRESH.to_owned(), Value::String(refresh_token.clone()));
        token_data.insert(META_REFRESH.to_owned(), Value::String(refresh_token));
        if !token.id_token.is_empty() {
            metadata.insert(META_ID_TOKEN.to_owned(), Value::String(token.id_token.clone()));
            token_data.insert(META_ID_TOKEN.to_owned(), Value::String(token.id_token));
        }
        if let Some(expires_at) = &expires_at {
            metadata.insert(META_EXPIRES_AT.to_owned(), Value::String(expires_at.clone()));
            metadata.insert(META_EXPIRED.to_owned(), Value::String(expires_at.clone()));
            token_data.insert(META_EXPIRES_AT.to_owned(), Value::String(expires_at.clone()));
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

    async fn count_tokens(
        &self,
        _auth: &AuthRecord,
        _req: ProviderRequest,
    ) -> Result<i64, ProviderError> {
        Err(ProviderError::Other(anyhow::anyhow!(
            "{PROVIDER_XAI} upstream exposes no token-counting endpoint"
        )))
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
