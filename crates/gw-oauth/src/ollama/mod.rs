//! Ollama Cloud (ollama.com API key — not localhost:11434).

pub mod cache;

pub use cache::{apply_cache, cache_session_id, OLLAMA_STABLE_SESSION};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::login::StartInput;
use crate::{Error, Family, FlowKind, Session, StartOutcome};

#[cfg(test)]
mod tests;

pub const ID: &str = "ollama";
pub const API_URL: &str = "https://ollama.com/v1/chat/completions";
pub const ME_URL: &str = "https://ollama.com/api/me";
pub const USAGE_URL: &str = "https://ollama.com/api/usage";
pub const TAGS_URL: &str = "https://ollama.com/api/tags";
pub const SHOW_URL: &str = "https://ollama.com/api/show";
pub const KEYS_URL: &str = "https://ollama.com/settings/keys";
/// Official docs: API keys do not expire. Milliseconds, matching oauth-subs.
pub const NEVER_EXPIRES_MS: i64 = 8_640_000_000_000_000;

/// Paste / env API key. Device-code and PKCE are not a Cloud hop.
///
/// # Errors
/// Missing or registry-public-key material; unsupported method.
pub async fn start(input: &StartInput) -> Result<StartOutcome, Error> {
    match method_of(input) {
        "device" | "pkce" | "authcode" | "idc" => Err(Error::UnsupportedFlow),
        "env" => ready_from_body(input, "env"),
        _ => ready_from_body(input, "paste"),
    }
}

/// Keys do not expire. Returns the session unchanged.
///
/// # Errors
/// Empty access token.
pub fn refresh(session: Session) -> Result<Session, Error> {
    if session.access_token.trim().is_empty() {
        return Err(Error::InvalidImport("ollama session needs an API key".to_owned()));
    }
    Ok(session)
}

/// Refresh never invalidates a stored Cloud key.
#[must_use]
pub fn is_permanent_refresh_error(_error: &Error) -> bool {
    false
}

/// Trim and reject empty / SSH registry public keys.
///
/// # Errors
/// Empty, shorter than 8, or `BEGIN PUBLIC KEY` (id_ed25519.pub).
pub fn parse_api_key(value: &str) -> Result<String, Error> {
    let key = value.trim();
    if key.len() < 8 {
        return Err(Error::InvalidImport("ollama API key is empty".to_owned()));
    }
    if key.contains("BEGIN") && key.contains("PUBLIC KEY") {
        return Err(Error::InvalidImport(
            "ollama id_ed25519.pub is the registry public key, not a cloud API key".to_owned(),
        ));
    }
    Ok(key.to_owned())
}

/// Build a stored Cloud session. `source` is `paste` or `env`.
///
/// # Errors
/// [`parse_api_key`].
pub fn session(access_token: &str, source: &str, account: Option<&str>) -> Result<Session, Error> {
    let key = parse_api_key(access_token)?;
    let source = if source == "env" { "env" } else { "paste" };
    let mut session = Session::new(Family::Ollama, key.clone(), key.clone());
    session.expires_at_ms = NEVER_EXPIRES_MS;
    session.account = account
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| default_account(&key));
    session.source = source.to_owned();
    Ok(session)
}

/// Opaque vault id that is not the raw key.
#[must_use]
pub fn default_account(key: &str) -> String {
    format!("ollama-{}", fingerprint(key))
}

/// `POST /api/me` is PascalCase (`Email` / `Name` / `Plan`). GET is 405.
#[must_use]
pub fn parse_me(value: &Value) -> Map<String, Value> {
    let user = value.get("user").filter(|item| item.is_object()).unwrap_or(value);
    let account = first_trimmed(&[
        user.get("Email"),
        user.get("email"),
        user.get("Name"),
        user.get("name"),
        user.get("preferred_username"),
        user.get("username"),
        value.get("Email"),
        value.get("email"),
        value.get("Name"),
        value.get("name"),
    ]);
    let plan = first_trimmed(&[
        user.get("Plan"),
        user.get("plan"),
        value.get("Plan"),
        value.get("plan"),
    ]);
    let mut out = Map::new();
    if let Some(account) = account {
        out.insert("account".to_owned(), Value::String(account));
    }
    if let Some(plan) = plan {
        out.insert("plan_type".to_owned(), Value::String(plan));
    }
    out
}

/// Bearer for ollama.com. Never a localhost:11434 hop.
#[must_use]
pub fn upstream_headers(session: &Session) -> http::HeaderMap {
    let mut headers = http::HeaderMap::new();
    if let Ok(value) = http::HeaderValue::from_str(&format!("Bearer {}", session.access_token)) {
        headers.insert(http::header::AUTHORIZATION, value);
    }
    headers
}

fn ready_from_body(input: &StartInput, source: &str) -> Result<StartOutcome, Error> {
    let key = key_from_body(&input.body).ok_or_else(|| {
        Error::InvalidImport("ollama API key is empty".to_owned())
    })?;
    let account = string_field(&input.body, &["account", "email"]);
    Ok(StartOutcome::Ready(session(&key, source, account.as_deref())?))
}

fn method_of(input: &StartInput) -> &str {
    let method = input.method.trim();
    if !method.is_empty() {
        return method;
    }
    input
        .body
        .get("method")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
}

fn key_from_body(body: &Value) -> Option<String> {
    if let Some(text) = body.as_str() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    string_field(
        body,
        &[
            "api_key",
            "apiKey",
            "key",
            "token",
            "access_token",
            "accessToken",
            "OLLAMA_API_KEY",
        ],
    )
}

fn string_field(body: &Value, keys: &[&str]) -> Option<String> {
    let object = body.as_object()?;
    for key in keys {
        if let Some(text) = object.get(*key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

fn first_trimmed(values: &[Option<&Value>]) -> Option<String> {
    for value in values {
        if let Some(text) = value.and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

fn fingerprint(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    hex::encode(&digest[..4])
}

#[must_use]
pub fn is_opaque_account(value: &str) -> bool {
    let text = value.trim();
    let Some(rest) = text
        .strip_prefix("ollama-")
        .or_else(|| text.strip_prefix("OLLAMA-"))
    else {
        return false;
    };
    rest.len() == 8 && rest.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Hop origin is ollama.com, never the local daemon.
#[must_use]
pub fn is_cloud_chat_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("ollama.com") && !lower.contains("127.0.0.1:11434") && !lower.contains("localhost:11434")
}

/// Analyzer-only. Not a login method.
pub const fn flow_kind() -> FlowKind {
    FlowKind::ApiKey
}
