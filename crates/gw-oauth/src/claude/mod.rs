//! Claude Code / claude.ai subscription. Kept from the previous gateway OAuth;
//! dsh-plugin-oauth-subs has no Claude family. No prompt-cache rewrite here.

pub const ID: &str = "claude";
pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
pub const TOKEN_URL: &str = "https://api.anthropic.com/v1/oauth/token";
pub const SCOPE: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";

/// PKCE authorize URL for Claude Code.
pub async fn start(input: &crate::login::StartInput) -> Result<crate::StartOutcome, crate::Error> {
    let pkce = crate::create_pkce()?;
    let params = [
        ("code", "true".to_owned()),
        ("client_id", CLIENT_ID.to_owned()),
        ("response_type", "code".to_owned()),
        ("redirect_uri", input.redirect_uri.clone()),
        ("scope", SCOPE.to_owned()),
        ("code_challenge", pkce.challenge.clone()),
        ("code_challenge_method", "S256".to_owned()),
        ("state", input.state.clone()),
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
