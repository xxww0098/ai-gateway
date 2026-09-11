//! RFC 8628 device-authorization for Grok. Default login; no loopback.

use serde_json::{Map, Value, json};

use super::{CLIENT_ID, SCOPE, post_form, session_from_tokens, token_url_or_err};
use crate::{Error, Session, StartOutcome};

#[cfg(test)]
mod tests;

const DEFAULT_INTERVAL_SECS: i64 = 5;
const DEFAULT_EXPIRES_IN_SECS: i64 = 900;
const SLOW_DOWN_BUMP_SECS: i64 = 5;
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// One RFC 8628 token-poll step.
#[derive(Debug, Clone)]
pub enum PollOutcome {
    Pending { interval_secs: i64 },
    SlowDown { interval_secs: i64 },
    Ready(Session),
}

/// Device-code start against already-validated endpoints.
pub(crate) async fn request_device(
    client: &reqwest::Client,
    device_url: &str,
    token_url: &str,
    state: &str,
) -> Result<StartOutcome, Error> {
    token_url_or_err(device_url)?;
    token_url_or_err(token_url)?;
    let (status, body) = post_form(
        client,
        device_url,
        &[
            ("client_id", CLIENT_ID.to_owned()),
            ("scope", SCOPE.to_owned()),
        ],
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(Error::TokenEndpoint { status, body });
    }
    let started = parse_device_start(&body)?;
    Ok(device_outcome(state, token_url, started))
}

/// One token poll. `authorization_pending` / `slow_down` keep waiting;
/// `invalid_grant` is not permanent here (the device code is still live).
///
/// # Errors
/// Non-x.ai token URL, transport, denied / expired / malformed token body.
pub async fn poll(
    token_endpoint: &str,
    device_code: &str,
    interval_secs: i64,
) -> Result<PollOutcome, Error> {
    let client = super::http_client()?;
    poll_at(&client, token_endpoint, device_code, interval_secs).await
}

pub(crate) async fn poll_at(
    client: &reqwest::Client,
    token_endpoint: &str,
    device_code: &str,
    interval_secs: i64,
) -> Result<PollOutcome, Error> {
    let url = token_url_or_err(token_endpoint)?;
    if device_code.trim().is_empty() {
        return Err(Error::Payload(
            "grok device poll is missing device_code".to_owned(),
        ));
    }
    let (status, body) = post_form(
        client,
        &url,
        &[
            ("client_id", CLIENT_ID.to_owned()),
            ("device_code", device_code.to_owned()),
            ("grant_type", DEVICE_GRANT.to_owned()),
        ],
    )
    .await?;
    match interpret_poll(status, &body, interval_secs)? {
        PollStep::Pending { interval_secs } => Ok(PollOutcome::Pending { interval_secs }),
        PollStep::SlowDown { interval_secs } => Ok(PollOutcome::SlowDown { interval_secs }),
        PollStep::Tokens(tokens) => {
            let session = session_from_tokens(&tokens, &url, None)?;
            Ok(PollOutcome::Ready(session))
        }
    }
}

#[derive(Debug)]
pub(crate) enum PollStep {
    Pending { interval_secs: i64 },
    SlowDown { interval_secs: i64 },
    Tokens(Value),
}

/// Classify a token-endpoint status + body. Pure: no I/O.
pub(crate) fn interpret_poll(
    status: u16,
    raw: &str,
    interval_secs: i64,
) -> Result<PollStep, Error> {
    let interval = if interval_secs > 0 {
        interval_secs
    } else {
        DEFAULT_INTERVAL_SECS
    };
    let parsed: Value = serde_json::from_str(raw.trim()).unwrap_or(Value::Null);
    let object = parsed.as_object();
    let error = object
        .and_then(|o| o.get("error"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    match error {
        "authorization_pending" => Ok(PollStep::Pending {
            interval_secs: interval,
        }),
        "slow_down" => Ok(PollStep::SlowDown {
            interval_secs: interval.saturating_add(SLOW_DOWN_BUMP_SECS),
        }),
        "access_denied" | "expired_token" => Err(Error::TokenEndpoint {
            status,
            body: raw.to_owned(),
        }),
        "" => {
            let access = object
                .and_then(|o| o.get("access_token").or_else(|| o.get("accessToken")))
                .and_then(Value::as_str)
                .unwrap_or("");
            if access.is_empty() {
                if (200..300).contains(&status) {
                    return Err(Error::Payload(
                        "grok token endpoint returned no access token".to_owned(),
                    ));
                }
                return Err(Error::TokenEndpoint {
                    status,
                    body: raw.to_owned(),
                });
            }
            Ok(PollStep::Tokens(parsed))
        }
        _ => Err(Error::TokenEndpoint {
            status,
            body: raw.to_owned(),
        }),
    }
}

#[derive(Debug)]
struct DeviceStart {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    interval_secs: i64,
    expires_in: i64,
}

fn parse_device_start(body: &str) -> Result<DeviceStart, Error> {
    let parsed: Value = serde_json::from_str(body)
        .map_err(|_| Error::Payload("grok device-code response is not JSON".to_owned()))?;
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| parsed.get(*key).and_then(Value::as_str))
            .unwrap_or("")
            .trim()
            .to_owned()
    };
    let device_code = text(&["device_code", "deviceCode"]);
    let user_code = text(&["user_code", "userCode"]);
    let verification_uri = text(&["verification_uri", "verificationUri"]);
    if device_code.is_empty() || user_code.is_empty() || verification_uri.is_empty() {
        return Err(Error::Payload(
            "grok device-code response is missing device_code/user_code/verification_uri"
                .to_owned(),
        ));
    }
    let mut complete = text(&["verification_uri_complete", "verificationUriComplete"]);
    if complete.is_empty() {
        complete = verification_uri.clone();
    }
    Ok(DeviceStart {
        device_code,
        user_code,
        verification_uri,
        verification_uri_complete: complete,
        interval_secs: positive_secs(&parsed, "interval").unwrap_or(DEFAULT_INTERVAL_SECS),
        expires_in: positive_secs(&parsed, "expires_in")
            .or_else(|| positive_secs(&parsed, "expiresIn"))
            .unwrap_or(DEFAULT_EXPIRES_IN_SECS),
    })
}

fn positive_secs(value: &Value, key: &str) -> Option<i64> {
    let n = value.get(key)?;
    let secs = n
        .as_i64()
        .or_else(|| n.as_u64().and_then(|v| i64::try_from(v).ok()))
        .or_else(|| n.as_f64().map(|f| f.round() as i64))?;
    (secs > 0).then_some(secs)
}

fn device_outcome(state: &str, token_url: &str, started: DeviceStart) -> StartOutcome {
    let mut extra = Map::new();
    extra.insert("device_code".to_owned(), json!(started.device_code));
    extra.insert("token_endpoint".to_owned(), json!(token_url));
    extra.insert("expires_in".to_owned(), json!(started.expires_in));
    extra.insert("client_id".to_owned(), json!(CLIENT_ID));
    extra.insert("interval".to_owned(), json!(started.interval_secs));
    extra.insert("grant_type".to_owned(), json!(DEVICE_GRANT));
    StartOutcome::Device {
        state: state.to_owned(),
        user_code: started.user_code,
        verification_uri: started.verification_uri,
        verification_uri_complete: started.verification_uri_complete,
        interval_secs: started.interval_secs,
        extra,
    }
}
