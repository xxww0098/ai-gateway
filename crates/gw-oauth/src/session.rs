//! In-memory session the families hand back. Panel maps this onto `AuthRecord`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::Family;

/// One stored account for one family.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub family: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Unix milliseconds, matching the oauth-subs `expiresAt` field.
    pub expires_at_ms: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub account: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plan_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Family-owned extras (region, client_id, project_id, githubToken, …).
    #[serde(default)]
    pub extra: Map<String, Value>,
}

impl Session {
    #[must_use]
    pub fn new(family: Family, access_token: impl Into<String>, refresh_token: impl Into<String>) -> Self {
        Self {
            family: family.as_str().to_owned(),
            access_token: access_token.into(),
            refresh_token: refresh_token.into(),
            expires_at_ms: 0,
            account: String::new(),
            plan_type: String::new(),
            source: String::new(),
            extra: Map::new(),
        }
    }

    #[must_use]
    pub fn expires_at(&self) -> Option<DateTime<Utc>> {
        if self.expires_at_ms <= 0 {
            return None;
        }
        DateTime::from_timestamp_millis(self.expires_at_ms)
    }
}

/// How a family wants the operator to authenticate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowKind {
    AuthorizationCode,
    Device,
    CliPoll,
    Import,
    ApiKey,
}

/// Outcome of `start_login`.
#[derive(Debug, Clone)]
pub enum StartOutcome {
    /// Open this URL; callback/poll later.
    Browser {
        authorize_url: String,
        state: String,
        verifier: String,
        redirect_uri: String,
        flow: FlowKind,
        extra: Map<String, Value>,
    },
    /// RFC 8628 (or vendor poll that looks like it).
    Device {
        state: String,
        user_code: String,
        verification_uri: String,
        verification_uri_complete: String,
        interval_secs: i64,
        extra: Map<String, Value>,
    },
    /// Import / paste produced a usable session immediately.
    Ready(Session),
}

/// Body rewrite + sticky identity produced by a family's cache module.
#[derive(Debug, Clone)]
pub struct CacheRewrite {
    pub payload: Value,
    pub cache_session_id: Option<String>,
}
