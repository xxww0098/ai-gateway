//! Live Cursor picker catalog. GetUsableModels + AvailableModels collapse
//! into one row per family. Empty live falls back to [`MODELS`].

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};

use super::proto::{AvailableModel, UsableModel};

#[cfg(test)]
mod tests;

pub const CATALOG_TTL_MS: i64 = 5 * 60_000;
pub const DEFAULT_CONTEXT_WINDOW: u32 = 200_000;
pub const DEFAULT_MAX_OUTPUT: u32 = 64_000;
pub const GPT56_DEFAULT_CONTEXT_WINDOW: u32 = 272_000;
pub const GPT56_MAX_PROMPT_TOKENS: u32 = 500_000;

const VISION: [&str; 2] = ["text", "image"];
const EFFORT_SUFFIXES: [&str; 7] = [
    "extra-high",
    "minimal",
    "xhigh",
    "medium",
    "high",
    "low",
    "none",
];
const CONTEXT_SUFFIXES: [&str; 5] = ["1m", "272k", "256k", "200k", "300k"];

/// One picker row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerModel {
    pub id: String,
    pub name: String,
    pub context_window: u32,
    pub max_tokens: u32,
    pub input: Vec<String>,
}

/// Static fallback aligned to cursor.com/docs/models-and-pricing.
#[must_use]
pub fn static_models() -> Vec<PickerModel> {
    [
        ("composer-2.5", "Composer 2.5", 200_000, 64_000),
        ("grok-4.6", "Grok 4.6", 256_000, 64_000),
        ("grok-4.5", "Grok 4.5", 256_000, 64_000),
        ("claude-fable-5-1", "Claude Fable 5.1", 300_000, 128_000),
        ("claude-opus-5", "Claude Opus 5", 300_000, 128_000),
        ("claude-sonnet-5", "Claude Sonnet 5", 200_000, 128_000),
        ("gemini-3.1-pro", "Gemini 3.1 Pro", 200_000, 64_000),
        ("gemini-3.8-flash", "Gemini 3.8 Flash", 200_000, 64_000),
        ("gpt-5.6-sol", "GPT-5.6 Sol", 272_000, 128_000),
        ("gpt-5.6-terra", "GPT-5.6 Terra", 272_000, 128_000),
        ("gpt-5.6-luna", "GPT-5.6 Luna", 272_000, 128_000),
        ("gpt-5.5", "GPT-5.5", 200_000, 128_000),
    ]
    .into_iter()
    .map(|(id, name, context_window, max_tokens)| PickerModel {
        id: id.to_owned(),
        name: name.to_owned(),
        context_window,
        max_tokens,
        input: VISION.iter().map(|s| (*s).to_owned()).collect(),
    })
    .collect()
}

struct CatalogCache {
    token_hash: String,
    models: Vec<PickerModel>,
    expires_at: i64,
}

fn cache() -> &'static Mutex<CatalogCache> {
    static CACHE: OnceLock<Mutex<CatalogCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(CatalogCache {
            token_hash: String::new(),
            models: Vec::new(),
            expires_at: 0,
        })
    })
}

/// Drop the in-process catalog (tests).
pub fn reset_catalog_cache() {
    if let Ok(mut cached) = cache().lock() {
        cached.token_hash.clear();
        cached.models.clear();
        cached.expires_at = 0;
    }
}

#[must_use]
pub fn catalog_token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
        .chars()
        .take(16)
        .collect()
}

#[must_use]
pub fn catalog_models() -> Vec<PickerModel> {
    cache()
        .lock()
        .ok()
        .filter(|c| !c.models.is_empty())
        .map(|c| c.models.clone())
        .unwrap_or_else(static_models)
}

#[must_use]
pub fn is_gpt56_model(id: &str, name: &str) -> bool {
    format!("{id} {name}")
        .to_ascii_lowercase()
        .contains("gpt-5.6")
}

#[must_use]
pub fn clamp_context_window(id: &str, name: &str, window: u32) -> u32 {
    if is_gpt56_model(id, name) && window > GPT56_MAX_PROMPT_TOKENS {
        GPT56_MAX_PROMPT_TOKENS
    } else {
        window
    }
}

fn hyphen_bounded(haystack: &str, token: &str) -> bool {
    if haystack == token {
        return true;
    }
    haystack.starts_with(&format!("{token}-"))
        || haystack.ends_with(&format!("-{token}"))
        || haystack.contains(&format!("-{token}-"))
}

fn has_size(text: &str, compact: &str, spaced: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    hyphen_bounded(&lower, compact) || lower.contains(spaced)
}

/// Infer context window when GetUsableModels has no window field.
#[must_use]
pub fn infer_context_window(id: &str, name: &str) -> u32 {
    let id_lower = id.to_ascii_lowercase();
    let text = format!("{id_lower} {name}").to_ascii_lowercase();
    if is_gpt56_model(id, name) {
        if hyphen_bounded(&id_lower, "1m") {
            return GPT56_MAX_PROMPT_TOKENS;
        }
        return GPT56_DEFAULT_CONTEXT_WINDOW;
    }
    if has_size(&text, "1m", "1 m") {
        return 1_000_000;
    }
    if has_size(&text, "300k", "300 k") {
        return 300_000;
    }
    if has_size(&text, "272k", "272 k") {
        return 272_000;
    }
    if has_size(&text, "256k", "256 k") {
        return 256_000;
    }
    if text.contains("claude-opus-5") || text.contains("claude-fable-5") {
        return 300_000;
    }
    if text.contains("grok-4.5")
        || text.contains("grok-4.6")
        || text.contains("grok 4.5")
        || text.contains("grok 4.6")
    {
        return 256_000;
    }
    DEFAULT_CONTEXT_WINDOW
}

/// Infer max output tokens from family name.
#[must_use]
pub fn infer_max_output_tokens(id: &str, name: &str) -> u32 {
    let text = format!("{id} {name}").to_ascii_lowercase();
    if text.contains("claude-fable")
        || text.contains("claude-opus")
        || text.contains("claude-sonnet")
        || text.contains("gpt-5")
    {
        return 128_000;
    }
    DEFAULT_MAX_OUTPUT
}

fn has_boundary_token(text: &str, token: &str) -> bool {
    let bytes = text.as_bytes();
    let needle = token.as_bytes();
    if needle.is_empty() {
        return false;
    }
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if bytes[i..i + needle.len()].eq_ignore_ascii_case(needle) {
            let before_ok = i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'_' | b'-');
            let after = i + needle.len();
            let after_ok =
                after == bytes.len() || matches!(bytes[after], b' ' | b'\t' | b'_' | b'-');
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn has_hyphen_token(key: &str, token: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    hyphen_bounded(&lower, token)
}

/// Tab / chat internals stay out of the picker.
#[must_use]
pub fn is_internal_model(id: &str, name: &str) -> bool {
    let key = id.trim().to_ascii_lowercase();
    if key.is_empty() {
        return true;
    }
    let text = format!("{key} {name}").to_ascii_lowercase();
    for token in ["tab", "cursor-small", "cmd-k", "cpp", "speculative"] {
        if has_boundary_token(&text, token) {
            return true;
        }
    }
    has_hyphen_token(&key, "tab") || has_hyphen_token(&key, "chat")
}

fn peel_suffix<'a>(id: &'a str, suffixes: &[&str]) -> &'a str {
    let lower = id.to_ascii_lowercase();
    for suffix in suffixes {
        let marker = format!("-{suffix}");
        if lower.ends_with(&marker) {
            return &id[..id.len() - marker.len()];
        }
    }
    id
}

fn peel_effort_context_max(mut s: String) -> String {
    loop {
        let prev = s.clone();
        let lower = s.to_ascii_lowercase();
        if lower.ends_with("-thinking") || lower.ends_with("-max-mode") {
            s.truncate(s.len() - 9);
        } else {
            let effort = peel_suffix(&s, &EFFORT_SUFFIXES);
            if effort != s {
                s = effort.to_owned();
            } else {
                let context = peel_suffix(&s, &CONTEXT_SUFFIXES);
                if context != s {
                    s = context.to_owned();
                } else if lower.ends_with("-max") && !lower.ends_with("codex-max") {
                    s.truncate(s.len() - 4);
                }
            }
        }
        if s == prev || s.is_empty() {
            return s;
        }
    }
}

/// After peeling effort / thinking / max-mode / window, does this source still end in `-fast`?
#[must_use]
pub fn source_is_fast(id: &str) -> bool {
    let s = id.trim();
    if s.is_empty() {
        return false;
    }
    peel_effort_context_max(s.to_owned())
        .to_ascii_lowercase()
        .ends_with("-fast")
}

/// One picker family id: drop effort / fast / thinking / max-mode / window suffixes.
#[must_use]
pub fn picker_family_id(id: &str) -> String {
    let mut s = id.trim().to_owned();
    if s.is_empty() {
        return String::new();
    }
    if s == "auto" || s == "default" {
        return "default".to_owned();
    }
    if let Some(rest) = s.strip_prefix("cursor-") {
        let lower = rest.to_ascii_lowercase();
        if ["claude-", "gpt-", "grok-", "gemini-", "composer-", "kimi-"]
            .iter()
            .any(|p| lower.starts_with(p))
        {
            s = rest.to_owned();
        }
    }
    loop {
        let prev = s.clone();
        let lower = s.to_ascii_lowercase();
        if lower.ends_with("-fast") {
            s.truncate(s.len() - 5);
        } else {
            s = peel_effort_context_max(s);
        }
        if s == prev || s.is_empty() {
            return s;
        }
    }
}

fn clean_picker_name(name: &str) -> String {
    let drop = [
        "extra high",
        "max mode",
        "none",
        "low",
        "medium",
        "high",
        "fast",
        "thinking",
        "max",
        "1m",
        "272k",
        "256k",
    ];
    let mut out = name.to_owned();
    loop {
        let lower = out.to_ascii_lowercase();
        let mut cut = None;
        for phrase in drop {
            if let Some(idx) = lower.find(phrase) {
                let before_ok = idx == 0 || lower.as_bytes()[idx - 1].is_ascii_whitespace();
                let after = idx + phrase.len();
                let after_ok = after == lower.len()
                    || lower
                        .as_bytes()
                        .get(after)
                        .is_some_and(|b| !b.is_ascii_alphanumeric());
                if before_ok && after_ok && (idx > 0 || after < lower.len()) {
                    cut = Some((idx, after));
                    break;
                }
            }
        }
        let Some((start, end)) = cut else {
            break;
        };
        out.replace_range(start..end, " ");
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn pretty_family_name(id: &str) -> String {
    if id == "default" {
        return "Cursor Auto".to_owned();
    }
    id.replace(['-', '_'], " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn as_positive(value: Option<u64>) -> Option<u32> {
    value.and_then(|n| u32::try_from(n).ok()).filter(|n| *n > 0)
}

struct LiveRow {
    id: String,
    name: String,
    context_window: Option<u32>,
    max_tokens: Option<u32>,
    supports_images: Option<bool>,
    has_fast: bool,
}

fn usable_rows(models: &[UsableModel]) -> Vec<LiveRow> {
    models
        .iter()
        .filter_map(|model| {
            let id = model.id.trim();
            if id.is_empty() {
                return None;
            }
            Some(LiveRow {
                id: id.to_owned(),
                name: if model.name.is_empty() {
                    id.to_owned()
                } else {
                    model.name.clone()
                },
                context_window: None,
                max_tokens: None,
                supports_images: None,
                has_fast: source_is_fast(id),
            })
        })
        .collect()
}

fn parameterized_rows(models: &[AvailableModel]) -> Vec<LiveRow> {
    models
        .iter()
        .filter_map(|model| {
            let id = model.name.trim();
            if id.is_empty() {
                return None;
            }
            Some(LiveRow {
                id: id.to_owned(),
                name: model
                    .client_display_name
                    .clone()
                    .or_else(|| model.server_model_name.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| id.to_owned()),
                context_window: as_positive(model.context_token_limit)
                    .or_else(|| as_positive(model.context_token_limit_for_max_mode)),
                max_tokens: None,
                supports_images: Some(model.supports_images),
                has_fast: source_is_fast(id),
            })
        })
        .collect()
}

fn picker_row(
    id: &str,
    name: &str,
    context_window: u32,
    max_tokens: u32,
    input: &[&str],
) -> PickerModel {
    PickerModel {
        id: id.to_owned(),
        name: name.to_owned(),
        context_window,
        max_tokens,
        input: input.iter().map(|s| (*s).to_owned()).collect(),
    }
}

struct Rank {
    group: i32,
    index: usize,
    fast: i32,
    key: String,
}

fn picker_rank(id: &str) -> Rank {
    if id == "default" {
        return Rank {
            group: -1,
            index: 0,
            fast: 0,
            key: id.to_owned(),
        };
    }
    let fast = i32::from(id.ends_with("-fast"));
    let base = if fast == 1 { &id[..id.len() - 5] } else { id };
    let floor = static_models();
    if let Some(index) = floor.iter().position(|model| model.id == base) {
        Rank {
            group: 0,
            index,
            fast,
            key: base.to_owned(),
        }
    } else {
        Rank {
            group: 1,
            index: 0,
            fast,
            key: base.to_owned(),
        }
    }
}

fn compare_picker(a: &PickerModel, b: &PickerModel) -> std::cmp::Ordering {
    let left = picker_rank(&a.id);
    let right = picker_rank(&b.id);
    left.group
        .cmp(&right.group)
        .then_with(|| {
            if left.group == 0 {
                left.index.cmp(&right.index)
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .then_with(|| left.key.cmp(&right.key))
        .then_with(|| left.fast.cmp(&right.fast))
}

/// Empty live → static floor. Non-empty live is trusted (retired models stay gone).
#[must_use]
pub fn merge_static_floor(live: &[PickerModel]) -> Vec<PickerModel> {
    let mut rows = if live.is_empty() {
        static_models()
    } else {
        live.to_vec()
    };
    rows.sort_by(compare_picker);
    rows
}

struct Group {
    names: Vec<String>,
    windows: Vec<u32>,
    outputs: Vec<u32>,
    images: Vec<bool>,
    has_fast: bool,
}

/// Collapse live ids into one picker row per family, plus `{family}-fast` when a source is Fast.
#[must_use]
pub fn to_picker_models(
    usable: &[UsableModel],
    parameterized: &[AvailableModel],
) -> Vec<PickerModel> {
    let mut groups: HashMap<String, Group> = HashMap::new();
    for row in usable_rows(usable)
        .into_iter()
        .chain(parameterized_rows(parameterized))
    {
        if is_internal_model(&row.id, &row.name) {
            continue;
        }
        let family = picker_family_id(&row.id);
        if family.is_empty() {
            continue;
        }
        let group = groups.entry(family).or_insert_with(|| Group {
            names: Vec::new(),
            windows: Vec::new(),
            outputs: Vec::new(),
            images: Vec::new(),
            has_fast: false,
        });
        group.names.push(row.name);
        if let Some(w) = row.context_window {
            group.windows.push(w);
        }
        if let Some(o) = row.max_tokens {
            group.outputs.push(o);
        }
        if let Some(flag) = row.supports_images {
            group.images.push(flag);
        }
        if row.has_fast {
            group.has_fast = true;
        }
    }
    let mut models = Vec::new();
    for (id, group) in groups {
        let mut cleaned: Vec<String> = group
            .names
            .iter()
            .map(|n| clean_picker_name(n))
            .filter(|n| !n.is_empty())
            .collect();
        cleaned.sort_by_key(String::len);
        let name = if id == "default" {
            pretty_family_name(&id)
        } else {
            cleaned
                .first()
                .cloned()
                .unwrap_or_else(|| pretty_family_name(&id))
        };
        let inferred = infer_context_window(&id, &name);
        let window = clamp_context_window(
            &id,
            &name,
            group.windows.iter().copied().max().unwrap_or(inferred),
        );
        let max_tokens = group
            .outputs
            .iter()
            .copied()
            .max()
            .unwrap_or_else(|| infer_max_output_tokens(&id, &name));
        let input: &[&str] =
            if group.images.iter().any(|flag| !*flag) && !group.images.iter().any(|flag| *flag) {
                &["text"]
            } else {
                &VISION
            };
        models.push(picker_row(&id, &name, window, max_tokens, input));
        if id != "default" && group.has_fast {
            models.push(picker_row(
                &format!("{id}-fast"),
                &format!("{name} Fast"),
                window,
                max_tokens,
                input,
            ));
        }
    }
    models.sort_by(compare_picker);
    models
}

/// Overlay live discovery onto the static floor and remember it for this access token.
#[must_use]
pub fn remember_catalog(
    token: &str,
    live: Vec<PickerModel>,
    now_ms: i64,
    ttl_ms: i64,
) -> Vec<PickerModel> {
    let models = merge_static_floor(&live);
    if models.is_empty() {
        return static_models();
    }
    if let Ok(mut cached) = cache().lock() {
        cached.token_hash = catalog_token_hash(token);
        cached.models = models.clone();
        cached.expires_at = now_ms.saturating_add(ttl_ms);
    }
    models
}

/// Cached live list when the token hash still matches and TTL has not elapsed.
#[must_use]
pub fn cached_catalog(token: &str, now_ms: i64) -> Option<Vec<PickerModel>> {
    let hash = catalog_token_hash(token);
    cache().lock().ok().and_then(|cached| {
        if cached.token_hash == hash && !cached.models.is_empty() && now_ms < cached.expires_at {
            Some(cached.models.clone())
        } else {
            None
        }
    })
}
