//! Config-seeded upstream credentials.
//!
//! Credentials that come from `config.yaml` are deliberately **not** persisted:
//! deleting a provider from the config must delete it from the gateway. But the
//! auth manager rebuilds its whole map from [`AuthStore::list`] on every reload,
//! so anything that only lives in memory disappears the first time it reloads.
//! [`RuntimeAuthStore`] fixes it by injecting the config-seeded records into
//! every `list()`.

use crate::{
    error::AuthError,
    record::{AuthRecord, AuthStore, BASE_URL_ATTRIBUTE, RUNTIME_ONLY_ATTRIBUTE, SOURCE_ATTRIBUTE},
};
use chrono::{DateTime, Utc};
use gw_config::{SdkConfig, SdkProviderConfig};
use std::{collections::HashSet, sync::Arc};

#[cfg(test)]
mod tests;

/// Value of the `source` attribute on every config-seeded credential.
pub const RUNTIME_SOURCE: &str = "cpa-gateway-config";

/// Provider identifier for the OpenAI-compatible upstream.
pub const PROVIDER_OPENAI: &str = "openai";
/// Provider identifier for the Claude upstream.
pub const PROVIDER_CLAUDE: &str = "claude";
/// Provider identifier for the Gemini upstream.
pub const PROVIDER_GEMINI: &str = "gemini";
/// Provider identifier for the Codex upstream.
pub const PROVIDER_CODEX: &str = "codex";
/// Provider identifier for the Vertex upstream.
pub const PROVIDER_VERTEX: &str = "vertex";

/// Default upstream base URL for Claude.
pub const CLAUDE_DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// Default upstream base URL for Gemini.
pub const GEMINI_DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
/// Default upstream base URL for Codex.
pub const CODEX_DEFAULT_BASE_URL: &str = "https://api.openai.com";

/// Metadata key holding the Codex access token.
pub const CODEX_METADATA_ACCESS_TOKEN: &str = "access_token";
/// Metadata key holding the Vertex service-account JSON.
pub const VERTEX_METADATA_SERVICE_ACCOUNT: &str = "service_account";

/// Builds the runtime-only credential for every provider configured under
/// `sdk:` in `config.yaml`.
///
/// The executor construction itself belongs to `gw-provider`. The gating is:
///
/// * OpenAI-compatible, Claude, Gemini — emitted when the provider is
///   [`SdkProviderConfig::complete`] (enabled **and** base URL **and** key).
/// * Codex, Vertex — emitted when the provider is
///   [`SdkProviderConfig::configured`] **and** carries a credential, because
///   their "api_key" is really an access token / service-account JSON that a
///   persisted DB auth may supply instead.
///
/// A provider without a config credential is simply absent from the result:
/// persisted `auth_records` rows for it still work.
///
/// # Errors
/// [`AuthError::InvalidBaseUrl`] when a configured `base_url` is not an
/// absolute URL.
pub fn build_runtime_upstreams(cfg: &SdkConfig) -> Result<Vec<AuthRecord>, AuthError> {
    build_runtime_upstreams_at(cfg, Utc::now())
}

/// The clock-injected core of [`build_runtime_upstreams`].
fn build_runtime_upstreams_at(
    cfg: &SdkConfig,
    now: DateTime<Utc>,
) -> Result<Vec<AuthRecord>, AuthError> {
    let mut auths = Vec::new();

    // OpenAI-compatible: gated on Complete() for the executor *and* the auth,
    // because without a base URL there is nothing to talk to.
    let openai = cfg.openai_provider_config();
    if openai.complete() {
        let base_url = resolve_base_url(PROVIDER_OPENAI, &openai, "")?;
        auths.push(runtime_auth(
            "cpa-gateway-openai-compatible",
            PROVIDER_OPENAI,
            "CPA-Gateway OpenAI-compatible upstream",
            &base_url,
            now,
        ));
    } else {
        tracing::warn!(
            "OpenAI-compatible upstream disabled: sdk.openai/openai_compatible or legacy \
             sdk.base_url/api_key is missing"
        );
    }

    // The remaining four always resolve (and therefore validate) their base URL
    // even without a credential, so persisted auths can still use them.
    let claude_base = resolve_base_url(PROVIDER_CLAUDE, &cfg.claude, CLAUDE_DEFAULT_BASE_URL)?;
    if cfg.claude.complete() {
        auths.push(runtime_auth(
            "cpa-gateway-claude",
            PROVIDER_CLAUDE,
            "CPA-Gateway Claude upstream",
            &claude_base,
            now,
        ));
    } else {
        tracing::info!(
            "Claude upstream has no config credential; persisted claude auths may still be used"
        );
    }

    let gemini_base = resolve_base_url(PROVIDER_GEMINI, &cfg.gemini, GEMINI_DEFAULT_BASE_URL)?;
    if cfg.gemini.complete() {
        auths.push(runtime_auth(
            "cpa-gateway-gemini",
            PROVIDER_GEMINI,
            "CPA-Gateway Gemini upstream",
            &gemini_base,
            now,
        ));
    } else {
        tracing::info!(
            "Gemini upstream has no config credential; persisted gemini auths may still be used"
        );
    }

    let codex_base = resolve_base_url(PROVIDER_CODEX, &cfg.codex, CODEX_DEFAULT_BASE_URL)?;
    if cfg.codex.configured() && !cfg.codex.api_key.trim().is_empty() {
        let mut auth = runtime_auth(
            "cpa-gateway-codex",
            PROVIDER_CODEX,
            "CPA-Gateway Codex upstream",
            &codex_base,
            now,
        );
        auth.metadata = serde_json::json!({
            CODEX_METADATA_ACCESS_TOKEN: cfg.codex.api_key.trim(),
        });
        auths.push(auth);
    } else {
        tracing::info!(
            "Codex upstream has no config access token; persisted codex auths may still be used"
        );
    }

    // Vertex derives its host from the request's region, so an empty base URL is
    // legitimate and is left empty rather than defaulted.
    let vertex_base = resolve_base_url(PROVIDER_VERTEX, &cfg.vertex, "")?;
    if cfg.vertex.configured() && !cfg.vertex.api_key.trim().is_empty() {
        let mut auth = runtime_auth(
            "cpa-gateway-vertex",
            PROVIDER_VERTEX,
            "CPA-Gateway Vertex upstream",
            &vertex_base,
            now,
        );
        auth.metadata = serde_json::json!({
            VERTEX_METADATA_SERVICE_ACCOUNT: cfg.vertex.api_key.trim(),
        });
        auths.push(auth);
    } else {
        tracing::info!(
            "Vertex upstream has no config service account; persisted vertex auths may still be \
             used"
        );
    }

    Ok(auths)
}

/// Builds one config-seeded credential with its standard attributes.
fn runtime_auth(
    id: &str,
    provider: &str,
    label: &str,
    base_url: &str,
    now: DateTime<Utc>,
) -> AuthRecord {
    let mut auth = AuthRecord::new(id, provider, now);
    auth.label = label.to_owned();
    auth.set_attribute(RUNTIME_ONLY_ATTRIBUTE, "true");
    auth.set_attribute(SOURCE_ATTRIBUTE, RUNTIME_SOURCE);
    auth.set_attribute(BASE_URL_ATTRIBUTE, base_url);
    auth
}

/// Resolves a provider's base URL: trim surrounding whitespace and trailing
/// slashes, fall back to the per-provider default, then validate.
fn resolve_base_url(
    provider: &'static str,
    cfg: &SdkProviderConfig,
    default: &str,
) -> Result<String, AuthError> {
    let trimmed = cfg.base_url.trim().trim_end_matches('/');
    let resolved = if trimmed.is_empty() { default } else { trimmed };
    if resolved.is_empty() {
        return Ok(String::new()); // Vertex: host is derived per request
    }
    validate_base_url(provider, resolved)?;
    Ok(resolved.to_owned())
}

/// Validates that a base URL is absolute, with a scheme and a host.
///
/// A hand-rolled check rather than a URL parser: the only property asserted is
/// "absolute, with a host", and this crate has no other need for a URL type.
fn validate_base_url(provider: &'static str, value: &str) -> Result<(), AuthError> {
    let invalid = || AuthError::InvalidBaseUrl {
        provider,
        value: value.to_owned(),
    };
    let (scheme, rest) = value.split_once("://").ok_or_else(invalid)?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if scheme.is_empty()
        || host.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return Err(invalid());
    }
    Ok(())
}

/// An [`AuthStore`] that adds the config-seeded credentials to every `list()`.
///
/// Persisted rows win on an id collision, and `save` / `delete` pass straight
/// through to the underlying store (which already refuses to persist runtime-only
/// records).
#[derive(Clone)]
pub struct RuntimeAuthStore {
    underlying: Arc<dyn AuthStore>,
    runtime: Vec<AuthRecord>,
}

impl std::fmt::Debug for RuntimeAuthStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeAuthStore")
            .field("runtime", &self.runtime.len())
            .finish_non_exhaustive()
    }
}

impl RuntimeAuthStore {
    /// Decorates `underlying` with `runtime` credentials.
    #[must_use]
    pub fn new(underlying: Arc<dyn AuthStore>, runtime: Vec<AuthRecord>) -> Self {
        Self {
            underlying,
            runtime,
        }
    }

    /// With no runtime credentials there is nothing to decorate, so the
    /// underlying store is returned as-is.
    #[must_use]
    pub fn wrap(underlying: Arc<dyn AuthStore>, runtime: Vec<AuthRecord>) -> Arc<dyn AuthStore> {
        if runtime.is_empty() {
            return underlying;
        }
        Arc::new(Self::new(underlying, runtime))
    }

    /// The config-seeded credentials this store injects.
    #[must_use]
    pub fn runtime_records(&self) -> &[AuthRecord] {
        &self.runtime
    }
}

#[async_trait::async_trait]
impl AuthStore for RuntimeAuthStore {
    async fn list(&self) -> anyhow::Result<Vec<AuthRecord>> {
        let mut out = self.underlying.list().await?;
        let seen: HashSet<&str> = out.iter().map(|record| record.id.as_str()).collect();

        let extra: Vec<AuthRecord> = self
            .runtime
            .iter()
            .filter(|record| !record.id.is_empty() && !seen.contains(record.id.as_str()))
            .cloned()
            .collect();
        out.extend(extra);
        Ok(out)
    }

    async fn get(&self, id: &str) -> anyhow::Result<Option<AuthRecord>> {
        if let Some(record) = self.underlying.get(id).await? {
            return Ok(Some(record)); // a persisted row with the same id wins
        }
        Ok(self
            .runtime
            .iter()
            .find(|record| !record.id.is_empty() && record.id == id)
            .cloned())
    }

    async fn save(&self, record: &AuthRecord) -> anyhow::Result<()> {
        self.underlying.save(record).await
    }

    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.underlying.delete(id).await
    }
}
