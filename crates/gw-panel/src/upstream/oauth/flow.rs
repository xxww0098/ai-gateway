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

#[cfg(test)]
mod tests;

// The endpoints and client ids below are the public, first-party values the
// gateway ships; they identify the *gateway* to each provider, not any operator.
const GEMINI_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/auth";
pub(super) const GEMINI_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub(super) const GEMINI_USERINFO_URL: &str =
    "https://www.googleapis.com/oauth2/v1/userinfo?alt=json";
pub(super) const GEMINI_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
pub(super) const GEMINI_SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform \
     https://www.googleapis.com/auth/userinfo.email \
     https://www.googleapis.com/auth/userinfo.profile";

const CLAUDE_AUTH_URL: &str = "https://claude.ai/oauth/authorize";
pub(super) const CLAUDE_TOKEN_URL: &str = "https://api.anthropic.com/v1/oauth/token";
pub(super) const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CLAUDE_SCOPES: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

const CODEX_AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
pub(super) const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub(super) const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_SCOPES: &str = "openid email profile offline_access";

/// The providers with a panel-driven OAuth flow.
///
/// `anthropic` is accepted as an inbound alias for `claude` in the auth-url key
/// but the credential is always stored under `claude`, so one provider never
/// ends up with two spellings in `auth_records`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Gemini,
    Claude,
    Codex,
    /// xAI Grok — device-code (RFC 8628), stored as `xai`.
    Xai,
    /// Kiro / AWS Builder ID — device-code, auth-code, or IDC.
    Kiro,
}

impl Provider {
    /// 对应 `sdkMgmtCanonicalOAuthProvider`。
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_lowercase().as_str() {
            "gemini" => Some(Self::Gemini),
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "xai" | "grok" => Some(Self::Xai),
            "kiro" => Some(Self::Kiro),
            _ => None,
        }
    }

    /// The `auth_records.provider` value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gemini => "gemini",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Xai => "xai",
            Self::Kiro => "kiro",
        }
    }

    /// Maps endpoint key to provider.
    ///
    /// `antigravity-`/`kimi-auth-url` are deliberately absent: they have no
    /// backend anymore, so they fall through to the 404 branch.
    #[must_use]
    pub fn from_auth_url_key(endpoint: &str) -> Option<Self> {
        match endpoint.trim() {
            "gemini-cli-auth-url" => Some(Self::Gemini),
            "anthropic-auth-url" => Some(Self::Claude),
            "codex-auth-url" => Some(Self::Codex),
            "xai-auth-url" | "grok-auth-url" => Some(Self::Xai),
            "kiro-auth-url" => Some(Self::Kiro),
            _ => None,
        }
    }
}

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
        Provider::Gemini => vec![
            ("client_id", GEMINI_CLIENT_ID.to_owned()),
            ("response_type", "code".to_owned()),
            ("redirect_uri", config.redirect_uri.clone()),
            ("scope", GEMINI_SCOPES.to_owned()),
            ("state", state.to_owned()),
            ("access_type", "offline".to_owned()),
            ("prompt", "consent".to_owned()),
        ],
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
        // Device / dynamic-client flows build their URL elsewhere.
        Provider::Xai | Provider::Kiro => return Ok(String::new()),
    };

    let base = match provider {
        Provider::Gemini => GEMINI_AUTH_URL,
        Provider::Claude => CLAUDE_AUTH_URL,
        Provider::Codex => CODEX_AUTH_URL,
        Provider::Xai | Provider::Kiro => "",
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
