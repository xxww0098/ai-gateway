//! Codex / OpenAI-OAuth upstream.
//!
//! OWNER: worker `provider-openai`.
//!
//! Codex speaks the OpenAI chat-completions wire format but authenticates with
//! a rotating OAuth access token instead of a static API key. This module owns
//! the whole HTTP lifecycle and treats persisted credential records as data
//! only, with [`gw_authcore::AuthRecord`] as the record.

use crate::common::{
    DEFAULT_STREAM_IDLE_TIMEOUT, PROVIDER_CODEX, ProviderConfig, attach_body,
    chat_completions_endpoint, ensure_include_usage, nested_string, request_surface,
    requested_model, resolve_timeout, responses_endpoint, shared_client, stream_response,
    string_from_map, usage_stream,
};
use crate::openai::bearer;
use crate::types::{
    Provider, ProviderError, ProviderRequest, ProviderResponse, StreamResponse,
    copy_outbound_headers,
};
use crate::usage::{parse_codex_stream_usage, parse_codex_usage};
use chrono::{SecondsFormat, Utc};
use gw_authcore::{AuthRecord, AuthStatus};
use gw_relay::Surface;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
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
    client: reqwest::Client,
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
            client: shared_client(),
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

    /// Assembles an outbound chat-completions request.
    fn build_request(
        &self,
        req: &ProviderRequest,
        stream: bool,
        access_token: &str,
        base_url: &str,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        if access_token.is_empty() {
            return Err(ProviderError::Credential(
                "codex access token is required".to_owned(),
            ));
        }
        // 与 openai executor 同一条规则：端点由**入口**决定（缺陷 #1）。
        // Responses 本来就是 Codex 的原生协议（`docs/relay-surface-plan.md` §3.6
        // 的 B×codex 是直通格），缺的只是把端点拼对。
        let surface = request_surface(req);
        let endpoint = match surface {
            Surface::OpenAiResponses => responses_endpoint(base_url, &req.query)?,
            Surface::OpenAiCompletions | Surface::AnthropicMessages => {
                chat_completions_endpoint(base_url, &req.query)?
            }
        };
        // Like the OpenAI executor: force the terminal usage envelope on
        // streams, but only after re-verifying `stream: true` in the body.
        // `None` 表示一个字节都不动。
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

    /// Exchanges a refresh token for a fresh access token.
    async fn refresh_oauth_token(
        &self,
        refresh_token: &str,
    ) -> Result<CodexRefreshResponse, ProviderError> {
        let response = self
            .client
            .post(CODEX_OAUTH_TOKEN_URL)
            .timeout(self.timeout)
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .form(&[
                ("client_id", CODEX_OAUTH_CLIENT_ID),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("scope", "openid profile email"),
            ])
            .send()
            .await
            .map_err(|err| {
                ProviderError::Other(anyhow::anyhow!("codex token refresh request failed: {err}"))
            })?;

        let status = response.status().as_u16();
        let payload = response.bytes().await.map_err(|err| {
            ProviderError::Other(anyhow::anyhow!("reading codex refresh response: {err}"))
        })?;
        if status >= 400 {
            return Err(ProviderError::Upstream {
                status,
                body: String::from_utf8_lossy(&payload).into_owned(),
            });
        }
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

/// Resolves the model name reported for billing: the body's `model` field, or
/// the router's translated hint when it is absent.
fn codex_billing_model(req: &ProviderRequest) -> String {
    let from_body = codex_model_from_body(&req.payload);
    if from_body.is_empty() {
        requested_model(req)
    } else {
        from_body
    }
}

#[async_trait::async_trait]
impl Provider for CodexProvider {
    fn name(&self) -> &'static str {
        PROVIDER_CODEX
    }

    /// Like the OpenAI executor this is the non-streaming path by definition
    /// and ignores [`ProviderRequest::stream`].
    async fn execute(
        &self,
        auth: &AuthRecord,
        req: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        let (access_token, base_url) = self.resolve_credentials(auth);
        let model = codex_billing_model(&req);
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
        let usage = parse_codex_usage(&body).map(|t| t.to_record(model, PROVIDER_CODEX));
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
        let model = codex_billing_model(&req);
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
                PROVIDER_CODEX,
                parse_codex_stream_usage,
                status,
            )
        }))
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

    /// **上游没有这个端点，所以这里报错，不编数字。**
    ///
    /// Codex 走的是 OpenAI 的 Chat Completions 兼容面，而 OpenAI 的 REST API
    /// 没有 token 计数端点 —— 分词在客户端（`tiktoken`）做。
    ///
    /// 这里原来返回 `payload.len() / 4` 的伪造值
    /// （`docs/relay-surface-plan.md` §2.1），且那个数还在按 LLM 价格计费。
    /// 理由与 [`crate::openai::OpenAiCompatibleProvider::count_tokens`] 逐字相同。
    async fn count_tokens(
        &self,
        _auth: &AuthRecord,
        _req: ProviderRequest,
    ) -> Result<i64, ProviderError> {
        Err(ProviderError::Other(anyhow::anyhow!(
            "{PROVIDER_CODEX} upstream exposes no token-counting endpoint"
        )))
    }
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
