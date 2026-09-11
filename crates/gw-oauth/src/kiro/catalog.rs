//! Offline Kiro picker catalog. Live ListAvailableModels is a later hop.

#[cfg(test)]
mod tests;

pub const CONTEXT_WINDOW: i64 = 200_000;
pub const LARGE_CONTEXT: i64 = 1_000_000;
pub const GPT_CONTEXT: i64 = 272_000;
pub const DEEPSEEK_CONTEXT: i64 = 128_000;
pub const QWEN_CONTEXT: i64 = 256_000;
pub const MAX_TOKENS: i64 = 64_000;

/// One static fallback row. `reasoning` is the DSH picker ladder, not a wire enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub context_window: i64,
    pub vision: bool,
    /// `none` / `claude` (max) / `xhigh` / `gpt` (`off` → wire `none`).
    pub reasoning: Reasoning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reasoning {
    None,
    Claude,
    ClaudeXhigh,
    Gpt,
}

const fn vision(id: &'static str, name: &'static str, window: i64, reasoning: Reasoning) -> Model {
    Model {
        id,
        name,
        context_window: window,
        vision: true,
        reasoning,
    }
}

const fn text(id: &'static str, name: &'static str, window: i64) -> Model {
    Model {
        id,
        name,
        context_window: window,
        vision: false,
        reasoning: Reasoning::None,
    }
}

/// Offline fallback aligned with kiro.dev/docs/models (plus Auto and Fable 5).
pub const MODELS: &[Model] = &[
    vision("gpt-5.6-sol", "GPT-5.6 Sol", GPT_CONTEXT, Reasoning::Gpt),
    vision(
        "gpt-5.6-terra",
        "GPT-5.6 Terra",
        GPT_CONTEXT,
        Reasoning::Gpt,
    ),
    vision("gpt-5.6-luna", "GPT-5.6 Luna", GPT_CONTEXT, Reasoning::Gpt),
    vision(
        "claude-opus-5",
        "Claude Opus 5",
        LARGE_CONTEXT,
        Reasoning::ClaudeXhigh,
    ),
    vision(
        "claude-opus-4.8",
        "Claude Opus 4.8",
        LARGE_CONTEXT,
        Reasoning::ClaudeXhigh,
    ),
    vision(
        "claude-opus-4.7",
        "Claude Opus 4.7",
        LARGE_CONTEXT,
        Reasoning::ClaudeXhigh,
    ),
    vision(
        "claude-opus-4.6",
        "Claude Opus 4.6",
        LARGE_CONTEXT,
        Reasoning::Claude,
    ),
    vision(
        "claude-opus-4.5",
        "Claude Opus 4.5",
        CONTEXT_WINDOW,
        Reasoning::None,
    ),
    vision(
        "claude-sonnet-5",
        "Claude Sonnet 5",
        LARGE_CONTEXT,
        Reasoning::ClaudeXhigh,
    ),
    vision(
        "claude-fable-5",
        "Claude Fable 5",
        LARGE_CONTEXT,
        Reasoning::ClaudeXhigh,
    ),
    vision(
        "claude-sonnet-4.6",
        "Claude Sonnet 4.6",
        LARGE_CONTEXT,
        Reasoning::Claude,
    ),
    vision(
        "claude-sonnet-4.5",
        "Claude Sonnet 4.5",
        CONTEXT_WINDOW,
        Reasoning::None,
    ),
    vision(
        "claude-sonnet-4",
        "Claude Sonnet 4",
        CONTEXT_WINDOW,
        Reasoning::None,
    ),
    vision("auto", "Auto", LARGE_CONTEXT, Reasoning::ClaudeXhigh),
    vision(
        "claude-haiku-4.5",
        "Claude Haiku 4.5",
        CONTEXT_WINDOW,
        Reasoning::None,
    ),
    text("deepseek-3.2", "DeepSeek 3.2", DEEPSEEK_CONTEXT),
    text("minimax-m2.5", "MiniMax M2.5", CONTEXT_WINDOW),
    text("glm-5", "GLM-5", CONTEXT_WINDOW),
    text("minimax-m2.1", "MiniMax M2.1", CONTEXT_WINDOW),
    text("qwen3-coder-next", "Qwen3 Coder Next", QWEN_CONTEXT),
];

/// Context window for usage fallback. Unknown ids use the default 200k window.
#[must_use]
pub fn context_window_of(model: &str) -> i64 {
    let id = model.trim();
    MODELS
        .iter()
        .find(|row| row.id == id)
        .map(|row| row.context_window)
        .unwrap_or_else(|| infer_window(id))
}

fn infer_window(id: &str) -> i64 {
    let key = id.to_ascii_lowercase();
    if key.starts_with("gpt-") {
        GPT_CONTEXT
    } else if key.contains("deepseek") {
        DEEPSEEK_CONTEXT
    } else if key.contains("qwen") {
        QWEN_CONTEXT
    } else if key == "auto"
        || key.contains("claude-opus-5")
        || key.contains("claude-opus-4.6")
        || key.contains("claude-opus-4.7")
        || key.contains("claude-opus-4.8")
        || key.contains("claude-sonnet-5")
        || key.contains("claude-sonnet-4.6")
        || key.contains("claude-fable-5")
    {
        LARGE_CONTEXT
    } else {
        CONTEXT_WINDOW
    }
}
