//! The upstream-credential record and its persistence contract.
//!
//! [`AuthRecord`] is the single most load-bearing type here, and [`AuthStore`]
//! is its persistence contract. Field-for-field it mirrors the credential entity
//! that the `auth_records` table holds.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[cfg(test)]
mod tests;

/// Attribute marking a credential that lives only in memory (seeded from
/// `config.yaml`) and must never be persisted.
pub const RUNTIME_ONLY_ATTRIBUTE: &str = "runtime_only";

/// Attribute recording where a runtime credential came from.
pub const SOURCE_ATTRIBUTE: &str = "source";

/// Attribute holding the resolved upstream base URL.
pub const BASE_URL_ATTRIBUTE: &str = "base_url";

/// One upstream provider credential.
///
/// The purely-runtime fields of the old credential type (`Index`, `FileName`,
/// `Storage`, counters) were already dropped at the persistence boundary and are
/// not reintroduced here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthRecord {
    /// Primary key. UUID for panel-created credentials, a stable
    /// `cpa-gateway-<provider>` string for config-seeded ones.
    pub id: String,
    /// Provider identifier: `openai` / `claude` / `gemini` / `codex` / `vertex`.
    pub provider: String,
    /// Optional grouping prefix used by the channel selector.
    #[serde(default)]
    pub prefix: String,
    /// Human-facing label shown in the admin panel.
    #[serde(default)]
    pub label: String,
    /// Lifecycle state; an empty column reads back as [`AuthStatus::Active`].
    #[serde(default)]
    pub status: AuthStatus,
    /// Free-text detail behind `status`.
    #[serde(default)]
    pub status_message: String,
    /// Operator-set kill switch.
    #[serde(default)]
    pub disabled: bool,
    /// Health-derived kill switch (set by the proxy, not by an operator).
    #[serde(default)]
    pub unavailable: bool,
    /// Per-credential outbound proxy.
    #[serde(default)]
    pub proxy_url: String,
    /// Non-secret settings (`base_url`, `runtime_only`, `source`, ...). Stays
    /// cleartext so operators can query it.
    #[serde(default)]
    pub attributes: HashMap<String, String>,
    /// Secret-bearing blob (`api_key`, OAuth tokens, service-account JSON).
    /// Encrypted at rest — see [`crate::credcrypto`].
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// Provider-reported quota snapshot.
    #[serde(default)]
    pub quota: serde_json::Value,
    /// Per-model availability snapshot.
    #[serde(default)]
    pub model_states: serde_json::Value,
    /// Last upstream error, `None` when the column is NULL.
    #[serde(default)]
    pub last_error: Option<serde_json::Value>,
    /// Row creation time.
    pub created_at: DateTime<Utc>,
    /// Row update time.
    pub updated_at: DateTime<Utc>,
    /// When the credential's token was last refreshed.
    #[serde(default)]
    pub last_refreshed_at: Option<DateTime<Utc>>,
    /// Earliest time a proactive refresh should run.
    #[serde(default)]
    pub next_refresh_after: Option<DateTime<Utc>>,
    /// Earliest time a failed credential may be retried.
    #[serde(default)]
    pub next_retry_after: Option<DateTime<Utc>>,
}

impl Default for AuthRecord {
    fn default() -> Self {
        Self {
            id: String::new(),
            provider: String::new(),
            prefix: String::new(),
            label: String::new(),
            status: AuthStatus::default(),
            status_message: String::new(),
            disabled: false,
            unavailable: false,
            proxy_url: String::new(),
            attributes: HashMap::new(),
            metadata: serde_json::Value::Object(serde_json::Map::new()),
            quota: serde_json::Value::Object(serde_json::Map::new()),
            model_states: serde_json::Value::Object(serde_json::Map::new()),
            last_error: None,
            created_at: DateTime::UNIX_EPOCH,
            updated_at: DateTime::UNIX_EPOCH,
            last_refreshed_at: None,
            next_refresh_after: None,
            next_retry_after: None,
        }
    }
}

impl AuthRecord {
    /// A fresh active credential stamped `now`.
    #[must_use]
    pub fn new(id: impl Into<String>, provider: impl Into<String>, now: DateTime<Utc>) -> Self {
        Self {
            id: id.into(),
            provider: provider.into(),
            created_at: now,
            updated_at: now,
            ..Self::default()
        }
    }

    /// Whether this credential is config-seeded and must not be persisted.
    ///
    /// A case-insensitive, whitespace-trimmed `"true"` in the `runtime_only`
    /// attribute.
    #[must_use]
    pub fn is_runtime_only(&self) -> bool {
        self.attributes
            .get(RUNTIME_ONLY_ATTRIBUTE)
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
    }

    /// Reads an attribute, trimmed of surrounding whitespace.
    #[must_use]
    pub fn attribute(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).map(|value| value.trim())
    }

    /// Sets an attribute, replacing any previous value.
    pub fn set_attribute(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.attributes.insert(key.into(), value.into());
    }

    /// Whether the proxy may route traffic to this credential.
    ///
    /// Rejects disabled/unavailable credentials, plus the health-derived
    /// `Unavailable` flag.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        !self.disabled && !self.unavailable && self.status != AuthStatus::Disabled
    }
}

/// Lifecycle state of a credential.
///
/// An open string type. It is serialised as a bare string (never a tagged enum)
/// and unrecognised values round-trip through [`AuthStatus::Other`] rather than
/// being coerced, because the panel shows the raw value to operators and the
/// column may hold states written by an older binary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AuthStatus {
    /// Active. Also what an empty column means.
    #[default]
    Active,
    /// Disabled.
    Disabled,
    /// The credential last failed.
    Error,
    /// Any other state the column happens to hold.
    Other(String),
}

impl AuthStatus {
    /// The stored string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Error => "error",
            Self::Other(raw) => raw,
        }
    }
}

impl From<&str> for AuthStatus {
    /// An empty string normalises to [`AuthStatus::Active`].
    fn from(raw: &str) -> Self {
        match raw {
            "" | "active" => Self::Active,
            "disabled" => Self::Disabled,
            "error" => Self::Error,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl std::fmt::Display for AuthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for AuthStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AuthStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::from(raw.as_str()))
    }
}

/// Credential persistence.
///
/// `metadata` is encrypted at rest with AES-GCM using `CREDENTIAL_ENCRYPTION_KEY`;
/// implementations decrypt on the way out so callers always see plaintext.
#[async_trait::async_trait]
pub trait AuthStore: Send + Sync {
    /// Every persisted credential, oldest first (`ORDER BY created_at ASC, id
    /// ASC`).
    async fn list(&self) -> anyhow::Result<Vec<AuthRecord>>;

    /// One credential by id, `None` when it does not exist.
    async fn get(&self, id: &str) -> anyhow::Result<Option<AuthRecord>>;

    /// Inserts or updates a credential by id.
    ///
    /// Runtime-only records are silently skipped — they exist only for the
    /// lifetime of the process.
    async fn save(&self, record: &AuthRecord) -> anyhow::Result<()>;

    /// Removes a credential by id. An empty id is a no-op.
    async fn delete(&self, id: &str) -> anyhow::Result<()>;
}
