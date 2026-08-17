//! **Vertex AI** (Gemini on Google Cloud) upstream.
//!
//! The longest of the five executors. Two things make it the odd one out:
//!
//! 1. **Credentials are minted, not stored.** A service account's RSA key signs
//!    a short-lived JWT assertion, which Google exchanges for an access token
//!    ([`VertexProvider::refresh`]). Signing per request would be wasteful, so
//!    minted tokens are cached until shortly before they expire.
//! 2. **The usage frame is unreliable.** `usageMetadata` is cumulative and may
//!    be split across TCP reads. This used to need a Vertex-only accumulator
//!    (per-chunk latch + a finish-time re-parse of the retained window, merged
//!    column-wise) because parsing *per chunk* simply cannot see a frame that
//!    straddles two reads. [`crate::streambuf::StreamUsageProbe`] now parses
//!    *per line* and carries the straddling half-line across frames, so the
//!    shared [`crate::common::usage_stream`] covers this case natively and the
//!    bespoke accumulator is gone — see `tests`.
//!
//! OWNER: worker `provider-claude`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use gw_authcore::{AuthRecord, AuthStatus};
use http::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use url::Url;

use crate::claude::shared::{
    self, append_query, default_content_negotiation, path_escape, trim_base_url, upstream_error,
};
use crate::common::{
    DEFAULT_STREAM_IDLE_TIMEOUT, PROVIDER_VERTEX, ProviderConfig, nested_string, requested_model,
    resolve_timeout, shared_client, stream_response, string_from_map, usage_stream,
};
use crate::types::{
    Provider, ProviderError, ProviderRequest, ProviderResponse, StreamResponse,
    copy_outbound_headers,
};
use crate::usage::{UsageTokens, parse_vertex_usage};

const VERTEX_DEFAULT_LOCATION: &str = "us-central1";
const VERTEX_DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const VERTEX_CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const VERTEX_JWT_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

const META_ACCESS_TOKEN: &str = "access_token";
const META_EXPIRES_AT: &str = "expires_at";
const META_EXPIRED: &str = "expired";
const META_LAST_REFRESH: &str = "last_refresh";
const META_TOKEN_DATA: &str = "token_data";

/// Metadata key holding the raw `service_account` JSON on an [`AuthRecord`].
pub const VERTEX_METADATA_SERVICE_ACCOUNT: &str = "service_account";

/// How long before nominal expiry a token counts as spent.
///
/// A token that expires mid-flight is worse than one minted a little early.
const VERTEX_REFRESH_SKEW: chrono::TimeDelta = chrono::TimeDelta::minutes(2);

/// Assumed lifetime when the token endpoint omits `expires_in`.
const VERTEX_TOKEN_FALLBACK_EXPIRATION: chrono::TimeDelta = chrono::TimeDelta::hours(1);

/// Lifetime of the signed assertion itself, in seconds. Google caps this at an
/// hour; the token it buys is what actually gets cached.
const VERTEX_ASSERTION_LIFETIME_SECS: i64 = 3600;

/// Vertex AI executor.
#[derive(Debug)]
pub struct VertexProvider {
    base_url: String,
    service_account_json: String,
    timeout: Duration,
    client: reqwest::Client,
    /// Access tokens minted so far, keyed by [`AuthRecord::id`].
    ///
    /// The [`Provider`] trait takes `&AuthRecord`, so the "sign once, reuse
    /// until it expires" behaviour lives here instead of on the record itself
    /// (which would need shared-mutable-credential aliasing).
    token_cache: Mutex<HashMap<String, CachedToken>>,
}

#[derive(Debug, Clone)]
struct CachedToken {
    token: String,
    expires_at: DateTime<Utc>,
}

/// The fields of a Google service-account key this executor needs.
#[derive(Debug, Clone, Default, Deserialize)]
struct VertexServiceAccount {
    #[serde(default)]
    client_email: String,
    #[serde(default)]
    private_key: String,
    #[serde(default)]
    token_uri: String,
    #[serde(default)]
    project_id: String,
}

#[derive(Debug, Default, Deserialize)]
struct VertexTokenResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    expires_in: i64,
}

/// Claims of the JWT assertion exchanged for an access token.
#[derive(Debug, Serialize)]
struct VertexAssertionClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    iat: i64,
    exp: i64,
}

/// Endpoint coordinates resolved from a record's attributes and its service
/// account.
#[derive(Debug, Clone, PartialEq, Eq)]
struct VertexEndpoint {
    base_url: String,
    project: String,
    location: String,
}

impl VertexProvider {
    /// Builds an executor from provider config.
    ///
    /// `cfg.api_key` carries the raw service-account JSON. A blank `base_url`
    /// is legal — unlike the other providers, the real host is derived from
    /// the resolved location.
    pub fn new(cfg: &ProviderConfig, timeout_seconds: i64) -> Result<Self, ProviderError> {
        let base_url = trim_base_url(&cfg.base_url);
        if !base_url.is_empty() {
            shared::require_absolute(&base_url, "invalid vertex base_url")?;
        }
        Ok(Self {
            base_url,
            service_account_json: cfg.api_key.trim().to_owned(),
            timeout: resolve_timeout(timeout_seconds),
            client: shared_client(),
            token_cache: Mutex::new(HashMap::new()),
        })
    }

    /// The configured upstream base URL; empty means "derive from location".
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The configured service-account JSON, used to seed persisted records.
    #[must_use]
    pub fn service_account_json(&self) -> &str {
        &self.service_account_json
    }

    /// Resolves the access token and endpoint for one request.
    ///
    /// The endpoint is resolved before the credential deliberately: a record
    /// with no project is unusable however good its key is, and saying so is
    /// free, whereas minting a token costs a signature and a round trip.
    async fn credentials_for_request(
        &self,
        auth: Option<&AuthRecord>,
    ) -> Result<(String, VertexEndpoint), ProviderError> {
        let service_account = self.resolve_service_account(auth);
        let endpoint = self.resolve_endpoint_settings(auth, service_account.as_ref().ok());
        if endpoint.project.is_empty() {
            return Err(ProviderError::Credential(
                "vertex project is required".to_owned(),
            ));
        }
        let now = Utc::now();
        if let Some(token) = Self::cached_access_token(auth, now) {
            return Ok((token, endpoint));
        }
        if let Some(token) = self.cached_executor_token(auth, now) {
            return Ok((token, endpoint));
        }
        // Only now does a missing or malformed key actually block the request.
        service_account?;

        let Some(auth) = auth else {
            return Err(ProviderError::Credential(
                "vertex access token refresh did not return a usable token".to_owned(),
            ));
        };
        let refreshed = self.refresh_auth(auth).await?;
        match Self::cached_access_token(Some(&refreshed), Utc::now()) {
            Some(token) => {
                self.store_executor_token(&refreshed);
                Ok((token, endpoint))
            }
            None => Err(ProviderError::Credential(
                "vertex access token refresh did not return a usable token".to_owned(),
            )),
        }
    }

    /// Signs an assertion, exchanges it, and stamps the result onto a clone of
    /// the record.
    async fn refresh_auth(&self, auth: &AuthRecord) -> Result<AuthRecord, ProviderError> {
        let mut refreshed = auth.clone();
        let service_account = self.resolve_service_account(Some(auth))?;
        let token = self.refresh_service_account_token(&service_account).await?;

        let now = Utc::now();
        let expires_at = if token.expires_in > 0 {
            now + chrono::TimeDelta::seconds(token.expires_in)
        } else {
            now + VERTEX_TOKEN_FALLBACK_EXPIRATION
        };
        let expires_at = shared::rfc3339(expires_at);
        let last_refresh = shared::rfc3339(now);
        let token_data = Self::updated_token_data(
            refreshed.metadata.get(META_TOKEN_DATA),
            &token.access_token,
            &expires_at,
            &last_refresh,
        );
        {
            let metadata = shared::metadata_object_mut(&mut refreshed.metadata);
            metadata.insert(META_ACCESS_TOKEN.to_owned(), token.access_token.into());
            metadata.insert(META_EXPIRES_AT.to_owned(), expires_at.clone().into());
            metadata.insert(META_EXPIRED.to_owned(), expires_at.into());
            metadata.insert(META_LAST_REFRESH.to_owned(), last_refresh.into());
            metadata.insert(META_TOKEN_DATA.to_owned(), Value::Object(token_data));
        }
        refreshed.status = AuthStatus::Active;
        refreshed.updated_at = now;
        refreshed.last_refreshed_at = Some(now);
        Ok(refreshed)
    }

    /// Merges the fresh token into whatever `token_data` the record already
    /// carried, rather than replacing it, so keys written by other tooling
    /// survive a refresh.
    fn updated_token_data(
        existing: Option<&Value>,
        access_token: &str,
        expires_at: &str,
        last_refresh: &str,
    ) -> Map<String, Value> {
        let mut token_data = existing.and_then(map_from_value).unwrap_or_default();
        token_data.insert(META_ACCESS_TOKEN.to_owned(), access_token.into());
        token_data.insert(META_EXPIRES_AT.to_owned(), expires_at.into());
        token_data.insert(META_EXPIRED.to_owned(), expires_at.into());
        token_data.insert(META_LAST_REFRESH.to_owned(), last_refresh.into());
        token_data
    }

    /// A token stored on the record, if its own expiry still vouches for it.
    fn cached_access_token(auth: Option<&AuthRecord>, now: DateTime<Utc>) -> Option<String> {
        let metadata = &auth?.metadata;
        if let Some(token) = string_from_map(metadata, META_ACCESS_TOKEN)
            && token_still_valid(metadata, now)
        {
            return Some(token);
        }
        if let Some(token) = nested_string(metadata, META_TOKEN_DATA, META_ACCESS_TOKEN)
            && nested_token_still_valid(metadata, now)
        {
            return Some(token);
        }
        None
    }

    /// A token this executor minted earlier for the same record.
    fn cached_executor_token(
        &self,
        auth: Option<&AuthRecord>,
        now: DateTime<Utc>,
    ) -> Option<String> {
        let id = &auth?.id;
        let cache = self.token_cache.lock().ok()?;
        let cached = cache.get(id)?;
        (cached.expires_at > now + VERTEX_REFRESH_SKEW).then(|| cached.token.clone())
    }

    /// Records a freshly minted token. An undated one is not cached: without an
    /// expiry there is nothing to stop it being served forever.
    fn store_executor_token(&self, refreshed: &AuthRecord) {
        let Some(token) = string_from_map(&refreshed.metadata, META_ACCESS_TOKEN) else {
            return;
        };
        let Some(expires_at) = string_from_map(&refreshed.metadata, META_EXPIRES_AT)
            .as_deref()
            .and_then(shared::parse_rfc3339)
        else {
            return;
        };
        if let Ok(mut cache) = self.token_cache.lock() {
            cache.insert(refreshed.id.clone(), CachedToken { token, expires_at });
        }
    }

    /// Finds and validates the service-account key.
    ///
    /// Looks in `metadata.service_account`, then
    /// `metadata.token_data.service_account`, then the persisted credential
    /// blob, and finally the executor's configured JSON.
    fn resolve_service_account(
        &self,
        auth: Option<&AuthRecord>,
    ) -> Result<VertexServiceAccount, ProviderError> {
        let mut raw = String::new();
        if let Some(auth) = auth {
            let metadata = &auth.metadata;
            raw = service_account_string(metadata.get(VERTEX_METADATA_SERVICE_ACCOUNT))
                .or_else(|| {
                    service_account_string(
                        metadata
                            .get(META_TOKEN_DATA)
                            .and_then(|value| value.get(VERTEX_METADATA_SERVICE_ACCOUNT)),
                    )
                })
                .or_else(|| service_account_string(metadata.get("storage")))
                .unwrap_or_default();
        }
        if raw.is_empty() {
            raw.clone_from(&self.service_account_json);
        }
        if raw.is_empty() {
            return Err(ProviderError::Credential(
                "vertex service_account is required".to_owned(),
            ));
        }
        let mut sa: VertexServiceAccount = serde_json::from_str(&raw).map_err(|_| {
            ProviderError::Credential("vertex service_account must be valid JSON".to_owned())
        })?;
        if sa.client_email.trim().is_empty() {
            return Err(ProviderError::Credential(
                "vertex service_account missing client_email".to_owned(),
            ));
        }
        if sa.private_key.trim().is_empty() {
            return Err(ProviderError::Credential(
                "vertex service_account missing private_key".to_owned(),
            ));
        }
        if sa.token_uri.trim().is_empty() {
            sa.token_uri = VERTEX_DEFAULT_TOKEN_URI.to_owned();
        }
        Ok(sa)
    }

    /// Resolves the host, project, and region for a record.
    ///
    /// Record attributes win over the service account's own `project_id`,
    /// which is what lets one key serve several projects.
    fn resolve_endpoint_settings(
        &self,
        auth: Option<&AuthRecord>,
        sa: Option<&VertexServiceAccount>,
    ) -> VertexEndpoint {
        let mut base_url = trim_base_url(&self.base_url);
        let mut location = VERTEX_DEFAULT_LOCATION.to_owned();
        let mut project = String::new();

        if let Some(auth) = auth {
            if let Some(override_url) = shared::base_url_attribute(auth) {
                base_url = override_url;
            }
            if let Some(value) = first_attribute(auth, &["project", "project_id"]) {
                project = value;
            }
            if let Some(value) = first_attribute(auth, &["location", "region"]) {
                location = value;
            }
        }
        if project.is_empty()
            && let Some(sa) = sa
        {
            project = sa.project_id.trim().to_owned();
        }
        if base_url.is_empty() {
            base_url = format!("https://{location}-aiplatform.googleapis.com");
        }
        VertexEndpoint {
            base_url,
            project,
            location,
        }
    }

    /// Signs the OAuth assertion.
    ///
    /// `jsonwebtoken`'s PEM loader accepts both PKCS#1 (`BEGIN RSA PRIVATE
    /// KEY`) and PKCS#8 (`BEGIN PRIVATE KEY`), so the two-step fallback
    /// collapses into one call.
    fn signed_assertion(
        sa: &VertexServiceAccount,
        now: DateTime<Utc>,
    ) -> Result<String, ProviderError> {
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(sa.private_key.trim().as_bytes())
            .map_err(|_| {
                ProviderError::Credential(
                    "vertex service_account private_key must be a PEM-encoded RSA key".to_owned(),
                )
            })?;
        let issued_at = now.timestamp();
        let claims = VertexAssertionClaims {
            iss: sa.client_email.trim(),
            scope: VERTEX_CLOUD_PLATFORM_SCOPE,
            aud: sa.token_uri.trim(),
            iat: issued_at,
            exp: issued_at + VERTEX_ASSERTION_LIFETIME_SECS,
        };
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        jsonwebtoken::encode(&header, &claims, &key).map_err(|_| {
            ProviderError::Credential("signing vertex service_account assertion failed".to_owned())
        })
    }

    /// Exchanges a signed assertion for an access token.
    ///
    /// Transport and upstream errors are deliberately re-worded rather than
    /// propagated: the assertion travels in the request body, and a formatted
    /// `reqwest` error can quote the request it failed on.
    async fn refresh_service_account_token(
        &self,
        sa: &VertexServiceAccount,
    ) -> Result<VertexTokenResponse, ProviderError> {
        let assertion = Self::signed_assertion(sa, Utc::now())?;
        let response = self
            .client
            .post(sa.token_uri.trim())
            .timeout(self.timeout)
            .header(http::header::ACCEPT, "application/json")
            .form(&[
                ("grant_type", VERTEX_JWT_GRANT_TYPE),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|_| {
                ProviderError::Credential("vertex token refresh request failed".to_owned())
            })?;
        let status = response.status().as_u16();
        let payload = response.bytes().await?;
        if status >= 400 {
            return Err(ProviderError::Credential(format!(
                "vertex token refresh failed with upstream status {status}"
            )));
        }
        let token: VertexTokenResponse = serde_json::from_slice(&payload).map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("parsing vertex token response: {err}"))
        })?;
        if token.access_token.trim().is_empty() {
            return Err(ProviderError::Credential(
                "vertex token response missing access_token".to_owned(),
            ));
        }
        Ok(token)
    }

    /// Builds the publisher-model endpoint.
    fn generate_content_endpoint(
        query: &[(String, String)],
        endpoint: &VertexEndpoint,
        model: &str,
        stream: bool,
    ) -> Result<Url, ProviderError> {
        let mut base = trim_base_url(&endpoint.base_url);
        if base.is_empty() {
            base = format!("https://{}-aiplatform.googleapis.com", endpoint.location);
        }
        shared::require_absolute(&base, "invalid vertex base_url")?;

        let action = if stream {
            "streamGenerateContent"
        } else {
            "generateContent"
        };
        let url = format!(
            "{base}/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:{action}",
            project = path_escape(&endpoint.project),
            location = path_escape(&endpoint.location),
            model = path_escape(strip_vertex_prefix(model)),
        );
        let mut parsed = Url::parse(&url).map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("invalid vertex base_url: {err}"))
        })?;
        append_query(&mut parsed, query);
        Ok(parsed)
    }

    /// Assembles an outbound GenerateContent request.
    fn build_request(
        &self,
        req: &ProviderRequest,
        stream: bool,
        access_token: &str,
        endpoint: &VertexEndpoint,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        if access_token.is_empty() {
            return Err(ProviderError::Credential(
                "vertex access token is required".to_owned(),
            ));
        }
        let model = vertex_requested_model(req);
        if model.is_empty() {
            return Err(ProviderError::Other(anyhow::anyhow!(
                "vertex model is required"
            )));
        }
        let url = Self::generate_content_endpoint(&req.query, endpoint, &model, stream)?;

        let mut headers = HeaderMap::new();
        copy_outbound_headers(&mut headers, &req.headers);
        default_content_negotiation(&mut headers, stream);
        headers.insert(
            http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {access_token}")).map_err(|_| {
                ProviderError::Credential(
                    "vertex access token is not a valid header value".to_owned(),
                )
            })?,
        );

        let mut builder = self
            .client
            .post(url)
            .headers(headers)
            .body(req.payload.clone());
        if !stream {
            builder = builder.timeout(self.timeout);
        }
        Ok(builder)
    }
}

/// The upstream-facing model name.
///
/// The router may hand over a `vertex/`-qualified name, but the publisher path
/// already says which publisher this is.
fn vertex_requested_model(req: &ProviderRequest) -> String {
    strip_vertex_prefix(requested_model(req)).to_owned()
}

fn strip_vertex_prefix(model: &str) -> &str {
    let trimmed = model.trim();
    trimmed
        .strip_prefix(PROVIDER_VERTEX)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(trimmed)
}

fn token_still_valid(metadata: &Value, now: DateTime<Utc>) -> bool {
    metadata_expiry_valid(metadata, META_EXPIRES_AT, now)
        || metadata_expiry_valid(metadata, META_EXPIRED, now)
}

/// A token nested under `token_data` is governed by the expiry nested beside
/// it — never by an unrelated top-level one, which may belong to a different
/// token.
fn nested_token_still_valid(metadata: &Value, now: DateTime<Utc>) -> bool {
    let Some(token_data) = metadata.get(META_TOKEN_DATA).and_then(map_from_value) else {
        return false;
    };
    let token_data = Value::Object(token_data);
    metadata_expiry_valid(&token_data, META_EXPIRES_AT, now)
        || metadata_expiry_valid(&token_data, META_EXPIRED, now)
}

/// A missing or unparseable stamp reads as expired, so a malformed record
/// re-mints instead of sending a dead token.
fn metadata_expiry_valid(values: &Value, key: &str, now: DateTime<Utc>) -> bool {
    string_from_map(values, key)
        .as_deref()
        .and_then(shared::parse_rfc3339)
        .is_some_and(|expires_at| expires_at > now + VERTEX_REFRESH_SKEW)
}

/// The blob may be stored as JSON text, as a nested object, or wrapped in
/// another object under `service_account`.
fn service_account_string(raw: Option<&Value>) -> Option<String> {
    match raw? {
        Value::Null => None,
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        value @ Value::Object(map) => {
            service_account_string(map.get(VERTEX_METADATA_SERVICE_ACCOUNT))
                .or_else(|| Some(value.to_string()))
        }
        other => Some(other.to_string()),
    }
}

/// Accepts an object, or a string holding an object.
fn map_from_value(raw: &Value) -> Option<Map<String, Value>> {
    match raw {
        Value::Object(map) => Some(map.clone()),
        Value::String(text) => serde_json::from_str(text.trim()).ok(),
        _ => None,
    }
}

/// First non-blank value among `keys`, trimmed.
fn first_attribute(auth: &AuthRecord, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| auth.attributes.get(*key))
        .map(|value| value.trim().to_owned())
        .find(|value| !value.is_empty())
}

/// Returns the latest `usageMetadata` inside one streamed chunk.
///
/// Every JSON-object line is tried, whether or not it carries an SSE `data:`
/// prefix, because Vertex answers `streamGenerateContent` with SSE for some
/// callers and a chunked JSON array for others.
#[must_use]
pub fn extract_latest_vertex_usage(chunk: &[u8]) -> Option<UsageTokens> {
    if chunk.trim_ascii().is_empty() {
        return None;
    }
    let mut latest = parse_vertex_usage(chunk);
    for line in chunk.split(|&b| b == b'\n') {
        let mut line = line.trim_ascii();
        if let Some(rest) = line.strip_prefix(b"data:".as_slice()) {
            line = rest.trim_ascii();
        }
        if line.first() != Some(&b'{') {
            continue;
        }
        if let Some(tokens) = parse_vertex_usage(line) {
            latest = Some(tokens);
        }
    }
    latest
}

#[async_trait::async_trait]
impl Provider for VertexProvider {
    fn name(&self) -> &'static str {
        PROVIDER_VERTEX
    }

    async fn execute(
        &self,
        auth: &AuthRecord,
        req: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        let (access_token, endpoint) = self.credentials_for_request(Some(auth)).await?;
        let model = vertex_requested_model(&req);
        let response = self
            .build_request(&req, false, &access_token, &endpoint)?
            .send()
            .await?;

        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.bytes().await?;
        if status >= 400 {
            return Err(upstream_error(status, &body));
        }
        Ok(ProviderResponse {
            status,
            headers,
            usage: parse_vertex_usage(&body).map(|t| t.to_record(model, PROVIDER_VERTEX)),
            body,
        })
    }

    async fn execute_stream(
        &self,
        auth: &AuthRecord,
        req: ProviderRequest,
    ) -> Result<StreamResponse, ProviderError> {
        let (access_token, endpoint) = self.credentials_for_request(Some(auth)).await?;
        let model = vertex_requested_model(&req);
        let response = self
            .build_request(&req, true, &access_token, &endpoint)?
            .send()
            .await?;

        let status = response.status().as_u16();
        if status >= 400 {
            let body = response.bytes().await.unwrap_or_default();
            return Err(upstream_error(status, &body));
        }
        Ok(stream_response(response, move |response, status| {
            // 曾经这里是一个 Vertex 专用的 `vertex_usage_stream`：per-chunk latch
            // 加收尾时对整个窗口再解析一遍再取列最大值，60 行代码只为了兜住
            // 「终局帧被读边界切成两半」。增量行解析把跨帧半行天然接上了，
            // 共享的 `usage_stream` 就够了 —— 见 `streambuf.rs` 模块文档。
            usage_stream(
                response.bytes_stream(),
                DEFAULT_STREAM_IDLE_TIMEOUT,
                model,
                PROVIDER_VERTEX,
                extract_latest_vertex_usage,
                status,
            )
        }))
    }

    async fn refresh(&self, auth: &AuthRecord) -> Result<AuthRecord, ProviderError> {
        let refreshed = self.refresh_auth(auth).await?;
        self.store_executor_token(&refreshed);
        Ok(refreshed)
    }

    /// **报错，不编数字** —— 理由与 [`crate::gemini::GeminiProvider::count_tokens`]
    /// 逐字相同：Vertex 上游确实有 `:countTokens`，但 `count_tokens` 的唯一入口
    /// `POST /v1/messages/count_tokens` 是 **Anthropic 方言**，body 原样送过去
    /// Google 会因未知字段回 400。
    ///
    /// 这里原来返回 `payload.len() / 4` 的伪造值
    /// （`docs/relay-surface-plan.md` §2.1 缺陷 ①），且那个数还在按 LLM 价格计费。
    /// 接上 `gw_relay::translate::google` 转义器之前，明确报错比假数字诚实。
    async fn count_tokens(
        &self,
        _auth: &AuthRecord,
        _req: ProviderRequest,
    ) -> Result<i64, ProviderError> {
        Err(ProviderError::Other(anyhow::anyhow!(
            "{PROVIDER_VERTEX} token counting is unavailable: the only entry point is the \
             Anthropic-dialect POST /v1/messages/count_tokens, and reaching Vertex's \
             :countTokens needs the anthropic->google translator wired into that path"
        )))
    }
}

#[cfg(test)]
mod tests;
