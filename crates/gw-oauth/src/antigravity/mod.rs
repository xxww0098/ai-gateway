//! Google Antigravity (hub / daily-cloudcode-pa).
//!
//! Official desktop to mimic: Antigravity.app hub (`--subclient_type hub`).
//! Ignore Antigravity IDE.app. Chat / loadCodeAssist send User-Agent only —
//! no `Client-Metadata` / `x-goog-api-client`. onboardUser keeps the longer
//! UA plus `x-goog-api-client`.

pub mod cache;
pub mod request;

pub use cache::{ANTIGRAVITY_STABLE_SESSION, apply_cache, cache_session_id, session_id_of};
pub use request::{
    antigravity_to_openai, cached_tokens_of, chat_headers, cloud_code_fallbacks, fetch_cloud_code,
    map_antigravity_usage, openai_to_antigravity, should_retry_on_prod,
};

use std::sync::OnceLock;
use std::time::Duration;

use chrono::Utc;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use crate::{Error, Family, FlowKind, Session, StartOutcome};

#[cfg(test)]
mod tests;

pub const ID: &str = "antigravity";
/// Public Google installed-app client (CLIProxyAPI `constants.go`, not a private secret).
pub const CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
pub const CLIENT_SECRET: &str = "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";
pub const CALLBACK_PORT: u16 = 51121;
pub const CALLBACK_PATH: &str = "/oauth-callback";
pub const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo?alt=json";
/// Hub default — Antigravity.app `--cloud_code_endpoint`.
pub const DAILY_API_URL: &str = "https://daily-cloudcode-pa.googleapis.com";
/// IDE / prod Cloud Code. Only used if daily fails with 5xx / transport.
pub const PROD_API_URL: &str = "https://cloudcode-pa.googleapis.com";
pub const API_URL: &str = DAILY_API_URL;
pub const API_VERSION: &str = "v1internal";
/// Cloud Code still rejects clients below 2.9.0.
pub const FALLBACK_VERSION: &str = "2.11.0";
pub const MAC_APP_PLIST: &str = "/Applications/Antigravity.app/Contents/Info.plist";
pub const NODE_API_CLIENT_UA: &str = "google-api-nodejs-client/10.3.0";
pub const GOOG_API_CLIENT_UA: &str = "gl-node/22.21.1";
pub const BODY_USER_AGENT: &str = "antigravity";
pub const VERIFY_CODE: &str = "VALIDATION_REQUIRED";
pub const VERIFY_MESSAGE: &str = "Google 需要验证此账号才能对话";
pub const SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";

const ONBOARD_ATTEMPTS: u32 = 5;
const ONBOARD_PAUSE: Duration = Duration::from_millis(2_000);

/// Hosts used by login / refresh. Production unless a test substitutes loopback.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub token: String,
    pub userinfo: String,
    pub daily: String,
    pub prod: String,
}

impl Endpoints {
    /// Google production hosts.
    #[must_use]
    pub fn production() -> Self {
        Self {
            token: TOKEN_URL.to_owned(),
            userinfo: USERINFO_URL.to_owned(),
            daily: DAILY_API_URL.to_owned(),
            prod: PROD_API_URL.to_owned(),
        }
    }

    fn load_code_assist(&self) -> String {
        format!("{}/{API_VERSION}:loadCodeAssist", self.daily)
    }

    fn onboard_user(&self) -> String {
        format!("{}/{API_VERSION}:onboardUser", self.daily)
    }
}

/// OS/arch token used in the hub User-Agent (`darwin|windows|linux` / `arm64|amd64`).
#[must_use]
pub fn platform() -> String {
    let os = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    let cpu = if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "amd64"
    };
    format!("{os}/{cpu}")
}

/// Hub version: installed Antigravity.app when readable, else [`FALLBACK_VERSION`].
#[must_use]
pub fn version() -> &'static str {
    static CACHED: OnceLock<String> = OnceLock::new();
    CACHED.get_or_init(detect_version).as_str()
}

/// Short runtime UA — token, userinfo, loadCodeAssist, chat.
#[must_use]
pub fn request_user_agent() -> String {
    format!("antigravity/hub/{} {}", version(), platform())
}

/// Long control-plane UA — onboardUser only.
#[must_use]
pub fn onboard_user_agent() -> String {
    format!("{} {NODE_API_CLIENT_UA}", request_user_agent())
}

/// Normalize FileVersion `2.11.0.0` → `2.11.0`; keep a real fourth component.
#[must_use]
pub fn normalize_version(value: &str) -> Option<String> {
    let raw = value.trim();
    if !is_version_token(raw) {
        return None;
    }
    let parts: Vec<&str> = raw.split('.').collect();
    if parts.len() == 4 && parts[3] == "0" {
        Some(parts[..3].join("."))
    } else {
        Some(raw.to_owned())
    }
}

/// SkillStar-style `CFBundleShortVersionString` extract. Callers pass Antigravity.app only.
#[must_use]
pub fn parse_plist_version(plist_xml: &str) -> Option<String> {
    if plist_xml.is_empty() {
        return None;
    }
    let xml = plist_xml.replace("\r\n", "\n");
    let tagged = capture_plist_tagged(&xml);
    if tagged.is_some() {
        return tagged;
    }
    let mut pending = false;
    for line in xml.lines() {
        let trimmed = line.trim();
        if trimmed == "<key>CFBundleShortVersionString</key>" {
            pending = true;
            continue;
        }
        if pending && trimmed.starts_with("<string>") && trimmed.ends_with("</string>") {
            let inner = &trimmed["<string>".len()..trimmed.len() - "</string>".len()];
            return normalize_version(inner);
        }
        if pending && !trimmed.is_empty() {
            pending = false;
        }
    }
    None
}

/// First `X.Y` / `X.Y.Z` / `X.Y.Z.W` token in CLI or PowerShell output.
#[must_use]
pub fn parse_version_text(text: &str) -> Option<String> {
    let mut start = None;
    let mut dots = 0u8;
    for (i, ch) in text.char_indices() {
        if ch.is_ascii_digit() {
            if start.is_none() {
                start = Some(i);
                dots = 0;
            }
        } else if ch == '.' {
            if start.is_some() {
                dots += 1;
                if dots > 3 {
                    if let Some(s) = start
                        && let Some(v) = normalize_version(&text[s..i])
                    {
                        return Some(v);
                    }
                    start = None;
                    dots = 0;
                }
            }
        } else if let Some(s) = start
            && let Some(v) = normalize_version(&text[s..i])
        {
            return Some(v);
        } else if start.is_some() {
            start = None;
            dots = 0;
        }
    }
    start.and_then(|s| normalize_version(&text[s..]))
}

/// Google installed-app authorize URL. No PKCE — Google's installed-app client
/// uses `client_secret` on the token hop instead.
pub async fn start(input: &crate::login::StartInput) -> Result<StartOutcome, Error> {
    let params = [
        ("access_type", "offline".to_owned()),
        ("client_id", CLIENT_ID.to_owned()),
        ("prompt", "consent".to_owned()),
        ("redirect_uri", input.redirect_uri.clone()),
        ("response_type", "code".to_owned()),
        ("scope", SCOPE.to_owned()),
        ("state", input.state.clone()),
    ];
    let authorize_url = format!("{AUTHORIZE_URL}?{}", crate::form::encode(&params));
    Ok(StartOutcome::Browser {
        authorize_url,
        state: input.state.clone(),
        verifier: String::new(),
        redirect_uri: input.redirect_uri.clone(),
        flow: FlowKind::AuthorizationCode,
        extra: Map::new(),
    })
}

/// Exchange an authorization code. Discovers `cloudaicompanionProject` via
/// loadCodeAssist (daily, then prod on 5xx) and onboardUser (daily-only).
pub async fn exchange(
    client: &reqwest::Client,
    code: &str,
    redirect_uri: &str,
) -> Result<Session, Error> {
    exchange_with(client, &Endpoints::production(), code, redirect_uri).await
}

/// [`exchange`] against caller-chosen hosts (tests point at loopback).
pub async fn exchange_with(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    code: &str,
    redirect_uri: &str,
) -> Result<Session, Error> {
    let body = crate::form::encode(&[
        ("code", code.to_owned()),
        ("client_id", CLIENT_ID.to_owned()),
        ("client_secret", CLIENT_SECRET.to_owned()),
        ("redirect_uri", redirect_uri.to_owned()),
        ("grant_type", "authorization_code".to_owned()),
    ]);
    let tokens = post_token(client, &endpoints.token, body).await?;
    complete_login(client, endpoints, &tokens, None).await
}

/// Refresh the access token. `invalid_grant` / `invalid_client` /
/// `unauthorized_client` are [`Error::Permanent`]. `VALIDATION_REQUIRED` is not.
pub async fn refresh(client: &reqwest::Client, session: &Session) -> Result<Session, Error> {
    refresh_with(client, &Endpoints::production(), session).await
}

/// [`refresh`] against caller-chosen hosts.
pub async fn refresh_with(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    session: &Session,
) -> Result<Session, Error> {
    let body = crate::form::encode(&[
        ("client_id", CLIENT_ID.to_owned()),
        ("client_secret", CLIENT_SECRET.to_owned()),
        ("grant_type", "refresh_token".to_owned()),
        ("refresh_token", session.refresh_token.clone()),
    ]);
    let tokens = post_token(client, &endpoints.token, body).await?;
    let access = token_str(&tokens, "access_token")
        .or_else(|| token_str(&tokens, "accessToken"))
        .ok_or_else(|| Error::Payload("antigravity refresh returned no access token".into()))?;
    let refresh_token = token_str(&tokens, "refresh_token")
        .or_else(|| token_str(&tokens, "refreshToken"))
        .unwrap_or_else(|| session.refresh_token.clone());
    let project_id = match project_id_of(session) {
        Some(id) => id,
        None => fetch_project(client, endpoints, &access).await?.project_id,
    };
    build_session(SessionParts {
        access_token: access,
        refresh_token,
        expires_at_ms: expires_at_ms(&tokens),
        account: session.account.clone(),
        project_id,
        plan_type: if session.plan_type.is_empty() {
            None
        } else {
            Some(session.plan_type.clone())
        },
        needs_validation: session
            .extra
            .get("needs_validation")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        validation_url: session
            .extra
            .get("validation_url")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// Detect Google Cloud Code `VALIDATION_REQUIRED` / "Verify your account".
#[must_use]
pub fn parse_validation(payload: &Value) -> Option<Validation> {
    if payload.is_null() {
        return None;
    }
    let text = blob_text(payload);
    let required = contains_ignore_ascii(&text, VERIFY_CODE)
        || contains_ignore_ascii(&text, "verify your account");
    if !required {
        return None;
    }
    Some(Validation {
        required: true,
        validation_url: walk_validation_url(payload, 0),
        message: VERIFY_MESSAGE.to_owned(),
        code: VERIFY_CODE.to_owned(),
    })
}

/// `invalid_grant` / `invalid_client` / `unauthorized_client` drop the session.
/// `VALIDATION_REQUIRED` does not — the operator must verify the Google account.
#[must_use]
pub fn is_permanent_refresh_error(payload: &Value) -> bool {
    if parse_validation(payload).is_some() {
        return false;
    }
    let code = error_code(payload);
    if code.eq_ignore_ascii_case(VERIFY_CODE) {
        return false;
    }
    matches!(
        code,
        "invalid_grant" | "invalid_client" | "unauthorized_client"
    )
}

/// Classify a token-endpoint body. Non-JSON text is treated as `{error: text}`.
#[must_use]
pub fn is_permanent_refresh_error_text(text: &str) -> bool {
    let payload: Value = serde_json::from_str(text).unwrap_or_else(|_| json!({"error": text}));
    is_permanent_refresh_error(&payload)
}

/// Stamp or clear the verify-account flag on a stored session.
#[must_use]
pub fn apply_validation(session: Session, info: Option<&Validation>) -> Session {
    let mut next = session;
    match info {
        Some(info) if info.required => {
            next.extra
                .insert("needs_validation".into(), Value::Bool(true));
            if let Some(url) = info
                .validation_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                next.extra
                    .insert("validation_url".into(), Value::String(url.to_owned()));
            }
        }
        _ => {
            next.extra.remove("needs_validation");
            next.extra.remove("validation_url");
        }
    }
    next
}

/// Account-verification payload parsed from Cloud Code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validation {
    pub required: bool,
    pub validation_url: Option<String>,
    pub message: String,
    pub code: String,
}

struct SessionParts {
    access_token: String,
    refresh_token: String,
    expires_at_ms: i64,
    account: String,
    project_id: String,
    plan_type: Option<String>,
    needs_validation: bool,
    validation_url: Option<String>,
}

struct DiscoveredProject {
    project_id: String,
    plan_type: Option<String>,
}

fn detect_version() -> String {
    if cfg!(target_os = "macos") {
        if let Ok(xml) = std::fs::read_to_string(MAC_APP_PLIST)
            && let Some(v) = parse_plist_version(&xml)
        {
            return v;
        }
        if let Some(v) = run_version_command(
            "plutil",
            &[
                "-extract",
                "CFBundleShortVersionString",
                "raw",
                "-o",
                "-",
                MAC_APP_PLIST,
            ],
        ) {
            return v;
        }
    } else if cfg!(target_os = "windows") {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let exe = format!("{local}\\Programs\\antigravity\\Antigravity.exe");
            let escaped = exe.replace('\'', "''");
            let cmd = format!("(Get-Item -LiteralPath '{escaped}').VersionInfo.FileVersion");
            if let Some(v) =
                run_version_command("powershell.exe", &["-NoProfile", "-Command", &cmd])
            {
                return v;
            }
        }
    } else if let Some(v) = run_version_command("antigravity", &["--version"]) {
        return v;
    }
    FALLBACK_VERSION.to_owned()
}

fn run_version_command(file: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(file).args(args).output().ok()?;
    parse_version_text(&String::from_utf8_lossy(&out.stdout))
}

fn is_version_token(raw: &str) -> bool {
    let mut parts = 0u8;
    let mut digits = 0u8;
    for ch in raw.chars() {
        if ch.is_ascii_digit() {
            digits = digits.saturating_add(1);
        } else if ch == '.' {
            if digits == 0 {
                return false;
            }
            parts = parts.saturating_add(1);
            digits = 0;
            if parts > 3 {
                return false;
            }
        } else {
            return false;
        }
    }
    digits > 0 && parts >= 1
}

fn capture_plist_tagged(xml: &str) -> Option<String> {
    let lower = xml.to_ascii_lowercase();
    let mut search = 0;
    while let Some(rel) = lower[search..].find("cfbundleshortversionstring") {
        let at = search + rel;
        let Some(end_key) = xml[at..].find("</key>") else {
            break;
        };
        let after_key = at + end_key + "</key>".len();
        let rest = xml[after_key..].trim_start();
        if let Some(inner) = rest.strip_prefix("<string>") {
            let end = inner.find("</string>")?;
            return normalize_version(inner[..end].trim());
        }
        search = at + 1;
    }
    None
}

fn token_headers() -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    headers.insert(
        http::header::USER_AGENT,
        ascii_header(&request_user_agent())?,
    );
    Ok(headers)
}

fn userinfo_headers(access_token: &str) -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        ascii_header(&format!("Bearer {access_token}"))?,
    );
    headers.insert(
        http::header::USER_AGENT,
        ascii_header(&request_user_agent())?,
    );
    Ok(headers)
}

fn load_code_assist_headers(access_token: &str) -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        ascii_header(&format!("Bearer {access_token}"))?,
    );
    headers.insert(http::header::ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        http::header::USER_AGENT,
        ascii_header(&request_user_agent())?,
    );
    Ok(headers)
}

fn onboard_user_headers(access_token: &str) -> Result<HeaderMap, Error> {
    let mut headers = load_code_assist_headers(access_token)?;
    headers.insert(
        http::header::USER_AGENT,
        ascii_header(&onboard_user_agent())?,
    );
    headers.insert(
        HeaderName::from_static("x-goog-api-client"),
        HeaderValue::from_static(GOOG_API_CLIENT_UA),
    );
    Ok(headers)
}

fn ascii_header(value: &str) -> Result<HeaderValue, Error> {
    HeaderValue::from_str(value).map_err(|_| Error::Payload("invalid header value".into()))
}

async fn post_token(client: &reqwest::Client, url: &str, body: String) -> Result<Value, Error> {
    let response = client
        .post(url)
        .headers(token_headers()?)
        .body(body)
        .send()
        .await?;
    let status = response.status().as_u16();
    let text = response.text().await?;
    if !(200..300).contains(&status) {
        return Err(map_token_status(status, &text));
    }
    if text.is_empty() {
        return Err(Error::Payload(
            "antigravity token returned an empty body".into(),
        ));
    }
    serde_json::from_str(&text).map_err(|err| Error::Payload(err.to_string()))
}

fn map_token_status(status: u16, body: &str) -> Error {
    if is_permanent_refresh_error_text(body) {
        Error::Permanent
    } else {
        Error::TokenEndpoint {
            status,
            body: body.to_owned(),
        }
    }
}

async fn complete_login(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    tokens: &Value,
    account: Option<String>,
) -> Result<Session, Error> {
    let access = token_str(tokens, "access_token")
        .or_else(|| token_str(tokens, "accessToken"))
        .ok_or_else(|| {
            Error::Payload("antigravity token exchange returned no access token".into())
        })?;
    let refresh_token = token_str(tokens, "refresh_token")
        .or_else(|| token_str(tokens, "refreshToken"))
        .ok_or_else(|| {
            Error::Payload("antigravity token exchange returned no refresh token".into())
        })?;
    let email = match account.and_then(|s| trimmed(&s)) {
        Some(existing) => existing,
        None => fetch_userinfo(client, endpoints, &access).await?,
    };
    let discovered = fetch_project(client, endpoints, &access).await?;
    build_session(SessionParts {
        access_token: access,
        refresh_token,
        expires_at_ms: expires_at_ms(tokens),
        account: email,
        project_id: discovered.project_id,
        plan_type: discovered.plan_type,
        needs_validation: false,
        validation_url: None,
    })
}

async fn fetch_userinfo(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    access_token: &str,
) -> Result<String, Error> {
    let response = client
        .get(&endpoints.userinfo)
        .headers(userinfo_headers(access_token)?)
        .send()
        .await?;
    let info = read_json(response, "antigravity userinfo").await?;
    token_str(&info, "email")
        .ok_or_else(|| Error::Payload("antigravity userinfo returned no email".into()))
}

async fn fetch_project(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    access_token: &str,
) -> Result<DiscoveredProject, Error> {
    let body = json!({ "metadata": { "ideType": "ANTIGRAVITY" } }).to_string();
    let response = request::fetch_cloud_code_at(
        client,
        &endpoints.load_code_assist(),
        &endpoints.daily,
        &endpoints.prod,
        load_code_assist_headers(access_token)?,
        body.into_bytes(),
    )
    .await?;
    let load_resp = read_json(response, "antigravity loadCodeAssist").await?;
    let plan_type = plan_type(&load_resp);
    if let Some(project_id) = extract_project(&load_resp) {
        return Ok(DiscoveredProject {
            project_id,
            plan_type,
        });
    }
    let project_id = onboard_user(
        client,
        endpoints,
        access_token,
        &default_tier_id(&load_resp),
    )
    .await?;
    Ok(DiscoveredProject {
        project_id,
        plan_type,
    })
}

async fn onboard_user(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    access_token: &str,
    tier_id: &str,
) -> Result<String, Error> {
    let body = json!({
        "tier_id": tier_id,
        "metadata": {
            "ide_type": "ANTIGRAVITY",
            "ide_version": version(),
            "ide_name": "antigravity",
        }
    })
    .to_string();
    for attempt in 1..=ONBOARD_ATTEMPTS {
        let response = client
            .post(endpoints.onboard_user())
            .headers(onboard_user_headers(access_token)?)
            .body(body.clone())
            .send()
            .await?;
        let data = read_json(response, "antigravity onboardUser").await?;
        if data.get("done").and_then(Value::as_bool).unwrap_or(false) {
            let project = data
                .get("response")
                .and_then(extract_project)
                .or_else(|| extract_project(&data));
            return project.ok_or_else(|| {
                Error::Payload("antigravity onboardUser completed without a project_id".into())
            });
        }
        if attempt < ONBOARD_ATTEMPTS {
            onboard_pause();
        }
    }
    Err(Error::Payload(format!(
        "antigravity onboardUser did not complete after {ONBOARD_ATTEMPTS} attempts"
    )))
}

fn onboard_pause() {
    if cfg!(test) {
        return;
    }
    std::thread::sleep(ONBOARD_PAUSE);
}

async fn read_json(response: reqwest::Response, label: &str) -> Result<Value, Error> {
    let status = response.status().as_u16();
    let text = response.text().await?;
    if !(200..300).contains(&status) {
        return Err(map_token_status(status, &text));
    }
    if text.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|err| Error::Payload(format!("{label}: {err}")))
}

fn build_session(parts: SessionParts) -> Result<Session, Error> {
    let access = trimmed(&parts.access_token)
        .ok_or_else(|| Error::Payload("antigravity session needs an access token".into()))?;
    let refresh = trimmed(&parts.refresh_token)
        .ok_or_else(|| Error::Payload("antigravity session needs a refresh token".into()))?;
    let project_id = trimmed(&parts.project_id)
        .ok_or_else(|| Error::Payload("antigravity session needs a project_id".into()))?;
    let mut session = Session::new(Family::Antigravity, access, refresh);
    session.expires_at_ms = parts.expires_at_ms;
    session.account = trimmed(&parts.account).unwrap_or_else(|| ID.to_owned());
    if let Some(plan) = parts.plan_type.and_then(|s| trimmed(&s)) {
        session.plan_type = plan;
    }
    session
        .extra
        .insert("project_id".into(), Value::String(project_id));
    if parts.needs_validation {
        session
            .extra
            .insert("needs_validation".into(), Value::Bool(true));
    }
    if let Some(url) = parts.validation_url.and_then(|s| trimmed(&s)) {
        session
            .extra
            .insert("validation_url".into(), Value::String(url));
    }
    Ok(session)
}

fn project_id_of(session: &Session) -> Option<String> {
    session
        .extra
        .get("project_id")
        .and_then(Value::as_str)
        .and_then(trimmed)
}

fn expires_at_ms(tokens: &Value) -> i64 {
    if let Some(at) = tokens
        .get("expiresAt")
        .and_then(Value::as_i64)
        .filter(|n| *n > 0)
    {
        return at;
    }
    let expires_in = tokens
        .get("expires_in")
        .or_else(|| tokens.get("expiresIn"))
        .and_then(as_f64)
        .unwrap_or(3600.0)
        .max(60.0);
    Utc::now().timestamp_millis() + (expires_in * 1000.0) as i64
}

fn extract_project(data: &Value) -> Option<String> {
    if !data.is_object() {
        return None;
    }
    for key in [
        "cloudaicompanionProject",
        "cloudaicompanionProjectId",
        "projectId",
        "project",
    ] {
        let Some(value) = data.get(key) else {
            continue;
        };
        if let Some(id) = value.as_str().and_then(trimmed) {
            return Some(id);
        }
        if let Some(id) = value.get("id").and_then(Value::as_str).and_then(trimmed) {
            return Some(id);
        }
    }
    None
}

fn default_tier_id(load_resp: &Value) -> String {
    if let Some(tiers) = load_resp.get("allowedTiers").and_then(Value::as_array) {
        for tier in tiers {
            if tier
                .get("isDefault")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && let Some(id) = tier.get("id").and_then(Value::as_str).and_then(trimmed)
            {
                return id;
            }
        }
    }
    load_resp
        .get("currentTier")
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
        .and_then(trimmed)
        .unwrap_or_else(|| "free-tier".to_owned())
}

fn plan_type(load_resp: &Value) -> Option<String> {
    for key in [
        "paidTier",
        "paid_tier",
        "subscriptionTier",
        "subscription_tier",
        "userTier",
        "user_tier",
        "planName",
        "plan_name",
        "tierId",
        "currentTier",
        "current_tier",
    ] {
        if let Some(claim) = google_ai_claim(load_resp.get(key)) {
            return Some(claim);
        }
    }
    None
}

fn google_ai_claim(value: Option<&Value>) -> Option<String> {
    let claim = tier_claim(value)?;
    if is_code_assist_only_plan(&claim) {
        None
    } else {
        Some(claim)
    }
}

fn tier_claim(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(s) = value.as_str().and_then(trimmed) {
        return Some(s);
    }
    value
        .get("name")
        .and_then(Value::as_str)
        .and_then(trimmed)
        .or_else(|| value.get("id").and_then(Value::as_str).and_then(trimmed))
        .or_else(|| {
            value
                .get("quotaTier")
                .and_then(Value::as_str)
                .and_then(trimmed)
        })
        .or_else(|| value.get("slug").and_then(Value::as_str).and_then(trimmed))
}

fn is_code_assist_only_plan(raw: &str) -> bool {
    let compact: String = raw
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    matches!(
        compact.as_str(),
        "standard" | "standardtier" | "legacy" | "legacytier"
    )
}

fn token_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).and_then(trimmed)
}

fn trimmed(value: &str) -> Option<String> {
    let t = value.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_owned())
    }
}

fn as_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|n| n as f64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn blob_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn contains_ignore_ascii(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

fn error_code(payload: &Value) -> &str {
    match payload.get("error") {
        Some(Value::String(s)) => s.as_str(),
        Some(Value::Object(map)) => map
            .get("status")
            .and_then(Value::as_str)
            .or_else(|| map.get("reason").and_then(Value::as_str))
            .unwrap_or(""),
        _ => payload.get("code").and_then(Value::as_str).unwrap_or(""),
    }
}

fn walk_validation_url(value: &Value, depth: u8) -> Option<String> {
    if depth > 6 {
        return None;
    }
    match value {
        Value::Null => None,
        Value::String(s) => {
            let href = s.trim();
            let google = href.len() >= 8 && href[..8].eq_ignore_ascii_case("https://") && {
                let rest = &href[8..];
                rest.to_ascii_lowercase()
                    .starts_with("accounts.google.com/")
            };
            if google
                && (href.contains("plt=") || href.to_ascii_lowercase().contains("signin/continue"))
            {
                Some(href.to_owned())
            } else {
                None
            }
        }
        Value::Array(items) => items
            .iter()
            .find_map(|item| walk_validation_url(item, depth + 1)),
        Value::Object(map) => {
            for key in ["validation_url", "validationUrl", "validationURL"] {
                if let Some(found) = map.get(key).and_then(|v| walk_validation_url(v, depth + 1)) {
                    return Some(found);
                }
            }
            map.values()
                .find_map(|item| walk_validation_url(item, depth + 1))
        }
        _ => None,
    }
}
