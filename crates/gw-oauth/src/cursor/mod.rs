//! Cursor subscription (Connect/protobuf AgentService/Run).

pub mod cache;
pub mod catalog;
pub mod h2;
pub mod import;
pub mod proto;
pub mod refresh_guard;
pub mod request;

pub use cache::{CURSOR_STABLE_SESSION, apply_cache, cache_session_id, conversation_id};
pub use request::openai_to_cursor;

use std::future::Future;

use http::{HeaderMap, HeaderName, HeaderValue};
use rand::RngCore as _;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{Error, Family, FlowKind, Session, StartOutcome};

#[cfg(test)]
mod tests;

pub const ID: &str = "cursor";
pub const LOGIN_URL: &str = "https://cursor.com/loginDeepControl";
pub const POLL_URL: &str = "https://api2.cursor.sh/auth/poll";
pub const REFRESH_URL: &str = "https://api2.cursor.sh/auth/exchange_user_api_key";
pub const AGENT_URL: &str = "https://agentn.us.api5.cursor.sh";
pub const API2_URL: &str = "https://api2.cursor.sh";
pub const USAGE_PATH: &str = "/aiserver.v1.DashboardService/GetCurrentPeriodUsage";
pub const STRIPE_PROFILE_PATH: &str = "/auth/full_stripe_profile";
pub const GET_EMAIL_PATH: &str = "/aiserver.v1.AuthService/GetEmail";
pub const GET_ME_PATH: &str = "/aiserver.v1.DashboardService/GetMe";
pub const RUN_PATH: &str = "/agent.v1.AgentService/Run";
pub const MODELS_PATH: &str = "/agent.v1.AgentService/GetUsableModels";
pub const AVAILABLE_MODELS_PATH: &str = "/aiserver.v1.AiService/AvailableModels";
pub const CLIENT_VERSION: &str = "cli-2026.07.23-e383d2b";
pub const CLIENT_TYPE: &str = "cli";
pub const PREEMPT_MS: i64 = 5 * 60_000;
pub const POLL_MAX_ATTEMPTS: u32 = 150;
pub const POLL_BASE_DELAY_MS: u64 = 1000;
pub const POLL_MAX_DELAY_MS: u64 = 10_000;
pub const POLL_BACKOFF: f64 = 1.2;

pub const SOURCES: &[&str] = &["pkce", "cli_keychain", "ide_vscdb", "env", "import"];

/// Picker reasoning keys → Cursor `RequestedModel.parameters` values.
pub const REASONING: &[(&str, &str)] = &[
    ("off", "none"),
    ("low", "low"),
    ("medium", "medium"),
    ("high", "high"),
    ("xhigh", "extra-high"),
];

/// PKCE pair plus loginDeepControl URL. No loopback callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginParams {
    pub verifier: String,
    pub challenge: String,
    pub uuid: String,
    pub login_url: String,
}

/// Access + refresh as returned by poll / refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorTokens {
    pub access_token: String,
    pub refresh_token: String,
}

/// Refresh result with JWT-derived expiry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshedTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: i64,
}

/// Fields collected before [`cursor_session`].
#[derive(Debug, Clone, Default)]
pub struct SessionBuild {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at_ms: Option<i64>,
    pub account: Option<String>,
    pub plan_type: Option<String>,
    pub cached_email: Option<String>,
    pub source: String,
}

fn trimmed(value: &str) -> Option<String> {
    let t = value.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_owned())
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn env_trimmed(key: &str) -> Option<String> {
    std::env::var(key).ok().and_then(|s| trimmed(&s))
}

/// CLI fingerprint. `PI_CURSOR_CLIENT_VERSION` may override.
#[must_use]
pub fn client_version() -> String {
    env_trimmed("PI_CURSOR_CLIENT_VERSION").unwrap_or_else(|| CLIENT_VERSION.to_owned())
}

/// AgentService origin. `PI_CURSOR_AGENT_URL` / `CURSOR_AGENT_URL` may override.
#[must_use]
pub fn agent_url() -> String {
    env_trimmed("PI_CURSOR_AGENT_URL")
        .or_else(|| env_trimmed("CURSOR_AGENT_URL"))
        .unwrap_or_else(|| AGENT_URL.to_owned())
}

/// JWT `exp` minus five minutes, or one hour from `now_ms` when the token has no `exp`.
#[must_use]
pub fn token_expiry_ms(token: &str, now_ms: i64) -> i64 {
    if let Some(claims) = crate::jwt::decode_payload(token)
        && let Some(exp) = claims
            .get("exp")
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
    {
        return exp.saturating_mul(1000).saturating_sub(PREEMPT_MS);
    }
    now_ms.saturating_add(60 * 60_000)
}

/// JWT `sub` / WorkOS / Auth0 / the literal `cursor` — vault keys only, never a card title.
#[must_use]
pub fn is_opaque_account(value: &str) -> bool {
    let raw = value.trim();
    if raw.is_empty() {
        return true;
    }
    if raw.eq_ignore_ascii_case("cursor") {
        return true;
    }
    let lower = raw.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("cursor-")
        && rest.len() >= 4
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return true;
    }
    if !raw.contains('@') {
        let mut parts = raw.split('|');
        if let (Some(a), Some(b), None) = (parts.next(), parts.next(), parts.next())
            && !a.is_empty()
            && !b.is_empty()
            && a.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            && b.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return true;
        }
    }
    if let Some(rest) = lower.strip_prefix("user_")
        && rest.len() >= 16
        && rest.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return true;
    }
    false
}

/// First non-opaque human identifier among `candidates`.
#[must_use]
pub fn pick_human_account<'a>(
    candidates: impl IntoIterator<Item = Option<&'a str>>,
) -> Option<String> {
    for value in candidates {
        if let Some(next) = value.and_then(trimmed)
            && !is_opaque_account(&next)
        {
            return Some(next);
        }
    }
    None
}

/// JWT `email` / `preferred_username` — not `sub`.
#[must_use]
pub fn account_from_token(token: &str) -> Option<String> {
    let claims = crate::jwt::decode_payload(token)?;
    let email = claims.get("email").and_then(Value::as_str);
    let preferred = claims.get("preferred_username").and_then(Value::as_str);
    pick_human_account([email, preferred])
}

fn vault_account_from_token(token: &str) -> Option<String> {
    crate::jwt::decode_payload(token)
        .and_then(|claims| claims.get("sub").and_then(Value::as_str).and_then(trimmed))
}

/// Card title: session account / cachedEmail / JWT email. Never `sub`.
#[must_use]
pub fn display_account(session: &Session) -> Option<String> {
    let cached = session.extra.get("cachedEmail").and_then(Value::as_str);
    pick_human_account([
        Some(session.account.as_str()),
        cached,
        account_from_token(&session.access_token).as_deref(),
    ])
}

/// GetEmail `{ email }` or GetMe `{ email, firstName, lastName }`. Email wins.
#[must_use]
pub fn name_from_profile(value: &Value) -> Option<String> {
    let obj = value.as_object()?;
    if let Some(email) = pick_human_account([obj.get("email").and_then(Value::as_str)]) {
        return Some(email);
    }
    let first = obj
        .get("firstName")
        .and_then(Value::as_str)
        .and_then(trimmed);
    let last = obj
        .get("lastName")
        .and_then(Value::as_str)
        .and_then(trimmed);
    let combined = [first, last]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    pick_human_account([Some(combined.as_str())])
}

#[must_use]
pub fn access_still_valid(token: &str, now_ms: i64) -> bool {
    trimmed(token).is_some() && now_ms < token_expiry_ms(token, now_ms)
}

/// 96-byte S256 PKCE pair (Cursor CLI, not the crate-wide 32-byte helper).
///
/// # Errors
/// When the OS entropy source fails.
pub fn create_cursor_pkce() -> Result<(String, String), Error> {
    let mut raw = [0u8; 96];
    rand::rngs::OsRng
        .try_fill_bytes(&mut raw)
        .map_err(|_| Error::Entropy)?;
    let verifier = base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, raw);
    let challenge = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        Sha256::digest(verifier.as_bytes()),
    );
    Ok((verifier, challenge))
}

fn query_escape(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// loginDeepControl URL with `mode=login` and `redirectTarget=cli`.
///
/// # Errors
/// Entropy failure while minting PKCE / uuid.
pub fn login_params() -> Result<LoginParams, Error> {
    let (verifier, challenge) = create_cursor_pkce()?;
    let id = uuid::Uuid::new_v4().to_string();
    let login_url = format!(
        "{LOGIN_URL}?challenge={}&uuid={}&mode=login&redirectTarget=cli",
        query_escape(&challenge),
        query_escape(&id)
    );
    Ok(LoginParams {
        verifier,
        challenge,
        uuid: id,
        login_url,
    })
}

/// Parse poll / refresh JSON. Accepts `accessToken`/`access` and `refreshToken`/`refresh`.
///
/// # Errors
/// Missing access token or a non-object body.
pub fn parse_token_response(value: &Value, endpoint: &str) -> Result<CursorTokens, Error> {
    let obj = value
        .as_object()
        .ok_or_else(|| Error::Payload(format!("{endpoint} returned an invalid token response")))?;
    let access = obj
        .get("accessToken")
        .or_else(|| obj.get("access"))
        .and_then(Value::as_str)
        .and_then(trimmed)
        .ok_or_else(|| Error::Payload(format!("{endpoint} returned no access token")))?;
    let refresh = obj
        .get("refreshToken")
        .or_else(|| obj.get("refresh"))
        .and_then(Value::as_str)
        .and_then(trimmed)
        .unwrap_or_default();
    Ok(CursorTokens {
        access_token: access,
        refresh_token: refresh,
    })
}

/// Build a stored Cursor session.
///
/// # Errors
/// Missing access token.
pub fn cursor_session(build: SessionBuild) -> Result<Session, Error> {
    let access = trimmed(&build.access_token)
        .ok_or_else(|| Error::Payload("cursor session needs an access token".into()))?;
    let refresh = build
        .refresh_token
        .as_deref()
        .and_then(trimmed)
        .unwrap_or_else(|| access.clone());
    let expires_at_ms = build
        .expires_at_ms
        .filter(|n| *n != 0)
        .unwrap_or_else(|| token_expiry_ms(&access, now_ms()));
    let cached = build
        .cached_email
        .as_deref()
        .and_then(|s| pick_human_account([Some(s)]));
    let human = pick_human_account([
        build.account.as_deref(),
        account_from_token(&access).as_deref(),
    ]);
    let vault = human
        .or_else(|| {
            build
                .account
                .as_deref()
                .and_then(trimmed)
                .filter(|s| !s.eq_ignore_ascii_case("cursor"))
        })
        .or_else(|| vault_account_from_token(&access));
    let source = if SOURCES.contains(&build.source.as_str()) {
        build.source
    } else {
        "pkce".to_owned()
    };
    let mut session = Session::new(Family::Cursor, access, refresh);
    session.expires_at_ms = expires_at_ms;
    if let Some(account) = vault {
        session.account = account;
    }
    session.source = source;
    if let Some(plan) = build.plan_type.as_deref().and_then(trimmed) {
        session.plan_type = plan;
    }
    if let Some(email) = cached {
        session
            .extra
            .insert("cachedEmail".to_owned(), Value::String(email));
    }
    Ok(session)
}

fn insert_header(headers: &mut HeaderMap, name: &str, value: &str) {
    if let (Ok(name), Ok(value)) = (
        HeaderName::from_bytes(name.as_bytes()),
        HeaderValue::from_str(value),
    ) {
        headers.insert(name, value);
    }
}

/// AgentService headers. `x-cursor-client-type` is `cli`, never `sdk`.
#[must_use]
pub fn chat_headers(access_token: &str, request_id: Option<&str>, unary: bool) -> HeaderMap {
    let id = request_id
        .and_then(trimmed)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut headers = HeaderMap::new();
    insert_header(
        &mut headers,
        "authorization",
        &format!("Bearer {access_token}"),
    );
    insert_header(&mut headers, "connect-protocol-version", "1");
    insert_header(
        &mut headers,
        "content-type",
        if unary {
            "application/proto"
        } else {
            "application/connect+proto"
        },
    );
    insert_header(&mut headers, "te", "trailers");
    insert_header(&mut headers, "x-ghost-mode", "true");
    insert_header(&mut headers, "x-cursor-client-version", &client_version());
    insert_header(&mut headers, "x-cursor-client-type", CLIENT_TYPE);
    insert_header(&mut headers, "x-request-id", &id);
    insert_header(&mut headers, "x-original-request-id", &id);
    headers
}

/// JSON usage / stripe / GetEmail headers. Still `cli`.
#[must_use]
pub fn usage_headers(access_token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    insert_header(
        &mut headers,
        "authorization",
        &format!("Bearer {access_token}"),
    );
    insert_header(&mut headers, "content-type", "application/json");
    insert_header(&mut headers, "x-cursor-client-version", &client_version());
    insert_header(&mut headers, "x-cursor-client-type", CLIENT_TYPE);
    headers
}

/// Wire model id: peel host-side `-fast`. Empty → `composer-2`.
#[must_use]
pub fn wire_model_id(model: &str) -> String {
    let raw = trimmed(model).unwrap_or_else(|| "composer-2".to_owned());
    let (peeled, _) = cache::peel_fast_suffix(&raw);
    if peeled.is_empty() {
        "composer-2".to_owned()
    } else {
        peeled
    }
}

#[must_use]
pub fn source_label(source: &str) -> Option<&'static str> {
    match source.trim() {
        "cli_keychain" => Some("CLI"),
        "ide_vscdb" => Some("IDE"),
        "env" => Some("env"),
        "pkce" => Some("PKCE"),
        "import" => Some("import"),
        _ => None,
    }
}

/// Next poll delay: `min(delay * 1.2, 10s)`.
#[must_use]
pub fn next_poll_delay_ms(delay: u64) -> u64 {
    let next = (delay as f64 * POLL_BACKOFF).floor() as u64;
    next.clamp(1, POLL_MAX_DELAY_MS)
}

#[must_use]
pub fn poll_query_url(uuid: &str, verifier: &str) -> String {
    format!(
        "{POLL_URL}?uuid={}&verifier={}",
        query_escape(uuid),
        query_escape(verifier)
    )
}

/// One poll GET. `None` means 404 still waiting.
///
/// # Errors
/// Non-success status (other than 404), missing tokens, or malformed JSON.
pub fn apply_poll_response(status: u16, body: &[u8]) -> Result<Option<CursorTokens>, Error> {
    if status == 404 {
        return Ok(None);
    }
    if !(200..300).contains(&status) {
        return Err(Error::TokenEndpoint {
            status,
            body: String::from_utf8_lossy(body).into_owned(),
        });
    }
    let value: Value = serde_json::from_slice(body)
        .map_err(|err| Error::Payload(format!("Cursor authentication polling: {err}")))?;
    let tokens = parse_token_response(&value, "Cursor authentication polling")?;
    if tokens.refresh_token.is_empty() {
        return Err(Error::Payload(
            "Cursor authentication polling returned no refresh token".into(),
        ));
    }
    Ok(Some(tokens))
}

/// Poll `auth/poll` until tokens arrive. `sleep` is the backoff between attempts.
///
/// # Errors
/// Timeout, three consecutive transport failures, or a malformed token body.
pub async fn poll_auth<F, Fut, S, SFut>(
    uuid: &str,
    verifier: &str,
    mut fetch: F,
    mut sleep: S,
    max_attempts: u32,
) -> Result<CursorTokens, Error>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<(u16, Vec<u8>), Error>>,
    S: FnMut(u64) -> SFut,
    SFut: Future<Output = ()>,
{
    let mut delay = POLL_BASE_DELAY_MS;
    let mut consecutive_errors = 0u32;
    for _ in 0..max_attempts {
        sleep(delay).await;
        let url = poll_query_url(uuid, verifier);
        match fetch(url).await {
            Ok((status, body)) => match apply_poll_response(status, &body) {
                Ok(Some(tokens)) => return Ok(tokens),
                Ok(None) => {
                    consecutive_errors = 0;
                    delay = next_poll_delay_ms(delay);
                }
                Err(err) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= 3 {
                        return Err(err);
                    }
                }
            },
            Err(err) => {
                consecutive_errors += 1;
                if consecutive_errors >= 3 {
                    return Err(err);
                }
            }
        }
    }
    Err(Error::Payload(
        "Cursor authentication polling timeout".into(),
    ))
}

/// Interpret `exchange_user_api_key` status + body. Marks the refresh-guard.
///
/// # Errors
/// Known-bad token, HTTP failure, or missing access token.
pub fn accept_refresh_body(
    used_refresh: &str,
    status: u16,
    body: &[u8],
    now_ms: i64,
) -> Result<RefreshedTokens, Error> {
    if !(200..300).contains(&status) {
        refresh_guard::mark_failed(used_refresh, now_ms);
        if status == 401 || status == 403 {
            return Err(Error::Permanent);
        }
        return Err(Error::TokenEndpoint {
            status,
            body: String::from_utf8_lossy(body).into_owned(),
        });
    }
    let value: Value = serde_json::from_slice(body)
        .map_err(|err| Error::Payload(format!("Cursor token refresh: {err}")))?;
    let parsed = parse_token_response(&value, "Cursor token refresh")?;
    refresh_guard::mark_succeeded(used_refresh);
    let refresh = if parsed.refresh_token.is_empty() {
        used_refresh.to_owned()
    } else {
        parsed.refresh_token
    };
    Ok(RefreshedTokens {
        expires_at_ms: token_expiry_ms(&parsed.access_token, now_ms),
        access_token: parsed.access_token,
        refresh_token: refresh,
    })
}

/// POST `auth/exchange_user_api_key` with `Authorization: Bearer <refresh>` and `{}`.
///
/// # Errors
/// Known-bad refresh, HTTP failure, or malformed body.
pub async fn refresh_tokens<F, Fut>(
    refresh_token: &str,
    fetch: F,
    now_ms: i64,
) -> Result<RefreshedTokens, Error>
where
    F: FnOnce(String, HeaderMap, Vec<u8>) -> Fut,
    Fut: Future<Output = Result<(u16, Vec<u8>), Error>>,
{
    let token = trimmed(refresh_token)
        .ok_or_else(|| Error::Payload("cursor refresh needs a refresh token".into()))?;
    if refresh_guard::is_known_bad(&token, now_ms) {
        return Err(Error::Permanent);
    }
    let mut headers = HeaderMap::new();
    insert_header(&mut headers, "authorization", &format!("Bearer {token}"));
    insert_header(&mut headers, "content-type", "application/json");
    let (status, body) = fetch(REFRESH_URL.to_owned(), headers, b"{}".to_vec()).await?;
    accept_refresh_body(&token, status, &body, now_ms)
}

/// Refresh a stored session. Env tokens never hit the network.
///
/// # Errors
/// Permanent when the env token is expired, or refresh fails 401/403 / known-bad.
pub async fn refresh_session<F, Fut>(
    session: &Session,
    fetch: F,
    now_ms: i64,
) -> Result<Session, Error>
where
    F: FnOnce(String, HeaderMap, Vec<u8>) -> Fut,
    Fut: Future<Output = Result<(u16, Vec<u8>), Error>>,
{
    if session.source == "env" || session.refresh_token == session.access_token {
        if access_still_valid(&session.access_token, now_ms) {
            return Ok(session.clone());
        }
        return Err(Error::Permanent);
    }
    let tokens = refresh_tokens(&session.refresh_token, fetch, now_ms).await?;
    cursor_session(SessionBuild {
        access_token: tokens.access_token,
        refresh_token: Some(tokens.refresh_token),
        expires_at_ms: Some(tokens.expires_at_ms),
        account: trimmed(&session.account),
        plan_type: trimmed(&session.plan_type),
        cached_email: session
            .extra
            .get("cachedEmail")
            .and_then(Value::as_str)
            .and_then(trimmed),
        source: session.source.clone(),
    })
}

/// PKCE poll (`loginDeepControl`, no loopback) or `method=import` token JSON.
///
/// # Errors
/// Entropy, unsupported method, or unparseable import JSON.
pub async fn start(input: &crate::login::StartInput) -> Result<StartOutcome, Error> {
    let method = {
        let from_input = input.method.trim().to_ascii_lowercase();
        if from_input.is_empty() {
            input
                .body
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        } else {
            from_input
        }
    };
    if method == "import" {
        let session = import::session_from_json(&input.body)?;
        return Ok(StartOutcome::Ready(session));
    }
    if !method.is_empty() && method != "pkce" && method != "poll" && method != "clipoll" {
        return Err(Error::UnsupportedFlow);
    }
    let started = login_params()?;
    let mut extra = Map::new();
    extra.insert("uuid".to_owned(), Value::String(started.uuid.clone()));
    extra.insert(
        "challenge".to_owned(),
        Value::String(started.challenge.clone()),
    );
    extra.insert("poll_url".to_owned(), Value::String(POLL_URL.to_owned()));
    extra.insert("mode".to_owned(), Value::String("cli".to_owned()));
    Ok(StartOutcome::Browser {
        authorize_url: started.login_url,
        state: input.state.clone(),
        verifier: started.verifier,
        redirect_uri: String::new(),
        flow: FlowKind::CliPoll,
        extra,
    })
}

/// Complete a successful poll with `source=pkce`.
///
/// # Errors
/// Missing access token.
pub fn complete_login(tokens: CursorTokens, source: &str) -> Result<Session, Error> {
    cursor_session(SessionBuild {
        access_token: tokens.access_token,
        refresh_token: Some(tokens.refresh_token),
        expires_at_ms: None,
        account: None,
        plan_type: None,
        cached_email: None,
        source: if source.is_empty() {
            "pkce".to_owned()
        } else {
            source.to_owned()
        },
    })
}
