//! Kiro credential import: 卡密 / JSON / CSV / ksk_. Original parsers.

use serde_json::{Map, Value, json};

use crate::{Error, Session};

use super::session::{
    has_access_token_field, infer_auth_method, is_kiro_credential, json_str, now_ms,
    session_from_import, trimmed,
};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    Json,
    Kami,
    Csv,
    Keys,
    RawToken,
    Empty,
}

impl ImportKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Kami => "kami",
            Self::Csv => "csv",
            Self::Keys => "keys",
            Self::RawToken => "raw-token",
            Self::Empty => "empty",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParsedImport {
    pub kind: ImportKind,
    pub sessions: Vec<Session>,
}

fn csv_header_key(header: &str) -> Option<&'static str> {
    match header.trim() {
        "邮箱" => return Some("email"),
        "昵称" => return Some("nickname"),
        "登录方式" => return Some("provider"),
        "密码" => return Some("password"),
        _ => {}
    }
    match header.trim().to_ascii_lowercase().as_str() {
        "email" => Some("email"),
        "nickname" => Some("nickname"),
        "provider" | "idp" => Some("provider"),
        "password" => Some("password"),
        "refreshtoken" | "refresh token" => Some("refreshToken"),
        "clientid" => Some("clientId"),
        "clientsecret" => Some("clientSecret"),
        "region" => Some("region"),
        "kiroapikey" | "api key" => Some("kiroApiKey"),
        _ => None,
    }
}

#[must_use]
pub fn flatten_import(raw: &Value) -> Vec<Value> {
    if raw.is_null() {
        return Vec::new();
    }
    if let Some(rows) = raw.as_array() {
        return rows.iter().filter(|row| row.is_object()).cloned().collect();
    }
    if !raw.is_object() {
        return Vec::new();
    }
    if let Some(creds) = raw.get("credentials").and_then(Value::as_array)
        && creds.iter().all(|row| row.is_object())
    {
        return creds.clone();
    }
    if let Some(accounts) = raw.get("accounts").and_then(Value::as_array) {
        return accounts
            .iter()
            .filter(|row| row.is_object())
            .cloned()
            .collect();
    }
    if raw
        .get("credentials")
        .is_some_and(|c| c.is_object() && !c.is_array())
        && (raw.get("email").is_some() || raw.get("idp").is_some() || raw.get("account").is_some())
    {
        return vec![raw.clone()];
    }
    vec![raw.clone()]
}

#[must_use]
pub fn sessions_from_auth(raw: &Value) -> Vec<Session> {
    let now = now_ms();
    flatten_import(raw)
        .into_iter()
        .filter(is_kiro_credential)
        .filter_map(|entry| session_from_import(&entry, now).ok())
        .collect()
}

fn split_kami_line(line: &str) -> Vec<String> {
    if line.contains("----") {
        return line.split("----").map(|s| s.to_owned()).collect();
    }
    if line.contains('\t') {
        return line.split('\t').map(|s| s.to_owned()).collect();
    }
    if line.contains("  ") {
        return split_on_runs(line, 2);
    }
    line.split(',').map(|s| s.to_owned()).collect()
}

fn split_on_runs(line: &str, min_spaces: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut spaces = 0usize;
    for ch in line.chars() {
        if ch == ' ' {
            spaces += 1;
            continue;
        }
        if spaces >= min_spaces && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        } else {
            for _ in 0..spaces {
                cur.push(' ');
            }
        }
        spaces = 0;
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn kami_entry(parts: &[String]) -> Option<Value> {
    let cells: Vec<String> = parts.iter().map(|p| p.trim().to_owned()).collect();
    if cells.len() == 1 && cells[0].starts_with("ksk_") {
        return Some(json!({
            "kiroApiKey": cells[0],
            "authMethod": "api_key",
        }));
    }
    let refresh = cells.get(2).map(|s| s.as_str()).unwrap_or("").to_owned();
    if refresh.is_empty() {
        return None;
    }
    let mut obj = Map::new();
    if let Some(email) = cells
        .first()
        .map(|s| s.as_str())
        .and_then(|s| trimmed(Some(s)))
    {
        obj.insert("email".to_owned(), Value::String(email));
    }
    obj.insert("refreshToken".to_owned(), Value::String(refresh));
    if let Some(id) = cells.get(3).and_then(|s| trimmed(Some(s))) {
        obj.insert("clientId".to_owned(), Value::String(id));
    }
    if let Some(secret) = cells.get(4).and_then(|s| trimmed(Some(s))) {
        obj.insert("clientSecret".to_owned(), Value::String(secret));
    }
    if let Some(provider) = cells.get(5).and_then(|s| trimmed(Some(s))) {
        obj.insert("provider".to_owned(), Value::String(provider));
    }
    Some(Value::Object(obj))
}

fn parse_kami_text(text: &str) -> Vec<Value> {
    text.split(['\n', '\r'])
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| kami_entry(&split_kami_line(line)))
        .collect()
}

fn parse_csv_row(row: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let chars: Vec<char> = row.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if quoted {
            if ch == '"' {
                if chars.get(i + 1) == Some(&'"') {
                    cur.push('"');
                    i += 1;
                } else {
                    quoted = false;
                }
            } else {
                cur.push(ch);
            }
        } else if ch == '"' {
            quoted = true;
        } else if ch == ',' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
        i += 1;
    }
    out.push(cur);
    out.into_iter().map(|cell| cell.trim().to_owned()).collect()
}

fn parse_csv_text(text: &str) -> Vec<Value> {
    let lines: Vec<&str> = text
        .trim_start_matches('\u{feff}')
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let headers: Vec<Option<&'static str>> = parse_csv_row(lines[0])
        .into_iter()
        .map(|h| csv_header_key(&h))
        .collect();
    if headers.iter().filter(|h| h.is_some()).count() < 2 {
        return parse_kami_text(text);
    }
    lines
        .iter()
        .skip(1)
        .filter_map(|line| {
            let cells = parse_csv_row(line);
            let mut item = Map::new();
            for (i, key) in headers.iter().enumerate() {
                let Some(key) = key else { continue };
                let Some(value) = cells.get(i).and_then(|s| trimmed(Some(s))) else {
                    continue;
                };
                item.insert((*key).to_owned(), Value::String(value));
            }
            if item.get("refreshToken").is_some() || item.get("kiroApiKey").is_some() {
                Some(Value::Object(item))
            } else {
                None
            }
        })
        .collect()
}

fn looks_like_csv(text: &str) -> bool {
    let first = text.split(['\n', '\r']).next().unwrap_or("");
    if !first.contains(',') || first.contains("----") {
        return false;
    }
    let lower = first.to_ascii_lowercase();
    lower.contains("email")
        || first.contains("邮箱")
        || lower.contains("refreshtoken")
        || first.contains("登录方式")
        || lower.contains("provider")
        || lower.contains("clientid")
        || lower.contains("kiroapikey")
}

/// Parse pasted Kiro text. JSON object imports still require an access token.
#[must_use]
pub fn parse_import_text(raw: &str) -> ParsedImport {
    let text = raw.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return ParsedImport {
            kind: ImportKind::Empty,
            sessions: Vec::new(),
        };
    }
    if (text.starts_with('{') || text.starts_with('['))
        && let Ok(parsed) = serde_json::from_str::<Value>(text)
    {
        return ParsedImport {
            kind: ImportKind::Json,
            sessions: sessions_from_auth(&parsed),
        };
    }
    let lines: Vec<&str> = text
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    if lines.len() == 1
        && lines[0].starts_with("ksk_")
        && !lines[0].contains("----")
        && !lines[0].contains(',')
    {
        let now = now_ms();
        let session = session_from_import(
            &json!({"kiroApiKey": lines[0], "authMethod": "api_key"}),
            now,
        )
        .ok();
        return ParsedImport {
            kind: ImportKind::Keys,
            sessions: session.into_iter().collect(),
        };
    }
    if looks_like_csv(text) {
        return ParsedImport {
            kind: ImportKind::Csv,
            sessions: sessions_from_auth(&Value::Array(parse_csv_text(text))),
        };
    }
    if text.contains("----") || (lines.len() > 1 && (text.contains('\t') || text.contains(','))) {
        return ParsedImport {
            kind: ImportKind::Kami,
            sessions: sessions_from_auth(&Value::Array(parse_kami_text(text))),
        };
    }
    if lines.len() == 1 && !lines[0].contains("----") && !lines[0].contains(',') {
        return ParsedImport {
            kind: ImportKind::RawToken,
            sessions: Vec::new(),
        };
    }
    ParsedImport {
        kind: ImportKind::Kami,
        sessions: sessions_from_auth(&Value::Array(parse_kami_text(text))),
    }
}

/// Merge IDE token + hashed OIDC registration (no filesystem).
#[must_use]
pub fn hydrate_sso_token(token: &Value, registration: Option<&Value>) -> Value {
    if !token.is_object() {
        return token.clone();
    }
    let method = json_str(token, &["authMethod", "auth_method"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    if method == "social" {
        return token.clone();
    }
    if json_str(token, &["clientId", "client_id"]).is_some()
        && json_str(token, &["clientSecret", "client_secret"]).is_some()
    {
        return token.clone();
    }
    let Some(registration) = registration.filter(|r| r.is_object()) else {
        return token.clone();
    };
    let mut out = token.clone();
    if let Some(obj) = out.as_object_mut() {
        if json_str(token, &["clientId", "client_id"]).is_none()
            && let Some(id) = json_str(registration, &["clientId", "client_id"])
        {
            obj.insert("clientId".to_owned(), Value::String(id));
        }
        if json_str(token, &["clientSecret", "client_secret"]).is_none()
            && let Some(secret) = json_str(registration, &["clientSecret", "client_secret"])
        {
            obj.insert("clientSecret".to_owned(), Value::String(secret));
        }
        if json_str(token, &["startUrl", "start_url"]).is_none() {
            obj.insert(
                "startUrl".to_owned(),
                Value::String(super::BUILDER_ID_START_URL.to_owned()),
            );
        }
    }
    out
}

pub fn import_sessions(body: &Value) -> Result<Vec<Session>, Error> {
    if let Some(token) = body.get("token") {
        if token.is_object() {
            return json_object_sessions(token);
        }
        if let Some(text) = token.as_str() {
            return text_sessions(text);
        }
    }
    if let Some(text) = body
        .get("text")
        .or_else(|| body.get("paste"))
        .or_else(|| body.get("raw"))
        .and_then(Value::as_str)
    {
        return text_sessions(text);
    }
    if body.is_object()
        && (has_access_token_field(body)
            || body.get("accounts").is_some()
            || body.get("credentials").is_some()
            || infer_auth_method(body) == "api_key")
    {
        return json_object_sessions(body);
    }
    Err(Error::InvalidImport(
        "Kiro import is missing access_token".into(),
    ))
}

fn json_object_sessions(token: &Value) -> Result<Vec<Session>, Error> {
    let rows = flatten_import(token);
    if rows.is_empty() {
        return Err(Error::InvalidImport(
            "Kiro import is missing access_token".into(),
        ));
    }
    let now = now_ms();
    let mut sessions = Vec::new();
    for row in rows {
        if !has_access_token_field(&row)
            && json_str(&row, &["kiroApiKey", "kiro_api_key"]).is_none()
        {
            continue;
        }
        if let Ok(session) = session_from_import(&row, now)
            && !session.access_token.is_empty()
        {
            sessions.push(session);
        }
    }
    if sessions.is_empty() {
        return Err(Error::InvalidImport(
            "Kiro import is missing access_token".into(),
        ));
    }
    Ok(sessions)
}

fn text_sessions(text: &str) -> Result<Vec<Session>, Error> {
    let parsed = parse_import_text(text);
    let sessions: Vec<Session> = parsed
        .sessions
        .into_iter()
        .filter(|s| !s.access_token.is_empty())
        .collect();
    if sessions.is_empty() {
        return Err(Error::InvalidImport(
            "Kiro import is missing access_token".into(),
        ));
    }
    Ok(sessions)
}

/// Ready session from an import body. First row with a non-empty access token.
pub fn imported_session(body: &Value) -> Result<Session, Error> {
    let mut sessions = import_sessions(body)?;
    Ok(sessions.remove(0))
}
