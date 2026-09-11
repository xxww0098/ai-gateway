//! Closed set of OAuth families. Storage `auth_records.provider` is [`Family::as_str`].

#[cfg(test)]
mod tests;

/// One vendor OAuth family. Claude is the existing gateway subscription hop
/// (not in dsh-plugin-oauth-subs); Gemini CLI OAuth is gone — use API keys
/// or [`Family::Antigravity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Codex,
    Grok,
    Glm,
    Kiro,
    Antigravity,
    Cursor,
    Ollama,
    Kimi,
    Copilot,
    /// Claude Code / claude.ai subscription. Kept from the previous gateway
    /// OAuth; the oauth-subs plugin has no Claude family.
    Claude,
}

impl Family {
    /// Every family this crate owns, in Settings tab order (Claude last).
    pub const ALL: [Self; 10] = [
        Self::Codex,
        Self::Grok,
        Self::Glm,
        Self::Kiro,
        Self::Antigravity,
        Self::Cursor,
        Self::Ollama,
        Self::Kimi,
        Self::Copilot,
        Self::Claude,
    ];

    /// Parse a stored provider id. Unknown strings are rejected, not aliased
    /// into a neighbour family.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "codex" => Some(Self::Codex),
            "grok" => Some(Self::Grok),
            "glm" | "zai" | "bigmodel" => Some(Self::Glm),
            "kiro" => Some(Self::Kiro),
            "antigravity" => Some(Self::Antigravity),
            "cursor" => Some(Self::Cursor),
            "ollama" => Some(Self::Ollama),
            "kimi" => Some(Self::Kimi),
            "copilot" => Some(Self::Copilot),
            "claude" => Some(Self::Claude),
            _ => None,
        }
    }

    /// The `auth_records.provider` value. GLM always stores `glm`; region
    /// (zai vs bigmodel) lives in metadata.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Glm => "glm",
            Self::Kiro => "kiro",
            Self::Antigravity => "antigravity",
            Self::Cursor => "cursor",
            Self::Ollama => "ollama",
            Self::Kimi => "kimi",
            Self::Copilot => "copilot",
            Self::Claude => "claude",
        }
    }

    /// Panel `/{key}` OAuth start. `gemini-cli-auth-url` is deliberately
    /// absent (Gemini CLI OAuth was deleted).
    #[must_use]
    pub fn from_auth_url_key(endpoint: &str) -> Option<Self> {
        match endpoint.trim() {
            "codex-auth-url" => Some(Self::Codex),
            "grok-auth-url" => Some(Self::Grok),
            "glm-auth-url" | "zai-auth-url" | "bigmodel-auth-url" => Some(Self::Glm),
            "kiro-auth-url" => Some(Self::Kiro),
            "antigravity-auth-url" => Some(Self::Antigravity),
            "cursor-auth-url" => Some(Self::Cursor),
            "ollama-auth-url" => Some(Self::Ollama),
            "kimi-auth-url" => Some(Self::Kimi),
            "copilot-auth-url" => Some(Self::Copilot),
            "anthropic-auth-url" => Some(Self::Claude),
            _ => None,
        }
    }

    /// Canonical auth-url key the panel advertises for this family.
    #[must_use]
    pub fn auth_url_key(self) -> &'static str {
        match self {
            Self::Codex => "codex-auth-url",
            Self::Grok => "grok-auth-url",
            Self::Glm => "glm-auth-url",
            Self::Kiro => "kiro-auth-url",
            Self::Antigravity => "antigravity-auth-url",
            Self::Cursor => "cursor-auth-url",
            Self::Ollama => "ollama-auth-url",
            Self::Kimi => "kimi-auth-url",
            Self::Copilot => "copilot-auth-url",
            Self::Claude => "anthropic-auth-url",
        }
    }

    /// Native chat hop this family uses on the wire.
    #[must_use]
    pub fn hop(self) -> Hop {
        match self {
            Self::Codex | Self::Grok => Hop::Responses,
            Self::Glm | Self::Claude => Hop::Anthropic,
            Self::Kiro
            | Self::Antigravity
            | Self::Cursor
            | Self::Ollama
            | Self::Kimi
            | Self::Copilot => Hop::Completions,
        }
    }
}

/// Vendor-native chat protocol among the three the gateway already speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hop {
    /// OpenAI Responses (`/v1/responses`).
    Responses,
    /// OpenAI Chat Completions (`/v1/chat/completions`), possibly translated.
    Completions,
    /// Anthropic Messages (`/v1/messages`).
    Anthropic,
}
