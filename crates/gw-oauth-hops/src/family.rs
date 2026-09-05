//! Hop family id. One variant per vendor cache, never a shared "oauth" bucket.

/// Subscription hop family. Adding a variant is a new `src/<id>.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    Codex,
    Grok,
    Kiro,
}

impl Family {
    /// Stable key used in logs and pin maps.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Kiro => "kiro",
        }
    }
}

#[cfg(test)]
mod tests;
