//! AWS SSO OIDC register + device_authorization (Builder ID / enterprise IdC).

use std::future::Future;

use serde_json::{Map, Value, json};

use crate::{Error, StartOutcome};

use super::net::{self, Body, Posted};
use super::session::{json_str, oidc_endpoint, trimmed};
use super::{BUILDER_ID_START_URL, CLIENT_NAME, DEFAULT_REGION, DEVICE_GRANT, OIDC_SCOPES};

#[cfg(test)]
mod tests;

const DEFAULT_INTERVAL_SEC: i64 = 5;
const DEFAULT_EXPIRES_IN_SEC: i64 = 900;

#[derive(Debug, Clone)]
pub struct RegisteredClient {
    pub client_id: String,
    pub client_secret: String,
    pub start_url: String,
    pub region: String,
}

#[must_use]
pub fn register_body(start_url: &str) -> Value {
    json!({
        "clientName": CLIENT_NAME,
        "clientType": "public",
        "scopes": OIDC_SCOPES,
        "grantTypes": [DEVICE_GRANT, "refresh_token"],
        "issuerUrl": start_url,
    })
}

#[must_use]
pub fn device_authorization_body(client_id: &str, client_secret: &str, start_url: &str) -> Value {
    json!({
        "clientId": client_id,
        "clientSecret": client_secret,
        "startUrl": start_url,
    })
}

pub(crate) async fn register_client<F, Fut>(
    region: &str,
    start_url: &str,
    mut post: F,
) -> Result<RegisteredClient, Error>
where
    F: FnMut(String, Vec<(String, String)>, Body) -> Fut,
    Fut: Future<Output = Result<Posted, Error>>,
{
    let url = format!("{}/client/register", oidc_endpoint(region));
    let posted = post(url, json_headers(), Body::Json(register_body(start_url))).await?;
    if !posted.is_ok() {
        return Err(net::token_error(&posted, "kiro oidc register"));
    }
    let client_id = json_str(&posted.json, &["clientId", "client_id"]).ok_or_else(|| {
        Error::Payload("kiro oidc register returned no clientId/clientSecret".into())
    })?;
    let client_secret =
        json_str(&posted.json, &["clientSecret", "client_secret"]).ok_or_else(|| {
            Error::Payload("kiro oidc register returned no clientId/clientSecret".into())
        })?;
    Ok(RegisteredClient {
        client_id,
        client_secret,
        start_url: start_url.to_owned(),
        region: region.to_owned(),
    })
}

fn json_headers() -> Vec<(String, String)> {
    vec![
        ("accept".to_owned(), "application/json".to_owned()),
        ("content-type".to_owned(), "application/json".to_owned()),
    ]
}

fn as_positive_i64(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    if let Some(n) = value.as_i64().filter(|n| *n > 0) {
        return Some(n);
    }
    if let Some(n) = value.as_f64().filter(|n| n.is_finite() && *n > 0.0) {
        return Some(n.round() as i64);
    }
    None
}

pub(crate) async fn start_device<F, Fut>(
    region: &str,
    start_url: &str,
    enterprise: bool,
    state: &str,
    mut post: F,
) -> Result<StartOutcome, Error>
where
    F: FnMut(String, Vec<(String, String)>, Body) -> Fut,
    Fut: Future<Output = Result<Posted, Error>>,
{
    let region = if region.trim().is_empty() {
        DEFAULT_REGION
    } else {
        region.trim()
    };
    let issuer = trimmed(Some(start_url)).unwrap_or_else(|| BUILDER_ID_START_URL.to_owned());
    if !issuer.starts_with("https://") {
        return Err(Error::Payload("Kiro start URL must be https".into()));
    }
    let registered = register_client(region, &issuer, &mut post).await?;
    let url = format!("{}/device_authorization", oidc_endpoint(region));
    let posted = post(
        url,
        json_headers(),
        Body::Json(device_authorization_body(
            &registered.client_id,
            &registered.client_secret,
            &issuer,
        )),
    )
    .await?;
    if !posted.is_ok() {
        return Err(net::token_error(&posted, "kiro device authorization"));
    }
    let device_code = json_str(&posted.json, &["deviceCode", "device_code"]).ok_or_else(|| {
        Error::Payload(
            "kiro device authorization is missing deviceCode/userCode/verificationUri".into(),
        )
    })?;
    let user_code = json_str(&posted.json, &["userCode", "user_code"]).ok_or_else(|| {
        Error::Payload(
            "kiro device authorization is missing deviceCode/userCode/verificationUri".into(),
        )
    })?;
    let verification_uri = json_str(&posted.json, &["verificationUri", "verification_uri"])
        .ok_or_else(|| {
            Error::Payload(
                "kiro device authorization is missing deviceCode/userCode/verificationUri".into(),
            )
        })?;
    let complete = json_str(
        &posted.json,
        &["verificationUriComplete", "verification_uri_complete"],
    )
    .unwrap_or_else(|| verification_uri.clone());
    let interval = as_positive_i64(posted.json.get("interval")).unwrap_or(DEFAULT_INTERVAL_SEC);
    let expires_in = as_positive_i64(posted.json.get("expiresIn"))
        .or_else(|| as_positive_i64(posted.json.get("expires_in")))
        .unwrap_or(DEFAULT_EXPIRES_IN_SEC);
    let kind = if enterprise || issuer != BUILDER_ID_START_URL {
        "idc"
    } else {
        "builder"
    };
    let mut extra = Map::new();
    extra.insert("client_id".to_owned(), json!(registered.client_id));
    extra.insert("client_secret".to_owned(), json!(registered.client_secret));
    extra.insert("device_code".to_owned(), json!(device_code));
    extra.insert("start_url".to_owned(), json!(issuer));
    extra.insert("region".to_owned(), json!(region));
    extra.insert("auth_method".to_owned(), json!("idc"));
    extra.insert("kind".to_owned(), json!(kind));
    extra.insert(
        "kiro_provider".to_owned(),
        json!(if kind == "builder" {
            "BuilderId"
        } else {
            "Enterprise"
        }),
    );
    extra.insert(
        "token_endpoint".to_owned(),
        json!(format!("{}/token", oidc_endpoint(region))),
    );
    extra.insert("expires_in".to_owned(), json!(expires_in));
    extra.insert("grant_type".to_owned(), json!(DEVICE_GRANT));
    Ok(StartOutcome::Device {
        state: state.to_owned(),
        user_code,
        verification_uri,
        verification_uri_complete: complete,
        interval_secs: interval,
        extra,
    })
}

pub(crate) fn idc_refresh_body(client_id: &str, client_secret: &str, refresh_token: &str) -> Value {
    json!({
        "clientId": client_id,
        "clientSecret": client_secret,
        "refreshToken": refresh_token,
        "grantType": "refresh_token",
    })
}

pub(crate) fn idc_refresh_headers() -> Vec<(String, String)> {
    vec![
        ("accept".to_owned(), "application/json".to_owned()),
        ("content-type".to_owned(), "application/json".to_owned()),
        (
            "user-agent".to_owned(),
            "aws-sdk-js/3.980.0 KiroIDE".to_owned(),
        ),
        (
            "x-amz-user-agent".to_owned(),
            "aws-sdk-js/3.980.0 KiroIDE".to_owned(),
        ),
    ]
}
