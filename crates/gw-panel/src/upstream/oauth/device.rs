//! Device-code (RFC 8628) and Kiro Builder ID / IDC helpers.
//!
//! xAI Grok uses the public Grok CLI OAuth client and the issuer
//! `https://auth.x.ai` (documented by xAI's first-party CLI and
//! https://help.router-for.me/configuration/provider/xai). Kiro uses AWS SSO
//! OIDC with a dynamically registered public client — there is no static
//! `client_id`. Neither flow is vendored; both are reimplemented against the
//! public endpoints.
//!
//! The token-poll state machine is a pure function so the pending / slow_down /
//! expired / denied transitions can be tested without a network.

use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use url::Url;

use super::exchange::{TokenResponse, claims_from_jwt, parse_token_body};
use super::flow::{self, Provider};
use super::{SESSION_TTL, SessionConfig, rfc3339};

#[cfg(test)]
mod tests;

/// Public Grok CLI OAuth client published by xAI's first-party CLI.
/// Source: CLIProxyAPI `internal/auth/xai/types.go` / router-for-me xAI docs.
const XAI_DISCOVERY: &str = "https://auth.x.ai/.well-known/openid-configuration";
const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const XAI_SCOPE: &str =
    "openid profile email offline_access grok-cli:access api:access";
const XAI_DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
pub(super) const XAI_API_BASE: &str = "https://api.x.ai/v1";

const KIRO_DEFAULT_REGION: &str = "us-east-1";
const KIRO_BUILDER_ID_START: &str = "https://view.awsapps.com/start";
const KIRO_CLIENT_NAME: &str = "AI-GateWay";
const KIRO_SCOPES: &[&str] = &[
    "codewhisperer:completions",
    "codewhisperer:analysis",
    "codewhisperer:conversations",
    "codewhisperer:transformations",
    "codewhisperer:taskassist",
];
const KIRO_DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const SLOW_DOWN_BUMP_SECS: i64 = 5;
const DEVICE_TTL_CAP: Duration = Duration::minutes(30);
const DEFAULT_INTERVAL: i64 = 5;

const USER_AGENT: &str = "AI-GateWay";

/// One step of a device-code token poll.
#[derive(Debug)]
pub enum DevicePollOutcome {
    Pending { interval: i64 },
    SlowDown { interval: i64 },
    Completed(TokenResponse),
    Failed { error: String, description: String },
}

impl DevicePollOutcome {
    #[must_use]
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending { .. } | Self::SlowDown { .. })
    }
}

/// Classifies a token-endpoint status + body. Pure: no I/O.
///
/// `authorization_pending` keeps waiting; `slow_down` raises the interval by
/// five seconds (RFC 8628); `expired_token` / `access_denied` fail the session.
#[must_use]
pub fn classify_device_token_body(status: u16, raw: &str, interval: i64) -> DevicePollOutcome {
    interpret_token_http(status, raw, interval)
}

/// Same as [`classify_device_token_body`]; named for the mock-HTTP tests.
#[must_use]
pub fn interpret_token_http(status: u16, raw: &str, interval: i64) -> DevicePollOutcome {
    let interval = if interval > 0 { interval } else { DEFAULT_INTERVAL };
    let parsed: Result<Map<String, Value>, _> = serde_json::from_str(raw.trim());
    let Ok(object) = parsed else {
        return DevicePollOutcome::Failed {
            error: "invalid_response".to_owned(),
            description: format!("token endpoint returned unparseable body (status {status})"),
        };
    };
    let object = normalize_token_keys(object);

    let error = object
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    let description = object
        .get("error_description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();

    if !error.is_empty() {
        return match error.as_str() {
            "authorization_pending" => DevicePollOutcome::Pending { interval },
            "slow_down" => DevicePollOutcome::SlowDown {
                interval: interval.saturating_add(SLOW_DOWN_BUMP_SECS),
            },
            other => DevicePollOutcome::Failed {
                error: other.to_owned(),
                description,
            },
        };
    }

    let tokens = parse_token_body(object);
    if tokens.access_token.trim().is_empty() {
        if (200..300).contains(&status) {
            return DevicePollOutcome::Failed {
                error: "invalid_response".to_owned(),
                description: "token endpoint returned no access_token".to_owned(),
            };
        }
        return DevicePollOutcome::Failed {
            error: format!("http_{status}"),
            description,
        };
    }
    DevicePollOutcome::Completed(tokens)
}


fn normalize_token_keys(mut object: Map<String, Value>) -> Map<String, Value> {
    for (camel, snake) in [
        ("accessToken", "access_token"),
        ("refreshToken", "refresh_token"),
        ("idToken", "id_token"),
        ("tokenType", "token_type"),
        ("expiresIn", "expires_in"),
        ("errorDescription", "error_description"),
    ] {
        if !object.contains_key(snake)
            && let Some(value) = object.remove(camel)
        {
            object.insert(snake.to_owned(), value);
        }
    }
    object
}

/// https + host `x.ai` or `*.x.ai`. Rejects anything else so a compromised
/// discovery document cannot redirect the token poll off-issuer.
#[must_use]
pub fn validate_xai_endpoint(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw.trim()) else {
        return false;
    };
    if url.scheme() != "https" {
        return false;
    }
    match url.host_str() {
        Some(host) => host.eq_ignore_ascii_case("x.ai") || host.to_ascii_lowercase().ends_with(".x.ai"),
        None => false,
    }
}

/// Device-session TTL: honour the provider's `expires_in`, never shorter than
/// the PKCE [`SESSION_TTL`], never longer than 30 minutes.
#[must_use]
pub fn device_session_ttl(expires_in: i64) -> Duration {
    if expires_in <= 0 {
        return SESSION_TTL;
    }
    let secs = expires_in.clamp(SESSION_TTL.num_seconds(), DEVICE_TTL_CAP.num_seconds());
    Duration::seconds(secs)
}

/// Whether this session is a device / IDC flow (no redirect_uri required).
#[must_use]
pub fn is_device_flow(config: &SessionConfig) -> bool {
    matches!(
        config.flow.trim(),
        "device" | "idc" | "import"
    ) || !config.device_code.trim().is_empty()
}

// ---------------------------------------------------------------- xAI

#[derive(Debug, Deserialize)]
struct OidcDiscovery {
    #[serde(default)]
    device_authorization_endpoint: String,
    #[serde(default)]
    token_endpoint: String,
}

#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    #[serde(default, alias = "deviceCode")]
    pub device_code: String,
    #[serde(default, alias = "userCode")]
    pub user_code: String,
    #[serde(default, alias = "verificationUri")]
    pub verification_uri: String,
    #[serde(default, alias = "verificationUriComplete")]
    pub verification_uri_complete: String,
    #[serde(default, alias = "expiresIn")]
    pub expires_in: i64,
    #[serde(default)]
    pub interval: i64,
}

/// Starts an xAI device-code session and fills `config`.
pub async fn start_xai_device(config: &mut SessionConfig) -> anyhow::Result<DeviceCodeResponse> {
    let discovery = get_json::<OidcDiscovery>(XAI_DISCOVERY).await?;
    anyhow::ensure!(
        validate_xai_endpoint(&discovery.device_authorization_endpoint),
        "xAI device_authorization_endpoint is not on x.ai"
    );
    anyhow::ensure!(
        validate_xai_endpoint(&discovery.token_endpoint),
        "xAI token_endpoint is not on x.ai"
    );

    let started = post_form_json::<DeviceCodeResponse>(
        &discovery.device_authorization_endpoint,
        &[
            ("client_id", XAI_CLIENT_ID.to_owned()),
            ("scope", XAI_SCOPE.to_owned()),
        ],
    )
    .await?;
    anyhow::ensure!(
        !started.device_code.trim().is_empty() && !started.user_code.trim().is_empty(),
        "xAI device authorization returned no device_code"
    );

    apply_device_start(config, &started, &discovery.token_endpoint, XAI_CLIENT_ID, "");
    config.flow = "device".to_owned();
    config.auth_method = "device".to_owned();
    Ok(started)
}

/// One xAI token poll.
pub async fn poll_xai_token(config: &SessionConfig) -> anyhow::Result<DevicePollOutcome> {
    anyhow::ensure!(
        validate_xai_endpoint(&config.token_endpoint),
        "xAI token_endpoint is not on x.ai"
    );
    let client_id = if config.client_id.trim().is_empty() {
        XAI_CLIENT_ID
    } else {
        config.client_id.trim()
    };
    let (status, body) = post_form_raw(
        &config.token_endpoint,
        &[
            ("grant_type", XAI_DEVICE_GRANT.to_owned()),
            ("device_code", config.device_code.clone()),
            ("client_id", client_id.to_owned()),
        ],
    )
    .await?;
    let mut outcome = interpret_token_http(status, &body, config.interval);
    if let DevicePollOutcome::Completed(tokens) = &mut outcome {
        decorate_xai_tokens(tokens);
    }
    Ok(outcome)
}

/// Refreshes an xAI access token. Used by the xAI executor.
pub async fn refresh_xai_token(
    token_endpoint: &str,
    client_id: &str,
    refresh_token: &str,
) -> anyhow::Result<TokenResponse> {
    anyhow::ensure!(
        validate_xai_endpoint(token_endpoint),
        "xAI token_endpoint is not on x.ai"
    );
    let (status, body) = post_form_raw(
        token_endpoint,
        &[
            ("grant_type", "refresh_token".to_owned()),
            ("client_id", client_id.to_owned()),
            ("refresh_token", refresh_token.to_owned()),
        ],
    )
    .await?;
    match interpret_token_http(status, &body, DEFAULT_INTERVAL) {
        DevicePollOutcome::Completed(mut tokens) => {
            decorate_xai_tokens(&mut tokens);
            Ok(tokens)
        }
        DevicePollOutcome::Failed { error, description } => {
            anyhow::bail!("xAI refresh failed: {error} {description}")
        }
        _ => anyhow::bail!("xAI refresh returned a pending response"),
    }
}

fn decorate_xai_tokens(tokens: &mut TokenResponse) {
    if tokens.email.is_empty() || tokens.account_id.is_empty() {
        let (email, account_id) = claims_from_jwt(&tokens.id_token);
        if tokens.email.is_empty() {
            tokens.email = email;
        }
        if tokens.account_id.is_empty() {
            tokens.account_id = account_id;
        }
    }
    tokens.extra.insert("api_key".to_owned(), json!(tokens.access_token));
    tokens
        .extra
        .insert("base_url".to_owned(), json!(XAI_API_BASE));
}

// ---------------------------------------------------------------- Kiro

/// Operator-facing start body for `POST /kiro-auth-url`.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct KiroStartBody {
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub start_url: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub token: Value,
}

impl KiroStartBody {
    #[must_use]
    pub fn from_value(value: Option<&Value>) -> Self {
        value
            .and_then(|raw| serde_json::from_value(raw.clone()).ok())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn method_key(&self) -> &'static str {
        match self.method.trim().to_ascii_lowercase().as_str() {
            "" | "builder-id" | "builder_id" | "device" => "device",
            "authcode" | "auth-code" | "authorization_code" | "authorization-code" => "authcode",
            "idc" | "iam" | "sso" => "idc",
            "import" => "import",
            _ => "device",
        }
    }
}

#[derive(Debug, Deserialize)]
struct RegisteredClient {
    #[serde(default, alias = "clientId")]
    client_id: String,
    #[serde(default, alias = "clientSecret")]
    client_secret: String,
}

/// Starts a Kiro Builder ID or IDC device-code session.
pub async fn start_kiro_device(
    config: &mut SessionConfig,
    body: &KiroStartBody,
) -> anyhow::Result<DeviceCodeResponse> {
    let region = if body.region.trim().is_empty() {
        KIRO_DEFAULT_REGION
    } else {
        body.region.trim()
    };
    let start_url = if body.start_url.trim().is_empty() {
        KIRO_BUILDER_ID_START
    } else {
        body.start_url.trim()
    };
    anyhow::ensure!(
        start_url.starts_with("https://"),
        "Kiro start URL must be https"
    );

    let oidc = oidc_base(region);
    let registered = register_kiro_client(&oidc, None, &[]).await?;
    let started = post_json_json::<DeviceCodeResponse>(
        &format!("{oidc}/device_authorization"),
        &json!({
            "clientId": registered.client_id,
            "clientSecret": registered.client_secret,
            "startUrl": start_url,
        }),
    )
    .await?;
    anyhow::ensure!(
        !started.device_code.trim().is_empty() && !started.user_code.trim().is_empty(),
        "Kiro device authorization returned no device_code"
    );

    let token_endpoint = format!("{oidc}/token");
    apply_device_start(
        config,
        &started,
        &token_endpoint,
        &registered.client_id,
        &registered.client_secret,
    );
    config.flow = if body.method_key() == "idc" {
        "idc".to_owned()
    } else {
        "device".to_owned()
    };
    config.auth_method = if body.method_key() == "idc" {
        "idc".to_owned()
    } else {
        "builder-id".to_owned()
    };
    config.start_url = start_url.to_owned();
    config.region = region.to_owned();
    Ok(started)
}

/// Registers a public client and builds a PKCE authorize URL for Builder ID.
pub async fn start_kiro_authcode(
    config: &mut SessionConfig,
    body: &KiroStartBody,
) -> anyhow::Result<String> {
    let region = if body.region.trim().is_empty() {
        KIRO_DEFAULT_REGION
    } else {
        body.region.trim()
    };
    let start_url = if body.start_url.trim().is_empty() {
        KIRO_BUILDER_ID_START
    } else {
        body.start_url.trim()
    };
    anyhow::ensure!(
        !config.redirect_uri.trim().is_empty(),
        "Kiro authorization-code flow needs a redirect_uri"
    );

    let oidc = oidc_base(region);
    let registered = register_kiro_client(
        &oidc,
        Some(config.redirect_uri.as_str()),
        &[config.redirect_uri.as_str()],
    )
    .await?;
    let challenge = flow::set_pkce(config)?;
    config.client_id = registered.client_id.clone();
    config.client_secret = registered.client_secret;
    config.token_endpoint = format!("{oidc}/token");
    config.flow = "authorization_code".to_owned();
    config.auth_method = "builder-id".to_owned();
    config.start_url = start_url.to_owned();
    config.region = region.to_owned();

    let params = [
        ("response_type", "code".to_owned()),
        ("client_id", registered.client_id),
        ("redirect_uri", config.redirect_uri.clone()),
        ("scopes", KIRO_SCOPES.join(" ")),
        ("state", config.state.clone()),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256".to_owned()),
    ];
    Ok(format!("{oidc}/authorize?{}", flow::form_encode(&params)))
}

/// One Kiro device-code token poll (AWS SSO OIDC JSON).
pub async fn poll_kiro_token(config: &SessionConfig) -> anyhow::Result<DevicePollOutcome> {
    anyhow::ensure!(
        config.token_endpoint.starts_with("https://"),
        "Kiro token_endpoint must be https"
    );
    let (status, body) = post_json_raw(
        &config.token_endpoint,
        &json!({
            "clientId": config.client_id,
            "clientSecret": config.client_secret,
            "grantType": KIRO_DEVICE_GRANT,
            "deviceCode": config.device_code,
        }),
    )
    .await?;
    let mut outcome = interpret_token_http(status, &body, config.interval);
    if let DevicePollOutcome::Completed(tokens) = &mut outcome {
        decorate_kiro_tokens(tokens, config);
    }
    Ok(outcome)
}

/// Exchanges a Kiro authorization code.
pub async fn exchange_kiro_code(
    config: &SessionConfig,
    code: &str,
) -> anyhow::Result<TokenResponse> {
    anyhow::ensure!(
        !config.client_id.trim().is_empty(),
        "missing Kiro client_id"
    );
    anyhow::ensure!(
        !config.token_endpoint.trim().is_empty(),
        "missing Kiro token_endpoint"
    );
    anyhow::ensure!(
        !config.code_verifier.trim().is_empty(),
        "missing PKCE verifier"
    );
    let (status, body) = post_json_raw(
        &config.token_endpoint,
        &json!({
            "clientId": config.client_id,
            "clientSecret": config.client_secret,
            "grantType": "authorization_code",
            "code": code,
            "redirectUri": config.redirect_uri,
            "codeVerifier": config.code_verifier,
        }),
    )
    .await?;
    match interpret_token_http(status, &body, DEFAULT_INTERVAL) {
        DevicePollOutcome::Completed(mut tokens) => {
            decorate_kiro_tokens(&mut tokens, config);
            Ok(tokens)
        }
        DevicePollOutcome::Failed { error, description } => {
            anyhow::bail!("Kiro code exchange failed: {error} {description}")
        }
        _ => anyhow::bail!("Kiro code exchange returned a pending response"),
    }
}

/// Refreshes a Kiro access token. Used by the Kiro executor.
pub async fn refresh_kiro_token(
    token_endpoint: &str,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> anyhow::Result<TokenResponse> {
    anyhow::ensure!(
        token_endpoint.starts_with("https://"),
        "Kiro token_endpoint must be https"
    );
    let (status, body) = post_json_raw(
        token_endpoint,
        &json!({
            "clientId": client_id,
            "clientSecret": client_secret,
            "grantType": "refresh_token",
            "refreshToken": refresh_token,
        }),
    )
    .await?;
    match interpret_token_http(status, &body, DEFAULT_INTERVAL) {
        DevicePollOutcome::Completed(tokens) => Ok(tokens),
        DevicePollOutcome::Failed { error, description } => {
            anyhow::bail!("Kiro refresh failed: {error} {description}")
        }
        _ => anyhow::bail!("Kiro refresh returned a pending response"),
    }
}

fn decorate_kiro_tokens(tokens: &mut TokenResponse, config: &SessionConfig) {
    if tokens.email.is_empty() || tokens.account_id.is_empty() {
        let (email, account_id) = claims_from_jwt(&tokens.id_token);
        if tokens.email.is_empty() {
            tokens.email = email;
        }
        if tokens.account_id.is_empty() {
            tokens.account_id = account_id;
        }
    }
    if !config.client_id.is_empty() {
        tokens
            .extra
            .insert("client_id".to_owned(), json!(config.client_id));
    }
    if !config.client_secret.is_empty() {
        tokens
            .extra
            .insert("client_secret".to_owned(), json!(config.client_secret));
    }
    if !config.auth_method.is_empty() {
        tokens
            .extra
            .insert("auth_method".to_owned(), json!(config.auth_method));
    }
    if !config.start_url.is_empty() {
        tokens
            .extra
            .insert("start_url".to_owned(), json!(config.start_url));
    }
    if !config.region.is_empty() {
        tokens
            .extra
            .insert("region".to_owned(), json!(config.region));
    }
    if !config.token_endpoint.is_empty() {
        tokens.extra.insert(
            "token_endpoint".to_owned(),
            json!(config.token_endpoint),
        );
    }
}

async fn register_kiro_client(
    oidc: &str,
    issuer_url: Option<&str>,
    redirect_uris: &[&str],
) -> anyhow::Result<RegisteredClient> {
    let mut body = json!({
        "clientName": KIRO_CLIENT_NAME,
        "clientType": "public",
        "scopes": KIRO_SCOPES,
        "grantTypes": if redirect_uris.is_empty() {
            vec![KIRO_DEVICE_GRANT, "refresh_token"]
        } else {
            vec!["authorization_code", "refresh_token"]
        },
    });
    if !redirect_uris.is_empty() {
        body["redirectUris"] = json!(redirect_uris);
    }
    if let Some(issuer) = issuer_url {
        body["issuerUrl"] = json!(issuer);
    }
    let registered = post_json_json::<RegisteredClient>(&format!("{oidc}/client/register"), &body)
        .await?;
    anyhow::ensure!(
        !registered.client_id.trim().is_empty(),
        "Kiro RegisterClient returned no client_id"
    );
    Ok(registered)
}

fn oidc_base(region: &str) -> String {
    format!("https://oidc.{region}.amazonaws.com")
}

// ---------------------------------------------------------------- import

/// Parses a pasted Kiro IDE cache document (snake_case or camelCase).
///
/// Path-traversal safe: JSON body only. The IDE cache typically lives at
/// `~/.aws/sso/cache/` on the operator's machine — this gateway never reads
/// that path.
pub fn parse_kiro_import(raw: &Value) -> anyhow::Result<TokenResponse> {
    let object = raw.as_object().ok_or_else(|| {
        anyhow::anyhow!("Kiro import must be a JSON object")
    })?;
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| object.get(*key).and_then(Value::as_str))
            .unwrap_or("")
            .trim()
            .to_owned()
    };
    let access_token = text(&["access_token", "accessToken"]);
    anyhow::ensure!(
        !access_token.is_empty(),
        "Kiro import is missing access_token"
    );
    let refresh_token = text(&["refresh_token", "refreshToken"]);
    let id_token = text(&["id_token", "idToken"]);
    let email = text(&["email"]);
    let account_id = text(&["account_id", "accountId", "sub"]);
    let expires_in = object
        .get("expires_in")
        .or_else(|| object.get("expiresIn"))
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
        .unwrap_or(0);

    let mut tokens = TokenResponse {
        access_token,
        refresh_token,
        id_token,
        token_type: text(&["token_type", "tokenType"]),
        expires_in,
        raw: object.clone(),
        email,
        account_id,
        extra: Map::new(),
    };
    for (src, dst) in [
        (["client_id", "clientId"], "client_id"),
        (["client_secret", "clientSecret"], "client_secret"),
        (["auth_method", "authMethod"], "auth_method"),
        (["start_url", "startUrl"], "start_url"),
        (["region", "region"], "region"),
        (["token_endpoint", "tokenEndpoint"], "token_endpoint"),
    ] {
        let value = text(&src);
        if !value.is_empty() {
            tokens.extra.insert(dst.to_owned(), json!(value));
        }
    }
    if tokens.auth_method_hint().is_empty() {
        tokens
            .extra
            .insert("auth_method".to_owned(), json!("import"));
    }
    Ok(tokens)
}

trait AuthMethodHint {
    fn auth_method_hint(&self) -> String;
}

impl AuthMethodHint for TokenResponse {
    fn auth_method_hint(&self) -> String {
        self.extra
            .get("auth_method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    }
}

// ---------------------------------------------------------------- session helpers

fn apply_device_start(
    config: &mut SessionConfig,
    started: &DeviceCodeResponse,
    token_endpoint: &str,
    client_id: &str,
    client_secret: &str,
) {
    config.device_code = started.device_code.clone();
    config.user_code = started.user_code.clone();
    config.verification_uri = started.verification_uri.clone();
    config.verification_uri_complete = started.verification_uri_complete.clone();
    config.interval = if started.interval > 0 {
        started.interval
    } else {
        DEFAULT_INTERVAL
    };
    config.token_endpoint = token_endpoint.to_owned();
    config.client_id = client_id.to_owned();
    config.client_secret = client_secret.to_owned();
}

/// Whether the interval since `last_poll_at` has elapsed.
#[must_use]
pub fn interval_elapsed(config: &SessionConfig, now: chrono::DateTime<Utc>) -> bool {
    if config.last_poll_at.trim().is_empty() {
        return true;
    }
    let Ok(last) = chrono::DateTime::parse_from_rfc3339(config.last_poll_at.trim()) else {
        return true;
    };
    let wait = Duration::seconds(config.interval.max(1));
    now >= last.with_timezone(&Utc) + wait
}

/// Marks a poll attempt so the next one respects `interval`.
pub fn mark_polled(config: &mut SessionConfig, now: chrono::DateTime<Utc>, interval: i64) {
    config.last_poll_at = rfc3339(now);
    if interval > 0 {
        config.interval = interval;
    }
}

/// JSON the console gets after a device start.
#[must_use]
pub fn device_start_payload(
    state: &str,
    started: &DeviceCodeResponse,
) -> Value {
    let open = if started.verification_uri_complete.trim().is_empty() {
        started.verification_uri.clone()
    } else {
        started.verification_uri_complete.clone()
    };
    json!({
        "auth_url": open,
        "url": open,
        "state": state,
        "user_code": started.user_code,
        "verification_uri": started.verification_uri,
        "verification_uri_complete": started.verification_uri_complete,
        "expires_in": started.expires_in,
        "interval": if started.interval > 0 { started.interval } else { DEFAULT_INTERVAL },
        "flow": "device",
    })
}

// ---------------------------------------------------------------- HTTP

fn http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
}

async fn get_json<T: for<'de> Deserialize<'de>>(url: &str) -> anyhow::Result<T> {
    let response = http_client()?.get(url).header("Accept", "application/json").send().await?;
    let status = response.status();
    let body = response.bytes().await?;
    anyhow::ensure!(
        status.is_success(),
        "GET {url} returned status {}",
        status.as_u16()
    );
    Ok(serde_json::from_slice(&body)?)
}

async fn post_form_json<T: for<'de> Deserialize<'de>>(
    url: &str,
    form: &[(&str, String)],
) -> anyhow::Result<T> {
    let (status, body) = post_form_raw(url, form).await?;
    anyhow::ensure!(
        (200..300).contains(&status),
        "POST {url} returned status {status}: {body}"
    );
    Ok(serde_json::from_str(&body)?)
}

async fn post_form_raw(url: &str, form: &[(&str, String)]) -> anyhow::Result<(u16, String)> {
    let response = http_client()?
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(flow::form_encode(form))
        .send()
        .await?;
    let status = response.status().as_u16();
    let body = response.text().await?;
    Ok((status, body))
}

async fn post_json_json<T: for<'de> Deserialize<'de>>(url: &str, body: &Value) -> anyhow::Result<T> {
    let (status, text) = post_json_raw(url, body).await?;
    anyhow::ensure!(
        (200..300).contains(&status),
        "POST {url} returned status {status}: {text}"
    );
    Ok(serde_json::from_str(&text)?)
}

async fn post_json_raw(url: &str, body: &Value) -> anyhow::Result<(u16, String)> {
    let response = http_client()?
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(serde_json::to_vec(body)?)
        .send()
        .await?;
    let status = response.status().as_u16();
    let text = response.text().await?;
    Ok((status, text))
}

/// Polls whichever provider owns this session.
pub async fn poll_provider_token(
    provider: Provider,
    config: &SessionConfig,
) -> anyhow::Result<DevicePollOutcome> {
    match provider {
        Provider::Xai => poll_xai_token(config).await,
        Provider::Kiro => poll_kiro_token(config).await,
        _ => anyhow::bail!("{} does not use the device-code flow", provider.as_str()),
    }
}
