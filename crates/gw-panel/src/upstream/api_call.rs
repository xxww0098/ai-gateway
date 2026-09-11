//! `POST /api-call` — operator-initiated proxy to a provider quota endpoint.
//!
//! The console already knows how to parse Claude / Codex / Gemini quota JSON.
//! This handler only substitutes the stored credential and forwards the HTTP
//! request to an allow-listed host. Anything else is SSRF.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use gw_authcore::AuthRecord;
use reqwest::redirect::Policy;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::time::Duration;

use super::auth_files::{find_by_name, sorted_records};
use super::record::{api_key, metadata_string};
use crate::{AdminUser, PanelState, err, ok};

#[cfg(test)]
mod tests;

const ERR_BAD_REQUEST: i32 = 4001;
const ERR_NOT_FOUND: i32 = 4040;
const ERR_UPSTREAM: i32 = 5020;

const MAX_BODY: usize = 1 << 20;
const TIMEOUT: Duration = Duration::from_secs(20);

/// Hosts the console is allowed to probe for quota. Exact match, HTTPS only.
const ALLOWED_HOSTS: &[&str] = &[
    "api.anthropic.com",
    "chatgpt.com",
    "cloudcode-pa.googleapis.com",
    "daily-cloudcode-pa.googleapis.com",
];

#[derive(Debug, Default, Deserialize)]
pub struct ApiCallRequest {
    #[serde(default)]
    method: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    header: HashMap<String, String>,
    #[serde(default)]
    data: Option<String>,
    #[serde(default, alias = "authIndex")]
    auth_index: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

/// `POST /api-call`.
pub async fn proxy(
    State(state): State<PanelState>,
    _admin: AdminUser,
    body: Option<axum::Json<ApiCallRequest>>,
) -> Response {
    let Some(axum::Json(req)) = body else {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "invalid JSON body",
        );
    };
    let method = req.method.trim().to_uppercase();
    if method != "GET" && method != "POST" {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "method must be GET or POST",
        );
    }
    let url = match parse_allowed_url(&req.url) {
        Ok(url) => url,
        Err(message) => return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, message),
    };

    let records = match sorted_records(&state).await {
        Ok(records) => records,
        Err(error) => {
            tracing::error!(%error, "failed to list credentials for api-call");
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                ERR_UPSTREAM,
                "failed to list credentials",
            );
        }
    };
    let target = req
        .auth_index
        .as_deref()
        .or(req.id.as_deref())
        .unwrap_or("")
        .trim();
    let Some(record) = resolve_record(&records, target) else {
        return err(StatusCode::NOT_FOUND, ERR_NOT_FOUND, "auth file not found");
    };

    let token = bearer_token(record);
    if token.is_empty() {
        return err(
            StatusCode::BAD_REQUEST,
            ERR_BAD_REQUEST,
            "credential has no access token or api key",
        );
    }

    let mut headers = req.header;
    fill_token(&mut headers, &token);

    let client = match http_client(record) {
        Ok(client) => client,
        Err(message) => return err(StatusCode::BAD_REQUEST, ERR_BAD_REQUEST, message),
    };

    let mut builder = match method.as_str() {
        "POST" => client.post(url),
        _ => client.get(url),
    };
    for (name, value) in &headers {
        builder = builder.header(name, value);
    }
    if method == "POST"
        && let Some(data) = req.data.as_deref()
    {
        builder = builder.body(data.to_owned());
    }

    let response = match builder.send().await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%error, "quota proxy upstream failed");
            return err(
                StatusCode::BAD_GATEWAY,
                ERR_UPSTREAM,
                "upstream request failed",
            );
        }
    };
    let status_code = response.status().as_u16();
    let mut header_out: Map<String, Value> = Map::new();
    for (name, value) in response.headers() {
        let Ok(text) = value.to_str() else {
            continue;
        };
        header_out
            .entry(name.as_str().to_owned())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .expect("just inserted an array")
            .push(json!(text));
    }
    let raw = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(%error, "quota proxy body read failed");
            return err(
                StatusCode::BAD_GATEWAY,
                ERR_UPSTREAM,
                "upstream body read failed",
            );
        }
    };
    let raw = if raw.len() > MAX_BODY {
        raw.slice(..MAX_BODY)
    } else {
        raw
    };
    let body_text = String::from_utf8_lossy(&raw).into_owned();
    let body_json: Value = serde_json::from_str(&body_text).unwrap_or(Value::Null);

    ok(json!({
        "status_code": status_code,
        "statusCode": status_code,
        "header": header_out,
        "headers": header_out,
        "bodyText": body_text,
        "body": body_json,
    }))
}

/// HTTPS + allow-listed host, nothing else.
pub fn parse_allowed_url(raw: &str) -> Result<reqwest::Url, &'static str> {
    let url = reqwest::Url::parse(raw.trim()).map_err(|_| "invalid url")?;
    if url.scheme() != "https" {
        return Err("url must be https");
    }
    let host = url.host_str().unwrap_or_default();
    if !ALLOWED_HOSTS.contains(&host) {
        return Err("host is not allow-listed");
    }
    if url.username() != "" || url.password().is_some() {
        return Err("url must not carry credentials");
    }
    Ok(url)
}

/// Replaces `$TOKEN$` in header values with the stored secret.
pub fn fill_token(headers: &mut HashMap<String, String>, token: &str) {
    for value in headers.values_mut() {
        if value.contains("$TOKEN$") {
            *value = value.replace("$TOKEN$", token);
        }
    }
}

/// Access token first (OAuth), then API key.
#[must_use]
pub fn bearer_token(record: &AuthRecord) -> String {
    let access = metadata_string(record, "access_token");
    if !access.is_empty() {
        return access;
    }
    api_key(record)
}

fn resolve_record<'a>(records: &'a [AuthRecord], target: &str) -> Option<&'a AuthRecord> {
    if target.is_empty() {
        return None;
    }
    if let Some(found) = find_by_name(records, target) {
        return Some(found);
    }
    if let Ok(index) = target.parse::<usize>() {
        return records.get(index);
    }
    None
}

fn http_client(record: &AuthRecord) -> Result<reqwest::Client, &'static str> {
    let mut builder = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(Policy::none());
    if !record.proxy_url.trim().is_empty() {
        let proxy =
            reqwest::Proxy::all(record.proxy_url.trim()).map_err(|_| "invalid proxy_url")?;
        builder = builder.proxy(proxy);
    }
    builder.build().map_err(|_| "failed to build http client")
}
