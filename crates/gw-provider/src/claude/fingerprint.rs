//! Claude Code client cloak for **OAuth / subscription** traffic.
//!
//! Console API keys never enter this module. A Claude Code access token is
//! rejected unless the request looks like the installed CLI: headers, first
//! system block, and a stable device/session identity. The cloak fills gaps
//! only — a real `claude` client already sent the right shape, and rewriting
//! its billing block would invalidate `cch`.
//!
//! # What this module does not do
//!
//! * **TLS / JA3.** `gw-relay` speaks rustls. Chrome-like ClientHello needs a
//!   different HTTP stack (uTLS). That is documented, not faked. See
//!   `docs/claude-fingerprint.md`.
//! * **Bun xxHash64 `cch`.** Current Claude Code signs the serialized body
//!   with a native hash after a `cch=00000` placeholder. This crate uses the
//!   published JS-side SHA-256 construction (NTT123 gist vectors) until a
//!   live capture on the operator's box says otherwise. Do not invent a
//!   second algorithm without that capture.

use std::sync::OnceLock;

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::types::ProviderRequest;

/// Pin used when `AGW_CLAUDE_CODE_VERSION` is unset and `claude --version`
/// is not on PATH. Bump this only when refreshing the documented pin — never
/// per request, and never downwards for a live process.
const FALLBACK_VERSION: &str = "2.1.233";

const VERSION_ENV: &str = "AGW_CLAUDE_CODE_VERSION";

/// Published Claude Code JS billing salt (NTT123 gist / community RE).
const BILLING_SALT: &str = "59cf53e54c78";

const BILLING_PREFIX: &str = "x-anthropic-billing-header:";
const CC_IDENTIFIER: &str = "You are Claude Code, Anthropic's official CLI for Claude.";

/// Fills Claude Code headers and, when the body is JSON, the billing system
/// block. Returns `None` when the inbound bytes stay untouched.
pub(super) fn cloak(req: &ProviderRequest, headers: &mut HeaderMap) -> Option<Bytes> {
    fill_headers(headers, &req.headers, &pinned_version());
    rewrite_body(&req.payload, &pinned_version())
}

/// Headers for identity probes (GET `/v1/models`). Not a Messages body.
#[cfg(test)]
#[must_use]
pub(crate) fn probe_headers() -> Vec<(String, String)> {
    let mut headers = HeaderMap::new();
    fill_headers(&mut headers, &HeaderMap::new(), &pinned_version());
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_owned(), v.to_owned()))
        })
        .collect()
}

/// Version string used on the wire (`2.1.233`, no `v` prefix).
#[must_use]
pub(crate) fn pinned_version() -> String {
    static PIN: OnceLock<String> = OnceLock::new();
    PIN.get_or_init(resolve_version).clone()
}

fn resolve_version() -> String {
    if let Some(from_env) = std::env::var(VERSION_ENV)
        .ok()
        .map(|raw| raw.trim().to_owned())
        .filter(|raw| !raw.is_empty())
    {
        return strip_v_prefix(&from_env);
    }
    discover_cli_version().unwrap_or_else(|| FALLBACK_VERSION.to_owned())
}

fn discover_cli_version() -> Option<String> {
    let output = std::process::Command::new("claude")
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_version_text(&String::from_utf8_lossy(&output.stdout))
}

fn parse_version_text(text: &str) -> Option<String> {
    let mut best: Option<String> = None;
    let mut current = String::new();
    let mut dots = 0u8;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if ch == '.' && !current.is_empty() {
            current.push(ch);
            dots = dots.saturating_add(1);
        } else {
            consider_version(&mut best, &current, dots);
            current.clear();
            dots = 0;
        }
    }
    consider_version(&mut best, &current, dots);
    best
}

fn consider_version(best: &mut Option<String>, candidate: &str, dots: u8) {
    if dots < 2 || candidate.ends_with('.') {
        return;
    }
    let trimmed = candidate.trim_end_matches('.');
    if best
        .as_deref()
        .is_none_or(|have| version_newer(trimmed, have))
    {
        *best = Some(trimmed.to_owned());
    }
}

fn version_newer(candidate: &str, have: &str) -> bool {
    let left: Vec<u32> = candidate
        .split('.')
        .filter_map(|p| p.parse().ok())
        .collect();
    let right: Vec<u32> = have.split('.').filter_map(|p| p.parse().ok()).collect();
    left > right
}

fn strip_v_prefix(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('v')
        .trim_start_matches('V')
        .to_owned()
}

fn fill_headers(headers: &mut HeaderMap, inbound: &HeaderMap, version: &str) {
    insert_gap(
        headers,
        inbound,
        http::header::USER_AGENT,
        &format!("claude-cli/{version} (external, cli)"),
    );
    insert_gap(headers, inbound, HeaderName::from_static("x-app"), "cli");
    insert_gap(
        headers,
        inbound,
        HeaderName::from_static("x-stainless-lang"),
        "js",
    );
    insert_gap(
        headers,
        inbound,
        HeaderName::from_static("x-stainless-runtime"),
        "node",
    );
    insert_gap(
        headers,
        inbound,
        HeaderName::from_static("x-stainless-os"),
        stainless_os(),
    );
    insert_gap(
        headers,
        inbound,
        HeaderName::from_static("x-stainless-arch"),
        stainless_arch(),
    );
    insert_gap(
        headers,
        inbound,
        HeaderName::from_static("x-claude-code-session-id"),
        session_id(),
    );
    insert_gap(
        headers,
        inbound,
        HeaderName::from_static("x-client-request-id"),
        &Uuid::new_v4().to_string(),
    );
}

fn insert_gap(headers: &mut HeaderMap, inbound: &HeaderMap, name: HeaderName, value: &str) {
    if inbound.contains_key(&name) || headers.contains_key(&name) {
        return;
    }
    if let Ok(header) = HeaderValue::from_str(value) {
        headers.insert(name, header);
    }
}

fn stainless_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "MacOS",
        "windows" => "Windows",
        _ => "Linux",
    }
}

fn stainless_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" | "arm64" => "arm64",
        _ => "x64",
    }
}

fn session_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| Uuid::new_v4().to_string())
}

fn account_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| Uuid::new_v4().to_string())
}

fn user_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        format!(
            "user_{}_account_{}_session_{}",
            Uuid::new_v4(),
            account_id(),
            session_id()
        )
    })
}

fn rewrite_body(payload: &[u8], version: &str) -> Option<Bytes> {
    if payload.is_empty() {
        return None;
    }
    let mut value: Value = serde_json::from_slice(payload).ok()?;
    if !value.is_object() {
        return None;
    }
    let first_user = first_user_text(&value);
    let already_cloaked = system_starts_with_billing(&value);
    let has_cache = json_contains_key(&value, "cache_control");
    {
        let object = value.as_object_mut()?;
        if !already_cloaked {
            prepend_identity(object, &billing_header(&first_user, version));
        }
        ensure_metadata_user_id(object);
        if !has_cache {
            if let Some(system) = object.get_mut("system") {
                mark_last_non_billing(system);
            }
            if let Some(tools) = object.get_mut("tools") {
                mark_last_object(tools);
            }
        }
    }
    serde_json::to_vec(&value).ok().map(Bytes::from)
}

/// JS-side billing line. Published vector: message `"hey"` + version
/// `2.1.37` → `cc_version=2.1.37.0d9` / `cch=fa690`.
#[must_use]
pub(crate) fn billing_header(message_text: &str, version: &str) -> String {
    let sampled: String = [4usize, 7, 20]
        .into_iter()
        .map(|i| message_text.chars().nth(i).unwrap_or('0'))
        .collect();
    let version_hash = &sha256_hex(&format!("{BILLING_SALT}{sampled}{version}"))[..3];
    let cch = &sha256_hex(message_text)[..5];
    format!("{BILLING_PREFIX} cc_version={version}.{version_hash}; cc_entrypoint=cli; cch={cch};")
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

fn prepend_identity(object: &mut serde_json::Map<String, Value>, billing: &str) {
    let mut blocks = vec![json!({ "type": "text", "text": billing })];
    let existing = object.remove("system");
    let has_identifier = match &existing {
        Some(Value::String(text)) => text.contains("Claude Code"),
        Some(Value::Array(items)) => items.iter().any(|item| {
            item.get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("Claude Code"))
        }),
        _ => false,
    };
    if !has_identifier {
        blocks.push(json!({ "type": "text", "text": CC_IDENTIFIER }));
    }
    match existing {
        Some(Value::String(text)) => {
            blocks.push(json!({ "type": "text", "text": text }));
        }
        Some(Value::Array(mut items)) => blocks.append(&mut items),
        Some(other) => blocks.push(other),
        None => {}
    }
    object.insert("system".to_owned(), Value::Array(blocks));
}

fn ensure_metadata_user_id(object: &mut serde_json::Map<String, Value>) {
    match object.get_mut("metadata") {
        Some(Value::Object(meta)) if !meta.contains_key("user_id") => {
            meta.insert("user_id".to_owned(), json!(user_id()));
        }
        Some(Value::Object(_)) => {}
        Some(_) => {}
        None => {
            object.insert("metadata".to_owned(), json!({ "user_id": user_id() }));
        }
    }
}

fn system_starts_with_billing(value: &Value) -> bool {
    match value.get("system") {
        Some(Value::String(text)) => text.starts_with(BILLING_PREFIX),
        Some(Value::Array(items)) => items
            .first()
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str)
            .is_some_and(|text| text.starts_with(BILLING_PREFIX)),
        _ => false,
    }
}

fn first_user_text(value: &Value) -> String {
    let Some(Value::Array(messages)) = value.get("messages") else {
        return String::new();
    };
    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        return match message.get("content") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Array(parts)) => parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(""),
            _ => String::new(),
        };
    }
    String::new()
}

fn mark_last_non_billing(system: &mut Value) -> bool {
    let Value::Array(items) = system else {
        return false;
    };
    let Some(last) = items.iter_mut().rev().find(|item| {
        !item
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| text.starts_with(BILLING_PREFIX))
    }) else {
        return false;
    };
    let Some(object) = last.as_object_mut() else {
        return false;
    };
    object.insert("cache_control".to_owned(), json!({ "type": "ephemeral" }));
    true
}

fn mark_last_object(value: &mut Value) -> bool {
    let Value::Array(items) = value else {
        return false;
    };
    let Some(last) = items.iter_mut().rev().find(|item| item.is_object()) else {
        return false;
    };
    let Some(object) = last.as_object_mut() else {
        return false;
    };
    object.insert("cache_control".to_owned(), json!({ "type": "ephemeral" }));
    true
}

fn json_contains_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(map) => {
            map.contains_key(key) || map.values().any(|child| json_contains_key(child, key))
        }
        Value::Array(items) => items.iter().any(|child| json_contains_key(child, key)),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
