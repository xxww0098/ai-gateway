//! Kiro credential shape, machine id, social redirect URIs, expiry math.

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{Error, Family, Session};

use super::{BUILDER_ID_START_URL, DEFAULT_REGION, ID};

#[cfg(test)]
mod tests;

const EXTERNAL_IDP_ALIASES: &[&str] = &[
    "external_idp",
    "azuread",
    "azure",
    "entra",
    "entra-id",
    "microsoft",
    "m365",
    "office365",
    "external",
];
const SOCIAL_PROVIDERS: &[&str] = &["github", "google", "gmail", "gh", "social", "oauth"];
const IDC_PROVIDERS: &[&str] = &[
    "builderid",
    "builder-id",
    "builder_id",
    "builder",
    "enterprise",
    "idc",
    "iam",
];
const ALLOWED_IDP_SUFFIXES: &[&str] = &[
    ".microsoftonline.com",
    ".microsoftonline.us",
    ".microsoftonline.cn",
];

/// API keys do not expire on the wire; TokenManager must not refresh them.
pub const NEVER_EXPIRES_MS: i64 = 8_640_000_000_000_000;

#[must_use]
pub fn trimmed(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

#[must_use]
pub fn json_str(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(found) = value
            .get(*key)
            .and_then(Value::as_str)
            .and_then(|s| trimmed(Some(s)))
        {
            return Some(found);
        }
    }
    None
}

#[must_use]
pub fn json_number(value: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        let Some(raw) = value.get(*key) else {
            continue;
        };
        if let Some(n) = as_positive_number(raw) {
            return Some(n);
        }
    }
    None
}

#[must_use]
pub fn as_positive_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64().filter(|x| x.is_finite() && *x > 0.0),
        Value::String(s) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|x| x.is_finite() && *x > 0.0),
        _ => None,
    }
}

fn duration_ms_of(value: &Value) -> Option<i64> {
    let n = as_positive_number(value)?;
    let ms = if n > 1_000_000.0 { n } else { n * 1000.0 };
    Some(ms.round() as i64)
}

/// Unix-ms expiry. Numbers above 1e12 are already milliseconds.
#[must_use]
pub fn expires_at_ms(expires_at: Option<&Value>, expires_in: Option<&Value>, now_ms: i64) -> i64 {
    if let Some(value) = expires_at {
        if let Some(n) = as_positive_number(value) {
            if n > 1e12 {
                return n.round() as i64;
            }
            return now_ms
                + if n > 1e6 {
                    n.round() as i64
                } else {
                    (n * 1000.0).round() as i64
                };
        }
        if let Some(text) = value.as_str().and_then(|s| trimmed(Some(s)))
            && let Some(ms) = parse_timestamp_ms(&text)
        {
            return ms;
        }
    }
    if let Some(value) = expires_in
        && let Some(ms) = duration_ms_of(value)
    {
        return now_ms + ms;
    }
    now_ms + 3_600_000
}

fn parse_timestamp_ms(text: &str) -> Option<i64> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(text) {
        return Some(dt.timestamp_millis());
    }
    if let Ok(n) = text.parse::<f64>()
        && n.is_finite()
        && n > 0.0
    {
        return Some(n.round() as i64);
    }
    None
}

/// Fresh TTL from the refresh JSON only. Never reuse a stored session expiresAt.
#[must_use]
pub fn refresh_expires_at_ms(body: &Value, now_ms: i64) -> i64 {
    if let Some(ms) = body
        .get("expiresIn")
        .or_else(|| body.get("expires_in"))
        .and_then(duration_ms_of)
    {
        return now_ms + ms;
    }
    let stamp = body.get("expiresAt").or_else(|| body.get("expires_at"));
    if stamp.is_some()
        && stamp != Some(&Value::Null)
        && stamp != Some(&Value::String(String::new()))
    {
        return expires_at_ms(stamp, None, now_ms);
    }
    now_ms + 3_600_000
}

#[must_use]
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Canonical stored `auth_method`. Builder ID is `idc`.
#[must_use]
pub fn canonicalize_method(value: &str, token_endpoint: Option<&str>) -> String {
    let raw = value.trim();
    if raw.is_empty() && token_endpoint.is_some() {
        return "external_idp".to_owned();
    }
    let lower = raw.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "builder-id" | "builder_id" | "builderid" | "builder" | "iam" | "enterprise" | "idc"
    ) {
        return "idc".to_owned();
    }
    if matches!(lower.as_str(), "api_key" | "apikey" | "ksk") {
        return "api_key".to_owned();
    }
    if EXTERNAL_IDP_ALIASES.contains(&lower.as_str()) {
        return "external_idp".to_owned();
    }
    if SOCIAL_PROVIDERS.contains(&lower.as_str()) {
        return "social".to_owned();
    }
    if token_endpoint.is_some() {
        return "external_idp".to_owned();
    }
    if raw.is_empty() {
        "social".to_owned()
    } else {
        lower
    }
}

#[must_use]
pub fn infer_auth_method(raw: &Value) -> String {
    let nested = nested_credentials(raw).unwrap_or(raw);
    let kiro_api_key = json_str(nested, &["kiroApiKey", "kiro_api_key"]);
    let access = json_str(nested, &["accessToken", "access_token"]);
    if kiro_api_key.is_some() || access.as_deref().is_some_and(|s| s.starts_with("ksk_")) {
        return "api_key".to_owned();
    }
    let token_endpoint = json_str(nested, &["tokenEndpoint", "token_endpoint"])
        .or_else(|| json_str(raw, &["tokenEndpoint", "token_endpoint"]));
    let declared = json_str(nested, &["authMethod", "auth_method"])
        .or_else(|| json_str(raw, &["authMethod", "auth_method"]));
    if declared.is_some() || token_endpoint.is_some() {
        return canonicalize_method(declared.as_deref().unwrap_or(""), token_endpoint.as_deref());
    }
    let provider = json_str(nested, &["provider", "idp"])
        .or_else(|| json_str(raw, &["provider", "idp", "kiroProvider"]))
        .unwrap_or_default()
        .to_ascii_lowercase();
    if SOCIAL_PROVIDERS.contains(&provider.as_str()) {
        return "social".to_owned();
    }
    if IDC_PROVIDERS.contains(&provider.as_str()) {
        return "idc".to_owned();
    }
    if EXTERNAL_IDP_ALIASES.contains(&provider.as_str()) {
        return "external_idp".to_owned();
    }
    let client_id = json_str(nested, &["clientId", "client_id"]);
    let client_secret = json_str(nested, &["clientSecret", "client_secret"]);
    if client_id.is_some() && client_secret.is_some() {
        return "idc".to_owned();
    }
    "social".to_owned()
}

#[must_use]
pub fn nested_credentials(raw: &Value) -> Option<&Value> {
    let nested = raw.get("credentials")?;
    if nested.is_object() && !nested.is_array() {
        Some(nested)
    } else {
        None
    }
}

#[must_use]
pub fn account_kind(session: &Session) -> &'static str {
    let method = extra_str(session, "auth_method");
    let token_endpoint = extra_str(session, "token_endpoint");
    let method = canonicalize_method(
        &method,
        if token_endpoint.is_empty() {
            None
        } else {
            Some(&token_endpoint)
        },
    );
    match method.as_str() {
        "external_idp" => "entra",
        "api_key" => "key",
        "idc" => {
            let start = extra_str(session, "start_url");
            let provider = extra_str(session, "kiro_provider");
            if provider == "Enterprise" || (!start.is_empty() && start != BUILDER_ID_START_URL) {
                "idc"
            } else {
                "builder"
            }
        }
        _ => "social",
    }
}

#[must_use]
pub fn method_label(session: &Session) -> &'static str {
    match account_kind(session) {
        "builder" => "Builder",
        "idc" => "IdC",
        "entra" => "Entra",
        "key" => "API key",
        _ => "Social",
    }
}

#[must_use]
pub fn extra_str(session: &Session, key: &str) -> String {
    session
        .extra
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_owned()
}

pub fn extra_set(extra: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value.filter(|s| !s.is_empty()) {
        extra.insert(key.to_owned(), Value::String(value));
    }
}

pub fn validate_refresh_token(value: &str) -> Result<String, Error> {
    let token =
        trimmed(Some(value)).ok_or_else(|| Error::Payload("kiro refresh token is empty".into()))?;
    if token.len() < 100 || token.contains("...") {
        return Err(Error::Payload(format!(
            "kiro refresh token looks truncated ({} chars)",
            token.len()
        )));
    }
    Ok(token)
}

pub fn validate_api_key(value: &str) -> Result<String, Error> {
    let key = trimmed(Some(value)).ok_or_else(|| Error::Payload("kiro API key is empty".into()))?;
    if !key.starts_with("ksk_") || key.len() < 12 {
        return Err(Error::Payload("kiro API key must start with ksk_".into()));
    }
    Ok(key)
}

pub fn validate_idp_endpoint(raw: &str) -> Result<String, Error> {
    let url = Url::parse(raw.trim())
        .map_err(|_| Error::Payload("kiro enterprise SSO token endpoint is not a URL".into()))?;
    if url.scheme() != "https" {
        return Err(Error::Payload(
            "kiro enterprise SSO token endpoint must be https".into(),
        ));
    }
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    if host.is_empty() || host.starts_with(|c: char| c.is_ascii_digit()) || host.contains(':') {
        return Err(Error::Payload(
            "kiro enterprise SSO token endpoint host is not allowed".into(),
        ));
    }
    let allowed = ALLOWED_IDP_SUFFIXES
        .iter()
        .any(|suffix| host.ends_with(suffix) && host.len() > suffix.len());
    if !allowed {
        return Err(Error::Payload(
            "kiro enterprise SSO token endpoint must be a microsoftonline host".into(),
        ));
    }
    Ok(url.to_string())
}

/// Origin-only Cognito redirect (`http://localhost:<port>`). Token exchange adds path.
pub fn social_redirect_uri(redirect_uri: &str) -> Result<String, Error> {
    let url = Url::parse(redirect_uri)
        .map_err(|_| Error::Payload("kiro social redirect_uri is not a URL".into()))?;
    let host = match url.host_str() {
        Some("127.0.0.1" | "::1") => "localhost",
        Some(host) => host,
        None => {
            return Err(Error::Payload(
                "kiro social redirect_uri has no host".into(),
            ));
        }
    };
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    Ok(format!("{}://{host}{port}", url.scheme()))
}

#[must_use]
pub fn social_login_option(value: Option<&str>) -> Option<String> {
    trimmed(value).map(|s| s.to_ascii_lowercase())
}

/// Token-exchange redirect_uri is the URL the browser actually hit.
pub fn social_token_redirect_uri(
    redirect_uri: &str,
    pathname: Option<&str>,
    login_option: Option<&str>,
) -> Result<String, Error> {
    let origin = social_redirect_uri(redirect_uri)?;
    let raw_path = pathname
        .and_then(|s| trimmed(Some(s)))
        .or_else(|| {
            Url::parse(redirect_uri).ok().map(|url| {
                let path = url.path();
                if path.is_empty() {
                    "/".to_owned()
                } else {
                    path.to_owned()
                }
            })
        })
        .unwrap_or_else(|| "/".to_owned());
    let path = if raw_path.starts_with('/') {
        raw_path
    } else {
        format!("/{raw_path}")
    };
    let login = social_login_option(login_option);
    Ok(match login {
        Some(option) => format!("{origin}{path}?login_option={option}"),
        None => format!("{origin}{path}"),
    })
}

pub fn social_authorize_url(
    state: &str,
    challenge: &str,
    redirect_uri: &str,
) -> Result<String, Error> {
    let origin = social_redirect_uri(redirect_uri)?;
    let params = [
        ("state", state.to_owned()),
        ("code_challenge", challenge.to_owned()),
        ("code_challenge_method", "S256".to_owned()),
        ("redirect_uri", origin),
        ("redirect_from", "KiroIDE".to_owned()),
    ];
    Ok(format!(
        "{}/signin?{}",
        super::PORTAL_URL,
        crate::form::encode(&params)
    ))
}

#[must_use]
pub fn oidc_endpoint(region: &str) -> String {
    let region = if region.trim().is_empty() {
        DEFAULT_REGION
    } else {
        region.trim()
    };
    format!("https://oidc.{region}.amazonaws.com")
}

#[must_use]
pub fn usage_host(region: &str) -> String {
    let region = if region.trim().is_empty() {
        DEFAULT_REGION
    } else {
        region.trim()
    };
    format!("q.{region}.amazonaws.com")
}

#[must_use]
pub fn social_refresh_host(region: &str) -> String {
    let region = if region.trim().is_empty() {
        DEFAULT_REGION
    } else {
        region.trim()
    };
    format!("prod.{region}.auth.desktop.kiro.dev")
}

#[must_use]
pub fn machine_id(stored: Option<&str>, method: &str, seed: &str) -> String {
    if let Some(stored) =
        stored.filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return stored.to_ascii_lowercase();
    }
    let prefix = if method == "api_key" {
        "KiroAPIKey/"
    } else {
        "KotlinNativeAPI/"
    };
    let material = if seed.is_empty() {
        format!("{prefix}{}", uuid::Uuid::new_v4())
    } else {
        format!("{prefix}{seed}")
    };
    hex::encode(Sha256::digest(material.as_bytes()))
}

#[must_use]
pub fn allocate_machine_id(prior: Option<&str>) -> String {
    machine_id(prior, "social", "")
}

#[must_use]
pub fn social_user_agent(machine: &str) -> String {
    format!("KiroIDE-{}-{machine}", super::USAGE_VERSION)
}

/// Builder ID placeholder ARN is not sent on usage or as a live profile.
#[must_use]
pub fn effective_profile_arn(arn: Option<&str>) -> Option<String> {
    let arn = trimmed(arn)?;
    if arn == super::BUILDER_ID_PROFILE_ARN {
        None
    } else {
        Some(arn)
    }
}

pub fn build_session(
    access_token: Option<String>,
    refresh_token: Option<String>,
    method: &str,
    fields: &Value,
    now_ms: i64,
) -> Result<Session, Error> {
    let method = canonicalize_method(
        method,
        json_str(fields, &["tokenEndpoint", "token_endpoint"]).as_deref(),
    );
    let kiro_api_key = if method == "api_key" {
        json_str(fields, &["kiroApiKey", "kiro_api_key"])
            .or_else(|| access_token.clone())
            .or_else(|| json_str(fields, &["accessToken", "access_token"]))
    } else {
        None
    };
    let access = access_token
        .or_else(|| json_str(fields, &["accessToken", "access_token"]))
        .or_else(|| kiro_api_key.clone());
    let refresh = refresh_token
        .or_else(|| json_str(fields, &["refreshToken", "refresh_token"]))
        .or_else(|| access.clone());
    if method == "api_key" {
        let key = validate_api_key(access.as_deref().unwrap_or(""))?;
        let mut session = Session::new(Family::Kiro, &key, &key);
        session.expires_at_ms = NEVER_EXPIRES_MS;
        session.account =
            json_str(fields, &["email", "account"]).unwrap_or_else(|| "api-key".to_owned());
        session.plan_type = json_str(
            fields,
            &["planType", "subscriptionTitle", "subscription_title"],
        )
        .unwrap_or_default();
        session.source = ID.to_owned();
        extra_set(
            &mut session.extra,
            "auth_method",
            Some("api_key".to_owned()),
        );
        extra_set(&mut session.extra, "kiro_api_key", Some(key.clone()));
        let machine = machine_id(
            json_str(fields, &["machineId", "machine_id"]).as_deref(),
            "api_key",
            &key,
        );
        extra_set(&mut session.extra, "machine_id", Some(machine));
        extra_set(
            &mut session.extra,
            "region",
            json_str(fields, &["region", "authRegion", "auth_region"])
                .or_else(|| Some(DEFAULT_REGION.to_owned())),
        );
        return Ok(session);
    }
    let refresh = refresh.ok_or_else(|| {
        Error::Payload("kiro session needs an access token or refresh token".into())
    })?;
    let access = access.unwrap_or_else(|| refresh.clone());
    if access.is_empty() && refresh.is_empty() {
        return Err(Error::Payload(
            "kiro session needs an access token or refresh token".into(),
        ));
    }
    let region = json_str(fields, &["region"])
        .or_else(|| json_str(fields, &["authRegion", "auth_region"]))
        .unwrap_or_else(|| DEFAULT_REGION.to_owned());
    let mut session = Session::new(Family::Kiro, &access, &refresh);
    session.expires_at_ms = expires_at_ms(
        fields.get("expiresAt").or_else(|| fields.get("expires_at")),
        fields.get("expiresIn").or_else(|| fields.get("expires_in")),
        now_ms,
    );
    session.account = json_str(fields, &["account", "email"]).unwrap_or_default();
    session.plan_type = json_str(
        fields,
        &["planType", "subscriptionTitle", "subscription_title"],
    )
    .unwrap_or_default();
    session.source = ID.to_owned();
    extra_set(&mut session.extra, "auth_method", Some(method.clone()));
    extra_set(
        &mut session.extra,
        "kiro_provider",
        json_str(fields, &["kiroProvider", "provider", "idp"]),
    );
    extra_set(
        &mut session.extra,
        "profile_arn",
        json_str(fields, &["profileArn", "profile_arn"]),
    );
    extra_set(
        &mut session.extra,
        "client_id",
        json_str(fields, &["clientId", "client_id"]),
    );
    extra_set(
        &mut session.extra,
        "client_secret",
        json_str(fields, &["clientSecret", "client_secret"]),
    );
    extra_set(
        &mut session.extra,
        "start_url",
        json_str(fields, &["startUrl", "start_url"]),
    );
    extra_set(
        &mut session.extra,
        "token_endpoint",
        json_str(fields, &["tokenEndpoint", "token_endpoint"]),
    );
    extra_set(
        &mut session.extra,
        "issuer_url",
        json_str(fields, &["issuerUrl", "issuer_url"]),
    );
    extra_set(&mut session.extra, "scopes", json_str(fields, &["scopes"]));
    extra_set(&mut session.extra, "region", Some(region.clone()));
    extra_set(
        &mut session.extra,
        "auth_region",
        json_str(fields, &["authRegion", "auth_region"]).or(Some(region.clone())),
    );
    extra_set(
        &mut session.extra,
        "api_region",
        json_str(fields, &["apiRegion", "api_region"]).or(Some(region)),
    );
    let seed = if method == "api_key" {
        &access
    } else {
        &refresh
    };
    let machine = machine_id(
        json_str(fields, &["machineId", "machine_id"]).as_deref(),
        &method,
        seed,
    );
    extra_set(&mut session.extra, "machine_id", Some(machine));
    if session.account.is_empty() {
        session.account = method_label(&session).to_owned();
    }
    Ok(session)
}

pub fn apply_refreshed_tokens(
    session: &Session,
    body: &Value,
    method: &str,
    now_ms: i64,
) -> Result<Session, Error> {
    let access = json_str(body, &["accessToken", "access_token"])
        .ok_or_else(|| Error::Payload("kiro refresh returned no access_token".into()))?;
    let refresh = json_str(body, &["refreshToken", "refresh_token"])
        .unwrap_or_else(|| session.refresh_token.clone());
    let mut next = session.clone();
    next.access_token = access;
    next.refresh_token = refresh;
    next.expires_at_ms = refresh_expires_at_ms(body, now_ms);
    extra_set(&mut next.extra, "auth_method", Some(method.to_owned()));
    if let Some(arn) = json_str(body, &["profileArn", "profile_arn"]) {
        extra_set(&mut next.extra, "profile_arn", Some(arn));
    }
    Ok(next)
}

#[must_use]
pub fn is_kiro_credential(raw: &Value) -> bool {
    if !raw.is_object() || raw.is_array() {
        return false;
    }
    let nested = nested_credentials(raw).unwrap_or(raw);
    if json_str(nested, &["kiroApiKey", "kiro_api_key"])
        .or_else(|| json_str(raw, &["kiroApiKey", "kiro_api_key"]))
        .is_some()
    {
        return true;
    }
    if json_str(nested, &["authMethod", "auth_method"])
        .or_else(|| json_str(raw, &["authMethod", "auth_method"]))
        .is_some()
    {
        return true;
    }
    if json_str(nested, &["profileArn", "profile_arn"])
        .or_else(|| json_str(raw, &["profileArn", "profile_arn"]))
        .is_some_and(|arn| arn.contains("codewhisperer"))
    {
        return true;
    }
    if json_str(nested, &["tokenEndpoint", "token_endpoint"])
        .or_else(|| json_str(raw, &["tokenEndpoint", "token_endpoint"]))
        .is_some_and(|ep| ep.contains("microsoftonline"))
    {
        return true;
    }
    let start = json_str(nested, &["startUrl", "start_url"])
        .or_else(|| json_str(raw, &["startUrl", "start_url"]));
    let client = json_str(nested, &["clientId", "client_id"])
        .or_else(|| json_str(raw, &["clientId", "client_id"]));
    if start.is_some_and(|s| s.to_ascii_lowercase().contains("awsapps.com/start"))
        && client.is_some()
    {
        return true;
    }
    let provider = json_str(nested, &["provider", "idp"])
        .or_else(|| json_str(raw, &["provider", "idp"]))
        .unwrap_or_default()
        .to_ascii_lowercase();
    if SOCIAL_PROVIDERS.contains(&provider.as_str())
        || IDC_PROVIDERS.contains(&provider.as_str())
        || EXTERNAL_IDP_ALIASES.contains(&provider.as_str())
    {
        return true;
    }
    let refresh = json_str(nested, &["refreshToken", "refresh_token"])
        .or_else(|| json_str(raw, &["refreshToken", "refresh_token"]));
    if refresh.as_ref().is_some_and(|rt| rt.len() >= 100)
        && (json_str(raw, &["email"]).is_some()
            || json_str(nested, &["email"]).is_some()
            || json_str(nested, &["accessToken", "access_token"]).is_some()
            || json_str(nested, &["clientId", "client_id"]).is_some())
    {
        return true;
    }
    false
}

pub fn unwrap_credential(raw: &Value) -> Value {
    let Some(nested) = nested_credentials(raw) else {
        return raw.clone();
    };
    let mut out = nested.clone();
    if let Some(obj) = out.as_object_mut() {
        if let Some(email) = json_str(raw, &["email"]).or_else(|| json_str(nested, &["email"])) {
            obj.entry("email".to_owned())
                .or_insert(Value::String(email));
        }
        if let Some(account) = json_str(raw, &["account"])
            .or_else(|| json_str(nested, &["account"]))
            .or_else(|| json_str(raw, &["email"]))
        {
            obj.entry("account".to_owned())
                .or_insert(Value::String(account));
        }
        if obj.get("provider").and_then(Value::as_str).is_none()
            && let Some(provider) =
                json_str(nested, &["provider"]).or_else(|| json_str(raw, &["idp", "provider"]))
        {
            obj.insert("provider".to_owned(), Value::String(provider));
        }
        if obj.get("authMethod").and_then(Value::as_str).is_none()
            && obj.get("auth_method").and_then(Value::as_str).is_none()
            && let Some(method) = json_str(nested, &["authMethod", "auth_method"])
                .or_else(|| json_str(raw, &["authMethod", "auth_method"]))
        {
            obj.insert("authMethod".to_owned(), Value::String(method));
        }
        let subscription = raw.get("subscription").and_then(Value::as_object);
        if let Some(title) = subscription
            .and_then(|s| s.get("title").and_then(Value::as_str))
            .map(str::to_owned)
            .or_else(|| json_str(raw, &["subscriptionTitle", "subscription_title"]))
        {
            obj.entry("subscriptionTitle".to_owned())
                .or_insert(Value::String(title));
        }
        if let Some(plan) = subscription
            .and_then(|s| s.get("type").and_then(Value::as_str))
            .map(str::to_owned)
            .or_else(|| json_str(raw, &["planType"]))
            .or_else(|| json_str(nested, &["planType"]))
        {
            obj.entry("planType".to_owned())
                .or_insert(Value::String(plan));
        }
        if obj.get("profileArn").and_then(Value::as_str).is_none()
            && let Some(arn) = json_str(nested, &["profileArn", "profile_arn"])
                .or_else(|| json_str(raw, &["profileArn"]))
        {
            obj.insert("profileArn".to_owned(), Value::String(arn));
        }
    }
    out
}

pub fn session_from_import(raw: &Value, now_ms: i64) -> Result<Session, Error> {
    if !raw.is_object() {
        return Err(Error::InvalidImport(
            "kiro credential is not an object".into(),
        ));
    }
    let entry = unwrap_credential(raw);
    let kiro_api_key = json_str(&entry, &["kiroApiKey", "kiro_api_key"]);
    let method = infer_auth_method(&entry);
    if method == "api_key" || kiro_api_key.is_some() {
        let access = json_str(&entry, &["accessToken", "access_token"]);
        let key = validate_api_key(kiro_api_key.as_deref().or(access.as_deref()).unwrap_or(""))?;
        let mut fields = entry;
        if let Some(obj) = fields.as_object_mut() {
            obj.insert("accessToken".to_owned(), Value::String(key));
            obj.insert("authMethod".to_owned(), json!("api_key"));
        }
        return build_session(None, None, "api_key", &fields, now_ms);
    }
    if method == "external_idp" {
        let endpoint = json_str(&entry, &["tokenEndpoint", "token_endpoint"]).unwrap_or_default();
        validate_idp_endpoint(&endpoint)?;
    }
    if let Some(refresh) = json_str(&entry, &["refreshToken", "refresh_token"]) {
        validate_refresh_token(&refresh)?;
    }
    let mut fields = entry;
    if method == "idc"
        && json_str(&fields, &["startUrl", "start_url"]).is_none()
        && let Some(obj) = fields.as_object_mut()
    {
        obj.insert("startUrl".to_owned(), json!(BUILDER_ID_START_URL));
    }
    build_session(None, None, &method, &fields, now_ms)
}

#[must_use]
pub fn has_access_token_field(raw: &Value) -> bool {
    let nested = nested_credentials(raw).unwrap_or(raw);
    json_str(
        nested,
        &["accessToken", "access_token", "kiroApiKey", "kiro_api_key"],
    )
    .or_else(|| {
        json_str(
            raw,
            &["accessToken", "access_token", "kiroApiKey", "kiro_api_key"],
        )
    })
    .is_some()
}
