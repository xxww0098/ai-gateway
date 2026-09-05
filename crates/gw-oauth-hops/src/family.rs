//! Hop family id. One variant per vendor cache, never a shared "oauth" bucket.

/// Subscription hop family. Adding a variant is a new `src/<id>.rs`.
///
/// Matches `dsh-plugin-oauth-subs` `src/oauth/<id>/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Codex,
    Grok,
    Kiro,
    Kimi,
    OpenCode,
    Copilot,
    Cursor,
    Antigravity,
    Glm,
    Ollama,
}

impl Family {
    /// Stable key used in logs and pin maps.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Kiro => "kiro",
            Self::Kimi => "kimi",
            Self::OpenCode => "opencode",
            Self::Copilot => "copilot",
            Self::Cursor => "cursor",
            Self::Antigravity => "antigravity",
            Self::Glm => "glm",
            Self::Ollama => "ollama",
        }
    }

    /// Every family this crate plans. Keep in lockstep with `src/<id>.rs`.
    pub const ALL: &[Family] = &[
        Self::Codex,
        Self::Grok,
        Self::Kiro,
        Self::Kimi,
        Self::OpenCode,
        Self::Copilot,
        Self::Cursor,
        Self::Antigravity,
        Self::Glm,
        Self::Ollama,
    ];
}

#[cfg(test)]
mod tests;
