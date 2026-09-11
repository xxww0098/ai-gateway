//! Short in-process backoff for a Cursor refresh token that already failed.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

#[cfg(test)]
mod tests;

const BACKOFF_MS: i64 = 10 * 60_000;

static FAILED: LazyLock<Mutex<HashMap<String, i64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn table() -> std::sync::MutexGuard<'static, HashMap<String, i64>> {
    FAILED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn key_of(token: &str) -> Option<String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// True while this refresh token is inside the failure backoff window.
#[must_use]
pub fn is_known_bad(token: &str, now_ms: i64) -> bool {
    let Some(key) = key_of(token) else {
        return false;
    };
    let mut table = table();
    let Some(&until) = table.get(&key) else {
        return false;
    };
    if until <= now_ms {
        table.remove(&key);
        return false;
    }
    true
}

/// Remember a failed refresh so the next snapshot does not stall on it.
pub fn mark_failed(token: &str, now_ms: i64) {
    let Some(key) = key_of(token) else {
        return;
    };
    table().insert(key, now_ms.saturating_add(BACKOFF_MS));
}

/// Clear the backoff after a successful refresh.
pub fn mark_succeeded(token: &str) {
    let Some(key) = key_of(token) else {
        return;
    };
    table().remove(&key);
}

/// Drop the in-process table (tests).
pub fn reset() {
    table().clear();
}
