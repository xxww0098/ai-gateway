//! Token-usage extraction for all five upstreams.
//!
//! OWNER: worker `provider-openai`.
//!
//! # Absent is not zero
//!
//! Every token column is an [`Option<i64>`]. A `None` means *the upstream did
//! not send that field*; `Some(0)` means it sent a literal zero. The parsers
//! keep both signals:
//!
//! - `parse_*_usage(..) -> None` means no parseable usage envelope at all. That
//!   is the "usage detail NOT present" signal the `UsagePlugin` fallback /
//!   strict paths trigger on — see [`usage_detail_present`].
//! - a per-column `None` inside a returned [`UsageTokens`] is the finer-grained
//!   "this one column was omitted" signal.

use crate::types::UsageRecord;
use gw_relay::endpoint::top_level_field;
use serde::Deserialize;

/// Provider-agnostic token tally extracted from an upstream response body.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UsageTokens {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cached: Option<i64>,
    pub reasoning: Option<i64>,
}

impl UsageTokens {
    /// Reports whether any of the four columns carries a non-zero count.
    ///
    /// The strict non-zero check is intentional and load-bearing: a payload
    /// that parses cleanly but carries all-zero counters is treated as "no
    /// usage info" so callers fall back to heuristic accounting. `Some(0)`
    /// counts as zero here — the column-level presence survives only inside a
    /// tally that already cleared this bar.
    #[must_use]
    pub fn has_values(&self) -> bool {
        [self.input, self.output, self.cached, self.reasoning]
            .iter()
            .any(|v| v.unwrap_or(0) != 0)
    }

    /// Field-wise maximum of two tallies.
    ///
    /// Gemini / Vertex `usageMetadata` is cumulative, so the larger value in
    /// each column is the later (authoritative) one; taking the max guards
    /// against an earlier, smaller per-chunk parse winning over the final
    /// aggregate when the terminal usage frame was split across stream reads.
    /// `None` never beats `Some`.
    #[must_use]
    pub fn max_merge(a: Self, b: Self) -> Self {
        fn pick(a: Option<i64>, b: Option<i64>) -> Option<i64> {
            match (a, b) {
                (Some(x), Some(y)) => Some(x.max(y)),
                (Some(x), None) => Some(x),
                (None, Some(y)) => Some(y),
                (None, None) => None,
            }
        }
        Self {
            input: pick(a.input, b.input),
            output: pick(a.output, b.output),
            cached: pick(a.cached, b.cached),
            reasoning: pick(a.reasoning, b.reasoning),
        }
    }

    /// `input + output`, or `None` when neither column was reported.
    #[must_use]
    pub fn total(&self) -> Option<i64> {
        match (self.input, self.output) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        }
    }

    /// Attaches the model / provider identity the parsers cannot know.
    #[must_use]
    pub fn to_record(self, model: impl Into<String>, provider: impl Into<String>) -> UsageRecord {
        UsageRecord {
            model: model.into(),
            provider: provider.into(),
            input_tokens: self.input,
            output_tokens: self.output,
            cached_tokens: self.cached,
            reasoning_tokens: self.reasoning,
        }
    }
}

/// Free-function form of [`UsageTokens::max_merge`], for the per-chunk
/// accumulators in `claude.rs` / `vertex.rs`.
#[must_use]
pub fn max_usage_tokens(a: UsageTokens, b: UsageTokens) -> UsageTokens {
    UsageTokens::max_merge(a, b)
}

/// Whether a terminal upstream usage envelope was observed.
///
/// The signal is structural and a missing marker is unrepresentable: `None`
/// *is* "not presented".
#[must_use]
pub fn usage_detail_present(parsed: Option<&UsageTokens>) -> bool {
    parsed.is_some()
}

/// Trims surrounding whitespace and rejects an empty remainder.
fn trimmed_json(payload: &[u8]) -> Option<&[u8]> {
    let start = payload.iter().position(|b| !b.is_ascii_whitespace())?;
    let end = payload.iter().rposition(|b| !b.is_ascii_whitespace())?;
    Some(&payload[start..=end])
}

// --- OpenAI -----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    prompt_tokens_details: Option<OpenAiPromptTokensDetails>,
    completion_tokens_details: Option<OpenAiCompletionTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct OpenAiPromptTokensDetails {
    cached_tokens: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct OpenAiCompletionTokensDetails {
    reasoning_tokens: Option<i64>,
}

/// Extracts [`UsageTokens`] from an OpenAI (Chat Completions / Responses API)
/// JSON payload. The envelope is:
///
/// ```text
/// {"usage": {"prompt_tokens": ..., "completion_tokens": ...,
///            "prompt_tokens_details": {"cached_tokens": ...},
///            "completion_tokens_details": {"reasoning_tokens": ...}}}
/// ```
///
/// Returns `None` for empty, malformed, missing-usage or all-zero payloads —
/// never panics.
#[must_use]
pub fn parse_openai_usage(payload: &[u8]) -> Option<UsageTokens> {
    let data = trimmed_json(payload)?;
    // 只把顶层 `usage` 对象交给 serde，choices/content 那些长字符串
    // 不再进反序列化器。嵌套里的 "usage" 不会被顶层扫描认走。
    let usage_bytes = top_level_field(data, "usage")?;
    let usage: OpenAiUsage = serde_json::from_slice(usage_bytes).ok()?;
    let tokens = UsageTokens {
        input: usage.prompt_tokens,
        output: usage.completion_tokens,
        cached: usage.prompt_tokens_details.and_then(|d| d.cached_tokens),
        reasoning: usage
            .completion_tokens_details
            .and_then(|d| d.reasoning_tokens),
    };
    tokens.has_values().then_some(tokens)
}

// --- Claude -----------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ClaudeUsageEnvelope {
    usage: Option<ClaudeUsage>,
    delta: Option<ClaudeDelta>,
    message: Option<ClaudeEmbeddedMessage>,
}

#[derive(Debug, Deserialize)]
struct ClaudeEmbeddedMessage {
    usage: Option<ClaudeUsage>,
}

#[derive(Debug, Deserialize)]
struct ClaudeDelta {
    usage: Option<ClaudeUsage>,
}

#[derive(Debug, Deserialize)]
struct ClaudeUsage {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_creation_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
}

/// Extracts [`UsageTokens`] from Anthropic Claude responses and streaming
/// events. Claude expresses usage in several shapes:
///
/// - `message_start`: `{"type":"message_start","message":{"usage":{...}}}`
/// - `message_delta`: `{"type":"message_delta","delta":{"usage":{"output_tokens":N}},
///   "message":{"usage":{"input_tokens":M}}}`
/// - non-streaming: `{"type":"message","usage":{...}}`
///
/// `cache_creation_input_tokens` + `cache_read_input_tokens` both contribute to
/// [`UsageTokens::cached`].
#[must_use]
pub fn parse_claude_usage(payload: &[u8]) -> Option<UsageTokens> {
    let data = trimmed_json(payload)?;
    let env: ClaudeUsageEnvelope = serde_json::from_slice(data).ok()?;

    let mut tokens = UsageTokens::default();
    let mut merge = |u: Option<ClaudeUsage>| {
        let Some(u) = u else { return };
        // Take the running max; a present-but-zero column still records
        // presence here, which is the whole point of the Option columns.
        if let Some(v) = u.input_tokens {
            tokens.input = Some(tokens.input.unwrap_or(0).max(v));
        }
        if let Some(v) = u.output_tokens {
            tokens.output = Some(tokens.output.unwrap_or(0).max(v));
        }
        if u.cache_creation_input_tokens.is_some() || u.cache_read_input_tokens.is_some() {
            let add =
                u.cache_creation_input_tokens.unwrap_or(0) + u.cache_read_input_tokens.unwrap_or(0);
            tokens.cached = Some(tokens.cached.unwrap_or(0) + add);
        }
    };
    merge(env.usage);
    merge(env.message.and_then(|m| m.usage));
    merge(env.delta.and_then(|d| d.usage));

    tokens.has_values().then_some(tokens)
}

// --- Gemini / Vertex --------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsageMetadata {
    prompt_token_count: Option<i64>,
    candidates_token_count: Option<i64>,
    thoughts_token_count: Option<i64>,
    cached_content_token_count: Option<i64>,
}

fn parse_gemini_shaped_usage(payload: &[u8]) -> Option<UsageTokens> {
    let data = trimmed_json(payload)?;
    let meta_bytes = top_level_field(data, "usageMetadata")?;
    let meta: GeminiUsageMetadata = serde_json::from_slice(meta_bytes).ok()?;
    let tokens = UsageTokens {
        input: meta.prompt_token_count,
        output: meta.candidates_token_count,
        cached: meta.cached_content_token_count,
        reasoning: meta.thoughts_token_count,
    };
    tokens.has_values().then_some(tokens)
}

/// Extracts [`UsageTokens`] from a Google AI Gemini `generateContent` /
/// `streamGenerateContent` response:
///
/// ```text
/// {"usageMetadata": {"promptTokenCount": ..., "candidatesTokenCount": ...,
///                    "thoughtsTokenCount": ..., "cachedContentTokenCount": ...}}
/// ```
#[must_use]
pub fn parse_gemini_usage(payload: &[u8]) -> Option<UsageTokens> {
    parse_gemini_shaped_usage(payload)
}

/// Extracts [`UsageTokens`] from a Vertex AI response. Vertex mirrors the
/// Gemini `usageMetadata` envelope, so the shared implementation is reused
/// directly.
#[must_use]
pub fn parse_vertex_usage(payload: &[u8]) -> Option<UsageTokens> {
    parse_gemini_shaped_usage(payload)
}

// --- Codex ------------------------------------------------------------------

/// Extracts [`UsageTokens`] from a Codex / OpenAI-OAuth response body. Codex
/// surfaces usage through the OpenAI-compatible `usage` field, so parsing is
/// delegated to [`parse_openai_usage`]. Header-only (`x-usage-*`) parsing stays
/// in the executor layer where the response headers are available.
#[must_use]
pub fn parse_codex_usage(payload: &[u8]) -> Option<UsageTokens> {
    parse_openai_usage(payload)
}

// --- SSE scanners -----------------------------------------------------------

/// Scans an OpenAI-style SSE buffer for the last `data: {...}` event carrying a
/// usage envelope. The stream format is:
///
/// ```text
/// data: {"choices":[...]}
///
/// data: {"choices":[...],"usage":{...}}
///
/// data: [DONE]
/// ```
///
/// Only the final usage-bearing JSON event matters; earlier deltas either omit
/// usage entirely or carry partial counts. Falls back to parsing the whole
/// buffer (non-stream path) when no SSE framing is detected.
#[must_use]
pub fn parse_openai_stream_usage(body: &[u8]) -> Option<UsageTokens> {
    parse_sse_usage(body, parse_openai_usage)
}

/// Codex streams OpenAI-compatible chunks, so the terminal non-`[DONE]` data
/// event carries the aggregate `usage` envelope.
#[must_use]
pub fn parse_codex_stream_usage(body: &[u8]) -> Option<UsageTokens> {
    parse_sse_usage(body, parse_codex_usage)
}

/// Shared body of the two SSE scanners, which were byte-identical.
fn parse_sse_usage(body: &[u8], parse: fn(&[u8]) -> Option<UsageTokens>) -> Option<UsageTokens> {
    // Fast path: plain JSON body with top-level usage (non-SSE).
    if let Some(tokens) = parse(body) {
        return Some(tokens);
    }
    let mut last = None;
    for line in body.split(|&b| b == b'\n') {
        let trimmed = trimmed_json(line).unwrap_or_default();
        let Some(rest) = trimmed.strip_prefix(b"data:") else {
            continue;
        };
        let payload = trimmed_json(rest).unwrap_or_default();
        if payload.is_empty() || payload == b"[DONE]" {
            continue;
        }
        if let Some(tokens) = parse(payload) {
            last = Some(tokens);
        }
    }
    last
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
