//! Read OAuth files already on the host (Codex CLI, Claude Code, Grok, Kiro).
//!
//! The gateway does not invent a second browser login for these CLIs. An
//! operator who already signed in locally can import the file the CLI wrote.
//! Discovery only walks well-known paths under a home directory — it never
//! takes a client-supplied filesystem path.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

#[cfg(test)]
mod tests;

/// One credential lifted out of a CLI auth file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalOauthCred {
    /// `claude` / `codex` / `xai` / `kiro`.
    pub provider: &'static str,
    /// Absolute path the bytes were read from. For logs only; never a secret.
    pub source: PathBuf,
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    /// RFC 3339 or the CLI's original stamp; empty when the file had none.
    pub expires_at: String,
    pub email: String,
}

impl LocalOauthCred {
    /// JSON an `/auth-files` upload already understands.
    #[must_use]
    pub fn to_upload_json(&self) -> Value {
        let mut token_data = Map::new();
        insert_nonempty(&mut token_data, "access_token", &self.access_token);
        insert_nonempty(&mut token_data, "refresh_token", &self.refresh_token);
        insert_nonempty(&mut token_data, "id_token", &self.id_token);
        insert_nonempty(&mut token_data, "expires_at", &self.expires_at);
        insert_nonempty(&mut token_data, "email", &self.email);

        let mut root = Map::new();
        root.insert("provider".to_owned(), json!(self.provider));
        insert_nonempty(&mut root, "access_token", &self.access_token);
        insert_nonempty(&mut root, "refresh_token", &self.refresh_token);
        insert_nonempty(&mut root, "id_token", &self.id_token);
        insert_nonempty(&mut root, "expires_at", &self.expires_at);
        insert_nonempty(&mut root, "email", &self.email);
        if !token_data.is_empty() {
            root.insert("token_data".to_owned(), Value::Object(token_data));
        }
        Value::Object(root)
    }
}

/// Process home: `AGW_LOCAL_OAUTH_HOME`, then `HOME`, then `USERPROFILE`.
#[must_use]
pub fn process_home() -> Option<PathBuf> {
    for key in ["AGW_LOCAL_OAUTH_HOME", "HOME", "USERPROFILE"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(PathBuf::from(trimmed));
            }
        }
    }
    None
}

/// Well-known CLI paths under `home` (plus Claude / Grok env overrides).
#[must_use]
pub fn well_known_paths(home: &Path) -> Vec<PathBuf> {
    let mut paths = vec![
        home.join(".codex").join("auth.json"),
        home.join(".claude").join(".credentials.json"),
        home.join(".grok").join("auth.json"),
        home.join(".hermes").join("auth.json"),
        home.join(".kiro").join("credentials.json"),
        home.join(".aws")
            .join("sso")
            .join("cache")
            .join("kiro-auth-token.json"),
    ];
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        let dir = dir.trim();
        if !dir.is_empty() {
            paths.push(PathBuf::from(dir).join(".credentials.json"));
        }
    }
    if let Ok(dir) = std::env::var("GROK_HOME") {
        let dir = dir.trim();
        if !dir.is_empty() {
            paths.push(PathBuf::from(dir).join("auth.json"));
        }
    }
    paths
}

/// Scan `home` for CLI OAuth files. Missing files are skipped, not errors.
#[must_use]
pub fn discover(home: &Path) -> Vec<LocalOauthCred> {
    let mut found = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for path in well_known_paths(home) {
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let Ok(bytes) = fs::read(&canonical) else {
            continue;
        };
        found.extend(parse_cli_bytes(&bytes, &canonical));
    }
    found
}

/// Parse one file. A Hermes store can yield more than one family.
#[must_use]
pub fn parse_cli_bytes(bytes: &[u8], source: &Path) -> Vec<LocalOauthCred> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    parse_cli_value(&value, source)
}

/// Infer the provider of a CLI / upload JSON object.
#[must_use]
pub fn infer_provider(payload: &Map<String, Value>) -> Option<&'static str> {
    if payload.contains_key("claudeAiOauth") {
        return Some("claude");
    }
    if looks_like_codex(payload) {
        return Some("codex");
    }
    if looks_like_grok(payload) {
        return Some("xai");
    }
    if looks_like_kiro(payload) {
        return Some("kiro");
    }
    None
}

/// Lift CLI nesting (`claudeAiOauth`, `tokens`, camelCase) so `/auth-files`
/// upload can copy the flat keys it already knows.
pub fn lift_cli_shape(payload: &mut Map<String, Value>) {
    if let Some(Value::Object(oauth)) = payload.get("claudeAiOauth").cloned() {
        lift_alias_map(&oauth, payload);
        payload.entry("provider").or_insert_with(|| json!("claude"));
    }
    if let Some(Value::Object(tokens)) = payload.get("tokens").cloned() {
        for key in ["access_token", "refresh_token", "id_token", "expires_in"] {
            if !payload.contains_key(key)
                && let Some(value) = tokens.get(key)
            {
                payload.insert(key.to_owned(), value.clone());
            }
        }
        payload
            .entry("token_data")
            .or_insert_with(|| Value::Object(tokens));
        if infer_provider(payload) == Some("codex") {
            payload.entry("provider").or_insert_with(|| json!("codex"));
        }
    }
    lift_alias_map(&payload.clone(), payload);
    if let Some(provider) = infer_provider(payload) {
        payload.entry("provider").or_insert_with(|| json!(provider));
    }
}

fn parse_cli_value(value: &Value, source: &Path) -> Vec<LocalOauthCred> {
    let Some(root) = value.as_object() else {
        return Vec::new();
    };
    let hint = source.to_string_lossy();
    let mut out = Vec::new();

    if let Some(cred) = from_claude(root, source) {
        out.push(cred);
    }
    if let Some(cred) = from_codex(root, source) {
        out.push(cred);
    }
    if let Some(cred) = from_grok(root, source) {
        out.push(cred);
    }
    for cred in hermes_all(root, source) {
        if out
            .iter()
            .all(|existing| existing.provider != cred.provider)
        {
            out.push(cred);
        }
    }
    if out.is_empty()
        && (hint.contains("kiro") || looks_like_kiro(root))
        && let Some(cred) = from_flat(root, "kiro", source)
    {
        out.push(cred);
    }
    out
}

fn from_claude(root: &Map<String, Value>, source: &Path) -> Option<LocalOauthCred> {
    let oauth = root.get("claudeAiOauth")?.as_object()?;
    from_flat(oauth, "claude", source)
}

fn from_codex(root: &Map<String, Value>, source: &Path) -> Option<LocalOauthCred> {
    let tokens = root
        .get("tokens")
        .and_then(Value::as_object)
        .unwrap_or(root);
    if !looks_like_codex(root) && source.to_string_lossy().contains(".codex") {
        return from_flat(tokens, "codex", source);
    }
    if looks_like_codex(root) {
        return from_flat(tokens, "codex", source);
    }
    None
}

fn from_grok(root: &Map<String, Value>, source: &Path) -> Option<LocalOauthCred> {
    if looks_like_grok(root) {
        return from_flat(root, "xai", source);
    }
    let hint = source.to_string_lossy();
    if hint.contains(".grok") || hint.contains("hermes") {
        if let Some(nested) = hermes_entry(root, GROK_HERMES_KEYS) {
            return from_flat(nested, "xai", source);
        }
        if pick(root, &["access_token", "accessToken"]).is_some()
            && pick(root, &["refresh_token", "refreshToken"]).is_some()
        {
            return from_flat(root, "xai", source);
        }
    }
    None
}

fn hermes_all(root: &Map<String, Value>, source: &Path) -> Vec<LocalOauthCred> {
    let mut out = Vec::new();
    if let Some(nested) = hermes_entry(root, CODEX_HERMES_KEYS)
        && let Some(cred) = from_flat(nested, "codex", source)
    {
        out.push(cred);
    }
    if let Some(nested) = hermes_entry(root, GROK_HERMES_KEYS)
        && let Some(cred) = from_flat(nested, "xai", source)
    {
        out.push(cred);
    }
    out
}

fn from_flat(
    map: &Map<String, Value>,
    provider: &'static str,
    source: &Path,
) -> Option<LocalOauthCred> {
    let access_token = pick(map, &["access_token", "accessToken", "key"])?;
    if access_token.is_empty() {
        return None;
    }
    Some(LocalOauthCred {
        provider,
        source: source.to_path_buf(),
        access_token,
        refresh_token: pick(map, &["refresh_token", "refreshToken"]).unwrap_or_default(),
        id_token: pick(map, &["id_token", "idToken"]).unwrap_or_default(),
        expires_at: expires_stamp(map),
        email: pick(map, &["email", "account"]).unwrap_or_default(),
    })
}

fn looks_like_codex(payload: &Map<String, Value>) -> bool {
    let tokens = payload
        .get("tokens")
        .and_then(Value::as_object)
        .unwrap_or(payload);
    tokens.get("access_token").and_then(Value::as_str).is_some()
        && (tokens.get("id_token").is_some()
            || payload.get("last_refresh").is_some()
            || payload.get("lastRefresh").is_some())
}

fn looks_like_grok(payload: &Map<String, Value>) -> bool {
    let mode = pick(payload, &["auth_mode", "authMode"]).unwrap_or_default();
    let mode = mode.to_ascii_lowercase();
    if mode == "oidc" || mode == "oauth" || mode == "supergrok" {
        return pick(payload, &["access_token", "accessToken", "key"]).is_some()
            && pick(payload, &["refresh_token", "refreshToken"]).is_some();
    }
    let issuer = pick(payload, &["oidc_issuer", "oidcIssuer", "issuer"]).unwrap_or_default();
    issuer.contains("auth.x.ai")
}

fn looks_like_kiro(payload: &Map<String, Value>) -> bool {
    pick(payload, &["start_url", "startUrl"]).is_some()
        || pick(payload, &["auth_method", "authMethod"]).is_some_and(|value| {
            let lower = value.to_ascii_lowercase();
            lower.contains("kiro") || lower == "import" || lower.contains("builder")
        })
}

const CODEX_HERMES_KEYS: &[&str] = &["openai-codex", "openai_codex", "codex", "chatgpt"];
const GROK_HERMES_KEYS: &[&str] = &[
    "xai-oauth",
    "grok-oauth",
    "x-ai-oauth",
    "xai-grok-oauth",
    "xai",
    "x-ai",
    "grok",
    "xai-grok",
];

fn hermes_entry<'a>(root: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Map<String, Value>> {
    let providers = root
        .get("providers")
        .or_else(|| root.get("auth"))
        .and_then(Value::as_object)
        .unwrap_or(root);
    for key in keys {
        if let Some(entry) = providers.get(*key).and_then(Value::as_object) {
            let nested = entry
                .get("tokens")
                .and_then(Value::as_object)
                .unwrap_or(entry);
            if pick(nested, &["access_token", "accessToken"]).is_some() {
                return Some(nested);
            }
        }
    }
    None
}

fn lift_alias_map(from: &Map<String, Value>, into: &mut Map<String, Value>) {
    const ALIASES: &[(&str, &str)] = &[
        ("accessToken", "access_token"),
        ("refreshToken", "refresh_token"),
        ("idToken", "id_token"),
        ("expiresAt", "expires_at"),
        ("expiresIn", "expires_in"),
    ];
    for (src, dst) in ALIASES {
        if into.contains_key(*dst) {
            continue;
        }
        if let Some(value) = from.get(*src) {
            into.insert((*dst).to_owned(), value.clone());
        }
    }
}

fn pick(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key).and_then(|value| match value {
            Value::String(text) => {
                let trimmed = text.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_owned())
            }
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
    })
}

fn expires_stamp(map: &Map<String, Value>) -> String {
    if let Some(text) = pick(map, &["expires_at", "expiresAt", "expired"]) {
        return text;
    }
    String::new()
}

fn insert_nonempty(map: &mut Map<String, Value>, key: &str, value: &str) {
    if !value.is_empty() {
        map.insert(key.to_owned(), json!(value));
    }
}
