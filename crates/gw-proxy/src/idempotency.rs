//! Idempotent request deduplication.
//!
//! The lifecycle is claim -> store | release:
//!
//! * [`IdempotencyManager::check`] replays a completed response, or reports an
//!   in-flight duplicate;
//! * [`IdempotencyManager::claim`] is a SETNX taken only AFTER a successful
//!   hold, so a request rejected at pre-flight never locks its key;
//! * [`IdempotencyManager::store`] overwrites the claim with the real response;
//! * [`IdempotencyManager::release`] drops the claim after a failure so a retry
//!   is not blocked until the sentinel expires.

use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::ports::{AuthCrypto, Id, IdempotencyStore};

/// Default lifetime of a cached response.
pub const DEFAULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// How long an in-flight claim survives if its owner crashes. Must exceed the
/// longest expected single request so a slow stream is not treated as abandoned
/// and re-billed by a duplicate.
pub const PROCESSING_TTL: Duration = Duration::from_secs(10 * 60);

/// Redis key prefix.
pub const KEY_PREFIX: &str = "cpa-gateway:idempotency:";

/// A response cached for idempotent replay.
///
/// The wire format is the established contract, field for field, because the
/// keys are read and written across a staged rollout. An entry that cannot be
/// decoded defeats the entire point of idempotency: the retry stops replaying
/// and starts re-executing, and re-billing with it.
///
/// The load-bearing detail is [`Self::body`]. The body is stored as a
/// **standard-alphabet, padded base64 string** — not as a JSON array and not
/// as raw text. See the round-trip tests, which pin the encoding rather than
/// trusting this comment.
///
/// A bare `null` is written for a nil map or slice (neither field carries
/// `omitempty`), which `claim` does on every sentinel it stores, so the
/// deserializers below accept `null` for both.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CachedResponse {
    pub status_code: u16,
    #[serde(default, deserialize_with = "null_as_default")]
    pub headers: std::collections::HashMap<String, String>,
    /// The raw response body. Serialized as base64.
    #[serde(default, with = "base64_bytes")]
    pub body: Vec<u8>,
    #[serde(default)]
    pub cost: f64,
    #[serde(default, deserialize_with = "null_as_default")]
    pub request_id: String,
    /// Marks an in-flight claim, set by `claim` and overwritten by `store`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub processing: bool,
    /// Response could not be captured faithfully (too large, or streamed): the
    /// entry still blocks a re-charge but cannot be replayed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// Accepts an explicit `null` for a nil map or string.
///
/// `#[serde(default)]` alone only covers an *absent* field; the key is emitted
/// with a `null` value, which would otherwise be a type error.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// Standard-alphabet base64 with padding, or `null` for a nil slice.
mod base64_bytes {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let Some(encoded) = Option::<String>::deserialize(d)? else {
            return Ok(Vec::new());
        };
        STANDARD.decode(&encoded).map_err(serde::de::Error::custom)
    }
}

impl IntoResponse for CachedResponse {
    /// Replays the cached response verbatim, so no new hold, upstream call, or
    /// settle occurs.
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status_code).unwrap_or(StatusCode::OK);
        let mut response = Response::new(axum::body::Body::from(self.body));
        *response.status_mut() = status;
        let mut saw_content_type = false;
        for (k, v) in &self.headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::try_from(k.as_str()),
                HeaderValue::from_str(v.as_str()),
            ) {
                saw_content_type |= name == axum::http::header::CONTENT_TYPE;
                response.headers_mut().insert(name, value);
            }
        }
        if !saw_content_type {
            response.headers_mut().insert(
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
        }
        response
    }
}

/// Deduplicates retried requests through a shared [`IdempotencyStore`].
pub struct IdempotencyManager {
    store: Arc<dyn IdempotencyStore>,
    crypto: Arc<dyn AuthCrypto>,
    ttl: Duration,
}

impl IdempotencyManager {
    /// Builds the manager; a zero `ttl` means [`DEFAULT_TTL`].
    pub fn new(
        store: Arc<dyn IdempotencyStore>,
        crypto: Arc<dyn AuthCrypto>,
        ttl: Duration,
    ) -> Self {
        Self {
            store,
            crypto,
            ttl: if ttl.is_zero() { DEFAULT_TTL } else { ttl },
        }
    }

    /// Derives the store key from the tenant, method, path and client key.
    ///
    /// Returns an empty string when the client supplied no key (idempotency is
    /// opt-in). Scoping by user is what prevents cross-tenant replay (blocker
    /// B6); method+path scoping additionally stops one client reusing a key
    /// across endpoints.
    pub fn scoped_key(&self, user_id: Id, method: &str, path: &str, client_key: &str) -> String {
        let client_key = client_key.trim();
        if client_key.is_empty() {
            return String::new();
        }
        self.crypto
            .sha256_hex(&format!("{user_id}\0{method}\0{path}\0{client_key}"))
    }

    /// Looks up a cached entry.
    pub async fn check(&self, key: &str) -> anyhow::Result<Option<CachedResponse>> {
        let Some(raw) = self.store.get(&redis_key(key)).await? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_slice(&raw)?))
    }

    /// Reserves the key for an in-flight request (SETNX + `processing`
    /// sentinel). `Ok(true)` means the caller now owns it and must later
    /// [`Self::store`] or [`Self::release`].
    pub async fn claim(&self, key: &str) -> anyhow::Result<bool> {
        let sentinel = serde_json::to_vec(&CachedResponse {
            processing: true,
            ..CachedResponse::default()
        })?;
        self.store
            .set_nx(&redis_key(key), sentinel, PROCESSING_TTL)
            .await
    }

    /// Overwrites the claim with the completed response.
    pub async fn store(&self, key: &str, response: &CachedResponse) -> anyhow::Result<()> {
        let data = serde_json::to_vec(response)?;
        self.store.set(&redis_key(key), data, self.ttl).await
    }

    /// Drops the claim so a retry can proceed immediately.
    pub async fn release(&self, key: &str) -> anyhow::Result<()> {
        self.store.delete(&redis_key(key)).await
    }
}

fn redis_key(key: &str) -> String {
    format!("{KEY_PREFIX}{key}")
}

#[cfg(test)]
mod tests;
