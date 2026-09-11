//! Import a Codex CLI / Hermes `auth.json` object. No Grok/GLM helpers.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::{Error, Session};

use super::login::{TokenSet, session_from_tokens, token_set_from_value};

#[cfg(test)]
mod tests;

const HERMES_KEYS: [&str; 4] = ["openai-codex", "openai_codex", "codex", "chatgpt"];

/// `~/.codex/auth.json` then `~/.hermes/auth.json`.
#[must_use]
pub fn search_paths() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
    else {
        return Vec::new();
    };
    vec![
        home.join(".codex").join("auth.json"),
        home.join(".hermes").join("auth.json"),
    ]
}

/// Try CLI tokens, then Hermes provider keys.
///
/// # Errors
/// Missing tokens, or a payload that cannot become a [`Session`].
pub fn session_from_json(raw: &Value) -> Result<Session, Error> {
    let tokens = tokens_from_cli(raw)
        .or_else(|| tokens_from_hermes(raw))
        .ok_or_else(|| Error::InvalidImport("no Codex session in JSON".to_owned()))?;
    session_from_tokens(with_expiry(tokens, raw), None)
}

/// Read the first path that holds a Codex session.
///
/// # Errors
/// Unreadable files (other than missing), unusable JSON, or nothing found.
pub fn import_from_paths(paths: &[PathBuf]) -> Result<Session, Error> {
    let mut tried = Vec::new();
    for path in paths {
        tried.push(path.display().to_string());
        let Some(raw) = read_json(path)? else {
            continue;
        };
        let from_cli = path
            .to_string_lossy()
            .contains(".codex")
            .then(|| tokens_from_cli(&raw))
            .flatten();
        let tokens = from_cli.or_else(|| tokens_from_hermes(&raw));
        let Some(tokens) = tokens else {
            continue;
        };
        let mut session = session_from_tokens(with_expiry(tokens, &raw), None)?;
        session.source = path.display().to_string();
        return Ok(session);
    }
    Err(Error::InvalidImport(format!(
        "no Codex session found in {}",
        tried.join(" or ")
    )))
}

fn read_json(path: &Path) -> Result<Option<Value>, Error> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|err| Error::InvalidImport(err.to_string())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(Error::InvalidImport(err.to_string())),
    }
}

fn tokens_from_cli(raw: &Value) -> Option<TokenSet> {
    let obj = raw.as_object()?;
    let nested = obj
        .get("tokens")
        .filter(|value| value.is_object())
        .unwrap_or(raw);
    let mut tokens = token_set_from_value(nested).ok()?;
    if tokens.refresh_token.is_none() {
        tokens.refresh_token = str_field(obj, &["refresh_token", "refreshToken"]);
    }
    if tokens.id_token.is_none() {
        tokens.id_token = str_field(obj, &["id_token", "idToken"]);
    }
    if tokens.expires_in.is_none() {
        tokens.expires_in = positive_i64(obj.get("expires_in").or_else(|| obj.get("expiresIn")));
    }
    Some(tokens)
}

fn tokens_from_hermes(raw: &Value) -> Option<TokenSet> {
    let obj = raw.as_object()?;
    let providers = obj
        .get("providers")
        .or_else(|| obj.get("auth"))
        .and_then(Value::as_object)
        .unwrap_or(obj);
    for key in HERMES_KEYS {
        if let Some(tokens) = hermes_entry_tokens(providers.get(key).or_else(|| obj.get(key))?) {
            return Some(tokens);
        }
    }
    let pool = obj
        .get("credential_pool")
        .or_else(|| obj.get("credentialPool"))
        .and_then(Value::as_object)?;
    for key in HERMES_KEYS {
        let Some(Value::Array(rows)) = pool.get(key) else {
            continue;
        };
        for row in rows {
            if let Some(tokens) = hermes_entry_tokens(row) {
                return Some(tokens);
            }
        }
    }
    None
}

fn hermes_entry_tokens(entry: &Value) -> Option<TokenSet> {
    let obj = entry.as_object()?;
    if let Some(nested) = obj.get("tokens").filter(|value| value.is_object())
        && let Ok(tokens) = token_set_from_value(nested)
    {
        return Some(merge_entry_tokens(tokens, obj));
    }
    token_set_from_value(entry)
        .ok()
        .map(|tokens| merge_entry_tokens(tokens, obj))
}

fn merge_entry_tokens(mut tokens: TokenSet, entry: &Map<String, Value>) -> TokenSet {
    if tokens.id_token.is_none() {
        tokens.id_token = str_field(entry, &["id_token", "idToken"]);
    }
    if tokens.refresh_token.is_none() {
        tokens.refresh_token = str_field(entry, &["refresh_token", "refreshToken"]);
    }
    if tokens.expires_in.is_none() {
        tokens.expires_in =
            positive_i64(entry.get("expires_in").or_else(|| entry.get("expiresIn")));
    }
    tokens
}

fn with_expiry(mut tokens: TokenSet, raw: &Value) -> TokenSet {
    if tokens.expires_in.is_some() {
        return tokens;
    }
    let nested = raw.get("tokens");
    let from_at = parse_time(
        raw.get("expires_at")
            .or_else(|| raw.get("expiresAt"))
            .or_else(|| nested.and_then(|value| value.get("expires_at")))
            .or_else(|| nested.and_then(|value| value.get("expiresAt"))),
    );
    if let Some(at) = from_at {
        tokens.expires_in = Some(((at - now_ms()) / 1000).max(60));
        return tokens;
    }
    let last_refresh = raw
        .get("last_refresh")
        .or_else(|| raw.get("lastRefresh"))
        .or_else(|| nested.and_then(|value| value.get("last_refresh")));
    if let Some(stamp) = parse_time(last_refresh) {
        tokens.expires_in = Some(((stamp + 3_600_000 - now_ms()) / 1000).max(60));
        return tokens;
    }
    tokens.expires_in = Some(3600);
    tokens
}

fn parse_time(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    match value {
        Value::Number(number) => {
            let n = number.as_f64().filter(|n| *n > 0.0)?;
            Some(if n > 1e12 {
                n.round() as i64
            } else {
                (n * 1000.0).round() as i64
            })
        }
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(n) = trimmed.parse::<f64>()
                && n > 0.0
            {
                return Some(if n > 1e12 {
                    n.round() as i64
                } else {
                    (n * 1000.0).round() as i64
                });
            }
            let iso = trimmed.replace(' ', "T");
            chrono::DateTime::parse_from_rfc3339(&iso)
                .ok()
                .map(|stamp| stamp.timestamp_millis())
        }
        _ => None,
    }
}

fn str_field(obj: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        obj.get(*name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

fn positive_i64(value: Option<&Value>) -> Option<i64> {
    let n = match value? {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse().ok()?,
        _ => return None,
    };
    if n.is_finite() && n > 0.0 {
        Some(n.round() as i64)
    } else {
        None
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
