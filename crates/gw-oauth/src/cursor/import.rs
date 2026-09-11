//! Operator-supplied Cursor token JSON. Not a second OAuth.

use serde_json::Value;

use super::{SessionBuild, cursor_session};
use crate::{Error, Session};

#[cfg(test)]
mod tests;

fn trimmed_field(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }
    None
}

fn object_of(value: &Value) -> Option<&Value> {
    if value.is_object() { Some(value) } else { None }
}

/// Parse pasted Cursor token JSON into a session.
///
/// # Errors
/// When neither `accessToken`/`access`/`access_token` nor a nested `session` is present.
pub fn session_from_json(body: &Value) -> Result<Session, Error> {
    let root =
        object_of(body).ok_or_else(|| Error::InvalidImport("expected a JSON object".into()))?;
    let nested = root
        .get("session")
        .and_then(object_of)
        .or_else(|| root.get("tokens").and_then(object_of));
    let source_obj = nested.unwrap_or(root);
    let access = trimmed_field(
        source_obj,
        &[
            "accessToken",
            "access_token",
            "access",
            "token",
            "CURSOR_ACCESS_TOKEN",
        ],
    )
    .or_else(|| trimmed_field(root, &["accessToken", "access_token", "access", "token"]));
    let Some(access) = access else {
        return Err(Error::InvalidImport(
            "missing accessToken / access / access_token".into(),
        ));
    };
    let refresh = trimmed_field(source_obj, &["refreshToken", "refresh_token", "refresh"])
        .or_else(|| trimmed_field(root, &["refreshToken", "refresh_token", "refresh"]))
        .unwrap_or_else(|| access.clone());
    let source = trimmed_field(source_obj, &["source"])
        .or_else(|| trimmed_field(root, &["source"]))
        .unwrap_or_else(|| "import".to_owned());
    let account = trimmed_field(source_obj, &["account", "email"])
        .or_else(|| trimmed_field(root, &["account", "email"]));
    let cached_email = trimmed_field(source_obj, &["cachedEmail", "cached_email"])
        .or_else(|| trimmed_field(root, &["cachedEmail", "cached_email"]));
    let plan_type = trimmed_field(source_obj, &["planType", "plan_type"])
        .or_else(|| trimmed_field(root, &["planType", "plan_type"]));
    cursor_session(SessionBuild {
        access_token: access,
        refresh_token: Some(refresh),
        expires_at_ms: None,
        account,
        plan_type,
        cached_email,
        source,
    })
}

/// Windows account that owns this WSL session — never Public / Default / others.
#[must_use]
pub fn windows_username_from_env(env: &[(&str, &str)]) -> Option<String> {
    let get = |key: &str| {
        env.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.trim())
            .filter(|v| !v.is_empty())
    };
    if let Some(profile) = get("USERPROFILE") {
        let normalized = profile.replace('\\', "/");
        if let Some(idx) = normalized
            .to_ascii_lowercase()
            .find("/users/")
            .map(|i| i + "/users/".len())
        {
            let rest = &normalized[idx..];
            let from_profile = rest.split('/').next().unwrap_or("").trim();
            if !from_profile.is_empty()
                && from_profile != "Public"
                && from_profile != "Default"
                && !from_profile.starts_with('.')
            {
                return Some(from_profile.to_owned());
            }
        }
    }
    let username = get("USERNAME")?;
    if username == "Public" || username == "Default" || username.starts_with('.') {
        None
    } else {
        Some(username.to_owned())
    }
}

/// IDE `state.vscdb` locations for this OS user. Never walks sibling profiles.
#[must_use]
pub fn vscdb_paths(platform: &str, home: &str, env: &[(&str, &str)]) -> Vec<String> {
    let mut paths = Vec::new();
    match platform {
        "darwin" | "macos" => {
            paths.push(format!(
                "{home}/Library/Application Support/Cursor/User/globalStorage/state.vscdb"
            ));
        }
        "win32" | "windows" => {
            if let Some((_, appdata)) = env.iter().find(|(k, _)| k.eq_ignore_ascii_case("APPDATA"))
                && !appdata.trim().is_empty()
            {
                paths.push(format!(
                    "{}/Cursor/User/globalStorage/state.vscdb",
                    appdata.trim()
                ));
            }
        }
        _ => {
            paths.push(format!(
                "{home}/.config/Cursor/User/globalStorage/state.vscdb"
            ));
            let wsl = env.iter().any(|(k, v)| {
                (*k == "WSL_DISTRO_NAME" || *k == "WSL_INTEROP") && !v.trim().is_empty()
            });
            if wsl && let Some(user) = windows_username_from_env(env) {
                paths.push(format!(
                    "/mnt/c/Users/{user}/AppData/Roaming/Cursor/User/globalStorage/state.vscdb"
                ));
            }
        }
    }
    paths
}
