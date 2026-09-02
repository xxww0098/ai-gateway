//! Anthropic **Messages API** upstream.
//!
//! This module owns the whole HTTP lifecycle — credential resolution, OAuth
//! refresh, endpoint assembly, and side-band usage extraction — rather than
//! delegating any of it to a vendored SDK.
//!
//! OWNER: worker `provider-claude`. The [`shared`] submodule additionally backs
//! `gemini.rs` and `vertex.rs`; see its docs for why it lives here.

use std::time::Duration;

use gw_authcore::{AuthRecord, AuthStatus};
use http::{HeaderMap, HeaderValue};
use serde::Deserialize;
use url::Url;

use crate::common::{
    PROVIDER_CLAUDE, ProviderConfig, Redacted, nested_string, relay_timeouts, resolve_timeout,
    string_from_map,
};
use crate::route::{RoutePlan, RoutePlanner};
use crate::types::{ProviderError, ProviderRequest};
use crate::usage::{UsageTokens, max_usage_tokens, parse_claude_usage};
use gw_relay::{Credential, UpstreamDialect};

use shared::{append_query, default_content_negotiation};

const CLAUDE_DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const CLAUDE_MESSAGES_PATH: &str = "/v1/messages";
const CLAUDE_ANTHROPIC_VERSION: &str = "2023-06-01";
const CLAUDE_OAUTH_TOKEN_URL: &str = "https://api.anthropic.com/v1/oauth/token";
const CLAUDE_OAUTH_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

const META_API_KEY: &str = "api_key";
const META_ACCESS_TOKEN: &str = "access_token";
const META_REFRESH_TOKEN: &str = "refresh_token";
const META_TOKEN_DATA: &str = "token_data";
const META_EXPIRES_AT: &str = "expires_at";
const META_EXPIRED: &str = "expired";
const META_LAST_REFRESH: &str = "last_refresh";
const META_EMAIL: &str = "email";

/// Anthropic Messages executor.
///
/// One pooled client suffices because `reqwest` scopes the timeout per request;
/// a whole-request timeout would also bound *reading the body* and truncate a
/// healthy long stream, so streaming simply attaches none.
#[derive(Clone)]
pub struct ClaudeProvider {
    base_url: String,
    api_key: String,
    timeout: Duration,
}

impl std::fmt::Debug for ClaudeProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeProvider")
            .field("base_url", &self.base_url)
            .field("api_key", &Redacted(&self.api_key))
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// A resolved upstream credential plus which shape it came from.
///
/// `source` is what makes the precedence ladder in
/// [`ClaudeProvider::resolve_credentials`] observable without a socket.
#[derive(Clone, PartialEq, Eq)]
pub struct ClaudeCredential {
    /// The secret sent as `x-api-key`.
    pub value: String,
    /// Which rung of the ladder produced [`ClaudeCredential::value`].
    pub source: CredentialSource,
}

impl std::fmt::Debug for ClaudeCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeCredential")
            .field("value", &Redacted(&self.value))
            .field("source", &self.source)
            .finish()
    }
}

/// Which credential shape [`ClaudeProvider::resolve_credentials`] selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    /// A long-lived Anthropic console API key.
    ApiKey,
    /// An OAuth access token minted from a refresh token.
    OauthToken,
}

impl CredentialSource {
    /// The wire name this source maps to.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::OauthToken => "oauth_token",
        }
    }
}

/// Body of `POST /v1/oauth/token`.
#[derive(Default, Deserialize)]
struct ClaudeRefreshResponse {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    account: ClaudeRefreshAccount,
}

impl std::fmt::Debug for ClaudeRefreshResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeRefreshResponse")
            .field("access_token", &Redacted(&self.access_token))
            .field("refresh_token", &Redacted(&self.refresh_token))
            .field("expires_in", &self.expires_in)
            .field("account", &self.account)
            .finish()
    }
}

#[derive(Debug, Default, Deserialize)]
struct ClaudeRefreshAccount {
    #[serde(default)]
    email_address: String,
}

impl ClaudeProvider {
    /// Builds an executor from provider config.
    ///
    /// A blank base URL falls back to the public Anthropic host; an unparseable
    /// one is rejected at wiring time rather than on the first request.
    pub fn new(cfg: &ProviderConfig, timeout_seconds: i64) -> Result<Self, ProviderError> {
        let mut base_url = shared::trim_base_url(&cfg.base_url);
        if base_url.is_empty() {
            base_url = CLAUDE_DEFAULT_BASE_URL.to_owned();
        }
        shared::require_absolute(&base_url, "invalid claude base_url")?;
        Ok(Self {
            base_url,
            api_key: cfg.api_key.trim().to_owned(),
            timeout: resolve_timeout(timeout_seconds),
        })
    }

    /// The configured upstream base URL.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Resolves the credential and base URL for one request.
    ///
    /// The ladder, highest rung first: `metadata.api_key`,
    /// `metadata.token_data.api_key`, `metadata.access_token`,
    /// `metadata.token_data.access_token`, the persisted credential blob, then
    /// the executor's configured key. A per-record `base_url` / `base-url`
    /// attribute overrides the configured host, which is how one gateway fronts
    /// several relays.
    fn resolve_credentials(&self, auth: Option<&AuthRecord>) -> (ClaudeCredential, String) {
        let fallback = ClaudeCredential {
            value: self.api_key.clone(),
            source: CredentialSource::ApiKey,
        };
        let mut base_url = shared::trim_base_url(&self.base_url);
        if base_url.is_empty() {
            base_url = CLAUDE_DEFAULT_BASE_URL.to_owned();
        }
        let Some(auth) = auth else {
            return (fallback, base_url);
        };
        if let Some(override_url) = shared::base_url_attribute(auth) {
            base_url = override_url;
        }

        if let Some(value) = string_from_map(&auth.metadata, META_API_KEY)
            .or_else(|| nested_string(&auth.metadata, META_TOKEN_DATA, META_API_KEY))
        {
            return (
                ClaudeCredential {
                    value,
                    source: CredentialSource::ApiKey,
                },
                base_url,
            );
        }
        if let Some(value) = string_from_map(&auth.metadata, META_ACCESS_TOKEN)
            .or_else(|| nested_string(&auth.metadata, META_TOKEN_DATA, META_ACCESS_TOKEN))
            .or_else(|| Self::resolve_credential_from_storage(auth))
        {
            return (
                ClaudeCredential {
                    value,
                    source: CredentialSource::OauthToken,
                },
                base_url,
            );
        }
        (fallback, base_url)
    }

    /// Reads the refresh token from the metadata or the persisted blob.
    fn resolve_refresh_token(auth: Option<&AuthRecord>) -> Option<String> {
        let auth = auth?;
        string_from_map(&auth.metadata, META_REFRESH_TOKEN)
            .or_else(|| nested_string(&auth.metadata, META_TOKEN_DATA, META_REFRESH_TOKEN))
            .or_else(|| shared::string_field_from_storage(auth, "RefreshToken", META_REFRESH_TOKEN))
    }

    /// Reads a credential out of the persisted blob.
    fn resolve_credential_from_storage(auth: &AuthRecord) -> Option<String> {
        shared::string_field_from_storage(auth, "APIKey", META_API_KEY)
            .or_else(|| shared::string_field_from_storage(auth, "AccessToken", META_ACCESS_TOKEN))
    }

    /// Builds the Messages endpoint for a base URL and appends the inbound
    /// query parameters.
    ///
    /// The base may already point at the full path, at `/v1`, or at the bare
    /// origin; all three converge. The *base* is what gets validated, not the
    /// assembled endpoint: `url` is lenient about slashes for special schemes,
    /// so a hostless `https://` would otherwise re-parse with `v1` as the host.
    fn messages_endpoint(
        raw_query: Option<&str>,
        query: &[(String, String)],
        base_url: &str,
    ) -> Result<Url, ProviderError> {
        let mut base = shared::trim_base_url(base_url);
        if base.is_empty() {
            base = CLAUDE_DEFAULT_BASE_URL.to_owned();
        }
        shared::require_absolute(&base, "invalid claude base_url")?;

        let endpoint = if base.ends_with(CLAUDE_MESSAGES_PATH) {
            base
        } else if base.ends_with("/v1") {
            format!("{base}/messages")
        } else {
            format!("{base}{CLAUDE_MESSAGES_PATH}")
        };
        let mut parsed = Url::parse(&endpoint).map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("invalid claude base_url: {err}"))
        })?;
        append_query(&mut parsed, raw_query, query);
        Ok(parsed)
    }

    /// Anthropic 的计数端点：Messages 端点再拼一段 `/count_tokens`。
    ///
    /// 刻意复用 [`Self::messages_endpoint`] 而不是再写一遍 base 归一化 ——
    /// 三种 base 形态（全路径 / `/v1` / 裸 origin）的收敛规则只该有一处。
    fn count_tokens_endpoint(
        raw_query: Option<&str>,
        query: &[(String, String)],
        base_url: &str,
    ) -> Result<Url, ProviderError> {
        let mut parsed = Self::messages_endpoint(None, &[], base_url)?;
        let path = format!("{}/count_tokens", parsed.path().trim_end_matches('/'));
        parsed.set_path(&path);
        append_query(&mut parsed, raw_query, query);
        Ok(parsed)
    }

    /// The provider-owned headers for an outbound Messages request.
    ///
    /// `anthropic-version` only fills a gap, so a caller can still pin an older
    /// API version. The credential is **not** here — it travels as
    /// [`RoutePlan::credential`] and the relay is what strips the client's own
    /// `x-api-key` and marks the replacement sensitive.
    fn outbound_headers(req: &ProviderRequest, stream: bool) -> HeaderMap {
        let mut headers = HeaderMap::new();
        default_content_negotiation(&mut headers, stream);
        // `default_content_negotiation` only fills gaps in the map it is given,
        // so anything the client already sent has to be honoured here.
        for name in [http::header::CONTENT_TYPE, http::header::ACCEPT] {
            if req.headers.contains_key(&name) && !stream {
                headers.remove(&name);
            }
        }
        if !req.headers.contains_key("anthropic-version") {
            headers.insert(
                "anthropic-version",
                HeaderValue::from_static(CLAUDE_ANTHROPIC_VERSION),
            );
        }
        headers
    }

    /// Plans an outbound Messages request.
    fn plan_messages(
        &self,
        req: &ProviderRequest,
        credential: &ClaudeCredential,
        base_url: &str,
    ) -> Result<RoutePlan, ProviderError> {
        if credential.value.is_empty() {
            return Err(ProviderError::Credential(
                "claude credential is required".to_owned(),
            ));
        }
        Ok(RoutePlan {
            provider: PROVIDER_CLAUDE,
            endpoint: Self::messages_endpoint(req.raw_query.as_deref(), &req.query, base_url)?,
            credential: Credential::XApiKey(credential.value.clone()),
            headers: Self::outbound_headers(req, req.stream),
            body: None,
            timeouts: relay_timeouts(self.timeout),
            dialect: UpstreamDialect::AnthropicMessages,
        })
    }

    async fn refresh_oauth_token(
        &self,
        refresh_token: &str,
    ) -> Result<ClaudeRefreshResponse, ProviderError> {
        let payload = crate::oauth::post_json(
            CLAUDE_OAUTH_TOKEN_URL,
            self.timeout,
            "claude",
            &serde_json::json!({
                "client_id": CLAUDE_OAUTH_CLIENT_ID,
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
            }),
        )
        .await?;
        serde_json::from_slice(&payload).map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("parsing claude refresh response: {err}"))
        })
    }
}

/// Merges usage across every `data:` frame of an Anthropic SSE buffer.
///
/// Claude splits the tally in two — `message_start` carries `input_tokens`, the
/// trailing `message_delta` carries the final `output_tokens` — so no single
/// frame is authoritative and the column-wise maximum across all of them is
/// what settles correctly. A plain JSON body takes the fast path, for upstreams
/// that answer a stream request with a whole envelope.
#[must_use]
pub fn parse_claude_stream_usage(body: &[u8]) -> Option<UsageTokens> {
    if let Some(tokens) = parse_claude_usage(body) {
        return Some(tokens);
    }
    let mut merged = UsageTokens::default();
    let mut seen = false;
    for line in body.split(|&b| b == b'\n') {
        let Some(payload) = shared::sse_data_payload(line) else {
            continue;
        };
        let Some(tokens) = parse_claude_usage(payload) else {
            continue;
        };
        merged = max_usage_tokens(merged, tokens);
        seen = true;
    }
    seen.then_some(merged)
}

#[async_trait::async_trait]
impl RoutePlanner for ClaudeProvider {
    fn name(&self) -> &'static str {
        PROVIDER_CLAUDE
    }

    async fn plan(
        &self,
        auth: &AuthRecord,
        req: &ProviderRequest,
    ) -> Result<RoutePlan, ProviderError> {
        let (credential, base_url) = self.resolve_credentials(Some(auth));
        self.plan_messages(req, &credential, &base_url)
    }

    /// Anthropic is the only upstream with a real counting endpoint, and the
    /// inbound surface is already its own dialect, so this is an **identity
    /// forward**: the same body, the same credential injection, one path
    /// segment further along.
    ///
    /// The response is relayed to the client verbatim — the gateway does not
    /// re-shape `{"input_tokens": N}`, and it certainly does not fall back to
    /// an estimate when the upstream declines to answer.
    async fn plan_count_tokens(
        &self,
        auth: &AuthRecord,
        req: &ProviderRequest,
    ) -> Result<RoutePlan, ProviderError> {
        let (credential, base_url) = self.resolve_credentials(Some(auth));
        if credential.value.is_empty() {
            return Err(ProviderError::Credential(
                "claude credential is required".to_owned(),
            ));
        }
        Ok(RoutePlan {
            provider: PROVIDER_CLAUDE,
            endpoint: Self::count_tokens_endpoint(req.raw_query.as_deref(), &req.query, &base_url)?,
            credential: Credential::XApiKey(credential.value.clone()),
            headers: Self::outbound_headers(req, false),
            body: None,
            timeouts: relay_timeouts(self.timeout),
            dialect: UpstreamDialect::AnthropicMessages,
        })
    }

    /// A record with no refresh token is a plain API-key record: it is only
    /// re-marked healthy, so loading a mixed credential set at startup does not
    /// fail on the non-OAuth entries.
    async fn refresh(&self, auth: &AuthRecord) -> Result<AuthRecord, ProviderError> {
        let mut refreshed = auth.clone();
        let Some(refresh_token) = Self::resolve_refresh_token(Some(auth)) else {
            refreshed.status = AuthStatus::Active;
            refreshed.updated_at = chrono::Utc::now();
            return Ok(refreshed);
        };

        let token = self.refresh_oauth_token(&refresh_token).await?;
        let now = chrono::Utc::now();
        {
            let metadata = shared::metadata_object_mut(&mut refreshed.metadata);
            if !token.access_token.is_empty() {
                metadata.insert(META_ACCESS_TOKEN.to_owned(), token.access_token.into());
            }
            // A rotated refresh token replaces the old one; an omitted one means
            // the upstream kept it, so writing the original back is what stops a
            // second refresh from arriving with an empty credential.
            let rotated = if token.refresh_token.is_empty() {
                refresh_token
            } else {
                token.refresh_token
            };
            metadata.insert(META_REFRESH_TOKEN.to_owned(), rotated.into());
            if !token.account.email_address.is_empty() {
                metadata.insert(META_EMAIL.to_owned(), token.account.email_address.into());
            }
            if token.expires_in > 0 {
                let expires_at =
                    shared::rfc3339(now + chrono::TimeDelta::seconds(token.expires_in));
                metadata.insert(META_EXPIRES_AT.to_owned(), expires_at.clone().into());
                metadata.insert(META_EXPIRED.to_owned(), expires_at.into());
            }
            metadata.insert(META_LAST_REFRESH.to_owned(), shared::rfc3339(now).into());
        }
        refreshed.status = AuthStatus::Active;
        refreshed.updated_at = now;
        refreshed.last_refreshed_at = Some(now);
        Ok(refreshed)
    }
}

pub(crate) mod shared {
    //! Helpers shared by the three executors this worker owns
    //! (`claude` / `gemini` / `vertex`).
    //!
    //! They live inside `claude.rs` rather than in `common.rs` because
    //! `common.rs` belongs to worker `provider-openai` and `lib.rs`'s module
    //! list is coordinator-owned, so no new top-level file can be declared.
    //! One copy here beats three that drift.

    use gw_authcore::AuthRecord;
    use http::{HeaderMap, HeaderValue, header};
    use url::Url;

    use crate::common::nested_string;
    use crate::types::ProviderError;

    /// Trims surrounding whitespace and a trailing `/`.
    pub(crate) fn trim_base_url(raw: &str) -> String {
        raw.trim().trim_end_matches('/').to_owned()
    }

    /// Rejects a base URL that is not absolute.
    pub(crate) fn require_absolute(raw: &str, message: &str) -> Result<Url, ProviderError> {
        let parsed =
            Url::parse(raw).map_err(|_| ProviderError::Other(anyhow::anyhow!("{message}")))?;
        if parsed.host_str().unwrap_or_default().is_empty() {
            return Err(ProviderError::Other(anyhow::anyhow!("{message}")));
        }
        Ok(parsed)
    }

    /// The per-record base URL override every executor honours:
    /// `attributes["base_url"]`, then the hyphenated spelling.
    pub(crate) fn base_url_attribute(auth: &AuthRecord) -> Option<String> {
        ["base_url", "base-url"]
            .into_iter()
            .filter_map(|key| auth.attributes.get(key))
            .map(|value| trim_base_url(value))
            .find(|value| !value.is_empty())
    }

    /// Reads a string field out of the persisted credential blob.
    ///
    /// [`AuthRecord`] has no dedicated storage field, so the blob is addressed
    /// as `metadata["storage"]`. Both the JSON spelling and the struct field
    /// name are tried, because records exist in both shapes.
    pub(crate) fn string_field_from_storage(
        auth: &AuthRecord,
        field_name: &str,
        json_name: &str,
    ) -> Option<String> {
        nested_string(&auth.metadata, "storage", json_name)
            .or_else(|| nested_string(&auth.metadata, "storage", field_name))
    }

    /// Ensures `metadata` is a JSON object so a refreshed token can be written
    /// into it.
    pub(crate) fn metadata_object_mut(
        metadata: &mut serde_json::Value,
    ) -> &mut serde_json::Map<String, serde_json::Value> {
        if !metadata.is_object() {
            *metadata = serde_json::Value::Object(serde_json::Map::new());
        }
        metadata
            .as_object_mut()
            .expect("metadata was just coerced to an object")
    }

    /// Formats a timestamp as RFC3339 — second precision, `Z` suffix.
    pub(crate) fn rfc3339(at: chrono::DateTime<chrono::Utc>) -> String {
        at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    /// Parses an RFC3339 metadata timestamp.
    ///
    /// An unparseable stamp reads as absent (and therefore expired) rather than
    /// as valid.
    pub(crate) fn parse_rfc3339(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
        chrono::DateTime::parse_from_rfc3339(raw.trim())
            .ok()
            .map(|at| at.with_timezone(&chrono::Utc))
    }

    /// Percent-encodes one path segment.
    ///
    /// `/` is escaped, so a caller-supplied model name containing a slash
    /// cannot add a path segment and repoint the request.
    pub(crate) fn path_escape(segment: &str) -> String {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut out = String::with_capacity(segment.len());
        for byte in segment.bytes() {
            match byte {
                b'A'..=b'Z'
                | b'a'..=b'z'
                | b'0'..=b'9'
                | b'-'
                | b'_'
                | b'.'
                | b'~'
                | b'$'
                | b'&'
                | b'+'
                | b':'
                | b'='
                | b'@' => out.push(byte as char),
                _ => {
                    out.push('%');
                    out.push(HEX[(byte >> 4) as usize] as char);
                    out.push(HEX[(byte & 0x0f) as usize] as char);
                }
            }
        }
        out
    }

    /// Appends inbound query parameters onto an endpoint.
    ///
    /// Order and duplicate keys are both significant, so this appends rather
    /// than merging into a map.
    pub(crate) fn append_query(url: &mut Url, raw_query: Option<&str>, query: &[(String, String)]) {
        if let Some(raw) = raw_query {
            crate::common::append_raw_query(url, raw);
            return;
        }
        if query.is_empty() {
            return;
        }
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }

    /// Drops every existing value for `key`, then appends one. Used for
    /// parameters the provider owns and a caller must not be able to override.
    pub(crate) fn set_query(query: &mut Vec<(String, String)>, key: &str, value: &str) {
        query.retain(|(existing, _)| existing != key);
        query.push((key.to_owned(), value.to_owned()));
    }

    /// Fills in the `Content-Type` / `Accept` defaults after inbound headers
    /// have been copied.
    ///
    /// Streaming pins `Accept` outright because the caller's value describes
    /// the *client* leg, not the upstream one.
    pub(crate) fn default_content_negotiation(headers: &mut HeaderMap, stream: bool) {
        if !headers.contains_key(header::CONTENT_TYPE) {
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
        }
        if stream {
            headers.insert(
                header::ACCEPT,
                HeaderValue::from_static("text/event-stream"),
            );
        } else if !headers.contains_key(header::ACCEPT) {
            headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        }
    }

    /// Extracts the JSON body of one `data:` SSE line, or `None` when the line
    /// is framing rather than payload.
    pub(crate) fn sse_data_payload(line: &[u8]) -> Option<&[u8]> {
        let rest = line.trim_ascii().strip_prefix(b"data:".as_slice())?;
        let payload = rest.trim_ascii();
        (!payload.is_empty() && payload != b"[DONE]").then_some(payload)
    }
}

#[cfg(test)]
mod tests;
