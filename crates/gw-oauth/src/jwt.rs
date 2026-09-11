//! Decode a JWT payload without verifying the signature.
//!
//! Used only to read account claims from tokens that just arrived over TLS
//! from the provider's own token endpoint. Never used to authorize.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Map, Value};

#[cfg(test)]
mod tests;

/// Decode the payload object. `None` when the token is not three base64url
/// segments or the payload is not a JSON object.
#[must_use]
pub fn decode_payload(token: &str) -> Option<Map<String, Value>> {
    if token.is_empty() {
        return None;
    }
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: Value = serde_json::from_slice(&decoded).ok()?;
    match value {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// Email and account-id claims used as display labels on a stored session.
#[must_use]
pub fn email_and_account(token: &str) -> (String, String) {
    let Some(claims) = decode_payload(token) else {
        return (String::new(), String::new());
    };
    let text = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned()
    };
    let email = text(claims.get("email"));
    let account_id = claims
        .get("https://api.openai.com/auth")
        .and_then(Value::as_object)
        .map(|auth| text(auth.get("account_id")))
        .filter(|value| !value.is_empty())
        .or_else(|| Some(text(claims.get("account_id"))).filter(|value| !value.is_empty()))
        .unwrap_or_else(|| text(claims.get("sub")));
    (email, account_id)
}
