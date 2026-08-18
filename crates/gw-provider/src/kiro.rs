//! Kiro / AWS Builder ID upstream.
//!
//! Stores and refreshes AWS SSO OIDC tokens (dynamic RegisterClient). The
//! 15-cell relay matrix has no Kiro variant, so `/v1` candidates never include
//! `"kiro"` today — an operator can export the auth file or call this
//! executor directly. There is no OpenAI-compat translation layer: the
//! payload is forwarded as-is with a Bearer token.

use crate::common::{
    DEFAULT_STREAM_IDLE_TIMEOUT, PROVIDER_KIRO, ProviderConfig, attach_body, nested_string,
    requested_model, resolve_timeout, shared_client, stream_response, string_from_map,
    usage_stream,
};
use crate::openai::bearer;
use crate::types::{
    Provider, ProviderError, ProviderRequest, ProviderResponse, StreamResponse,
    copy_outbound_headers,
};
use crate::usage::{parse_openai_stream_usage, parse_openai_usage};
use chrono::{SecondsFormat, Utc};
use gw_authcore::{AuthRecord, AuthStatus};
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::time::Duration;

/// Default CodeWhisperer runtime host (us-east-1). Override per-record via
/// the `base_url` attribute.
pub const KIRO_DEFAULT_BASE_URL: &str = "https://codewhisperer.us-east-1.amazonaws.com";
const DEFAULT_REGION: &str = "us-east-1";

const META_ACCESS: &str = "access_token";
const META_REFRESH: &str = "refresh_token";
const META_TOKEN_DATA: &str = "token_data";
const META_CLIENT_ID: &str = "client_id";
const META_CLIENT_SECRET: &str = "client_secret";
const META_REGION: &str = "region";
const META_TOKEN_ENDPOINT: &str = "token_endpoint";
const META_EXPIRES_AT: &str = "expires_at";
const META_EXPIRED: &str = "expired";
const META_LAST_REFRESH: &str = "last_refresh";

#[derive(Debug, Default, Deserialize)]
struct KiroRefreshResponse {
    #[serde(default, alias = "accessToken")]
    access_token: String,
    #[serde(default, alias = "refreshToken")]
    refresh_token: String,
    #[serde(default, alias = "idToken")]
    id_token: String,
    #[serde(default, alias = "expiresIn")]
    expires_in: i64,
}

/// Executor for Kiro / AWS Builder ID credentials.
#[derive(Debug, Clone)]
pub struct KiroProvider {
    base_url: String,
    access_token: String,
    timeout: Duration,
    client: reqwest::Client,
}

impl KiroProvider {
    /// Builds an executor. An empty base URL falls back to
    /// [`KIRO_DEFAULT_BASE_URL`].
    pub fn new(cfg: &ProviderConfig, timeout_seconds: i64) -> Result<Self, ProviderError> {
        let mut base_url = cfg.base_url.trim().trim_end_matches('/').to_owned();
        if base_url.is_empty() {
            base_url = KIRO_DEFAULT_BASE_URL.to_owned();
        }
        let parsed = url::Url::parse(&base_url)
            .map_err(|_| ProviderError::Other(anyhow::anyhow!("invalid kiro base_url")))?;
        if parsed.host_str().unwrap_or_default().is_empty() {
            return Err(ProviderError::Other(anyhow::anyhow!(
                "invalid kiro base_url"
            )));
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
            .unwrap_or_else(|| self.access_token.trim().to_owned());
        (token, base_url)
    }

    fn resolve_refresh_token(auth: &AuthRecord) -> Option<String> {
        string_from_map(&auth.metadata, META_REFRESH)
            .or_else(|| nested_string(&auth.metadata, META_TOKEN_DATA, META_REFRESH))
    }

    fn token_endpoint(auth: &AuthRecord) -> String {
        if let Some(endpoint) = string_from_map(&auth.metadata, META_TOKEN_ENDPOINT)
            && endpoint.starts_with("https://")
        {
            return endpoint;
        }
        let region = string_from_map(&auth.metadata, META_REGION)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_REGION.to_owned());
        format!("https://oidc.{region}.amazonaws.com/token")
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
                "kiro access token is required".to_owned(),
            ));
        }
        let endpoint = base_url.trim_end_matches('/').to_owned();
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
        let mut builder = attach_body(self.client.post(endpoint).headers(headers), &req.payload, None);
        if !stream {
            builder = builder.timeout(self.timeout);
        }
        Ok(builder)
    }

    async fn refresh_oauth_token(
        &self,
        endpoint: &str,
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
    ) -> Result<KiroRefreshResponse, ProviderError> {
        if !endpoint.starts_with("https://") {
            return Err(ProviderError::Other(anyhow::anyhow!(
                "kiro token_endpoint must be https"
            )));
        }
        let response = self
            .client
            .post(endpoint)
            .timeout(self.timeout)
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .json(&serde_json::json!({
                "clientId": client_id,
                "clientSecret": client_secret,
                "grantType": "refresh_token",
                "refreshToken": refresh_token,
            }))
            .send()
            .await
            .map_err(|err| {
                ProviderError::Other(anyhow::anyhow!("kiro token refresh request failed: {err}"))
            })?;
        let status = response.status().as_u16();
        let payload = response.bytes().await.map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("reading kiro refresh response: {err}"))
        })?;
        if status >= 400 {
            return Err(ProviderError::Upstream {
                status,
                body: String::from_utf8_lossy(&payload).into_owned(),
            });
        }
        serde_json::from_slice(&payload).map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("parsing kiro refresh response: {err}"))
        })
    }
}

#[async_trait::async_trait]
impl Provider for KiroProvider {
    fn name(&self) -> &'static str {
        PROVIDER_KIRO
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
        let usage = parse_openai_usage(&body).map(|t| t.to_record(model, PROVIDER_KIRO));
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
                PROVIDER_KIRO,
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
        let client_id = string_from_map(&auth.metadata, META_CLIENT_ID).unwrap_or_default();
        let client_secret = string_from_map(&auth.metadata, META_CLIENT_SECRET).unwrap_or_default();
        if client_id.is_empty() {
            return Err(ProviderError::Credential(
                "kiro client_id is required to refresh".to_owned(),
            ));
        }
        let endpoint = Self::token_endpoint(auth);
        let token = self
            .refresh_oauth_token(&endpoint, &client_id, &client_secret, &previous)
            .await?;
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
            metadata.insert("id_token".to_owned(), Value::String(token.id_token.clone()));
            token_data.insert("id_token".to_owned(), Value::String(token.id_token));
        }
        if let Some(expires_at) = &expires_at {
            metadata.insert(META_EXPIRES_AT.to_owned(), Value::String(expires_at.clone()));
            metadata.insert(META_EXPIRED.to_owned(), Value::String(expires_at.clone()));
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
            "{PROVIDER_KIRO} upstream exposes no token-counting endpoint"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(metadata: Value) -> AuthRecord {
        let mut record = AuthRecord::new("id-1", PROVIDER_KIRO, Utc::now());
        record.metadata = metadata;
        record
    }

    #[test]
    fn the_token_is_read_from_metadata() {
        let provider = KiroProvider::new(
            &ProviderConfig {
                base_url: String::new(),
                api_key: String::new(),
                enabled: true,
            },
            30,
        )
        .expect("builds");
        let auth = record(json!({"access_token": "at", "token_data": {"access_token": "nested"}}));
        let (token, base) = provider.resolve_credentials(&auth);
        assert_eq!(token, "at");
        assert_eq!(base, KIRO_DEFAULT_BASE_URL);
    }

    #[test]
    fn the_token_endpoint_follows_the_stored_region() {
        let auth = record(json!({"region": "eu-west-1"}));
        assert_eq!(
            KiroProvider::token_endpoint(&auth),
            "https://oidc.eu-west-1.amazonaws.com/token"
        );
    }
}
