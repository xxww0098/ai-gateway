//! xAI Grok subscription OAuth.
//!
//! Design: [`README.md`]. Default login is RFC 8628 device-code against
//! `auth.x.ai` (discovery host must be `x.ai` / `*.x.ai`). Chat hop is
//! grok-build Responses. Cache lives only in [`cache`].

pub mod cache;
pub mod credits;
pub mod device;
pub mod quota;
pub mod request;

pub use cache::{GROK_STABLE_SESSION, affinity_headers, apply_cache, cache_session_id};
pub use credits::{CreditsSnapshot, EMPTY_FRAME, decode_credits_frame};
pub use device::{PollOutcome, poll};
pub use quota::{Quota, QuotaRow, QuotaUrls, apply_credits_snapshot, fetch_quota, parse_billing};
pub use request::normalize_request;

use std::sync::Mutex;
use std::time::Duration;

use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};
use url::Url;

use crate::{Error, FlowKind, Session, StartOutcome};

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

/// Stored `auth_records.provider` / family id.
pub const ID: &str = "grok";
pub const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub const CLIENT_VERSION: &str = "0.2.93";
pub const USER_AGENT: &str = "grok-cli/0.2.93";
pub const TOKEN_URL: &str = "https://auth.x.ai/oauth/token";
pub const API_URL: &str = "https://api.x.ai/v1/responses";
pub const DISCOVERY_URL: &str = "https://auth.x.ai/.well-known/openid-configuration";
pub const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
pub const BILLING_URL: &str = "https://cli-chat-proxy.grok.com/v1/billing?format=credits";
pub const CLI_USER_URL: &str = "https://cli-chat-proxy.grok.com/v1/user?include=subscription";
pub const CREDITS_URL: &str = "https://grok.com/grok_api_v2.GrokBuildBilling/GetGrokCreditsConfig";
const DEVICE_CODE_FALLBACK: &str = "https://auth.x.ai/oauth2/device/code";
const TOKEN_AUTH: &str = "xai-grok-cli";
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// OIDC endpoints after host checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub authorization: String,
    pub token: String,
    pub device_authorization: String,
}

static DISCOVERY_CACHE: Mutex<Option<Endpoints>> = Mutex::new(None);

/// Drop the in-process OIDC cache (tests).
pub fn reset_discovery() {
    *DISCOVERY_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// Device-code (default) or PKCE. Panel supplies its own `redirect_uri` for PKCE.
///
/// # Errors
/// Unsupported method, vendor HTTP, discovery that is not on x.ai.
pub async fn start(input: &crate::login::StartInput) -> Result<StartOutcome, Error> {
    let client = http_client()?;
    start_with(&client, input, DISCOVERY_URL).await
}

pub(crate) async fn start_with(
    client: &reqwest::Client,
    input: &crate::login::StartInput,
    discovery_url: &str,
) -> Result<StartOutcome, Error> {
    match method_of(input).as_str() {
        "" | "device" => {
            let endpoints = discover_from(client, discovery_url).await?;
            device::request_device(
                client,
                &endpoints.device_authorization,
                &endpoints.token,
                &input.state,
            )
            .await
        }
        "pkce" | "authcode" | "authorization_code" => {
            let endpoints = discover_from(client, discovery_url).await?;
            authorize_browser(input, &endpoints)
        }
        _ => Err(Error::UnsupportedFlow),
    }
}

/// Authorization-code exchange. `token_endpoint` comes from start extra;
/// empty falls back to [`TOKEN_URL`] after the x.ai host check.
///
/// # Errors
/// Missing verifier, HTTP 403 (plan has no API OAuth), other token errors.
pub async fn exchange(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    challenge: &str,
    token_endpoint: &str,
) -> Result<Session, Error> {
    let client = http_client()?;
    exchange_at(
        &client,
        token_endpoint,
        code,
        verifier,
        redirect_uri,
        challenge,
    )
    .await
}

pub(crate) async fn exchange_at(
    client: &reqwest::Client,
    token_endpoint: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    challenge: &str,
) -> Result<Session, Error> {
    if verifier.trim().is_empty() {
        return Err(Error::MissingVerifier);
    }
    if code.trim().is_empty() {
        return Err(Error::Payload(
            "grok authorization code is empty".to_owned(),
        ));
    }
    let url = token_url_or_err(if token_endpoint.trim().is_empty() {
        TOKEN_URL
    } else {
        token_endpoint.trim()
    })?;
    let (status, body) = post_form(
        client,
        &url,
        &[
            ("grant_type", "authorization_code".to_owned()),
            ("client_id", CLIENT_ID.to_owned()),
            ("code", code.to_owned()),
            ("redirect_uri", redirect_uri.to_owned()),
            ("code_verifier", verifier.to_owned()),
            ("code_challenge", challenge.to_owned()),
            ("code_challenge_method", "S256".to_owned()),
        ],
    )
    .await?;
    if status == 403 {
        return Err(Error::TokenEndpoint {
            status,
            body: "grok token endpoint refused the exchange (HTTP 403): your X plan does not include the API OAuth entitlement; an X Premium or xAI subscription with API access is required".to_owned(),
        });
    }
    if !(200..300).contains(&status) {
        return Err(token_error(status, &body, false));
    }
    let tokens: Value = serde_json::from_str(&body)
        .map_err(|_| Error::Payload("grok token endpoint returned non-JSON".to_owned()))?;
    session_from_tokens(&tokens, &url, None)
}

/// Form refresh. `invalid_grant` is [`Error::Permanent`].
///
/// # Errors
/// Permanent refresh failure, other token HTTP, missing refresh token.
pub async fn refresh(session: &Session) -> Result<Session, Error> {
    let client = http_client()?;
    refresh_at(&client, session).await
}

pub(crate) async fn refresh_at(
    client: &reqwest::Client,
    session: &Session,
) -> Result<Session, Error> {
    if session.refresh_token.trim().is_empty() {
        return Err(Error::Permanent);
    }
    let token_endpoint = session
        .extra
        .get("token_endpoint")
        .and_then(Value::as_str)
        .unwrap_or(TOKEN_URL);
    let url = token_url_or_err(token_endpoint)?;
    let client_id = session
        .extra
        .get("client_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(CLIENT_ID);
    let (status, body) = post_form(
        client,
        &url,
        &[
            ("grant_type", "refresh_token".to_owned()),
            ("client_id", client_id.to_owned()),
            ("refresh_token", session.refresh_token.clone()),
        ],
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(token_error(status, &body, true));
    }
    let tokens: Value = serde_json::from_str(&body)
        .map_err(|_| Error::Payload("grok token endpoint returned non-JSON".to_owned()))?;
    let mut next = session_from_tokens(&tokens, &url, Some(session))?;
    if next.account.is_empty() {
        next.account = session.account.clone();
    }
    if next.plan_type.is_empty() {
        next.plan_type = session.plan_type.clone();
    }
    if !session.extra.contains_key("client_id") {
        next.extra.remove("client_id");
    } else if !next.extra.contains_key("client_id")
        && let Some(id) = session.extra.get("client_id")
    {
        next.extra.insert("client_id".to_owned(), id.clone());
    }
    Ok(next)
}

/// Build a stored session from a token-endpoint JSON object.
///
/// # Errors
/// Missing access / refresh token, or no usable expiry (`expires_in` or JWT `exp`).
pub fn session_from_tokens(
    tokens: &Value,
    token_endpoint: &str,
    fallback: Option<&Session>,
) -> Result<Session, Error> {
    let access = json_text(tokens, &["access_token", "accessToken"]);
    if access.is_empty() {
        return Err(Error::Payload(
            "grok token endpoint returned no access token".to_owned(),
        ));
    }
    let refresh_token = json_text(tokens, &["refresh_token", "refreshToken"]);
    let refresh_token = if refresh_token.is_empty() {
        fallback
            .map(|s| s.refresh_token.clone())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                Error::Payload("grok token endpoint returned no refresh token".to_owned())
            })?
    } else {
        refresh_token
    };
    let expires_at_ms = expires_at_ms(tokens, &access)?;
    let id_token = json_text(tokens, &["id_token", "idToken"]);
    let account = grok_account(&id_token)
        .or_else(|| fallback.and_then(|s| (!s.account.is_empty()).then(|| s.account.clone())))
        .or_else(|| tier_name_from_token(&access));
    let plan_type = tier_name_from_token(&access)
        .or_else(|| fallback.and_then(|s| (!s.plan_type.is_empty()).then(|| s.plan_type.clone())));
    let client_id = json_text(tokens, &["client_id", "clientId"]);
    let scopes = json_text(tokens, &["scope", "scopes"]);
    let mut session = Session::new(crate::Family::Grok, access, refresh_token);
    session.expires_at_ms = expires_at_ms;
    if let Some(account) = account {
        session.account = account;
    }
    if let Some(plan_type) = plan_type {
        session.plan_type = crate::format_plan_label(&plan_type, crate::Family::Grok);
    }
    session
        .extra
        .insert("token_endpoint".to_owned(), json!(token_endpoint));
    if !scopes.is_empty() {
        session.extra.insert("scopes".to_owned(), json!(scopes));
    } else if let Some(prev) = fallback.and_then(|s| s.extra.get("scopes")) {
        session.extra.insert("scopes".to_owned(), prev.clone());
    }
    if !client_id.is_empty() {
        session
            .extra
            .insert("client_id".to_owned(), json!(client_id));
    } else if let Some(prev) = fallback.and_then(|s| s.extra.get("client_id")) {
        session.extra.insert("client_id".to_owned(), prev.clone());
    }
    Ok(session)
}

/// JWT numeric / string tier → SuperGrok / X Premium+ / …
#[must_use]
pub fn tier_from_value(value: &Value) -> Option<String> {
    if let Some(n) = numeric(value) {
        let tier = n.trunc() as i64;
        return Some(
            tier_name(tier)
                .map(str::to_owned)
                .unwrap_or_else(|| tier.to_string()),
        );
    }
    let trimmed = value.as_str()?.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().all(|c| c.is_ascii_digit())
        && let Ok(n) = trimmed.parse::<i64>()
    {
        return tier_from_value(&Value::from(n));
    }
    Some(trimmed.to_owned())
}

/// https + host `x.ai` or `*.x.ai`.
///
/// # Errors
/// Unparseable URL, non-https, or a host that is not x.ai.
pub fn validate_xai_endpoint(raw: &str) -> Result<String, Error> {
    let trimmed = raw.trim();
    let url = Url::parse(trimmed).map_err(|_| {
        Error::Payload("grok OIDC discovery returned an invalid endpoint".to_owned())
    })?;
    if url.scheme() != "https" {
        return Err(Error::Payload(format!(
            "grok OIDC discovery returned a non-x.ai endpoint: {trimmed}"
        )));
    }
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if host == "x.ai" || host.ends_with(".x.ai") {
        Ok(trimmed.to_owned())
    } else {
        Err(Error::Payload(format!(
            "grok OIDC discovery returned a non-x.ai endpoint: {trimmed}"
        )))
    }
}

/// Parse an OIDC discovery document. Device endpoint falls back to auth.x.ai
/// only when the field is absent, never when it points off-issuer.
///
/// # Errors
/// Missing endpoints, or any endpoint not on x.ai.
pub fn parse_discovery(document: &Value) -> Result<Endpoints, Error> {
    let authorization = document
        .get("authorization_endpoint")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            Error::Payload("grok OIDC discovery document is missing endpoints".to_owned())
        })?;
    let token = document
        .get("token_endpoint")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            Error::Payload("grok OIDC discovery document is missing endpoints".to_owned())
        })?;
    let device = match document
        .get("device_authorization_endpoint")
        .and_then(Value::as_str)
    {
        Some(url) => validate_xai_endpoint(url)?,
        None => DEVICE_CODE_FALLBACK.to_owned(),
    };
    Ok(Endpoints {
        authorization: validate_xai_endpoint(authorization)?,
        token: validate_xai_endpoint(token)?,
        device_authorization: device,
    })
}

/// grok-cli User-Agent only.
#[must_use]
pub fn credential_headers() -> HeaderMap {
    header_map(&[("user-agent", USER_AGENT)])
}

/// Responses hop: Bearer + `x-xai-token-auth`. Never Codex `session-id`.
#[must_use]
pub fn upstream_headers(session: &Session) -> HeaderMap {
    let authorization = format!("Bearer {}", session.access_token);
    let mut headers = header_map(&[
        ("authorization", authorization.as_str()),
        ("x-xai-token-auth", TOKEN_AUTH),
        ("accept", "application/json"),
        ("user-agent", USER_AGENT),
    ]);
    if let Some(user_id) = grok_user_id(session)
        && let Ok(value) = HeaderValue::from_str(&user_id)
    {
        headers.insert(HeaderName::from_static("x-userid"), value);
    }
    headers
}

/// grok.com GetGrokCreditsConfig gRPC-web headers.
#[must_use]
pub fn credits_headers(session: &Session) -> HeaderMap {
    let authorization = format!("Bearer {}", session.access_token);
    header_map(&[
        ("authorization", authorization.as_str()),
        ("content-type", "application/grpc-web+proto"),
        ("x-grpc-web", "1"),
        ("accept", "*/*"),
        ("origin", "https://grok.com"),
        ("referer", "https://grok.com/?_s=usage"),
        ("x-user-agent", "connect-es/2.1.1"),
        ("user-agent", USER_AGENT),
    ])
}

fn grok_user_id(session: &Session) -> Option<String> {
    let claims = crate::decode_payload(&session.access_token)?;
    claims
        .get("sub")
        .or_else(|| claims.get("user_id"))
        .or_else(|| claims.get("userId"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn header_map(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(value) else {
            continue;
        };
        headers.insert(name, value);
    }
    headers
}

fn method_of(input: &crate::login::StartInput) -> String {
    let field = input.method.trim();
    if !field.is_empty() {
        return field.to_ascii_lowercase();
    }
    input
        .body
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

fn authorize_browser(
    input: &crate::login::StartInput,
    endpoints: &Endpoints,
) -> Result<StartOutcome, Error> {
    if input.redirect_uri.trim().is_empty() {
        return Err(Error::Payload(
            "grok PKCE start needs a redirect_uri".to_owned(),
        ));
    }
    let pkce = crate::create_pkce()?;
    let nonce = crate::random_hex(16)?;
    let params = [
        ("response_type", "code".to_owned()),
        ("client_id", CLIENT_ID.to_owned()),
        ("redirect_uri", input.redirect_uri.clone()),
        ("scope", SCOPE.to_owned()),
        ("code_challenge", pkce.challenge.clone()),
        ("code_challenge_method", "S256".to_owned()),
        ("state", input.state.clone()),
        ("nonce", nonce.clone()),
        ("plan", "generic".to_owned()),
    ];
    let authorize_url = format!(
        "{}?{}",
        endpoints.authorization,
        crate::form::encode(&params)
    );
    let mut extra = Map::new();
    extra.insert("code_challenge".to_owned(), json!(pkce.challenge));
    extra.insert("nonce".to_owned(), json!(nonce));
    extra.insert("token_endpoint".to_owned(), json!(endpoints.token));
    extra.insert("client_id".to_owned(), json!(CLIENT_ID));
    Ok(StartOutcome::Browser {
        authorize_url,
        state: input.state.clone(),
        verifier: pkce.verifier,
        redirect_uri: input.redirect_uri.clone(),
        flow: FlowKind::AuthorizationCode,
        extra,
    })
}

pub(crate) async fn discover_from(client: &reqwest::Client, url: &str) -> Result<Endpoints, Error> {
    if url == DISCOVERY_URL
        && let Some(cached) = DISCOVERY_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    {
        return Ok(cached);
    }
    let response = client
        .get(url)
        .header("accept", "application/json")
        .header("user-agent", USER_AGENT)
        .send()
        .await?;
    let status = response.status().as_u16();
    let body = response.text().await?;
    if !(200..300).contains(&status) {
        return Err(Error::TokenEndpoint { status, body });
    }
    let document: Value = serde_json::from_str(&body)
        .map_err(|_| Error::Payload("grok OIDC discovery document is not JSON".to_owned()))?;
    let endpoints = parse_discovery(&document)?;
    if url == DISCOVERY_URL {
        *DISCOVERY_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(endpoints.clone());
    }
    Ok(endpoints)
}

pub(crate) fn http_client() -> Result<reqwest::Client, Error> {
    Ok(reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

pub(crate) async fn post_form(
    client: &reqwest::Client,
    url: &str,
    params: &[(&str, String)],
) -> Result<(u16, String), Error> {
    let response = client
        .post(url)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .header("user-agent", USER_AGENT)
        .body(crate::form::encode(params))
        .send()
        .await?;
    let status = response.status().as_u16();
    let body = response.text().await?;
    Ok((status, body))
}

pub(crate) fn token_url_or_err(raw: &str) -> Result<String, Error> {
    #[cfg(test)]
    {
        if let Ok(url) = Url::parse(raw.trim()) {
            let host = url.host_str().unwrap_or("");
            if url.scheme() == "http" && (host == "127.0.0.1" || host == "localhost") {
                return Ok(raw.trim().to_owned());
            }
        }
    }
    validate_xai_endpoint(raw)
}

fn token_error(status: u16, body: &str, refresh: bool) -> Error {
    if refresh && oauth_code(body).as_deref() == Some("invalid_grant") {
        Error::Permanent
    } else {
        Error::TokenEndpoint {
            status,
            body: body.to_owned(),
        }
    }
}

fn oauth_code(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    parsed
        .get("error")
        .or_else(|| parsed.get("error_code"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn json_text(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .unwrap_or("")
        .trim()
        .to_owned()
}

fn expires_at_ms(tokens: &Value, access_token: &str) -> Result<i64, Error> {
    if let Some(secs) =
        positive_number(tokens.get("expires_in").or_else(|| tokens.get("expiresIn")))
    {
        return Ok(chrono::Utc::now().timestamp_millis() + secs.saturating_mul(1000));
    }
    let exp = crate::decode_payload(access_token)
        .and_then(|claims| claims.get("exp").cloned())
        .and_then(|v| numeric(&v))
        .filter(|n| *n > 0.0)
        .map(|n| (n * 1000.0).round() as i64);
    exp.ok_or_else(|| Error::Payload("grok token endpoint returned no usable expiry".to_owned()))
}

fn positive_number(value: Option<&Value>) -> Option<i64> {
    let n = numeric(value?)?;
    (n > 0.0).then_some(n.round() as i64)
}

fn numeric(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn grok_account(id_token: &str) -> Option<String> {
    if id_token.is_empty() {
        return None;
    }
    let claims = crate::decode_payload(id_token)?;
    ["email", "preferred_username", "name", "sub"]
        .into_iter()
        .find_map(|key| {
            claims
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
}

fn tier_name_from_token(access_token: &str) -> Option<String> {
    let claims = crate::decode_payload(access_token)?;
    let raw = claims
        .get("tier")
        .or_else(|| claims.get("subscription_tier"))
        .or_else(|| claims.get("subscriptionTier"))?;
    tier_from_value(raw)
}

fn tier_name(tier: i64) -> Option<&'static str> {
    Some(match tier {
        0 => "Free",
        1 => "SuperGrok",
        2 => "X Basic",
        3 => "X Premium",
        4 => "X Premium+",
        5 => "SuperGrok Heavy",
        6 => "SuperGrok Lite",
        7 => "SuperGrok Plus",
        _ => return None,
    })
}
