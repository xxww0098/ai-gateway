//! Turning an authorization code into a stored credential.
//!
//! 对应 `sdkMgmtExchangeOAuthToken`、`sdkMgmtPostFormToken`、
//! `sdkMgmtPostJSONToken`、`sdkMgmtDoTokenRequest`、`sdkMgmtFetchOAuthEmail`、
//! `sdkMgmtClaudeCodeAndState`、`sdkMgmtClaimsFromJWT`、
//! `sdkMgmtOAuthAuthRecord` 和 `sdkMgmtGeminiTokenMetadata`。
//!
//! This is the only outbound-HTTP code in the panel. The two timeouts are
//! deliberate and they differ on purpose: a token exchange is on the operator's critical
//! path and gets 30s, while the userinfo lookup is cosmetic and gets 15s —
//! failing it costs a display name, not the credential.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, Utc};
use gw_authcore::AuthRecord;
use serde_json::{Map, Value, json};

use super::flow::{
    self, CLAUDE_CLIENT_ID, CLAUDE_TOKEN_URL, CODEX_CLIENT_ID, CODEX_TOKEN_URL, GEMINI_CLIENT_ID,
    GEMINI_SCOPES, GEMINI_TOKEN_URL, GEMINI_USERINFO_URL, Provider,
};
use super::{SessionConfig, rfc3339};

#[cfg(test)]
mod tests;

/// Token exchange budget（对应 30 秒超时）。
const TOKEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Userinfo budget（对应 15 秒超时）。
const USERINFO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// What a provider's token endpoint returned.
#[derive(Debug, Default, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub token_type: String,
    pub expires_in: i64,
    /// The whole body, because Claude carries the account email in a nested
    /// object the typed fields do not cover.
    pub raw: Map<String, Value>,
    pub email: String,
    pub account_id: String,
    /// Provider-specific extras (Kiro client_id/secret, xAI base_url, …).
    pub extra: Map<String, Value>,
}

/// Exchanges an authorization code for tokens.
/// 对应 `sdkMgmtExchangeOAuthToken`。
///
/// # Errors
/// Any transport failure, non-2xx status, unparseable body, or — for the two
/// PKCE providers — a session that somehow lost its verifier.
pub async fn exchange(
    provider: Provider,
    code: &str,
    config: &SessionConfig,
) -> anyhow::Result<TokenResponse> {
    match provider {
        Provider::Gemini => {
            let mut tokens = post_form(
                GEMINI_TOKEN_URL,
                &[
                    ("grant_type", "authorization_code".to_owned()),
                    ("client_id", GEMINI_CLIENT_ID.to_owned()),
                    ("code", code.to_owned()),
                    ("redirect_uri", config.redirect_uri.clone()),
                ],
            )
            .await?;
            tokens.email = fetch_email(GEMINI_USERINFO_URL, &tokens.access_token).await;
            Ok(tokens)
        }
        Provider::Claude => {
            anyhow::ensure!(
                !config.code_verifier.trim().is_empty(),
                "missing PKCE verifier"
            );
            // Claude's console pastes back `code#state`; the fragment after the
            // `#` is the state it wants echoed, which may differ from ours.
            let (code, callback_state) = split_claude_code(code);
            let state = if callback_state.is_empty() {
                config.state.clone()
            } else {
                callback_state
            };
            let tokens = post_json(
                CLAUDE_TOKEN_URL,
                &json!({
                    "code": code,
                    "state": state,
                    "grant_type": "authorization_code",
                    "client_id": CLAUDE_CLIENT_ID,
                    "redirect_uri": config.redirect_uri,
                    "code_verifier": config.code_verifier,
                }),
            )
            .await?;
            let email = tokens
                .raw
                .get("account")
                .and_then(Value::as_object)
                .and_then(|account| account.get("email_address"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned();
            let account_id = tokens
                .raw
                .get("organization_uuid")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned();
            Ok(TokenResponse {
                email,
                account_id,
                ..tokens
            })
        }
        Provider::Codex => {
            anyhow::ensure!(
                !config.code_verifier.trim().is_empty(),
                "missing PKCE verifier"
            );
            let tokens = post_form(
                CODEX_TOKEN_URL,
                &[
                    ("grant_type", "authorization_code".to_owned()),
                    ("client_id", CODEX_CLIENT_ID.to_owned()),
                    ("code", code.to_owned()),
                    ("redirect_uri", config.redirect_uri.clone()),
                    ("code_verifier", config.code_verifier.clone()),
                ],
            )
            .await?;
            let (email, account_id) = claims_from_jwt(&tokens.id_token);
            Ok(TokenResponse {
                email,
                account_id,
                ..tokens
            })
        }
        Provider::Kiro => super::device::exchange_kiro_code(config, code).await,
        Provider::Xai => anyhow::bail!(
            "xAI uses the device-code flow, not an authorization-code callback"
        ),
    }
}

/// 对应 `sdkMgmtClaudeCodeAndState`。
#[must_use]
pub fn split_claude_code(code: &str) -> (String, String) {
    match code.split_once('#') {
        Some((code, state)) => (code.trim().to_owned(), state.trim().to_owned()),
        None => (code.trim().to_owned(), String::new()),
    }
}

async fn post_form(endpoint: &str, form: &[(&str, String)]) -> anyhow::Result<TokenResponse> {
    let response = http_client(TOKEN_TIMEOUT)?
        .post(endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .body(flow::form_encode(form))
        .send()
        .await?;
    decode_token_response(response).await
}

async fn post_json(endpoint: &str, body: &Value) -> anyhow::Result<TokenResponse> {
    let response = http_client(TOKEN_TIMEOUT)?
        .post(endpoint)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(serde_json::to_vec(body)?)
        .send()
        .await?;
    decode_token_response(response).await
}

fn http_client(timeout: std::time::Duration) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder().timeout(timeout).build()
}

async fn decode_token_response(response: reqwest::Response) -> anyhow::Result<TokenResponse> {
    let status = response.status();
    let body = response.bytes().await?;
    anyhow::ensure!(
        status.is_success(),
        "token endpoint returned status {}",
        status.as_u16()
    );
    let raw: Map<String, Value> = serde_json::from_slice(&body)?;
    Ok(parse_token_body(raw))
}

/// Splits a token body into the typed fields plus the untouched original.
///
/// `expires_in` arrives as a number from some providers and a string from
/// others, so both are accepted rather than dropping the expiry.
#[must_use]
pub fn parse_token_body(raw: Map<String, Value>) -> TokenResponse {
    let text = |key: &str| {
        raw.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let expires_in = raw
        .get("expires_in")
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
        .unwrap_or_default();
    TokenResponse {
        access_token: text("access_token"),
        refresh_token: text("refresh_token"),
        id_token: text("id_token"),
        token_type: text("token_type"),
        expires_in,
        raw,
        email: String::new(),
        account_id: String::new(),
        extra: Map::new(),
    }
}

/// Best-effort userinfo lookup. 对应 `sdkMgmtFetchOAuthEmail`。
///
/// Every failure yields `""`: a credential without an email is still usable,
/// and failing the whole flow over a cosmetic field would be worse.
async fn fetch_email(endpoint: &str, access_token: &str) -> String {
    if access_token.trim().is_empty() {
        return String::new();
    }
    let Ok(client) = http_client(USERINFO_TIMEOUT) else {
        return String::new();
    };
    let Ok(response) = client
        .get(endpoint)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
    else {
        return String::new();
    };
    if !response.status().is_success() {
        return String::new();
    }
    let Ok(payload) = response.json::<Value>().await else {
        return String::new();
    };
    payload
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// Reads the email and account id out of an unverified `id_token`.
/// 对应 `sdkMgmtClaimsFromJWT`。
///
/// The signature is deliberately **not** checked: the token just came back over
/// TLS from the provider's own token endpoint in direct response to our code,
/// and nothing is authorized on these two fields — they are display labels.
#[must_use]
pub fn claims_from_jwt(token: &str) -> (String, String) {
    let Some(payload) = token.split('.').nth(1) else {
        return (String::new(), String::new());
    };
    let Ok(decoded) = URL_SAFE_NO_PAD.decode(payload) else {
        return (String::new(), String::new());
    };
    let Ok(claims) = serde_json::from_slice::<Map<String, Value>>(&decoded) else {
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

/// Builds the credential an OAuth flow produces. 对应 `sdkMgmtOAuthAuthRecord`。
///
/// The tokens are written twice — flat and nested under `token_data` — because
/// the executors read the flat keys while an exported auth file carries the
/// nested shape. Keeping both is what makes export/re-import round-trip.
#[must_use]
pub fn oauth_record(provider: Provider, tokens: &TokenResponse, now: DateTime<Utc>) -> AuthRecord {
    let expires_at =
        (tokens.expires_in > 0).then(|| rfc3339(now + Duration::seconds(tokens.expires_in)));

    let mut token_data = Map::new();
    token_data.insert("access_token".to_owned(), json!(tokens.access_token));

    let mut metadata = Map::new();
    metadata.insert("access_token".to_owned(), json!(tokens.access_token));
    metadata.insert("last_refresh".to_owned(), json!(rfc3339(now)));
    metadata.insert("oauth".to_owned(), json!(true));

    if !tokens.refresh_token.is_empty() {
        metadata.insert("refresh_token".to_owned(), json!(tokens.refresh_token));
        token_data.insert("refresh_token".to_owned(), json!(tokens.refresh_token));
    }
    if !tokens.id_token.is_empty() {
        metadata.insert("id_token".to_owned(), json!(tokens.id_token));
        token_data.insert("id_token".to_owned(), json!(tokens.id_token));
    }
    if let Some(expires_at) = &expires_at {
        // Both spellings: `expires_at` is the panel's, `expired` is what the
        // SDK's own refresher looked for.
        for key in ["expires_at", "expired"] {
            metadata.insert(key.to_owned(), json!(expires_at));
            token_data.insert(key.to_owned(), json!(expires_at));
        }
    }
    if !tokens.email.is_empty() {
        metadata.insert("email".to_owned(), json!(tokens.email));
        token_data.insert("email".to_owned(), json!(tokens.email));
    }
    if !tokens.account_id.is_empty() {
        metadata.insert("account_id".to_owned(), json!(tokens.account_id));
        token_data.insert("account_id".to_owned(), json!(tokens.account_id));
    }
    metadata.insert("token_data".to_owned(), Value::Object(token_data));

    if provider == Provider::Gemini {
        metadata.insert("token".to_owned(), gemini_token_metadata(tokens, now));
    }
    for (key, value) in &tokens.extra {
        metadata.insert(key.clone(), value.clone());
    }
    if provider == Provider::Xai && !tokens.access_token.is_empty() {
        metadata.insert("api_key".to_owned(), json!(tokens.access_token));
    }

    let label = if tokens.email.is_empty() {
        format!("{} OAuth", provider.as_str())
    } else {
        format!("{} OAuth ({})", provider.as_str(), tokens.email)
    };

    let mut record = AuthRecord::new(uuid::Uuid::new_v4().to_string(), provider.as_str(), now);
    record.label = label;
    record.metadata = Value::Object(metadata);
    record.last_refreshed_at = Some(now);
    record.set_attribute("oauth", "true");
    if provider == Provider::Xai {
        record.set_attribute("base_url", super::device::XAI_API_BASE);
    }
    record
}

/// The extra blob a Google credential file carries.
/// 对应 `sdkMgmtGeminiTokenMetadata`。
fn gemini_token_metadata(tokens: &TokenResponse, now: DateTime<Utc>) -> Value {
    let mut values = Map::new();
    values.insert("access_token".to_owned(), json!(tokens.access_token));
    values.insert("refresh_token".to_owned(), json!(tokens.refresh_token));
    values.insert("token_type".to_owned(), json!(tokens.token_type));
    values.insert("token_uri".to_owned(), json!(GEMINI_TOKEN_URL));
    values.insert("client_id".to_owned(), json!(GEMINI_CLIENT_ID));
    values.insert(
        "scopes".to_owned(),
        json!(GEMINI_SCOPES.split_whitespace().collect::<Vec<_>>()),
    );
    values.insert("universe_domain".to_owned(), json!("googleapis.com"));
    // Omitted rather than set to "now" when the provider gave no lifetime: a
    // wrong expiry would make the refresher throw away a working token.
    if tokens.expires_in > 0 {
        values.insert(
            "expiry".to_owned(),
            json!(rfc3339(now + Duration::seconds(tokens.expires_in))),
        );
    }
    Value::Object(values)
}
