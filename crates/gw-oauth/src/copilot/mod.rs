//! GitHub Copilot Chat (`ghu_` → `tid=`).

pub mod cache;
pub mod request;

pub use cache::{apply_cache, cache_headers, cache_session_id, COPILOT_STABLE_SESSION};
pub use request::{apply_thinking, map_usage};

use chrono::Utc;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::login::StartInput;
use crate::{Error, Family, Session, StartOutcome};

#[cfg(test)]
mod tests;

pub const ID: &str = "copilot";
/// VS Code GitHub Copilot App. Do not use OpenCode `Ov23li8…` (issues `gho_`, exchange 404).
pub const CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
pub const SCOPE: &str = "read:user";
pub const API_URL: &str = "https://api.githubcopilot.com/chat/completions";
pub const DEVICE_URL: &str = "https://github.com/login/device/code";
pub const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
pub const EXCHANGE_URL: &str = "https://api.github.com/copilot_internal/v2/token";
pub const USER_URL: &str = "https://api.github.com/user";
pub const QUOTA_URL: &str = "https://api.github.com/copilot_internal/user";
pub const API_ORIGIN: &str = "https://api.githubcopilot.com";
pub const USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
pub const EDITOR_VERSION: &str = "vscode/1.107.0";
pub const PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
pub const INTEGRATION_ID: &str = "vscode-chat";
pub const API_VERSION: &str = "2026-06-01";
pub const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
pub const NEVER_EXPIRES_MS: i64 = 8_640_000_000_000_000;
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const DEFAULT_INTERVAL: i64 = 5;

pub(crate) struct Endpoints {
    pub device_url: String,
    pub token_url: String,
    pub exchange_url: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            device_url: DEVICE_URL.to_owned(),
            token_url: TOKEN_URL.to_owned(),
            exchange_url: EXCHANGE_URL.to_owned(),
        }
    }
}

/// Device-code JSON (VS Code GitHub App) or hosts.json / pasted `ghu_` import.
///
/// # Errors
/// Unsupported method, empty import, device HTTP, or exchange failure.
pub async fn start(input: &StartInput) -> Result<StartOutcome, Error> {
    start_at(input, &Endpoints::default()).await
}

async fn start_at(input: &StartInput, endpoints: &Endpoints) -> Result<StartOutcome, Error> {
    match method_of(input) {
        "pkce" | "authcode" | "idc" => Err(Error::UnsupportedFlow),
        "import" | "paste" | "env" | "key" | "cli" => import_from_body(input, endpoints).await,
        _ if github_token_from_body(&input.body).is_some() => import_from_body(input, endpoints).await,
        _ => start_device(input, endpoints).await,
    }
}

/// Re-exchange `ghu_` → `tid=`. Key sources keep their github token.
///
/// # Errors
/// Missing GitHub token, or 401 / 403 on exchange (permanent).
pub async fn refresh(session: Session) -> Result<Session, Error> {
    refresh_at(session, &Endpoints::default()).await
}

async fn refresh_at(session: Session, endpoints: &Endpoints) -> Result<Session, Error> {
    let mut github = github_token_of(&session);
    let mut github_refresh = extra_str(&session, "githubRefreshToken").map(str::to_owned);
    if github.is_none() {
        let rotated = refresh_github_token(&session, endpoints).await?;
        github = Some(rotated.0);
        github_refresh = Some(rotated.1);
    }
    let Some(github) = github else {
        return Err(Error::InvalidImport(
            "copilot session needs a GitHub token".to_owned(),
        ));
    };
    match exchange_at(&github, &endpoints.exchange_url).await {
        Ok(exchanged) => mint_from_exchange(&session, &github, github_refresh.as_deref(), &exchanged),
        Err(error) if error.is_permanent() => match refresh_github_token(&session, endpoints).await {
            Ok((github, github_refresh)) => {
                let exchanged = exchange_at(&github, &endpoints.exchange_url).await?;
                mint_from_exchange(&session, &github, Some(&github_refresh), &exchanged)
            }
            Err(_) => Err(error),
        },
        Err(error) => Err(error),
    }
}

/// 401 / 403 require a new login.
#[must_use]
pub fn is_permanent_refresh_error(error: &Error) -> bool {
    error.is_permanent()
}

/// JSON device-authorization body: `client_id` + `read:user`. Never OpenCode.
#[must_use]
pub fn device_authorization_json() -> Value {
    serde_json::json!({
        "client_id": CLIENT_ID,
        "scope": SCOPE,
    })
}

/// VS Code Copilot identity. Missing these 403s Business / preview.
#[must_use]
pub fn identity_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    insert_header(&mut headers, "user-agent", USER_AGENT);
    insert_header(&mut headers, "editor-version", EDITOR_VERSION);
    insert_header(&mut headers, "editor-plugin-version", PLUGIN_VERSION);
    insert_header(&mut headers, "copilot-integration-id", INTEGRATION_ID);
    headers
}

/// Quota uses `token <ghu_>`, never `tid=` as Bearer.
///
/// # Errors
/// No GitHub user token on the session.
pub fn quota_authorization(session: &Session) -> Result<String, Error> {
    let github = github_token_of(session).ok_or_else(|| {
        Error::InvalidImport("copilot session needs a GitHub token".to_owned())
    })?;
    Ok(format!("token {github}"))
}

/// Chat hop `Authorization: Bearer <tid=>` plus vscode-chat identity.
#[must_use]
pub fn hop_headers(session: &Session, cache_session_id: Option<&str>, vision: bool, initiator: &str) -> HeaderMap {
    let mut headers = identity_headers();
    if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", session.access_token)) {
        headers.insert(http::header::AUTHORIZATION, value);
    }
    insert_header(&mut headers, "accept", "application/json");
    insert_header(&mut headers, "openai-intent", "conversation-edits");
    insert_header(&mut headers, "x-github-api-version", API_VERSION);
    let pin = crate::copilot::cache::cache_session_id(cache_session_id)
        .unwrap_or_else(|| COPILOT_STABLE_SESSION.to_owned());
    insert_header(&mut headers, "x-interaction-id", &pin);
    let initiator = if initiator == "agent" { "agent" } else { "user" };
    insert_header(&mut headers, "x-initiator", initiator);
    if vision {
        insert_header(&mut headers, "copilot-vision-request", "true");
    }
    headers
}

/// `GET /copilot_internal/v2/token`. `gho_` 404 falls back to the raw token.
///
/// # Errors
/// Empty GitHub token, or non-404 exchange failure.
pub async fn exchange_github_token(github_token: &str) -> Result<Exchanged, Error> {
    exchange_at(github_token, EXCHANGE_URL).await
}

async fn exchange_at(github_token: &str, exchange_url: &str) -> Result<Exchanged, Error> {
    let token = github_token.trim();
    if token.is_empty() {
        return Err(Error::InvalidImport("copilot GitHub token is empty".to_owned()));
    }
    let mut headers = identity_headers();
    if let Ok(value) = HeaderValue::from_str(&format!("token {token}")) {
        headers.insert(http::header::AUTHORIZATION, value);
    }
    let response = http_client()?
        .get(exchange_url)
        .header(http::header::ACCEPT, "application/json")
        .headers(headers)
        .send()
        .await?;
    let status = response.status().as_u16();
    let text = response.text().await?;
    if (200..300).contains(&status) {
        let payload: Value = serde_json::from_str(&text)
            .map_err(|error| Error::Payload(error.to_string()))?;
        return parse_exchange_payload(&payload)
            .ok_or_else(|| Error::Payload("copilot token exchange returned no token".to_owned()));
    }
    if status == 404 && token.starts_with("gho_") {
        return Ok(Exchanged {
            token: token.to_owned(),
            expires_at_ms: Utc::now().timestamp_millis().saturating_add(8 * 3_600_000),
            api_endpoint: API_ORIGIN.to_owned(),
        });
    }
    if status == 401 || status == 403 {
        return Err(Error::Permanent);
    }
    Err(Error::TokenEndpoint { status, body: text })
}

/// Parsed `/copilot_internal/v2/token` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchanged {
    pub token: String,
    pub expires_at_ms: i64,
    pub api_endpoint: String,
}

/// Parse an exchange JSON body. `expires_at` may be seconds or ms.
#[must_use]
pub fn parse_exchange_payload(payload: &Value) -> Option<Exchanged> {
    let token = string_field(payload, &["token"])?;
    let expires_at_ms = unix_ms(payload.get("expires_at"))
        .or_else(|| {
            positive_i64(payload.get("refresh_in")).map(|seconds| {
                Utc::now().timestamp_millis().saturating_add(seconds.saturating_mul(1000))
            })
        })
        .unwrap_or_else(|| Utc::now().timestamp_millis().saturating_add(25 * 60_000));
    let api = payload
        .get("endpoints")
        .and_then(|endpoints| string_field(endpoints, &["api"]))
        .unwrap_or_else(|| API_ORIGIN.to_owned());
    Some(Exchanged {
        token,
        expires_at_ms,
        api_endpoint: api.trim_end_matches('/').to_owned(),
    })
}

/// Device tokens (`ghu_`) plus exchange result.
///
/// # Errors
/// Missing GitHub token or exchange failure.
pub async fn complete_device(tokens: &Value) -> Result<Session, Error> {
    complete_device_at(tokens, EXCHANGE_URL).await
}

async fn complete_device_at(tokens: &Value, exchange_url: &str) -> Result<Session, Error> {
    let github = string_field(tokens, &["access_token", "accessToken"])
        .ok_or_else(|| Error::Payload("copilot device flow returned no access token".to_owned()))?;
    let exchanged = exchange_at(&github, exchange_url).await?;
    copilot_session(CopilotSession {
        access_token: &exchanged.token,
        refresh_token: string_field(tokens, &["refresh_token", "refreshToken"]).as_deref(),
        expires_at_ms: Some(exchanged.expires_at_ms),
        account: None,
        plan_type: None,
        source: "oauth",
        github_token: Some(&github),
        github_refresh_token: string_field(tokens, &["refresh_token", "refreshToken"]).as_deref(),
        api_endpoint: Some(&exchanged.api_endpoint),
    })
}

/// Trim a pasted GitHub token.
///
/// # Errors
/// Empty or shorter than 8.
pub fn parse_api_key(value: &str) -> Result<String, Error> {
    let key = value.trim();
    if key.len() < 8 {
        return Err(Error::InvalidImport("copilot token is empty".to_owned()));
    }
    Ok(key.to_owned())
}

/// `GET /user` login.
#[must_use]
pub fn parse_user(payload: &Value) -> Option<String> {
    string_field(payload, &["login"]).or_else(|| string_field(payload, &["name"]))
}

/// Chat URL from `endpoints.api`, defaulting to api.githubcopilot.com.
#[must_use]
pub fn chat_url(session: &Session) -> String {
    extra_str(session, "apiEndpoint")
        .map(|base| format!("{}/chat/completions", base.trim_end_matches('/')))
        .unwrap_or_else(|| API_URL.to_owned())
}

#[must_use]
pub fn is_github_user_token(value: &str) -> bool {
    let token = value.trim();
    token.starts_with("ghu_")
        || token.starts_with("gho_")
        || token.starts_with("ghp_")
        || token.starts_with("github_pat_")
}

#[must_use]
pub fn is_session_token(value: &str) -> bool {
    let token = value.trim();
    token.starts_with("tid=") || (token.contains(";exp=") && !is_github_user_token(token))
}

struct CopilotSession<'a> {
    access_token: &'a str,
    refresh_token: Option<&'a str>,
    expires_at_ms: Option<i64>,
    account: Option<&'a str>,
    plan_type: Option<&'a str>,
    source: &'a str,
    github_token: Option<&'a str>,
    github_refresh_token: Option<&'a str>,
    api_endpoint: Option<&'a str>,
}

fn copilot_session(input: CopilotSession<'_>) -> Result<Session, Error> {
    let access = input.access_token.trim();
    if access.is_empty() {
        return Err(Error::Payload(
            "copilot token endpoint returned no access token".to_owned(),
        ));
    }
    let github = input
        .github_token
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| is_github_user_token(access).then(|| access.to_owned()));
    let key = is_key_source(input.source);
    let refresh = input
        .refresh_token
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            input
                .github_refresh_token
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| github.clone())
        .or_else(|| key.then(|| access.to_owned()))
        .ok_or_else(|| {
            Error::Payload("copilot token endpoint returned no refresh token".to_owned())
        })?;
    let expires = input
        .expires_at_ms
        .filter(|v| *v > 0)
        .or_else(|| key.then_some(NEVER_EXPIRES_MS));
    let Some(expires) = expires else {
        return Err(Error::Payload(
            "copilot token endpoint returned no usable expiry".to_owned(),
        ));
    };
    let source = match input.source {
        "cli" | "paste" | "env" | "oauth" => input.source,
        _ => "oauth",
    };
    let mut session = Session::new(Family::Copilot, access, refresh);
    session.expires_at_ms = expires;
    let account_seed = github.as_deref().unwrap_or(access);
    session.account = input
        .account
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| default_account(account_seed));
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
    if let Some(github) = github {
        session
            .extra
            .insert("githubToken".to_owned(), Value::String(github));
    }
    if let Some(refresh) = input
        .github_refresh_token
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        session
            .extra
            .insert("githubRefreshToken".to_owned(), Value::String(refresh.to_owned()));
    }
    if let Some(api) = input.api_endpoint.map(str::trim).filter(|s| !s.is_empty()) {
        session.extra.insert(
            "apiEndpoint".to_owned(),
            Value::String(api.trim_end_matches('/').to_owned()),
        );
    }
    Ok(session)
}

fn mint_from_exchange(
    session: &Session,
    github: &str,
    github_refresh: Option<&str>,
    exchanged: &Exchanged,
) -> Result<Session, Error> {
    let source = if matches!(session.source.as_str(), "cli" | "paste" | "env") {
        session.source.as_str()
    } else {
        "oauth"
    };
    copilot_session(CopilotSession {
        access_token: &exchanged.token,
        refresh_token: github_refresh.or(Some(session.refresh_token.as_str())),
        expires_at_ms: Some(exchanged.expires_at_ms),
        account: Some(session.account.as_str()).filter(|s| !s.is_empty()),
        plan_type: Some(session.plan_type.as_str()).filter(|s| !s.is_empty()),
        source,
        github_token: Some(github),
        github_refresh_token: github_refresh,
        api_endpoint: Some(&exchanged.api_endpoint),
    })
}

async fn start_device(input: &StartInput, endpoints: &Endpoints) -> Result<StartOutcome, Error> {
    let body = device_authorization_json();
    let mut headers = HeaderMap::new();
    insert_header(&mut headers, "user-agent", USER_AGENT);
    let (status, text) = post_json(&endpoints.device_url, &body, &headers).await?;
    if !(200..300).contains(&status) {
        return Err(Error::TokenEndpoint { status, body: text });
    }
    let wire: Value = serde_json::from_str(&text)
        .map_err(|error| Error::Payload(error.to_string()))?;
    let started = parse_device_response(&wire)?;
    let mut extra = Map::new();
    extra.insert("device_code".to_owned(), Value::String(started.device_code));
    extra.insert(
        "token_endpoint".to_owned(),
        Value::String(endpoints.token_url.clone()),
    );
    extra.insert("client_id".to_owned(), Value::String(CLIENT_ID.to_owned()));
    extra.insert("grant_type".to_owned(), Value::String(DEVICE_GRANT.to_owned()));
    extra.insert("json_body".to_owned(), Value::Bool(true));
    extra.insert("scope".to_owned(), Value::String(SCOPE.to_owned()));
    extra.insert("restart_on_expired".to_owned(), Value::Bool(true));
    extra.insert("expires_in".to_owned(), Value::Number(started.expires_in.into()));
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
        .ok_or_else(|| Error::Payload("copilot device-code response is missing device_code".to_owned()))?;
    let user_code = string_field(wire, &["user_code", "userCode"])
        .ok_or_else(|| Error::Payload("copilot device-code response is missing user_code".to_owned()))?;
    let verification_uri = string_field(wire, &["verification_uri", "verificationUri"])
        .ok_or_else(|| {
            Error::Payload("copilot device-code response is missing verification_uri".to_owned())
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

async fn import_from_body(input: &StartInput, endpoints: &Endpoints) -> Result<StartOutcome, Error> {
    let github = github_token_from_body(&input.body).ok_or_else(|| {
        Error::InvalidImport("copilot token is empty".to_owned())
    })?;
    let github = parse_api_key(&github)?;
    let source = match method_of(input) {
        "env" => "env",
        "cli" | "import" => "cli",
        _ => "paste",
    };
    let exchanged = exchange_at(&github, &endpoints.exchange_url).await?;
    Ok(StartOutcome::Ready(copilot_session(CopilotSession {
        access_token: &exchanged.token,
        refresh_token: Some(&github),
        expires_at_ms: Some(exchanged.expires_at_ms),
        account: string_field(&input.body, &["account", "login"]).as_deref(),
        plan_type: None,
        source,
        github_token: Some(&github),
        github_refresh_token: None,
        api_endpoint: Some(&exchanged.api_endpoint),
    })?))
}

async fn refresh_github_token(
    session: &Session,
    endpoints: &Endpoints,
) -> Result<(String, String), Error> {
    let refresh = extra_str(session, "githubRefreshToken")
        .map(str::to_owned)
        .or_else(|| {
            let candidate = session.refresh_token.trim();
            (!candidate.is_empty() && !is_github_user_token(candidate)).then(|| candidate.to_owned())
        });
    let Some(refresh) = refresh else {
        return Err(Error::InvalidImport(
            "copilot session needs a GitHub token".to_owned(),
        ));
    };
    if extra_str(session, "githubToken") == Some(refresh.as_str()) {
        return Err(Error::InvalidImport(
            "copilot session needs a GitHub token".to_owned(),
        ));
    }
    let body = serde_json::json!({
        "client_id": client_id_of(session),
        "grant_type": "refresh_token",
        "refresh_token": refresh,
    });
    let mut headers = HeaderMap::new();
    insert_header(&mut headers, "user-agent", USER_AGENT);
    let (status, text) = post_json(&endpoints.token_url, &body, &headers).await?;
    if status == 401 || status == 403 {
        return Err(Error::Permanent);
    }
    if !(200..300).contains(&status) {
        return Err(Error::TokenEndpoint { status, body: text });
    }
    let payload: Value = serde_json::from_str(&text)
        .map_err(|error| Error::Payload(error.to_string()))?;
    let access = string_field(&payload, &["access_token", "accessToken"]).ok_or_else(|| {
        Error::Payload("copilot GitHub refresh returned no access token".to_owned())
    })?;
    let next_refresh = string_field(&payload, &["refresh_token", "refreshToken"]).unwrap_or(refresh);
    Ok((access, next_refresh))
}

fn github_token_from_body(body: &Value) -> Option<String> {
    if let Some(text) = body.as_str() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }
    if let Some(token) = string_field(
        body,
        &[
            "github_token",
            "githubToken",
            "oauth_token",
            "token",
            "access_token",
            "COPILOT_GITHUB_TOKEN",
            "GITHUB_TOKEN",
            "GH_TOKEN",
        ],
    ) {
        return Some(token);
    }
    token_from_hosts(body)
}

fn token_from_hosts(data: &Value) -> Option<String> {
    let object = data.as_object()?;
    for key in ["github.com", "github"] {
        if let Some(github) = object.get(key)
            && let Some(token) = string_field(github, &["oauth_token", "token", "access_token"])
        {
            return Some(token);
        }
    }
    for value in object.values() {
        if let Some(token) = string_field(value, &["oauth_token", "token"])
            && is_github_user_token(&token)
        {
            return Some(token);
        }
    }
    None
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
    format!("copilot-{}", fingerprint(token))
}

fn fingerprint(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(&digest[..4])
}

fn github_token_of(session: &Session) -> Option<String> {
    extra_str(session, "githubToken")
        .map(str::to_owned)
        .or_else(|| {
            is_github_user_token(&session.refresh_token).then(|| session.refresh_token.trim().to_owned())
        })
        .or_else(|| {
            is_github_user_token(&session.access_token).then(|| session.access_token.trim().to_owned())
        })
}

fn extra_str<'a>(session: &'a Session, key: &str) -> Option<&'a str> {
    session
        .extra
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn client_id_of(session: &Session) -> String {
    extra_str(session, "client_id")
        .unwrap_or(CLIENT_ID)
        .to_owned()
}

fn unix_ms(value: Option<&Value>) -> Option<i64> {
    let raw = positive_i64(value)?;
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

fn insert_header(headers: &mut HeaderMap, name: &'static str, value: &str) {
    let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
        return;
    };
    let Ok(value) = HeaderValue::from_str(value) else {
        return;
    };
    headers.insert(name, value);
}

fn http_client() -> Result<reqwest::Client, Error> {
    Ok(reqwest::Client::builder().timeout(HTTP_TIMEOUT).build()?)
}

async fn post_json(url: &str, body: &Value, headers: &HeaderMap) -> Result<(u16, String), Error> {
    let response = http_client()?
        .post(url)
        .header(http::header::ACCEPT, "application/json")
        .header(http::header::CONTENT_TYPE, "application/json")
        .headers(headers.clone())
        .body(serde_json::to_vec(body).map_err(|error| Error::Payload(error.to_string()))?)
        .send()
        .await?;
    let status = response.status().as_u16();
    let text = response.text().await?;
    Ok((status, text))
}
