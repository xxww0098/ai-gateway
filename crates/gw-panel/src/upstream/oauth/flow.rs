//! Which provider, where it sends the operator, and how the authorize URL is
//! built.
//!
//! 对应 `sdkMgmtCanonicalOAuthProvider`、`sdkMgmtAuthURLProviders`、
//! `sdkMgmtOAuthRedirectURI`、`sdkMgmtBuildOAuthAuthURL` 和
//! `sdkMgmtGeneratePKCE`。
//!
//! Nothing here touches the database or the network, which is the point: the
//! security-relevant half of an OAuth start — the state binding and the PKCE
//! pair — is a pure function and can be tested as one.

use axum::http::HeaderMap;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use super::SessionConfig;

pub use gw_oauth::Family as Provider;
pub(super) use gw_oauth::claude::{CLIENT_ID as CLAUDE_CLIENT_ID, TOKEN_URL as CLAUDE_TOKEN_URL};
pub(super) use gw_oauth::codex::{CLIENT_ID as CODEX_CLIENT_ID, TOKEN_URL as CODEX_TOKEN_URL};

#[cfg(test)]
mod tests;

const CLAUDE_AUTH_URL: &str = gw_oauth::claude::AUTHORIZE_URL;
const CLAUDE_SCOPES: &str = gw_oauth::claude::SCOPE;
const CODEX_AUTH_URL: &str = gw_oauth::codex::AUTHORIZE_URL;
const CODEX_SCOPES: &str = gw_oauth::codex::SCOPE;

/// Where the provider sends the operator back to.
///
/// 对应 `sdkMgmtOAuthRedirectURI` —— built from the *request*, because the gateway
/// does not know its own external URL. `X-Forwarded-*` wins over the direct
/// values so the address is the one the browser actually used.
///
/// 原实现还检查了 `c.Request.TLS != nil`。axum's handler has no equivalent ——
/// the listener is plain HTTP whenever there is a proxy in front, which is the
/// only deployment where the distinction matters — so the scheme comes from
/// `X-Forwarded-Proto` alone. A gateway terminating TLS itself must send that
/// header, or the provider will redirect the operator to `http://`.
#[must_use]
pub fn redirect_uri(headers: &HeaderMap, provider: Provider) -> String {
    let scheme = if header(headers, "x-forwarded-proto").eq_ignore_ascii_case("https") {
        "https"
    } else {
        "http"
    };
    let forwarded_host = header(headers, "x-forwarded-host");
    let host = if forwarded_host.is_empty() {
        header(headers, "host")
    } else {
        forwarded_host
    };
    format!(
        "{scheme}://{host}/api/panel/admin/sdk-management/oauth-callback/{}",
        provider.as_str()
    )
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .unwrap_or_default()
}

/// Builds a provider's authorize URL, filling the PKCE fields into `config` as
/// a side effect. 对应 `sdkMgmtBuildOAuthAuthURL`。
///
/// Gemini uses `access_type=offline` + `prompt=consent` rather than PKCE, which
/// is why only two of the three set a verifier.
///
/// # Errors
/// When the OS entropy source fails, which is the only way PKCE generation can.
pub fn build_authorize_url(
    provider: Provider,
    state: &str,
    config: &mut SessionConfig,
) -> Result<String, rand::Error> {
    let params: Vec<(&str, String)> = match provider {
        Provider::Claude => {
            let challenge = set_pkce(config)?;
            vec![
                ("code", "true".to_owned()),
                ("client_id", CLAUDE_CLIENT_ID.to_owned()),
                ("response_type", "code".to_owned()),
                ("redirect_uri", config.redirect_uri.clone()),
                ("scope", CLAUDE_SCOPES.to_owned()),
                ("code_challenge", challenge),
                ("code_challenge_method", "S256".to_owned()),
                ("state", state.to_owned()),
            ]
        }
        Provider::Codex => {
            let challenge = set_pkce(config)?;
            vec![
                ("client_id", CODEX_CLIENT_ID.to_owned()),
                ("response_type", "code".to_owned()),
                ("redirect_uri", config.redirect_uri.clone()),
                ("scope", CODEX_SCOPES.to_owned()),
                ("state", state.to_owned()),
                ("code_challenge", challenge),
                ("code_challenge_method", "S256".to_owned()),
                ("prompt", "login".to_owned()),
                ("id_token_add_organizations", "true".to_owned()),
                ("codex_cli_simplified_flow", "true".to_owned()),
            ]
        }
        Provider::Antigravity => vec![
            ("client_id", gw_oauth::antigravity::CLIENT_ID.to_owned()),
            ("response_type", "code".to_owned()),
            ("redirect_uri", config.redirect_uri.clone()),
            ("state", state.to_owned()),
            ("access_type", "offline".to_owned()),
            ("prompt", "consent".to_owned()),
        ],
        // Device / CLI-poll / import families build their URL in `gw_oauth::start`.
        Provider::Grok
        | Provider::Glm
        | Provider::Kiro
        | Provider::Cursor
        | Provider::Ollama
        | Provider::Kimi
        | Provider::Copilot => return Ok(String::new()),
    };

    let base = match provider {
        Provider::Claude => CLAUDE_AUTH_URL,
        Provider::Codex => CODEX_AUTH_URL,
        Provider::Antigravity => "https://accounts.google.com/o/oauth2/v2/auth",
        Provider::Grok
        | Provider::Glm
        | Provider::Kiro
        | Provider::Cursor
        | Provider::Ollama
        | Provider::Kimi
        | Provider::Copilot => "",
    };
    Ok(format!("{base}?{}", form_encode(&params)))
}

/// Generates the PKCE pair, storing the verifier and returning the challenge.
/// 对应 `sdkMgmtGeneratePKCE` —— 96 random bytes, base64url without padding.
pub(super) fn set_pkce(config: &mut SessionConfig) -> Result<String, rand::Error> {
    use rand::RngCore as _;
    let mut raw = [0u8; 96];
    rand::rngs::OsRng.try_fill_bytes(&mut raw)?;
    let verifier = URL_SAFE_NO_PAD.encode(raw);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    config.code_verifier = verifier;
    config.code_challenge_method = "S256".to_owned();
    Ok(challenge)
}

/// `application/x-www-form-urlencoded`, sorted by key.
///
/// 这里按键排序（对标 `url.Values.Encode()` 的行为）—— the providers do
/// not care, but a stable order makes an authorize URL diffable in a log.
#[must_use]
pub(super) fn form_encode(params: &[(&str, String)]) -> String {
    let mut sorted: Vec<&(&str, String)> = params.iter().collect();
    sorted.sort_by_key(|(key, _)| *key);
    sorted
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                urlencode(key.as_bytes()),
                urlencode(value.as_bytes())
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// 对标 `url.QueryEscape`：unreserved characters pass, a space becomes `+`,
/// everything else is percent-encoded.
fn urlencode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for byte in bytes {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
