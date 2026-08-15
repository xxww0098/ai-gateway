//! In-process L1 caches for the hot auth path.
//!
//! Both caches are cheap-to-clone shared handles (`Arc` inside) rather than
//! `&'static` singletons: `gw-proxy`'s access provider and `gw-panel`'s JWT
//! middleware must observe the *same* entries.

use std::sync::{Arc, Weak};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use dashmap::DashMap;
use gw_model::Id;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

#[cfg(test)]
mod tests;

/// Sweep cadence of both background sweepers.
pub const DEFAULT_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// Ceiling on a [`UserStatusCache`] entry's lifetime: no cached status may
/// outlive the API-key cache's own 5-minute default, or an admin's status
/// flip could stay invisible for longer than one key-cache generation.
pub const MAX_USER_STATUS_TTL: Duration = Duration::from_secs(5 * 60);

/// A validated API key, cached under the SHA-256 hash of the plaintext key.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedKey {
    /// Owner of the key.
    pub user_id: Id,
    /// The `api_keys.id` row this entry was built from.
    pub api_key_id: Id,
    /// Group the key is bound to, if any.
    pub group_id: Option<Id>,
    /// Group rate multiplier applied to this key's billing.
    pub rate_mult: f64,
    /// The owner's `users.status` at the time of caching.
    pub status: String,
    /// When this entry stops being served. Set by the caller (the SDK access
    /// provider owns the TTL constant).
    pub expires_at: DateTime<Utc>,
}

/// L1 cache of validated API keys.
///
/// Cloning yields another handle to the same entries.
#[derive(Debug, Clone, Default)]
pub struct ApiKeyCache {
    entries: Arc<DashMap<String, Arc<CachedKey>>>,
}

impl ApiKeyCache {
    /// Creates an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the entry for `hash` when present and unexpired, evicting it as
    /// a side effect when it has expired.
    pub fn get(&self, hash: &str) -> Option<Arc<CachedKey>> {
        let entry = self.entries.get(hash).map(|e| Arc::clone(e.value()))?;
        if Utc::now() > entry.expires_at {
            self.entries.remove(hash);
            return None;
        }
        Some(entry)
    }

    /// Stores `entry` under `hash`.
    pub fn set(&self, hash: impl Into<String>, entry: CachedKey) {
        self.entries.insert(hash.into(), Arc::new(entry));
    }

    /// Drops the entry for `hash`.
    pub fn delete(&self, hash: &str) {
        self.entries.remove(hash);
    }

    /// Number of entries currently held, expired ones included.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds no entries at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Removes every expired entry, returning how many were dropped.
    pub fn sweep(&self) -> usize {
        sweep_expired(&self.entries)
    }

    /// Spawns the background sweeper. The returned [`SweepHandle`] is the
    /// cancellation token — dropping it stops the task, and
    /// [`SweepHandle::shutdown`] additionally waits for it.
    ///
    /// The task holds only a weak reference, so it also exits once the last
    /// cache handle is dropped.
    pub fn spawn_sweeper(&self, interval: Duration) -> SweepHandle {
        spawn_sweeper(&self.entries, interval, "api_key", sweep_expired)
    }
}

/// A cached `users.status` lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserStatus {
    /// The `users.status` column value.
    pub status: String,
    /// When this entry stops being served.
    pub expires_at: DateTime<Utc>,
}

/// L1 cache of `users.status`, keyed by user id.
///
/// Cloning yields another handle to the same entries, which is what lets the
/// `/v1/*` access provider and the panel's auth middleware share one instance.
#[derive(Debug, Clone, Default)]
pub struct UserStatusCache {
    entries: Arc<DashMap<Id, UserStatus>>,
}

impl UserStatusCache {
    /// Creates an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the cached status for `user_id` when present and unexpired,
    /// evicting it as a side effect when it has expired.
    pub fn get(&self, user_id: Id) -> Option<UserStatus> {
        let entry = self.entries.get(&user_id).map(|e| e.value().clone())?;
        if Utc::now() > entry.expires_at {
            self.entries.remove(&user_id);
            return None;
        }
        Some(entry)
    }

    /// Caches `status` for `ttl`, with both guards: a zero `ttl` is a no-op
    /// (callers must not poison the cache with an already-expired entry — the
    /// negative half is unrepresentable in a [`Duration`]), and the TTL is
    /// clamped to [`MAX_USER_STATUS_TTL`].
    pub fn set(&self, user_id: Id, status: impl Into<String>, ttl: Duration) {
        if ttl.is_zero() {
            return;
        }
        let ttl = ttl.min(MAX_USER_STATUS_TTL);
        let ttl_ms = i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX);
        self.entries.insert(
            user_id,
            UserStatus {
                status: status.into(),
                expires_at: Utc::now() + TimeDelta::milliseconds(ttl_ms),
            },
        );
    }

    /// Drops the entry for `user_id` so the next auth attempt re-reads the
    /// database. Called by every admin path that flips a user's status.
    pub fn invalidate_user(&self, user_id: Id) {
        self.entries.remove(&user_id);
    }

    /// Number of entries currently held, expired ones included.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds no entries at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Removes every expired entry, returning how many were dropped.
    pub fn sweep(&self) -> usize {
        sweep_expired(&self.entries)
    }

    /// Spawns the background sweeper. See [`ApiKeyCache::spawn_sweeper`] for
    /// the cancellation contract.
    pub fn spawn_sweeper(&self, interval: Duration) -> SweepHandle {
        spawn_sweeper(&self.entries, interval, "user_status", sweep_expired)
    }
}

/// Cancellation handle for a cache sweeper task.
///
/// Dropping the handle cancels the task, and [`Self::shutdown`] waits for it
/// to actually return.
#[derive(Debug)]
pub struct SweepHandle {
    cancel: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl SweepHandle {
    /// Signals the sweeper to stop and waits for it to finish.
    pub async fn shutdown(mut self) {
        let _ = self.cancel.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }

    /// Whether the sweeper task has already returned.
    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for SweepHandle {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}

/// Anything a cache stores under its own expiry stamp.
trait Expiring {
    fn expires_at(&self) -> DateTime<Utc>;
}

impl Expiring for Arc<CachedKey> {
    fn expires_at(&self) -> DateTime<Utc> {
        self.as_ref().expires_at
    }
}

impl Expiring for UserStatus {
    fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }
}

/// Drops every entry whose deadline has passed, returning how many went.
///
/// The count is approximate under concurrent writers — inserts racing the
/// sweep are counted against it — so it saturates instead of underflowing.
fn sweep_expired<K: Eq + std::hash::Hash, V: Expiring>(entries: &DashMap<K, V>) -> usize {
    let now = Utc::now();
    let before = entries.len();
    entries.retain(|_, entry| entry.expires_at() >= now);
    before.saturating_sub(entries.len())
}

/// Spawns a periodic sweep task over `state`, cancelled by the returned handle.
///
/// `state` is captured weakly so a forgotten handle cannot keep a dropped cache
/// alive: once the last strong handle goes, the next tick fails to upgrade and
/// the task returns.
fn spawn_sweeper<S: Send + Sync + 'static>(
    state: &Arc<S>,
    interval: Duration,
    label: &'static str,
    sweep: impl Fn(&S) -> usize + Send + 'static,
) -> SweepHandle {
    let (cancel, mut cancelled) = watch::channel(false);
    let alive: Weak<S> = Arc::downgrade(state);
    // tokio::time::interval panics on a zero period; a zero interval here can
    // only come from a misconfiguration, and busy-looping is not a better answer.
    let period = interval.max(Duration::from_millis(1));

    let task = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(period);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // tokio's first tick completes immediately; a one-shot Ticker would not
        // fire until one period has elapsed.
        ticker.tick().await;

        loop {
            tokio::select! {
                // Ok => cancel() was called, Err => the handle was dropped.
                _ = cancelled.changed() => break,
                _ = ticker.tick() => {
                    let Some(state) = alive.upgrade() else { break };
                    let removed = sweep(&state);
                    if removed > 0 {
                        tracing::debug!(cache = label, removed, "swept expired cache entries");
                    }
                }
            }
        }
    });

    SweepHandle {
        cancel,
        task: Some(task),
    }
}
