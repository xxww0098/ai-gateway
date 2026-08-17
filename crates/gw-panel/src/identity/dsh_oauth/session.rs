//! Device-code session state machine for DeepSeek Harness → AI-GateWay login.
//!
//! Pure transitions: start → pending → approve | deny; poll reports the
//! current state. Persistence and key minting live in the handler layer.

use chrono::{DateTime, Duration, Utc};
use rand::RngCore;

#[cfg(test)]
mod tests;

/// How long a freshly started session stays pending.
pub const DEFAULT_TTL: Duration = Duration::seconds(600);

/// Suggested poll interval returned to the client (seconds).
pub const DEFAULT_INTERVAL_SECS: i64 = 2;

/// A device-code grant that the plugin polls and the user approves in a browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSession {
    pub device_code: String,
    pub user_code: String,
    pub status: DeviceStatus,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub user_id: Option<i64>,
    pub api_key: Option<String>,
    pub origin: String,
}

/// Lifecycle of one device-code grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStatus {
    Pending,
    Approved,
    Denied,
}

/// Why [`approve`] / [`deny`] refused the transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    Expired,
    AlreadyResolved,
}

/// What [`poll`] tells the waiting plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    Pending,
    Approved { api_key: String, origin: String },
    Denied,
    Expired,
}

/// Start a pending session. `ttl` is the caller's lifetime, not a hidden constant.
#[must_use]
pub fn start(now: DateTime<Utc>, ttl: Duration, origin: String) -> DeviceSession {
    DeviceSession {
        device_code: random_hex(32),
        user_code: random_user_code(),
        status: DeviceStatus::Pending,
        created_at: now,
        expires_at: now + ttl,
        user_id: None,
        api_key: None,
        origin,
    }
}

/// Bind a minted AI-GateWay API key to a pending session.
///
/// # Errors
/// [`TransitionError::Expired`] when `now` is at or past `expires_at`.
/// [`TransitionError::AlreadyResolved`] when the session is no longer pending.
pub fn approve(
    session: &DeviceSession,
    now: DateTime<Utc>,
    user_id: i64,
    api_key: String,
) -> Result<DeviceSession, TransitionError> {
    require_pending(session, now)?;
    let mut next = session.clone();
    next.status = DeviceStatus::Approved;
    next.user_id = Some(user_id);
    next.api_key = Some(api_key);
    Ok(next)
}

/// Reject a pending session.
///
/// # Errors
/// Same as [`approve`].
pub fn deny(session: &DeviceSession, now: DateTime<Utc>) -> Result<DeviceSession, TransitionError> {
    require_pending(session, now)?;
    let mut next = session.clone();
    next.status = DeviceStatus::Denied;
    Ok(next)
}

/// Read the session for the polling plugin. Expired pending grants become
/// [`PollOutcome::Expired`]; a resolved grant stays resolved after expiry so
/// a late poll can still collect the key.
#[must_use]
pub fn poll(session: &DeviceSession, now: DateTime<Utc>) -> PollOutcome {
    match session.status {
        DeviceStatus::Pending if now >= session.expires_at => PollOutcome::Expired,
        DeviceStatus::Pending => PollOutcome::Pending,
        DeviceStatus::Denied => PollOutcome::Denied,
        DeviceStatus::Approved => match session.api_key.as_deref() {
            Some(api_key) => PollOutcome::Approved {
                api_key: api_key.to_owned(),
                origin: session.origin.clone(),
            },
            None => PollOutcome::Expired,
        },
    }
}

/// Compare user codes without caring about dashes or case.
#[must_use]
pub fn normalize_user_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

fn require_pending(session: &DeviceSession, now: DateTime<Utc>) -> Result<(), TransitionError> {
    if now >= session.expires_at {
        return Err(TransitionError::Expired);
    }
    if session.status != DeviceStatus::Pending {
        return Err(TransitionError::AlreadyResolved);
    }
    Ok(())
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    hex::encode(buf)
}

fn random_user_code() -> String {
    // Crockford-ish alphabet: no 0/O/1/I so a human can read it off a phone.
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut buf = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    let chars: String = buf
        .iter()
        .map(|b| char::from(ALPHABET[usize::from(*b) % ALPHABET.len()]))
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}
