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

use arc_swap::ArcSwap;
use dashmap::DashMap;
use gw_authcore::AuthRecord;

use crate::ports::{ChannelPolicy, ChannelPolicyStore};

/// Consecutive failures before an account is benched.
pub const DEFAULT_FAILURE_THRESHOLD: u32 = 3;

/// How long a benched account stays out of rotation.
pub const DEFAULT_COOLDOWN: Duration = Duration::from_secs(30);

/// Upper clamp on a configured weight, so a misconfigured row cannot blow up
/// the selector's arithmetic.
pub const MAX_WEIGHT: i64 = 100;

/// Process-local affinity is an optimisation, not durable routing state. Bound
/// it so untrusted `(tenant, model)` cardinality cannot grow memory forever.
pub const MAX_AFFINITY_ENTRIES: usize = 16 * 1024;

/// Per-account health, so a failing upstream is taken out of rotation and
/// returned automatically after a cooldown.
///
/// A sharded map is used instead of one global mutex: unrelated upstream
/// accounts must not serialize every health read and result report.
pub struct ChannelHealth {
    state: DashMap<String, ChannelState>,
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
            state: DashMap::new(),
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
        if success {
            self.state.remove(auth_id);
            return;
        }

        let mut entry = self.state.entry(auth_id.to_owned()).or_default();
        entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
        if entry.consecutive_failures >= self.failure_threshold {
            let cool = retry_after
                .filter(|r| *r > self.cooldown)
                .unwrap_or(self.cooldown);
            entry.cooldown_until = Some((self.clock)() + cool);
        }
    }

    /// Whether the account may currently be selected. An unknown account, or
    /// one whose cooldown elapsed, is healthy. Expiry resets the streak in
    /// place, avoiding a remove-versus-new-failure race on the same key.
    pub fn is_healthy(&self, auth_id: &str) -> bool {
        if auth_id.is_empty() {
            return true;
        }
        let Some(mut entry) = self.state.get_mut(auth_id) else {
            return true;
        };
        let Some(until) = entry.cooldown_until else {
            return true;
        };
        if (self.clock)() >= until {
            *entry = ChannelState::default();
            return true;
        }
        false
    }

    /// How many accounts are benched right now. Feeds the
    /// `agw_channel_benched_total` gauge.
    pub fn benched_count(&self) -> i64 {
        let now = (self.clock)();
        self.state
            .iter()
            .filter(|s| s.cooldown_until.is_some_and(|until| now < until))
            .count() as i64
    }
}

/// In-memory snapshot of `channel_policies`, refreshed off the hot path.
pub struct ChannelPolicyCache {
    store: Arc<dyn ChannelPolicyStore>,
    /// Readers take an atomic snapshot; a refresh never blocks inference.
    snapshot: ArcSwap<HashMap<String, ChannelPolicy>>,
}

impl ChannelPolicyCache {
    /// Empty until [`Self::refresh`] runs.
    pub fn new(store: Arc<dyn ChannelPolicyStore>) -> Self {
        Self {
            store,
            snapshot: ArcSwap::from_pointee(HashMap::new()),
        }
    }

    /// Reloads every policy row and publishes it atomically.
    pub async fn refresh(&self) -> anyhow::Result<()> {
        let rows = self.store.list_channel_policies().await?;
        let next = rows
            .into_iter()
            .map(|p| (p.auth_id.clone(), p))
            .collect::<HashMap<_, _>>();
        self.snapshot.store(Arc::new(next));
        Ok(())
    }

    /// Policy for `auth_id`, defaulting to weight 1 / priority 0 / enabled.
    pub fn lookup(&self, auth_id: &str) -> ChannelPolicy {
        self.snapshot
            .load()
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
/// Selection allocates only one candidate vector and never expands it by
/// weight. Weight is applied by walking cumulative ranges, so the cost remains
/// O(number of accounts), independent of configured weight.
pub struct ChannelPool {
    health: Arc<ChannelHealth>,
    policies: Option<Arc<ChannelPolicyCache>>,
    cursor: AtomicUsize,
    /// NewAPI 风格的渠道亲和：同一租户打同一模型时粘在上次成功的账号上。
    /// 进程内、无持久化；账号不健康或已被排除时回落到加权轮询。
    affinity: DashMap<(i64, String), String>,
}

#[derive(Debug, Clone, Copy)]
struct Candidate<'a> {
    auth: &'a AuthRecord,
    weight: usize,
    priority: i64,
    healthy: bool,
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

    /// Normalised policy for one account.
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
    /// Drops operator-disabled, unusable and policy-disabled accounts, keeps
    /// only the highest-priority tier, and weighted-round-robins it. If every
    /// otherwise eligible account is merely in health cooldown, selection
    /// fails open across those eligible accounts. Kill switches are never
    /// bypassed by fail-open.
    pub fn pick<'a>(&self, auths: &'a [AuthRecord]) -> Option<&'a AuthRecord> {
        self.pick_from(auths, &[])
    }

    /// 加权轮询，跳过本请求已经试过的账号。**不克隆 `AuthRecord`**。
    pub fn pick_excluding<'a>(
        &self,
        auths: &'a [AuthRecord],
        exclude: &[String],
    ) -> Option<&'a AuthRecord> {
        self.pick_from(auths, exclude)
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
        let key = (user_id, model.to_owned());
        if self.affinity.len() >= MAX_AFFINITY_ENTRIES && !self.affinity.contains_key(&key) {
            // Affinity is only a latency hint. Dropping old hints is preferable
            // to letting attacker-controlled model cardinality become a leak.
            self.affinity.clear();
        }
        self.affinity.insert(key, auth_id.to_owned());
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

    fn pick_from<'a>(
        &self,
        auths: &'a [AuthRecord],
        exclude: &[String],
    ) -> Option<&'a AuthRecord> {
        let policy_snapshot = self
            .policies
            .as_ref()
            .map(|cache| cache.snapshot.load_full());
        let mut candidates = Vec::with_capacity(auths.len());

        for auth in auths {
            if !auth.is_usable() || exclude.iter().any(|id| id == &auth.id) {
                continue;
            }
            let stored = policy_snapshot
                .as_deref()
                .and_then(|snapshot| snapshot.get(&auth.id));
            if stored.is_some_and(|policy| !policy.enabled) {
                continue;
            }
            let weight = stored
                .map_or(1, |policy| policy.weight)
                .clamp(1, MAX_WEIGHT) as usize;
            let priority = stored.map_or(0, |policy| policy.priority);
            candidates.push(Candidate {
                auth,
                weight,
                priority,
                healthy: self.health.is_healthy(&auth.id),
            });
        }

        let use_health = candidates.iter().any(|candidate| candidate.healthy);
        let max_priority = candidates
            .iter()
            .filter(|candidate| !use_health || candidate.healthy)
            .map(|candidate| candidate.priority)
            .max()?;
        let total_weight = candidates
            .iter()
            .filter(|candidate| {
                (!use_health || candidate.healthy) && candidate.priority == max_priority
            })
            .fold(0_usize, |sum, candidate| {
                sum.saturating_add(candidate.weight)
            });
        if total_weight == 0 {
            return None;
        }

        let mut ticket = self.cursor.fetch_add(1, Ordering::Relaxed) % total_weight;
        for candidate in candidates {
            if (use_health && !candidate.healthy) || candidate.priority != max_priority {
                continue;
            }
            if ticket < candidate.weight {
                return Some(candidate.auth);
            }
            ticket -= candidate.weight;
        }
        None
    }
}

#[cfg(test)]
mod tests;
