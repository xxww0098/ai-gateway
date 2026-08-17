//! Upstream account pool: selection, health, and policy.
//! — the account selector and health hook.
//!
//! The availability rule is: bench an account after repeated failures, route
//! around it, and let it back in after a cooldown. The billing rule that makes
//! that safe is in [`crate::settlectx::RequestBilling::claim_finalize`] — a
//! cross-account retry settles ONCE, on the final response, so failing over
//! never bills twice.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use gw_authcore::AuthRecord;
use parking_lot::{Mutex, RwLock};

use crate::ports::{ChannelPolicy, ChannelPolicyStore};

/// Consecutive failures before an account is benched.
pub const DEFAULT_FAILURE_THRESHOLD: u32 = 3;

/// How long a benched account stays out of rotation.
pub const DEFAULT_COOLDOWN: Duration = Duration::from_secs(30);

/// Upper clamp on a configured weight, so a misconfigured row cannot blow up
/// the candidate list.
pub const MAX_WEIGHT: i64 = 100;

/// Per-account health, so a failing upstream is taken out of rotation and
/// returned automatically after a cooldown.
///
/// A pure, concurrency-safe tracker with no billing side effects and an
/// injectable clock for deterministic tests.
pub struct ChannelHealth {
    state: Mutex<HashMap<String, ChannelState>>,
    failure_threshold: u32,
    cooldown: Duration,
    clock: Box<dyn Fn() -> Instant + Send + Sync>,
}

#[derive(Debug, Clone, Default)]
struct ChannelState {
    consecutive_failures: u32,
    cooldown_until: Option<Instant>,
}

impl std::fmt::Debug for ChannelHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelHealth")
            .field("failure_threshold", &self.failure_threshold)
            .field("cooldown", &self.cooldown)
            .finish_non_exhaustive()
    }
}

impl ChannelHealth {
    /// Non-positive arguments take the defaults.
    pub fn new(failure_threshold: u32, cooldown: Duration) -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
            failure_threshold: if failure_threshold == 0 {
                DEFAULT_FAILURE_THRESHOLD
            } else {
                failure_threshold
            },
            cooldown: if cooldown.is_zero() {
                DEFAULT_COOLDOWN
            } else {
                cooldown
            },
            clock: Box::new(Instant::now),
        }
    }

    /// Replaces the clock, for deterministic cooldown tests.
    #[must_use]
    pub fn with_clock(mut self, clock: impl Fn() -> Instant + Send + Sync + 'static) -> Self {
        self.clock = Box::new(clock);
        self
    }

    /// Records an execution outcome. A success clears the account; a failure
    /// extends its streak and benches it once the threshold is reached, for the
    /// cooldown or the upstream-supplied `retry_after`, whichever is longer.
    pub fn record_result(&self, auth_id: &str, success: bool, retry_after: Option<Duration>) {
        if auth_id.is_empty() {
            return;
        }
        let mut state = self.state.lock();
        if success {
            state.remove(auth_id);
            return;
        }
        let entry = state.entry(auth_id.to_owned()).or_default();
        entry.consecutive_failures += 1;
        if entry.consecutive_failures >= self.failure_threshold {
            let cool = retry_after
                .filter(|r| *r > self.cooldown)
                .unwrap_or(self.cooldown);
            entry.cooldown_until = Some((self.clock)() + cool);
        }
    }

    /// Whether the account may currently be selected. An unknown account, or
    /// one whose cooldown elapsed, is healthy; an elapsed cooldown is cleared
    /// so the next attempt is a clean half-open probe.
    pub fn is_healthy(&self, auth_id: &str) -> bool {
        if auth_id.is_empty() {
            return true;
        }
        let mut state = self.state.lock();
        let Some(entry) = state.get(auth_id) else {
            return true;
        };
        let Some(until) = entry.cooldown_until else {
            return true;
        };
        if (self.clock)() >= until {
            state.remove(auth_id); // cooldown elapsed -> half-open probe
            return true;
        }
        false
    }

    /// How many accounts are benched right now. Feeds the
    /// `agw_channel_benched_total` gauge.
    pub fn benched_count(&self) -> i64 {
        let now = (self.clock)();
        self.state
            .lock()
            .values()
            .filter(|s| s.cooldown_until.is_some_and(|until| now < until))
            .count() as i64
    }
}

/// In-memory snapshot of `channel_policies`, refreshed off the hot path.
pub struct ChannelPolicyCache {
    store: Arc<dyn ChannelPolicyStore>,
    snapshot: RwLock<HashMap<String, ChannelPolicy>>,
}

impl ChannelPolicyCache {
    /// Empty until [`Self::refresh`] runs.
    pub fn new(store: Arc<dyn ChannelPolicyStore>) -> Self {
        Self {
            store,
            snapshot: RwLock::new(HashMap::new()),
        }
    }

    /// Reloads every policy row.
    pub async fn refresh(&self) -> anyhow::Result<()> {
        let rows = self.store.list_channel_policies().await?;
        let next = rows
            .into_iter()
            .map(|p| (p.auth_id.clone(), p))
            .collect::<HashMap<_, _>>();
        *self.snapshot.write() = next;
        Ok(())
    }

    /// Policy for `auth_id`, defaulting to weight 1 / priority 0 / enabled.
    pub fn lookup(&self, auth_id: &str) -> ChannelPolicy {
        self.snapshot
            .read()
            .get(auth_id)
            .cloned()
            .unwrap_or_else(|| ChannelPolicy {
                auth_id: auth_id.to_owned(),
                weight: 1,
                priority: 0,
                enabled: true,
            })
    }

    /// Refreshes on a ticker until the task is dropped.
    pub fn spawn_refresh(self: Arc<Self>, interval: Duration) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await; // the first tick fires immediately
            loop {
                ticker.tick().await;
                if let Err(err) = self.refresh().await {
                    tracing::warn!(%err, "channel policy refresh failed");
                }
            }
        })
    }
}

/// Health- and policy-aware upstream account selection.
///
/// The inner round-robin is inlined because there is no SDK selector left to
/// delegate to.
pub struct ChannelPool {
    health: Arc<ChannelHealth>,
    policies: Option<Arc<ChannelPolicyCache>>,
    cursor: AtomicUsize,
    /// NewAPI 风格的渠道亲和：同一租户打同一模型时粘在上次成功的账号上。
    /// 进程内、无持久化；账号不健康或已被排除时回落到加权轮询。
    affinity: DashMap<(i64, String), String>,
}

impl ChannelPool {
    /// Builds the pool over a health tracker.
    pub fn new(health: Arc<ChannelHealth>) -> Self {
        Self {
            health,
            policies: None,
            cursor: AtomicUsize::new(0),
            affinity: DashMap::new(),
        }
    }

    /// Installs the per-account policy source.
    #[must_use]
    pub fn with_policies(mut self, policies: Arc<ChannelPolicyCache>) -> Self {
        self.policies = Some(policies);
        self
    }

    /// Health tracker, so the dispatcher can report results back.
    pub fn health(&self) -> &Arc<ChannelHealth> {
        &self.health
    }

    /// Normalised policy for one account, with the weight clamped to
    /// `[1, MAX_WEIGHT]`.
    fn policy(&self, auth_id: &str) -> ChannelPolicy {
        let mut policy = match &self.policies {
            Some(cache) => cache.lookup(auth_id),
            None => ChannelPolicy {
                auth_id: auth_id.to_owned(),
                weight: 1,
                priority: 0,
                enabled: true,
            },
        };
        policy.weight = policy.weight.clamp(1, MAX_WEIGHT);
        policy
    }

    /// Picks one account for `(provider, model)`.
    ///
    /// Drops unhealthy and disabled accounts, keeps only the highest-priority
    /// tier that still has a survivor, expands by weight, and round-robins. If
    /// nothing is healthy and enabled it **fails open** over the full set:
    /// trying a benched account beats telling the client "no auth available".
    pub fn pick<'a>(&self, auths: &'a [AuthRecord]) -> Option<&'a AuthRecord> {
        self.pick_from(auths.iter())
    }

    /// 加权轮询，跳过本请求已经试过的账号。**不克隆 `AuthRecord`**
    /// （凭证解密结果在热路径上再 memcpy 一遍没有意义）。
    pub fn pick_excluding<'a>(
        &self,
        auths: &'a [AuthRecord],
        exclude: &[String],
    ) -> Option<&'a AuthRecord> {
        self.pick_from(
            auths
                .iter()
                .filter(|a| exclude.iter().all(|id| id != &a.id)),
        )
    }

    /// 先粘上次成功的账号（仍健康、未被排除），否则回落 [`Self::pick_excluding`]。
    pub fn pick_sticky<'a>(
        &self,
        auths: &'a [AuthRecord],
        preferred: Option<&str>,
        exclude: &[String],
    ) -> Option<&'a AuthRecord> {
        if let Some(id) = preferred
            && exclude.iter().all(|e| e != id)
            && let Some(auth) = auths.iter().find(|a| {
                a.id == id
                    && a.is_usable()
                    && self.health.is_healthy(&a.id)
                    && self.policy(&a.id).enabled
            })
        {
            return Some(auth);
        }
        self.pick_excluding(auths, exclude)
    }

    /// 记下这次成功的账号，供下一次同租户同模型粘住。
    pub fn remember(&self, user_id: i64, model: &str, auth_id: &str) {
        if user_id == 0 || model.is_empty() || auth_id.is_empty() {
            return;
        }
        self.affinity
            .insert((user_id, model.to_owned()), auth_id.to_owned());
    }

    /// 上次成功打这个 (user, model) 的账号。
    #[must_use]
    pub fn preferred(&self, user_id: i64, model: &str) -> Option<String> {
        if user_id == 0 || model.is_empty() {
            return None;
        }
        self.affinity
            .get(&(user_id, model.to_owned()))
            .map(|v| v.clone())
    }

    fn pick_from<'a, I>(&self, auths: I) -> Option<&'a AuthRecord>
    where
        I: IntoIterator<Item = &'a AuthRecord>,
    {
        let all: Vec<&AuthRecord> = auths.into_iter().collect();
        if all.is_empty() {
            return None;
        }
        let usable: Vec<(&AuthRecord, ChannelPolicy)> = all
            .iter()
            .copied()
            .filter(|a| a.is_usable())
            .filter(|a| self.health.is_healthy(&a.id))
            .map(|a| (a, self.policy(&a.id)))
            .filter(|(_, p)| p.enabled)
            .collect();

        if usable.is_empty() {
            return self.round_robin(&all).copied();
        }

        let max_priority = usable.iter().map(|(_, p)| p.priority).max().unwrap_or(0);
        let mut expanded: Vec<&AuthRecord> = Vec::new();
        for (auth, policy) in &usable {
            if policy.priority != max_priority {
                continue;
            }
            for _ in 0..policy.weight {
                expanded.push(*auth);
            }
        }
        self.round_robin(&expanded).copied()
    }

    fn round_robin<'a, T>(&self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            return None;
        }
        let n = self.cursor.fetch_add(1, Ordering::Relaxed);
        items.get(n % items.len())
    }
}

#[cfg(test)]
mod tests;
