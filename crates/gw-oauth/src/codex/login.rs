//! Token exchange, refresh, session claims, and Codex hop headers.

use std::time::Duration;

use chrono::Utc;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value};

use crate::{Error, Family, Session};

use super::{CLIENT_ID, CLIENT_VERSION, ORIGINATOR, TOKEN_URL, USER_AGENT};

#[cfg(test)]
mod tests;

const PERMANENT_REFRESH_CODES: [&str; 4] = [
    "refresh_token_expired",
    "refresh_token_reused",
    "refresh_token_invalidated",
    "invalid_grant",
];

/// Form POST authorization-code exchange.
///
/// # Errors
/// Missing verifier, token endpoint failure, or a payload without access /
/// refresh / expiry / ChatGPT account id.
pub async fn exchange_code(
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<Session, Error> {
    exchange_code_at(TOKEN_URL, redirect_uri, code, verifier).await
}

/// JSON refresh. Permanent vendor codes become [`Error::Permanent`].
///
/// # Errors
/// Token endpoint failure, permanent refresh codes, or a malformed payload.
pub async fn refresh(session: &Session) -> Result<Session, Error> {
    refresh_at(TOKEN_URL, session).await
}

/// Codex CLI `x-codex-routing-hint`: `model=<id>` or `model=<id>;tier=priority`.
#[must_use]
pub fn routing_hint(model: &str, service_tier: Option<&str>) -> Option<String> {
    if model.is_empty() {
        return None;
    }
    match service_tier.map(str::trim).filter(|tier| !tier.is_empty()) {
        Some(tier) => Some(format!("model={model};tier={tier}")),
        None => Some(format!("model={model}")),
    }
}

/// `originator` + `user-agent` pair the token endpoint and Responses API expect.
#[must_use]
pub fn credential_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    insert_str(&mut headers, "originator", ORIGINATOR);
    insert_str(&mut headers, "user-agent", USER_AGENT);
    headers
}

/// Upstream ChatGPT Codex Responses headers for `session`.
#[must_use]
pub fn upstream_headers(session: &Session) -> HeaderMap {
    let mut headers = credential_headers();
    insert_str(
        &mut headers,
        "authorization",
        &format!("Bearer {}", session.access_token),
    );
    insert_str(&mut headers, "chatgpt-account-id", account_id(session));
    insert_str(&mut headers, "openai-version", CLIENT_VERSION);
    insert_str(&mut headers, "openai-beta", "responses=experimental");
    insert_str(&mut headers, "accept", "application/json");
    headers
}

pub(super) struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub expires_in: Option<i64>,
}

pub(super) fn session_from_tokens(
    tokens: TokenSet,
    fallback: Option<&Session>,
) -> Result<Session, Error> {
    if tokens.access_token.is_empty() {
        return Err(Error::Payload(
            "codex token endpoint returned no access token".to_owned(),
        ));
    }
    let refresh_token = tokens
        .refresh_token
        .filter(|value| !value.is_empty())
        .or_else(|| fallback.map(|session| session.refresh_token.clone()))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            Error::Payload("codex token endpoint returned no refresh token".to_owned())
        })?;
    let expires_at_ms = match tokens.expires_in.filter(|value| *value > 0) {
        Some(seconds) => now_ms().saturating_add(seconds.saturating_mul(1000)),
        None => jwt_exp_ms(&tokens.access_token).ok_or_else(|| {
            Error::Payload("codex token endpoint returned no usable expiry".to_owned())
        })?,
    };

    let mut session = Session::new(Family::Codex, tokens.access_token, refresh_token);
    session.expires_at_ms = expires_at_ms;
    if let Some(previous) = fallback {
        session.account = previous.account.clone();
        session.plan_type = previous.plan_type.clone();
        session.extra = previous.extra.clone();
        session.source = previous.source.clone();
    }

    let fresh_id_token = tokens.id_token.filter(|value| !value.is_empty());
    match &fresh_id_token {
        Some(token) => {
            let id = chatgpt_account_id(token)?;
            session
                .extra
                .insert("account_id".to_owned(), Value::String(id));
            let (email, plan) = profile_claims(token);
            if !email.is_empty() {
                session.account = email;
            }
            if !plan.is_empty() {
                session.plan_type = plan;
            }
            session
                .extra
                .insert("id_token".to_owned(), Value::String(token.clone()));
        }
        None if fallback.is_some() => {}
        None => return Err(missing_account()),
    }
    Ok(session)
}

pub(super) fn token_set_from_value(value: &Value) -> Result<TokenSet, Error> {
    let obj = value.as_object().ok_or_else(|| {
        Error::Payload("codex token endpoint returned no access token".to_owned())
    })?;
    let access_token = obj
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::Payload("codex token endpoint returned no access token".to_owned()))?
        .to_owned();
    Ok(TokenSet {
        access_token,
        refresh_token: obj
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        id_token: obj
            .get("id_token")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        expires_in: positive_i64(obj.get("expires_in")),
    })
}

async fn exchange_code_at(
    url: &str,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<Session, Error> {
    if verifier.trim().is_empty() {
        return Err(Error::MissingVerifier);
    }
    let body = crate::form::encode(&[
        ("grant_type", "authorization_code".to_owned()),
        ("code", code.to_owned()),
        ("redirect_uri", redirect_uri.to_owned()),
        ("client_id", CLIENT_ID.to_owned()),
        ("code_verifier", verifier.to_owned()),
    ]);
    let value = post_token(url, "application/x-www-form-urlencoded", body, false).await?;
    session_from_tokens(token_set_from_value(&value)?, None)
}

async fn refresh_at(url: &str, session: &Session) -> Result<Session, Error> {
    let body = serde_json::to_string(&Value::Object(Map::from_iter([
        ("client_id".to_owned(), Value::String(CLIENT_ID.to_owned())),
        (
            "grant_type".to_owned(),
            Value::String("refresh_token".to_owned()),
        ),
        (
            "refresh_token".to_owned(),
            Value::String(session.refresh_token.clone()),
        ),
    ])))
    .map_err(|err| Error::Payload(err.to_string()))?;
    let value = post_token(url, "application/json", body, true).await?;
    session_from_tokens(token_set_from_value(&value)?, Some(session))
}

async fn post_token(
    url: &str,
    content_type: &str,
    body: String,
    refresh: bool,
) -> Result<Value, Error> {
    let response = http_client()
        .post(url)
        .timeout(Duration::from_secs(30))
        .header("content-type", content_type)
        .header("originator", ORIGINATOR)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/json")
        .body(body)
        .send()
        .await?;
    let status = response.status().as_u16();
    let text = response.text().await?;
    if !(200..300).contains(&status) {
        return Err(token_error(status, &text, refresh));
    }
    serde_json::from_str(&text).map_err(|err| Error::Payload(err.to_string()))
}

fn token_error(status: u16, body: &str, refresh: bool) -> Error {
    if refresh && is_permanent_refresh_code(oauth_code(body).as_deref()) {
        return Error::Permanent;
    }
    Error::TokenEndpoint {
        status,
        body: body.to_owned(),
    }
}

fn is_permanent_refresh_code(code: Option<&str>) -> bool {
    code.is_some_and(|code| PERMANENT_REFRESH_CODES.contains(&code))
}

fn oauth_code(body: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    parsed
        .get("error")
        .or_else(|| parsed.get("error_code"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn chatgpt_account_id(id_token: &str) -> Result<String, Error> {
    let claims = crate::jwt::decode_payload(id_token).ok_or_else(missing_account)?;
    let auth = claims
        .get("https://api.openai.com/auth")
        .and_then(Value::as_object);
    auth.and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(missing_account)
}

fn missing_account() -> Error {
    Error::Payload(
        "codex login did not return a chatgpt account id; cannot use the subscription".to_owned(),
    )
}

fn profile_claims(id_token: &str) -> (String, String) {
    let Some(claims) = crate::jwt::decode_payload(id_token) else {
        return (String::new(), String::new());
    };
    let profile_email = claims
        .get("https://api.openai.com/profile")
        .and_then(Value::as_object)
        .and_then(|profile| profile.get("email"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let email = claims
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or(profile_email)
        .trim()
        .to_owned();
    let plan = claims
        .get("https://api.openai.com/auth")
        .and_then(Value::as_object)
        .and_then(|auth| auth.get("chatgpt_plan_type"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    (email, plan)
}

fn jwt_exp_ms(access_token: &str) -> Option<i64> {
    let claims = crate::jwt::decode_payload(access_token)?;
    let exp = claims.get("exp")?;
    let seconds = exp
        .as_i64()
        .or_else(|| exp.as_f64().map(|n| n.round() as i64))?;
    if seconds > 0 {
        Some(seconds.saturating_mul(1000))
    } else {
        None
    }
}

fn account_id(session: &Session) -> &str {
    session
        .extra
        .get("account_id")
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn positive_i64(value: Option<&Value>) -> Option<i64> {
    let n = match value? {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse().ok()?,
        _ => return None,
    };
    if n.is_finite() && n > 0.0 {
        Some(n.round() as i64)
    } else {
        None
    }
}

fn insert_str(headers: &mut HeaderMap, name: &str, value: &str) {
    let Ok(header) = HeaderName::from_bytes(name.as_bytes()) else {
        return;
    };
    let Ok(header_value) = HeaderValue::from_str(value) else {
        return;
    };
    headers.insert(header, header_value);
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
