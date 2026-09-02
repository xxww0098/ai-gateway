//! OpenAI and OpenAI-compatible upstreams.
//!
//! OWNER: worker `provider-openai`.

use crate::common::{
    PROVIDER_OPENAI, ProviderConfig, Redacted, chat_completions_endpoint_for, ensure_include_usage,
    nested_string, relay_timeouts, request_surface, resolve_timeout, responses_endpoint_for,
    string_from_map, upstream_dialect,
};
use crate::route::{RoutePlan, RoutePlanner};
use crate::types::{ProviderError, ProviderRequest};
use gw_authcore::{AuthRecord, AuthStatus};
use gw_relay::{Credential, Surface};
use http::header::{ACCEPT, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue};
use std::borrow::Cow;
use std::time::Duration;

/// Executor for any OpenAI-compatible API.
#[derive(Clone)]
pub struct OpenAiCompatibleProvider {
    provider: &'static str,
    base_url: String,
    api_key: String,
    timeout: Duration,
}

impl std::fmt::Debug for OpenAiCompatibleProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatibleProvider")
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field("api_key", &Redacted(&self.api_key))
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl OpenAiCompatibleProvider {
    /// Builds an executor from provider config.
    ///
    /// A disabled provider, a blank base URL or a blank API key are all
    /// rejected up front, so a misconfigured upstream fails at wiring time
    /// rather than on the first request.
    pub fn new(cfg: &ProviderConfig, timeout_seconds: i64) -> Result<Self, ProviderError> {
        let base_url = cfg.base_url.trim().trim_end_matches('/').to_owned();
        let api_key = cfg.api_key.trim().to_owned();
        if !cfg.enabled || base_url.is_empty() || api_key.is_empty() {
            return Err(ProviderError::Other(anyhow::anyhow!(
                "sdk base_url and api_key are required"
            )));
        }
        let parsed = url::Url::parse(&base_url)
            .map_err(|_| ProviderError::Other(anyhow::anyhow!("invalid sdk base_url")))?;
        if parsed.host_str().unwrap_or_default().is_empty() {
            return Err(ProviderError::Other(anyhow::anyhow!(
                "invalid sdk base_url"
            )));
        }
        Ok(Self {
            provider: PROVIDER_OPENAI,
            base_url,
            api_key,
            timeout: resolve_timeout(timeout_seconds),
        })
    }

    /// The configured upstream base URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Per-credential overrides layered over the configured defaults.
    ///
    /// `base-url` is accepted alongside `base_url` because both spellings
    /// exist in stored credentials.
    fn resolve_credentials<'a>(&'a self, auth: &'a AuthRecord) -> (Cow<'a, str>, Cow<'a, str>) {
        let api_key = string_from_map(&auth.metadata, "api_key")
            .or_else(|| string_from_map(&auth.metadata, "access_token"))
            .or_else(|| nested_string(&auth.metadata, "token_data", "access_token"))
            .map(Cow::Owned)
            .unwrap_or(Cow::Borrowed(self.api_key.as_str()));
        let mut base_url = Cow::Borrowed(self.base_url.as_str());
        for key in ["base_url", "base-url"] {
            if let Some(value) = auth.attributes.get(key) {
                let value = value.trim().trim_end_matches('/');
                if !value.is_empty() {
                    base_url = Cow::Owned(value.to_owned());
                    break;
                }
            }
        }
        (api_key, base_url)
    }

    /// Plans an outbound chat-completions / responses request.
    ///
    /// No client, no socket: the result is a [`RoutePlan`] the relay executes.
    fn plan_request(
        &self,
        req: &ProviderRequest,
        api_key: &str,
        base_url: &str,
    ) -> Result<RoutePlan, ProviderError> {
        if api_key.trim().is_empty() {
            return Err(ProviderError::Credential(
                "openai api key is required".to_owned(),
            ));
        }
        // 端点由**入口**决定，不由 provider 名或 model 名猜 —— 那正是缺陷 #1
        // （S1）的成因。入口 B（`/v1/responses`）在此之前会被发到
        // chat/completions 端点，上游必 400。
        let surface = request_surface(req);
        let endpoint = match surface {
            Surface::OpenAiResponses => responses_endpoint_for(base_url, req)?,
            Surface::OpenAiCompletions | Surface::AnthropicMessages => {
                chat_completions_endpoint_for(base_url, req)?
            }
        };
        let endpoint = url::Url::parse(&endpoint).map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("invalid openai endpoint: {err}"))
        })?;

        // Streaming requests must ask for the terminal usage envelope so the
        // billing pipeline settles on precise token counts instead of its
        // fallback estimate. The helper re-checks `stream: true` in the body
        // itself, so a mis-set `req.stream` cannot force include_usage onto a
        // non-streaming payload. `None` means "not one byte is touched".
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
            provider: self.provider,
            endpoint,
            credential: Credential::Bearer(api_key.to_owned()),
            headers,
            body,
            timeouts: relay_timeouts(self.timeout),
            dialect: upstream_dialect(surface),
        })
    }
}

#[async_trait::async_trait]
impl RoutePlanner for OpenAiCompatibleProvider {
    fn name(&self) -> &'static str {
        self.provider
    }

    async fn plan(
        &self,
        auth: &AuthRecord,
        req: &ProviderRequest,
    ) -> Result<RoutePlan, ProviderError> {
        let (api_key, base_url) = self.resolve_credentials(auth);
        self.plan_request(req, &api_key, &base_url)
    }

    /// A static API key never expires, so the record is only re-marked
    /// healthy.
    async fn refresh(&self, auth: &AuthRecord) -> Result<AuthRecord, ProviderError> {
        let mut refreshed = auth.clone();
        refreshed.status = AuthStatus::Active;
        refreshed.updated_at = chrono::Utc::now();
        Ok(refreshed)
    }
}

#[cfg(test)]
#[path = "openai_tests.rs"]
mod tests;
