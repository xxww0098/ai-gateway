//! ChatGPT Codex subscription OAuth.
//!
//! Design: [`README.md`]. Cache lives only in [`cache`].

pub mod cache;
pub mod import;
pub mod quota;
pub mod request;

mod login;

pub use cache::{apply_cache, cache_headers, cache_session_id};
pub use import::{import_from_paths, search_paths, session_from_json};
pub use login::{credential_headers, exchange_code, refresh, routing_hint, upstream_headers};
pub use quota::{
    ResetCredit, ResetCredits, Usage, UsageRow, consume_reset_body, is_available_reset_credit,
    parse_reset_credits, parse_usage,
};
pub use request::normalize_request;

/// Stored `auth_records.provider` / family id.
pub const ID: &str = "codex";
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const API_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
pub const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
pub const RESET_CREDITS_URL: &str = "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits";
pub const RESET_CONSUME_URL: &str =
    "https://chatgpt.com/backend-api/wham/rate-limit-reset-credits/consume";
pub const MODELS_URL: &str = "https://chatgpt.com/backend-api/codex/models";
pub const CLIENT_VERSION: &str = "0.153.4";
pub const ORIGINATOR: &str = "codex_cli_rs";
pub const USER_AGENT: &str = "codex_cli_rs/0.153.4";
pub const SCOPE: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";
pub const CALLBACK_PATH: &str = "/auth/callback";

/// PKCE authorize URL for ChatGPT Codex.
pub async fn start(input: &crate::login::StartInput) -> Result<crate::StartOutcome, crate::Error> {
    let pkce = crate::create_pkce()?;
    let params = [
        ("client_id", CLIENT_ID.to_owned()),
        ("response_type", "code".to_owned()),
        ("redirect_uri", input.redirect_uri.clone()),
        ("scope", SCOPE.to_owned()),
        ("state", input.state.clone()),
        ("code_challenge", pkce.challenge.clone()),
        ("code_challenge_method", "S256".to_owned()),
        ("prompt", "login".to_owned()),
        ("id_token_add_organizations", "true".to_owned()),
        ("codex_cli_simplified_flow", "true".to_owned()),
        ("originator", ORIGINATOR.to_owned()),
    ];
    let authorize_url = format!("{AUTHORIZE_URL}?{}", crate::form::encode(&params));
    Ok(crate::StartOutcome::Browser {
        authorize_url,
        state: input.state.clone(),
        verifier: pkce.verifier,
        redirect_uri: input.redirect_uri.clone(),
        flow: crate::FlowKind::AuthorizationCode,
        extra: serde_json::Map::new(),
    })
}
