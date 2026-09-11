//! Family-dispatched login start. Each family owns the actual hop.

use serde_json::Value;

use crate::{Error, Family, StartOutcome};

/// What the panel collected before calling a family.
#[derive(Debug, Clone, Default)]
pub struct StartInput {
    pub redirect_uri: String,
    pub state: String,
    /// Family-specific method (`device`, `authcode`, `idc`, `import`, `zai`, `pkce`, …).
    pub method: String,
    pub body: Value,
}

/// Start an OAuth / import / device flow for `family`.
///
/// # Errors
/// Family-specific: missing verifier, vendor HTTP, unsupported method.
pub async fn start(family: Family, input: &StartInput) -> Result<StartOutcome, Error> {
    match family {
        Family::Codex => crate::codex::start(input).await,
        Family::Grok => crate::grok::start(input).await,
        Family::Glm => crate::glm::start(input).await,
        Family::Kiro => crate::kiro::start(input).await,
        Family::Antigravity => crate::antigravity::start(input).await,
        Family::Cursor => crate::cursor::start(input).await,
        Family::Ollama => crate::ollama::start(input).await,
        Family::Kimi => crate::kimi::start(input).await,
        Family::Copilot => crate::copilot::start(input).await,
        Family::Claude => crate::claude::start(input).await,
    }
}
