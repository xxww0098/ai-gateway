//! OpenAI and OpenAI-compatible upstreams.
//!
//! OWNER: worker `provider-openai`.

use crate::common::{
    DEFAULT_STREAM_IDLE_TIMEOUT, PROVIDER_OPENAI, ProviderConfig, approximate_tokens_from_bytes,
    chat_completions_endpoint, ensure_include_usage, requested_model, resolve_timeout,
    shared_client, stream_response, string_from_map, usage_stream,
};
use crate::types::{
    Provider, ProviderError, ProviderRequest, ProviderResponse, StreamResponse,
    copy_outbound_headers,
};
use crate::usage::{parse_openai_stream_usage, parse_openai_usage};
use gw_authcore::{AuthRecord, AuthStatus};
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue};
use std::borrow::Cow;
use std::time::Duration;

/// Executor for any OpenAI-compatible API.
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    provider: &'static str,
    base_url: String,
    api_key: String,
    timeout: Duration,
    client: reqwest::Client,
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
            client: shared_client(),
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
    fn resolve_credentials(&self, auth: &AuthRecord) -> (String, String) {
        let mut api_key = self.api_key.clone();
        let mut base_url = self.base_url.clone();

        if let Some(candidate) = string_from_map(&auth.metadata, "api_key") {
            api_key = candidate;
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
        (api_key, base_url)
    }

    /// Assembles an outbound chat-completions request.
    fn build_request(
        &self,
        req: &ProviderRequest,
        stream: bool,
        api_key: &str,
        base_url: &str,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        let endpoint = chat_completions_endpoint(base_url, &req.query)?;
        // Streaming requests must ask for the terminal usage envelope so the
        // billing pipeline settles on precise token counts instead of its
        // fallback estimate. The helper re-checks `stream: true` in the body
        // itself, so a mis-set `req.stream` cannot force include_usage onto a
        // non-streaming payload.
        let payload: Cow<'_, [u8]> = if stream {
            ensure_include_usage(&req.payload)
        } else {
            Cow::Borrowed(&req.payload)
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
        headers.insert(AUTHORIZATION, bearer(api_key)?);

        let mut builder = self
            .client
            .post(endpoint)
            .headers(headers)
            .body(payload.into_owned());
        // reqwest scopes the cap per request, so streaming simply attaches no
        // whole-request timeout and relies on the idle watchdog instead — a
        // whole-request timeout would also bound reading the body and truncate
        // healthy long streams.
        if !stream {
            builder = builder.timeout(self.timeout);
        }
        Ok(builder)
    }
}

/// Builds an `Authorization: Bearer …` value, rejecting credentials that cannot
/// be encoded in a header rather than panicking deep inside `reqwest`.
pub(crate) fn bearer(token: &str) -> Result<HeaderValue, ProviderError> {
    HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| ProviderError::Credential("credential is not a valid header value".to_owned()))
}

#[async_trait::async_trait]
impl Provider for OpenAiCompatibleProvider {
    fn name(&self) -> &'static str {
        self.provider
    }

    /// On a non-2xx the caller receives [`ProviderError::Upstream`] carrying
    /// the `(status, payload)` pair.
    ///
    /// [`ProviderRequest::stream`] is ignored here: the trait already splits
    /// streaming onto [`Provider::execute_stream`], so this method is the
    /// non-streaming path by definition and never rewrites the payload.
    async fn execute(
        &self,
        auth: &AuthRecord,
        req: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        let (api_key, base_url) = self.resolve_credentials(auth);
        let model = requested_model(&req);
        let response = self
            .build_request(&req, false, &api_key, &base_url)?
            .send()
            .await?;

        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.bytes().await?.to_vec();
        if status >= 400 {
            return Err(ProviderError::Upstream {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }
        let usage = parse_openai_usage(&body).map(|t| t.to_record(model, self.provider));
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
        let (api_key, base_url) = self.resolve_credentials(auth);
        let model = requested_model(&req);
        let response = self
            .build_request(&req, true, &api_key, &base_url)?
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
        let provider = self.provider;
        Ok(stream_response(response, move |response, status| {
            usage_stream(
                response.bytes_stream(),
                DEFAULT_STREAM_IDLE_TIMEOUT,
                model,
                provider,
                parse_openai_stream_usage,
                status,
            )
        }))
    }

    /// A static API key never expires, so the record is only re-marked
    /// healthy.
    async fn refresh(&self, auth: &AuthRecord) -> Result<AuthRecord, ProviderError> {
        let mut refreshed = auth.clone();
        refreshed.status = AuthStatus::Active;
        refreshed.updated_at = chrono::Utc::now();
        Ok(refreshed)
    }

    /// Estimates token count from the payload byte length.
    async fn count_tokens(
        &self,
        _auth: &AuthRecord,
        req: ProviderRequest,
    ) -> Result<i64, ProviderError> {
        Ok(approximate_tokens_from_bytes(req.payload.len()))
    }
}

#[cfg(test)]
#[path = "openai_tests.rs"]
mod tests;
