//! AWS Kiro (Social / Builder ID / IdC / Entra / API key).

use std::future::Future;

use serde_json::{Map, Value, json};

use crate::login::StartInput;
use crate::{Error, FlowKind, Session, StartOutcome};

pub mod cache;
pub mod catalog;
pub mod idc;
pub mod import;
mod net;
pub mod request;
pub mod session;

pub use cache::{
    KIRO_STABLE_SESSION, apply_cache, cache_session_id, conversation_id, pin_system_prefix,
    reset_system_pins,
};
pub use request::{SYSTEM_ACK, classify_hop_error, openai_to_kiro};
pub use session::{
    canonicalize_method, social_authorize_url, social_redirect_uri, social_token_redirect_uri,
};

use net::{Body, Posted};
use session::{
    allocate_machine_id, apply_refreshed_tokens, extra_str, json_str, now_ms, oidc_endpoint,
    social_refresh_host, social_user_agent, validate_idp_endpoint, validate_refresh_token,
};

#[cfg(test)]
mod tests;

pub const ID: &str = "kiro";
pub const DEFAULT_REGION: &str = "us-east-1";
pub const SOCIAL_TOKEN_URL: &str = "https://prod.us-east-1.auth.desktop.kiro.dev/oauth/token";
pub const PORTAL_URL: &str = "https://app.kiro.dev";
pub const BUILDER_ID_START_URL: &str = "https://view.awsapps.com/start";
pub const BUILDER_ID_PROFILE_ARN: &str =
    "arn:aws:codewhisperer:us-east-1:638616132270:profile/AAAACCCCXXXX";
pub const SOCIAL_PROFILE_ARN: &str =
    "arn:aws:codewhisperer:us-east-1:699475941385:profile/EHGA3GRVQMUK";
pub const USAGE_VERSION: &str = "1.0.0";
pub const CLIENT_NAME: &str = "AI-GateWay";
pub const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
pub const CALLBACK_PATHS: &[&str] = &["/", "/oauth/callback", "/signin/callback"];
pub const OIDC_SCOPES: &[&str] = &[
    "codewhisperer:completions",
    "codewhisperer:analysis",
    "codewhisperer:conversations",
    "codewhisperer:transformations",
    "codewhisperer:taskassist",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartKind {
    Device { enterprise: bool },
    Social,
    Import,
}

fn start_kind(input: &StartInput) -> Result<StartKind, Error> {
    let from_body = input
        .body
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("");
    let raw = if input.method.trim().is_empty() {
        from_body
    } else {
        input.method.as_str()
    };
    Ok(match raw.trim().to_ascii_lowercase().as_str() {
        "" | "builder-id" | "builder_id" | "builderid" | "builder" | "device" | "iam" => {
            StartKind::Device { enterprise: false }
        }
        "idc" | "enterprise" | "sso" => StartKind::Device { enterprise: true },
        "authcode" | "auth-code" | "authorization_code" | "authorization-code" | "social"
        | "oauth" | "github" | "google" | "gmail" | "gh" => StartKind::Social,
        "import" | "api_key" | "apikey" | "ksk" | "paste" => StartKind::Import,
        _ => return Err(Error::UnsupportedFlow),
    })
}

fn region_of(input: &StartInput) -> String {
    json_str(&input.body, &["region", "auth_region", "authRegion"])
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_REGION.to_owned())
}

fn start_url_of(input: &StartInput) -> String {
    json_str(&input.body, &["start_url", "startUrl"]).unwrap_or_default()
}

async fn live_post(
    url: String,
    headers: Vec<(String, String)>,
    body: Body,
) -> Result<Posted, Error> {
    let refs: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    net::send(&url, &refs, &body).await
}

/// Social PKCE / IdC device / import.
pub async fn start(input: &StartInput) -> Result<StartOutcome, Error> {
    start_with(input, live_post).await
}

pub(crate) async fn start_with<F, Fut>(
    input: &StartInput,
    mut post: F,
) -> Result<StartOutcome, Error>
where
    F: FnMut(String, Vec<(String, String)>, Body) -> Fut,
    Fut: Future<Output = Result<Posted, Error>>,
{
    match start_kind(input)? {
        StartKind::Device { enterprise } => {
            idc::start_device(
                &region_of(input),
                &start_url_of(input),
                enterprise,
                &input.state,
                &mut post,
            )
            .await
        }
        StartKind::Social => start_social(input),
        StartKind::Import => {
            let session = import::imported_session(&input.body)?;
            Ok(StartOutcome::Ready(session))
        }
    }
}

fn start_social(input: &StartInput) -> Result<StartOutcome, Error> {
    if input.redirect_uri.trim().is_empty() {
        return Err(Error::Payload(
            "Kiro authorization-code flow needs a redirect_uri".into(),
        ));
    }
    let pkce = crate::create_pkce()?;
    let authorize_url = social_authorize_url(&input.state, &pkce.challenge, &input.redirect_uri)?;
    let origin = social_redirect_uri(&input.redirect_uri)?;
    let prior = json_str(&input.body, &["machine_id", "machineId"]);
    let machine = allocate_machine_id(prior.as_deref());
    let mut extra = Map::new();
    extra.insert("machine_id".to_owned(), json!(machine));
    extra.insert("auth_method".to_owned(), json!("social"));
    extra.insert("callback_paths".to_owned(), json!(CALLBACK_PATHS));
    extra.insert("authorize_redirect_uri".to_owned(), json!(origin.clone()));
    Ok(StartOutcome::Browser {
        authorize_url,
        state: input.state.clone(),
        verifier: pkce.verifier,
        redirect_uri: origin,
        flow: FlowKind::AuthorizationCode,
        extra,
    })
}

/// Exchange a portal authorization code. `redirect_uri` for the token hop is
/// origin + landed path + `login_option`.
pub async fn exchange_social_code(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    pathname: Option<&str>,
    login_option: Option<&str>,
    machine_id: Option<&str>,
) -> Result<Session, Error> {
    exchange_social_code_with(
        code,
        verifier,
        redirect_uri,
        pathname,
        login_option,
        machine_id,
        live_post,
    )
    .await
}

pub(crate) async fn exchange_social_code_with<F, Fut>(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    pathname: Option<&str>,
    login_option: Option<&str>,
    machine_id: Option<&str>,
    mut post: F,
) -> Result<Session, Error>
where
    F: FnMut(String, Vec<(String, String)>, Body) -> Fut,
    Fut: Future<Output = Result<Posted, Error>>,
{
    if verifier.trim().is_empty() {
        return Err(Error::MissingVerifier);
    }
    let machine = allocate_machine_id(machine_id);
    let token_redirect = social_token_redirect_uri(redirect_uri, pathname, login_option)?;
    let posted = post(
        SOCIAL_TOKEN_URL.to_owned(),
        vec![
            (
                "accept".to_owned(),
                "application/json, text/plain, */*".to_owned(),
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
            ("user-agent".to_owned(), social_user_agent(&machine)),
        ],
        Body::Json(json!({
            "code": code,
            "code_verifier": verifier,
            "redirect_uri": token_redirect,
        })),
    )
    .await?;
    if !posted.is_ok() {
        return Err(net::token_error(&posted, "kiro social token"));
    }
    let access = json_str(&posted.json, &["accessToken", "access_token"]).ok_or_else(|| {
        Error::Payload("kiro social token exchange returned no access_token".into())
    })?;
    let mut fields = posted.json.clone();
    if let Some(obj) = fields.as_object_mut() {
        obj.insert("authMethod".to_owned(), json!("social"));
        obj.insert("kiroProvider".to_owned(), json!("Social"));
        obj.insert("machineId".to_owned(), json!(machine));
        obj.entry("profileArn").or_insert(json!(SOCIAL_PROFILE_ARN));
        obj.insert("accessToken".to_owned(), json!(access));
    }
    session::build_session(None, None, "social", &fields, now_ms())
}

/// Refresh must rewrite `expiresAt` from the refresh JSON (never the stored stamp).
pub async fn refresh(session: &Session) -> Result<Session, Error> {
    refresh_with(session, now_ms(), live_post).await
}

pub(crate) async fn refresh_with<F, Fut>(
    session: &Session,
    now: i64,
    mut post: F,
) -> Result<Session, Error>
where
    F: FnMut(String, Vec<(String, String)>, Body) -> Fut,
    Fut: Future<Output = Result<Posted, Error>>,
{
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
    if method == "api_key" {
        return Ok(session.clone());
    }
    match method.as_str() {
        "external_idp" => refresh_entra(session, now, &mut post).await,
        "idc" => refresh_idc(session, now, &mut post).await,
        _ => refresh_social(session, now, &mut post).await,
    }
}

async fn refresh_social<F, Fut>(session: &Session, now: i64, post: &mut F) -> Result<Session, Error>
where
    F: FnMut(String, Vec<(String, String)>, Body) -> Fut,
    Fut: Future<Output = Result<Posted, Error>>,
{
    let refresh_token = validate_refresh_token(&session.refresh_token)?;
    let region = extra_str(session, "auth_region");
    let region = if region.is_empty() {
        extra_str(session, "region")
    } else {
        region
    };
    let url = format!("https://{}/refreshToken", social_refresh_host(&region));
    let machine = extra_str(session, "machine_id");
    let posted = post(
        url,
        vec![
            (
                "accept".to_owned(),
                "application/json, text/plain, */*".to_owned(),
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
            ("user-agent".to_owned(), social_user_agent(&machine)),
        ],
        Body::Json(json!({ "refreshToken": refresh_token })),
    )
    .await?;
    if !posted.is_ok() {
        return Err(net::token_error(&posted, "kiro social refresh"));
    }
    apply_refreshed_tokens(session, &posted.json, "social", now)
}

async fn refresh_idc<F, Fut>(session: &Session, now: i64, post: &mut F) -> Result<Session, Error>
where
    F: FnMut(String, Vec<(String, String)>, Body) -> Fut,
    Fut: Future<Output = Result<Posted, Error>>,
{
    let refresh_token = validate_refresh_token(&session.refresh_token)?;
    let region = extra_str(session, "auth_region");
    let region = if region.is_empty() {
        extra_str(session, "region")
    } else {
        region
    };
    let url = format!("{}/token", oidc_endpoint(&region));
    let posted = post(
        url,
        idc::idc_refresh_headers(),
        Body::Json(idc::idc_refresh_body(
            &extra_str(session, "client_id"),
            &extra_str(session, "client_secret"),
            &refresh_token,
        )),
    )
    .await?;
    if !posted.is_ok() {
        return Err(net::token_error(&posted, "kiro idc refresh"));
    }
    apply_refreshed_tokens(session, &posted.json, "idc", now)
}

async fn refresh_entra<F, Fut>(session: &Session, now: i64, post: &mut F) -> Result<Session, Error>
where
    F: FnMut(String, Vec<(String, String)>, Body) -> Fut,
    Fut: Future<Output = Result<Posted, Error>>,
{
    let endpoint = validate_idp_endpoint(&extra_str(session, "token_endpoint"))?;
    let refresh_token = validate_refresh_token(&session.refresh_token)?;
    let client_id = extra_str(session, "client_id");
    if client_id.is_empty() {
        return Err(Error::Payload(
            "kiro enterprise SSO needs a client id".into(),
        ));
    }
    let mut params = vec![
        ("client_id", client_id),
        ("grant_type", "refresh_token".to_owned()),
        ("refresh_token", refresh_token),
    ];
    let scopes = extra_str(session, "scopes");
    if !scopes.is_empty() {
        params.push(("scope", scopes));
    }
    let posted = post(
        endpoint,
        vec![
            ("accept".to_owned(), "application/json".to_owned()),
            (
                "content-type".to_owned(),
                "application/x-www-form-urlencoded".to_owned(),
            ),
        ],
        Body::Form(crate::form::encode(&params)),
    )
    .await?;
    if !posted.is_ok() {
        return Err(net::token_error(&posted, "kiro enterprise SSO refresh"));
    }
    apply_refreshed_tokens(session, &posted.json, "external_idp", now)
}
