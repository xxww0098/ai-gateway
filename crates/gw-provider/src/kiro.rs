//! Kiro / AWS Builder ID upstream.
//!
//! Stores and refreshes AWS SSO OIDC tokens (dynamic RegisterClient). The
//! 15-cell relay matrix has no Kiro variant, so `/v1` candidates never include
//! `"kiro"` today — an operator can export the auth file or call this
//! executor directly. There is no OpenAI-compat translation layer: the
//! payload is forwarded as-is with a Bearer token.

use crate::common::{
    PROVIDER_KIRO, ProviderConfig, nested_string, relay_timeouts, resolve_timeout, string_from_map,
};
use crate::route::{RoutePlan, RoutePlanner};
use crate::types::{ProviderError, ProviderRequest};
use chrono::{SecondsFormat, Utc};
use gw_authcore::{AuthRecord, AuthStatus};
use gw_relay::{Credential, UpstreamDialect};
use http::header::{ACCEPT, CONTENT_TYPE};
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

    /// Plans an outbound CodeWhisperer request.
    ///
    /// There is no protocol translation here: the payload is forwarded as-is
    /// under a Bearer token, and the endpoint is the account's base URL itself.
    fn plan_request(
        &self,
        req: &ProviderRequest,
        access_token: &str,
        base_url: &str,
    ) -> Result<RoutePlan, ProviderError> {
        if access_token.is_empty() {
            return Err(ProviderError::Credential(
                "kiro access token is required".to_owned(),
            ));
        }
        let endpoint = url::Url::parse(base_url.trim_end_matches('/'))
            .map_err(|err| ProviderError::Other(anyhow::anyhow!("invalid kiro base_url: {err}")))?;

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
            provider: PROVIDER_KIRO,
            endpoint,
            credential: Credential::Bearer(access_token.to_owned()),
            headers,
            body: None,
            timeouts: relay_timeouts(self.timeout),
            // Kiro has no cell in the 15-cell matrix; the payload is whatever
            // the caller sent, so the nearest honest label is the OpenAI chat
            // shape the `/v1` surfaces speak.
            dialect: UpstreamDialect::OpenAiChat,
        })
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
        let payload = crate::oauth::post_json(
            endpoint,
            self.timeout,
            "kiro",
            &serde_json::json!({
                "clientId": client_id,
                "clientSecret": client_secret,
                "grantType": "refresh_token",
                "refreshToken": refresh_token,
            }),
        )
        .await?;
        serde_json::from_slice(&payload).map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("parsing kiro refresh response: {err}"))
        })
    }
}

#[async_trait::async_trait]
impl RoutePlanner for KiroProvider {
    fn name(&self) -> &'static str {
        PROVIDER_KIRO
    }

    async fn plan(
        &self,
        auth: &AuthRecord,
        req: &ProviderRequest,
    ) -> Result<RoutePlan, ProviderError> {
        let (access_token, base_url) = self.resolve_credentials(auth);
        self.plan_request(req, &access_token, &base_url)
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
            metadata.insert(
                META_ACCESS.to_owned(),
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
            metadata.insert("id_token".to_owned(), Value::String(token.id_token.clone()));
            token_data.insert("id_token".to_owned(), Value::String(token.id_token));
        }
        if let Some(expires_at) = &expires_at {
            metadata.insert(
                META_EXPIRES_AT.to_owned(),
                Value::String(expires_at.clone()),
            );
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
