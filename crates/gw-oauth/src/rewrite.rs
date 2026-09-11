//! Dispatch request-body cache rewrite. Each family owns its helper;
//! this file only switches.

use http::HeaderMap;
use serde_json::Value;

use crate::codex;
use crate::copilot;
use crate::cursor;
use crate::glm;
use crate::grok;
use crate::kimi;
use crate::kiro;
use crate::ollama;
use crate::{CacheRewrite, Family, antigravity};

#[cfg(test)]
mod tests;

/// Rewrite `payload` for `family`. `explicit_pin` is an already-chosen
/// session id (panel / DSH `session_id`); families may ignore it.
#[must_use]
pub fn rewrite_body(family: Family, payload: Value, explicit_pin: Option<&str>) -> CacheRewrite {
    match family {
        Family::Codex => {
            let payload = crate::codex::normalize_request(payload);
            crate::codex::apply_cache(payload)
        }
        Family::Grok => grok::apply_cache(grok::normalize_request(payload)),
        Family::Glm => glm::apply_anthropic_cache(glm::normalize_request(payload)),
        Family::Kiro => kiro::apply_cache(payload, explicit_pin),
        Family::Antigravity => antigravity::apply_cache(payload, explicit_pin),
        Family::Cursor => cursor::apply_cache(payload),
        Family::Ollama => ollama::apply_cache(payload),
        Family::Kimi => kimi::apply_cache(kimi::apply_thinking(payload)),
        Family::Copilot => copilot::apply_cache(copilot::apply_thinking(payload)),
        Family::Claude => CacheRewrite {
            payload,
            cache_session_id: None,
        },
    }
}

/// Sticky HTTP headers for this family. Empty map when the vendor does not
/// sticky-route on headers.
#[must_use]
pub fn cache_headers(
    family: Family,
    cache_session_id: Option<&str>,
    extra: &HeaderExtra,
) -> HeaderMap {
    match family {
        Family::Codex => codex::cache_headers(cache_session_id),
        Family::Grok => grok::affinity_headers(cache_session_id, extra),
        Family::Glm => glm::session_headers(cache_session_id),
        Family::Copilot => copilot::cache_headers(cache_session_id),
        Family::Kiro
        | Family::Antigravity
        | Family::Cursor
        | Family::Ollama
        | Family::Kimi
        | Family::Claude => HeaderMap::new(),
    }
}

/// Per-request extras Grok (and only Grok) stamps on affinity headers.
#[derive(Debug, Clone, Default)]
pub struct HeaderExtra {
    pub req_id: Option<String>,
    pub model: Option<String>,
    pub retry_attempt: i64,
}
