//! Google **Generative Language API** (Gemini) upstream.
//!
//! Gemini authenticates with a plain API key rather than OAuth, so the
//! load-bearing logic here is endpoint assembly — `:generateContent` vs
//! `:streamGenerateContent`, and the forced `alt=sse` framing — plus the two
//! stream shapes the upstream may answer with.
//!
//! OWNER: worker `provider-claude`.

use std::time::Duration;

use gw_authcore::{AuthRecord, AuthStatus};
use http::{HeaderMap, HeaderValue};
use url::Url;

use crate::claude::shared::{
    self, append_query, default_content_negotiation, path_escape, set_query, trim_base_url,
    upstream_error,
};
use crate::common::{
    DEFAULT_STREAM_IDLE_TIMEOUT, PROVIDER_GEMINI, ProviderConfig, approximate_tokens_from_bytes,
    nested_string, requested_model, resolve_timeout, shared_client, stream_response,
    string_from_map, usage_stream,
};
use crate::types::{
    Provider, ProviderError, ProviderRequest, ProviderResponse, StreamResponse,
    copy_outbound_headers,
};
use crate::usage::{UsageTokens, parse_gemini_usage};

const GEMINI_DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const GEMINI_MODELS_PATH: &str = "/v1beta/models/";
const GEMINI_METADATA_API_KEY: &str = "api_key";
const GEMINI_TOKEN_DATA: &str = "token_data";
const GEMINI_ACCESS_TOKEN: &str = "access_token";
/// Query parameter selecting the response framing. The relay parses SSE, so
/// this is provider-owned and not caller-overridable.
const GEMINI_ALT_QUERY: &str = "alt";

/// Generative Language API executor.
#[derive(Debug, Clone)]
pub struct GeminiProvider {
    base_url: String,
    api_key: String,
    timeout: Duration,
    client: reqwest::Client,
}

impl GeminiProvider {
    /// Builds an executor from provider config.
    pub fn new(cfg: &ProviderConfig, timeout_seconds: i64) -> Result<Self, ProviderError> {
        let mut base_url = trim_base_url(&cfg.base_url);
        if base_url.is_empty() {
            base_url = GEMINI_DEFAULT_BASE_URL.to_owned();
        }
        shared::require_absolute(&base_url, "invalid gemini base_url")?;
        Ok(Self {
            base_url,
            api_key: cfg.api_key.trim().to_owned(),
            timeout: resolve_timeout(timeout_seconds),
            client: shared_client(),
        })
    }

    /// The configured upstream base URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Resolves the API key and base URL for one request.
    ///
    /// The ladder, highest rung first: `metadata.api_key`,
    /// `metadata.token_data.api_key`, `metadata.access_token`,
    /// `metadata.token_data.access_token`, then the executor's configured key.
    /// An OAuth access token is accepted in the API-key slot because Google
    /// honours both on this endpoint.
    fn resolve_credentials(&self, auth: Option<&AuthRecord>) -> (String, String) {
        let fallback = self.api_key.trim().to_owned();
        let mut base_url = trim_base_url(&self.base_url);
        if base_url.is_empty() {
            base_url = GEMINI_DEFAULT_BASE_URL.to_owned();
        }
        let Some(auth) = auth else {
            return (fallback, base_url);
        };
        if let Some(override_url) = shared::base_url_attribute(auth) {
            base_url = override_url;
        }
        let resolved = string_from_map(&auth.metadata, GEMINI_METADATA_API_KEY)
            .or_else(|| nested_string(&auth.metadata, GEMINI_TOKEN_DATA, GEMINI_METADATA_API_KEY))
            .or_else(|| string_from_map(&auth.metadata, GEMINI_ACCESS_TOKEN))
            .or_else(|| nested_string(&auth.metadata, GEMINI_TOKEN_DATA, GEMINI_ACCESS_TOKEN))
            .unwrap_or(fallback);
        (resolved, base_url)
    }

    /// Builds the `:generateContent` / `:streamGenerateContent` endpoint.
    ///
    /// `alt=sse` is `set` after the caller's parameters are appended, so a
    /// caller cannot downgrade the framing the usage relay is built to parse.
    fn generate_content_endpoint(
        query: &[(String, String)],
        base_url: &str,
        model: &str,
        stream: bool,
    ) -> Result<Url, ProviderError> {
        let mut base = trim_base_url(base_url);
        if base.is_empty() {
            base = GEMINI_DEFAULT_BASE_URL.to_owned();
        }
        shared::require_absolute(&base, "invalid gemini base_url")?;

        let action = if stream {
            "streamGenerateContent"
        } else {
            "generateContent"
        };
        let endpoint = format!(
            "{base}{GEMINI_MODELS_PATH}{model}:{action}",
            model = path_escape(model)
        );
        let mut parsed = Url::parse(&endpoint).map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("invalid gemini base_url: {err}"))
        })?;

        let mut params = query.to_vec();
        if stream {
            set_query(&mut params, GEMINI_ALT_QUERY, "sse");
        }
        append_query(&mut parsed, &params);
        Ok(parsed)
    }

    /// Stamps the API key onto an outbound header map.
    ///
    /// With no configured key nothing is injected and the request still goes
    /// out: the caller may have supplied `?key=` on the URL itself, and Google
    /// — not the gateway — is the right place to reject a request with neither.
    fn inject_api_key(headers: &mut HeaderMap, api_key: &str) -> Result<(), ProviderError> {
        let trimmed = api_key.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let value = HeaderValue::from_str(trimmed).map_err(|_| {
            ProviderError::Credential("gemini api key is not a valid header value".to_owned())
        })?;
        headers.insert("x-goog-api-key", value);
        Ok(())
    }

    /// Assembles an outbound GenerateContent request.
    fn build_request(
        &self,
        req: &ProviderRequest,
        stream: bool,
        api_key: &str,
        base_url: &str,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        let model = requested_model(req);
        if model.is_empty() {
            return Err(ProviderError::Other(anyhow::anyhow!(
                "gemini model is required"
            )));
        }
        let endpoint = Self::generate_content_endpoint(&req.query, base_url, &model, stream)?;

        let mut headers = HeaderMap::new();
        copy_outbound_headers(&mut headers, &req.headers);
        default_content_negotiation(&mut headers, stream);
        Self::inject_api_key(&mut headers, api_key)?;

        let mut builder = self
            .client
            .post(endpoint)
            .headers(headers)
            .body(req.payload.clone());
        if !stream {
            builder = builder.timeout(self.timeout);
        }
        Ok(builder)
    }
}

/// Scans a `streamGenerateContent` body for the last chunk carrying
/// `usageMetadata`.
///
/// Three framings are tolerated because the upstream picks between them: a
/// plain JSON body, `alt=sse` `data:` frames, and blank-line-separated JSON
/// chunks. `usageMetadata` is cumulative, so the *last* parse — not a merge —
/// is the authoritative one.
#[must_use]
pub fn parse_gemini_stream_usage(body: &[u8]) -> Option<UsageTokens> {
    if let Some(tokens) = parse_gemini_usage(body) {
        return Some(tokens);
    }

    let mut last = None;
    for line in body.split(|&b| b == b'\n') {
        let Some(payload) = shared::sse_data_payload(line) else {
            continue;
        };
        if let Some(tokens) = parse_gemini_usage(payload) {
            last = Some(tokens);
        }
    }
    if last.is_some() {
        return last;
    }
    for chunk in split_on_blank_line(body) {
        if let Some(tokens) = parse_gemini_usage(chunk) {
            last = Some(tokens);
        }
    }
    last
}

/// Splits on blank lines, dropping blank pieces.
fn split_on_blank_line(body: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut idx = 0;
    while idx + 1 < body.len() {
        if body[idx] == b'\n' && body[idx + 1] == b'\n' {
            chunks.push(&body[start..idx]);
            idx += 2;
            start = idx;
        } else {
            idx += 1;
        }
    }
    chunks.push(&body[start..]);
    chunks
        .into_iter()
        .map(<[u8]>::trim_ascii)
        .filter(|chunk| !chunk.is_empty())
}

#[async_trait::async_trait]
impl Provider for GeminiProvider {
    fn name(&self) -> &'static str {
        PROVIDER_GEMINI
    }

    async fn execute(
        &self,
        auth: &AuthRecord,
        req: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        let (api_key, base_url) = self.resolve_credentials(Some(auth));
        let model = requested_model(&req);
        let response = self
            .build_request(&req, false, &api_key, &base_url)?
            .send()
            .await?;

        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.bytes().await?.to_vec();
        if status >= 400 {
            return Err(upstream_error(status, &body));
        }
        Ok(ProviderResponse {
            status,
            headers,
            usage: parse_gemini_usage(&body).map(|t| t.to_record(model, PROVIDER_GEMINI)),
            body,
        })
    }

    async fn execute_stream(
        &self,
        auth: &AuthRecord,
        req: ProviderRequest,
    ) -> Result<StreamResponse, ProviderError> {
        let (api_key, base_url) = self.resolve_credentials(Some(auth));
        let model = requested_model(&req);
        let response = self
            .build_request(&req, true, &api_key, &base_url)?
            .send()
            .await?;

        let status = response.status().as_u16();
        if status >= 400 {
            let body = response.bytes().await.unwrap_or_default();
            return Err(upstream_error(status, &body));
        }
        Ok(stream_response(response, move |response, status| {
            usage_stream(
                response.bytes_stream(),
                DEFAULT_STREAM_IDLE_TIMEOUT,
                model,
                PROVIDER_GEMINI,
                parse_gemini_stream_usage,
                status,
            )
        }))
    }

    /// Deliberately a no-op: Gemini records hold an API key, so there is
    /// nothing to rotate. They are
    /// re-marked healthy rather than rejected, so loading a persisted
    /// OAuth-shaped record at startup does not fail the whole credential set.
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
mod tests;
