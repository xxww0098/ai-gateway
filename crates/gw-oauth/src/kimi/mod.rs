//! Moonshot Kimi Code Plan (device-code, no PKCE).

pub mod cache;
pub mod request;

pub use cache::{apply_cache, cache_session_id, KIMI_STABLE_SESSION};
pub use request::apply_thinking;

use chrono::Utc;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::login::StartInput;
use crate::{Error, Family, Session, StartOutcome};

#[cfg(test)]
mod tests;

pub const ID: &str = "kimi";
pub const CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
pub const API_URL: &str = "https://api.kimi.com/coding/v1/chat/completions";
pub const DEVICE_URL: &str = "https://auth.kimi.com/api/oauth/device_authorization";
pub const TOKEN_URL: &str = "https://auth.kimi.com/api/oauth/token";
pub const ME_URL: &str = "https://api.kimi.com/coding/v1/me";
pub const USAGE_URL: &str = "https://api.kimi.com/coding/v1/usages";
pub const USER_AGENT: &str = "AI-GateWay";
pub const PLATFORM: &str = "agw";
pub const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
pub const NEVER_EXPIRES_MS: i64 = 8_640_000_000_000_000;
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const DEFAULT_INTERVAL: i64 = 5;

/// RFC 8628 device-code (body is `client_id` only) or CLI / API-key import.
///
/// # Errors
/// Unsupported method, empty import, or device-authorization HTTP failure.
pub async fn start(input: &StartInput) -> Result<StartOutcome, Error> {
    start_at(input, DEVICE_URL).await
}

async fn start_at(input: &StartInput, device_url: &str) -> Result<StartOutcome, Error> {
    match method_of(input) {
        "pkce" | "authcode" | "idc" => Err(Error::UnsupportedFlow),
        "import" | "paste" | "env" | "key" | "cli" => import_from_body(input),
        _ if has_import_payload(&input.body) => import_from_body(input),
        _ => start_device(input, device_url).await,
    }
}

/// Refresh token grant. Paste / env keys are a no-op.
///
/// # Errors
/// Missing refresh token, or 401 / 403 / `invalid_grant` (permanent).
pub async fn refresh(session: Session) -> Result<Session, Error> {
    refresh_at(session, TOKEN_URL).await
}

async fn refresh_at(session: Session, token_url: &str) -> Result<Session, Error> {
    if is_key_source(&session.source) || session.refresh_token == session.access_token {
        if session.access_token.trim().is_empty() {
            return Err(Error::InvalidImport("kimi session needs an API key".to_owned()));
        }
        return Ok(session);
    }
    if session.refresh_token.trim().is_empty() {
        return Err(Error::InvalidImport(
            "kimi token endpoint returned no refresh token".to_owned(),
        ));
    }
    let body = crate::form::encode(&[
        ("grant_type", "refresh_token".to_owned()),
        ("client_id", client_id_of(&session)),
        ("refresh_token", session.refresh_token.clone()),
    ]);
    let (status, text) = post_form(token_url, body, &credential_headers()).await?;
    if status == 401 || status == 403 || body_error(&text) == "invalid_grant" {
        return Err(Error::Permanent);
    }
    if !(200..300).contains(&status) {
        return Err(Error::TokenEndpoint { status, body: text });
    }
    let tokens: Value = serde_json::from_str(&text)
        .map_err(|error| Error::Payload(error.to_string()))?;
    let mut next = session_from_tokens(&tokens, Some(&session))?;
    if next.account.is_empty() {
        next.account = session.account;
    }
    if next.plan_type.is_empty() {
        next.plan_type = session.plan_type;
    }
    next.source = if session.source == "cli" { "cli" } else { "oauth" }.to_owned();
    Ok(next)
}

/// 401 / 403 / `invalid_grant` require a new login.
#[must_use]
pub fn is_permanent_refresh_error(error: &Error) -> bool {
    matches!(error, Error::Permanent)
}

/// RFC 8628 device authorization body: `client_id` only, no `scope`, no PKCE.
#[must_use]
pub fn device_authorization_body() -> String {
    crate::form::encode(&[("client_id", CLIENT_ID.to_owned())])
}

/// Build a session from a token endpoint payload.
///
/// # Errors
/// Missing access / refresh / expiry on OAuth sources.
pub fn session_from_tokens(tokens: &Value, fallback: Option<&Session>) -> Result<Session, Error> {
    let access = string_field(tokens, &["access_token", "accessToken"])
        .or_else(|| fallback.map(|s| s.access_token.clone()).filter(|s| !s.is_empty()))
        .ok_or_else(|| Error::Payload("kimi token endpoint returned no access token".to_owned()))?;
    let refresh = string_field(tokens, &["refresh_token", "refreshToken"])
        .or_else(|| fallback.map(|s| s.refresh_token.clone()).filter(|s| !s.is_empty()));
    let expires = expires_from_tokens(tokens).or_else(|| fallback.map(|s| s.expires_at_ms).filter(|v| *v > 0));
    let source = fallback
        .map(|s| s.source.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("oauth");
    kimi_session(KimiSession {
        access_token: &access,
        refresh_token: refresh.as_deref(),
        expires_at_ms: expires,
        account: fallback.map(|s| s.account.as_str()).filter(|s| !s.is_empty()),
        plan_type: fallback.map(|s| s.plan_type.as_str()).filter(|s| !s.is_empty()),
        source,
    })
}

/// Device-code completion: map token JSON onto a session.
///
/// # Errors
/// Same as [`session_from_tokens`].
pub fn complete_device(tokens: &Value) -> Result<Session, Error> {
    session_from_tokens(tokens, None)
}

/// Trim an API key (`sk-` / `KIMI_API_KEY`).
///
/// # Errors
/// Empty or shorter than 8.
pub fn parse_api_key(value: &str) -> Result<String, Error> {
    let key = value.trim();
    if key.len() < 8 {
        return Err(Error::InvalidImport("kimi API key is empty".to_owned()));
    }
    Ok(key.to_owned())
}

/// Official `kimi-code.json` (`access_token` + `refresh_token`).
///
/// # Errors
/// Missing tokens or expiry.
pub fn session_from_cli_file(data: &Value) -> Result<Session, Error> {
    let access = string_field(data, &["access_token", "accessToken"])
        .ok_or_else(|| Error::InvalidImport("kimi-code.json has no access_token".to_owned()))?;
    let refresh = string_field(data, &["refresh_token", "refreshToken"])
        .ok_or_else(|| Error::InvalidImport("kimi-code.json has no refresh_token".to_owned()))?;
    let expires = expires_from_tokens(data);
    kimi_session(KimiSession {
        access_token: &access,
        refresh_token: Some(&refresh),
        expires_at_ms: expires,
        account: None,
        plan_type: None,
        source: "cli",
    })
}

/// Kimi Code–compatible `X-Msh-*` headers. Not Pi.
#[must_use]
pub fn credential_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    insert_header(&mut headers, "user-agent", USER_AGENT);
    insert_header(&mut headers, "x-msh-platform", PLATFORM);
    insert_header(&mut headers, "x-msh-version", USER_AGENT);
    insert_header(&mut headers, "x-msh-device-name", "unknown");
    insert_header(
        &mut headers,
        "x-msh-device-model",
        &format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    );
    insert_header(&mut headers, "x-msh-os-version", std::env::consts::OS);
    insert_header(&mut headers, "x-msh-device-id", &stable_device_id());
    headers
}

/// Bearer + credential headers for coding/v1.
#[must_use]
pub fn upstream_headers(session: &Session) -> HeaderMap {
    let mut headers = credential_headers();
    if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", session.access_token)) {
        headers.insert(http::header::AUTHORIZATION, value);
    }
    headers.insert(http::header::ACCEPT, HeaderValue::from_static("application/json"));
    headers
}

/// `/coding/v1/me` identity. Never the raw token.
#[must_use]
pub fn parse_user_info(payload: &Value) -> Map<String, Value> {
    let email = string_field(payload, &["email"]);
    let nickname = string_field(payload, &["nickname"]);
    let user_id = string_field(payload, &["user_id", "userId"]);
    let plan = string_field(
        payload,
        &["user_level_name", "userLevelName", "plan", "plan_type"],
    );
    let account = email.or(nickname).or(user_id);
    let mut out = Map::new();
    if let Some(account) = account {
        out.insert("account".to_owned(), Value::String(account));
    }
    if let Some(plan) = plan {
        out.insert("plan_type".to_owned(), Value::String(plan));
    }
    out
}

struct KimiSession<'a> {
    access_token: &'a str,
    refresh_token: Option<&'a str>,
    expires_at_ms: Option<i64>,
    account: Option<&'a str>,
    plan_type: Option<&'a str>,
    source: &'a str,
}

fn kimi_session(input: KimiSession<'_>) -> Result<Session, Error> {
    let access = input.access_token.trim();
    if access.is_empty() {
        return Err(Error::Payload(
            "kimi token endpoint returned no access token".to_owned(),
        ));
    }
    let key = is_key_source(input.source);
    let refresh = input
        .refresh_token
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| key.then(|| access.to_owned()))
        .ok_or_else(|| Error::Payload("kimi token endpoint returned no refresh token".to_owned()))?;
    let expires = input
        .expires_at_ms
        .filter(|v| *v > 0)
        .or_else(|| key.then_some(NEVER_EXPIRES_MS));
    let Some(expires) = expires else {
        return Err(Error::Payload(
            "kimi token endpoint returned no usable expiry".to_owned(),
        ));
    };
    let source = match input.source {
        "cli" | "paste" | "env" | "oauth" => input.source,
        _ => "oauth",
    };
    let mut session = Session::new(Family::Kimi, access, refresh);
    session.expires_at_ms = expires;
    session.account = input
        .account
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| default_account(access));
    session.source = source.to_owned();
    if let Some(plan) = input.plan_type.map(str::trim).filter(|s| !s.is_empty()) {
        session.plan_type = plan.to_owned();
    }
    session
        .extra
        .insert("client_id".to_owned(), Value::String(CLIENT_ID.to_owned()));
    session
        .extra
        .insert("token_endpoint".to_owned(), Value::String(TOKEN_URL.to_owned()));
    Ok(session)
}

async fn start_device(input: &StartInput, device_url: &str) -> Result<StartOutcome, Error> {
    let body = device_authorization_body();
    let (status, text) = post_form(device_url, body, &credential_headers()).await?;
    if !(200..300).contains(&status) {
        return Err(Error::TokenEndpoint { status, body: text });
    }
    let wire: Value = serde_json::from_str(&text)
        .map_err(|error| Error::Payload(error.to_string()))?;
    let started = parse_device_response(&wire)?;
    let mut extra = Map::new();
    extra.insert("device_code".to_owned(), Value::String(started.device_code));
    extra.insert("token_endpoint".to_owned(), Value::String(TOKEN_URL.to_owned()));
    extra.insert("client_id".to_owned(), Value::String(CLIENT_ID.to_owned()));
    extra.insert("grant_type".to_owned(), Value::String(DEVICE_GRANT.to_owned()));
    extra.insert("json_body".to_owned(), Value::Bool(false));
    extra.insert("restart_on_expired".to_owned(), Value::Bool(true));
    extra.insert("expires_in".to_owned(), json_i64(started.expires_in));
    Ok(StartOutcome::Device {
        state: input.state.clone(),
        user_code: started.user_code,
        verification_uri: started.verification_uri.clone(),
        verification_uri_complete: started.verification_uri_complete,
        interval_secs: started.interval,
        extra,
    })
}

struct DeviceStart {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    interval: i64,
    expires_in: i64,
}

fn parse_device_response(wire: &Value) -> Result<DeviceStart, Error> {
    let device_code = string_field(wire, &["device_code", "deviceCode"])
        .ok_or_else(|| Error::Payload("kimi device-code response is missing device_code".to_owned()))?;
    let user_code = string_field(wire, &["user_code", "userCode"])
        .ok_or_else(|| Error::Payload("kimi device-code response is missing user_code".to_owned()))?;
    let verification_uri = string_field(wire, &["verification_uri", "verificationUri"])
        .ok_or_else(|| {
            Error::Payload("kimi device-code response is missing verification_uri".to_owned())
        })?;
    let complete = string_field(
        wire,
        &["verification_uri_complete", "verificationUriComplete"],
    )
    .unwrap_or_else(|| verification_uri.clone());
    Ok(DeviceStart {
        device_code,
        user_code,
        verification_uri_complete: complete,
        verification_uri,
        interval: positive_i64(wire.get("interval")).unwrap_or(DEFAULT_INTERVAL),
        expires_in: positive_i64(wire.get("expires_in"))
            .or_else(|| positive_i64(wire.get("expiresIn")))
            .unwrap_or(900),
    })
}

fn import_from_body(input: &StartInput) -> Result<StartOutcome, Error> {
    if let Ok(session) = session_from_cli_file(&input.body) {
        return Ok(StartOutcome::Ready(session));
    }
    if let Some(nested) = input.body.get("credentials").or_else(|| input.body.get("json"))
        && let Ok(session) = session_from_cli_file(nested)
    {
        return Ok(StartOutcome::Ready(session));
    }
    let key = key_from_body(&input.body).ok_or_else(|| {
        Error::InvalidImport("kimi API key is empty".to_owned())
    })?;
    let source = if method_of(input) == "env" { "env" } else { "paste" };
    Ok(StartOutcome::Ready(kimi_session(KimiSession {
        access_token: &parse_api_key(&key)?,
        refresh_token: None,
        expires_at_ms: None,
        account: string_field(&input.body, &["account", "email"]).as_deref(),
        plan_type: None,
        source,
    })?))
}

fn has_import_payload(body: &Value) -> bool {
    session_from_cli_file(body).is_ok()
        || body
            .get("credentials")
            .or_else(|| body.get("json"))
            .is_some_and(|nested| session_from_cli_file(nested).is_ok())
        || key_from_body(body).is_some()
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
            "KIMI_API_KEY",
        ],
    )
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

fn is_key_source(source: &str) -> bool {
    source == "paste" || source == "env"
}

fn default_account(token: &str) -> String {
    format!("kimi-{}", fingerprint(token))
}

fn fingerprint(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(&digest[..4])
}

fn client_id_of(session: &Session) -> String {
    session
        .extra
        .get("client_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(CLIENT_ID)
        .to_owned()
}

fn expires_from_tokens(tokens: &Value) -> Option<i64> {
    if let Some(seconds) = positive_i64(tokens.get("expires_in")).or_else(|| positive_i64(tokens.get("expiresIn")))
    {
        return Some(Utc::now().timestamp_millis().saturating_add(seconds.saturating_mul(1000)));
    }
    let raw = positive_i64(tokens.get("expires_at")).or_else(|| positive_i64(tokens.get("expiresAt")))?;
    Some(if raw > 1_000_000_000_000 {
        raw
    } else {
        raw.saturating_mul(1000)
    })
}

fn positive_i64(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    let n = value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| value.as_f64().map(|n| n as i64))?;
    (n > 0).then_some(n)
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

fn json_i64(value: i64) -> Value {
    Value::Number(value.into())
}

fn body_error(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| string_field(&value, &["error"]))
        .unwrap_or_default()
}

fn insert_header(headers: &mut HeaderMap, name: &'static str, value: &str) {
    let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
        return;
    };
    let Ok(value) = HeaderValue::from_str(value) else {
        return;
    };
    headers.insert(name, value);
}

fn stable_device_id() -> String {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| crate::random_hex(16).unwrap_or_else(|_| "unknown".to_owned()))
        .clone()
}

fn http_client() -> Result<reqwest::Client, Error> {
    Ok(reqwest::Client::builder().timeout(HTTP_TIMEOUT).build()?)
}

async fn post_form(url: &str, body: String, headers: &HeaderMap) -> Result<(u16, String), Error> {
    let response = http_client()?
        .post(url)
        .header(http::header::ACCEPT, "application/json")
        .header(http::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .headers(headers.clone())
        .body(body)
        .send()
        .await?;
    let status = response.status().as_u16();
    let text = response.text().await?;
    Ok((status, text))
}
