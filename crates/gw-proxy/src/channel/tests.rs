//! Account health, policy and selection.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::testsupport::{FakePolicyStore, auth_record};

/// A clock the test drives by hand, so cooldown expiry is deterministic.
struct TestClock {
    base: Instant,
    offset_ms: Arc<AtomicU64>,
}

impl TestClock {
    fn new() -> (Self, Arc<AtomicU64>) {
        let offset = Arc::new(AtomicU64::new(0));
        (
            Self {
                base: Instant::now(),
                offset_ms: offset.clone(),
            },
            offset,
        )
    }

    fn into_fn(self) -> impl Fn() -> Instant + Send + Sync + 'static {
        move || self.base + Duration::from_millis(self.offset_ms.load(Ordering::SeqCst))
    }
}

fn health_with_clock(threshold: u32, cooldown: Duration) -> (ChannelHealth, Arc<AtomicU64>) {
    let (clock, offset) = TestClock::new();
    (
        ChannelHealth::new(threshold, cooldown).with_clock(clock.into_fn()),
        offset,
    )
}

// ---------------------------------------------------------------- health

#[test]
fn an_account_stays_in_rotation_until_the_streak_reaches_the_threshold() {
    let (health, _) = health_with_clock(3, Duration::from_secs(30));
    for _ in 0..2 {
        health.record_result("a", false, None);
        assert!(health.is_healthy("a"), "benched too early");
    }
    health.record_result("a", false, None);
    assert!(
        !health.is_healthy("a"),
        "should be benched at the threshold"
    );
    assert_eq!(health.benched_count(), 1);
}

#[test]
fn a_success_clears_the_streak_so_it_takes_a_full_run_to_bench() {
    let (health, _) = health_with_clock(3, Duration::from_secs(30));
    health.record_result("a", false, None);
    health.record_result("a", false, None);
    health.record_result("a", true, None);
    health.record_result("a", false, None);
    health.record_result("a", false, None);
    assert!(health.is_healthy("a"));
}

#[test]
fn a_benched_account_returns_as_a_half_open_probe_once_the_cooldown_elapses() {
    let (health, clock) = health_with_clock(1, Duration::from_millis(500));
    health.record_result("a", false, None);
    assert!(!health.is_healthy("a"));

    clock.store(499, Ordering::SeqCst);
    assert!(!health.is_healthy("a"));

    clock.store(500, Ordering::SeqCst);
    assert!(health.is_healthy("a"));
    assert_eq!(
        health.benched_count(),
        0,
        "an elapsed cooldown must clear the record, not leave it benched",
    );

    // Half-open: one more failure benches it again immediately.
    health.record_result("a", false, None);
    assert!(!health.is_healthy("a"));
}

#[test]
fn an_upstream_retry_after_extends_the_cooldown_but_never_shortens_it() {
    let (health, clock) = health_with_clock(1, Duration::from_millis(500));
    health.record_result("a", false, Some(Duration::from_millis(2_000)));
    clock.store(1_000, Ordering::SeqCst);
    assert!(!health.is_healthy("a"), "the longer window must win");

    let (health, clock) = health_with_clock(1, Duration::from_millis(500));
    health.record_result("b", false, Some(Duration::from_millis(10)));
    clock.store(100, Ordering::SeqCst);
    assert!(
        !health.is_healthy("b"),
        "a shorter retry-after must not undercut the configured cooldown",
    );
}

#[test]
fn an_unknown_or_unnamed_account_is_healthy() {
    let (health, _) = health_with_clock(1, Duration::from_secs(1));
    assert!(health.is_healthy("never-seen"));
    assert!(health.is_healthy(""));
    health.record_result("", false, None);
    assert_eq!(health.benched_count(), 0);
}

// ---------------------------------------------------------------- policy

#[tokio::test]
async fn an_account_without_a_policy_row_gets_the_permissive_default() {
    let cache = ChannelPolicyCache::new(Arc::new(FakePolicyStore::default()));
    let policy = cache.lookup("acct-1");
    assert_eq!(policy.weight, 1);
    assert_eq!(policy.priority, 0);
    assert!(
        policy.enabled,
        "an unconfigured account must still be usable"
    );
}

#[tokio::test]
async fn a_refresh_replaces_the_whole_snapshot() {
    let store = Arc::new(FakePolicyStore::default());
    store.policies.lock().push(ChannelPolicy {
        auth_id: "acct-1".to_owned(),
        weight: 5,
        priority: 2,
        enabled: false,
    });
    let cache = ChannelPolicyCache::new(store.clone());
    cache.refresh().await.expect("refresh");
    assert_eq!(cache.lookup("acct-1").weight, 5);
    assert!(!cache.lookup("acct-1").enabled);

    store.policies.lock().clear();
    cache.refresh().await.expect("refresh");
    assert!(
        cache.lookup("acct-1").enabled,
        "a deleted row must fall back to the default, not linger",
    );
}

// ---------------------------------------------------------------- selection

/// Builds a pool whose policy cache is already populated.
///
/// `refresh` is async but the fake store never yields, so a manual poll is
/// enough and keeps the selection tests synchronous.
fn pool_with(policies: Vec<ChannelPolicy>) -> (Arc<ChannelHealth>, ChannelPool) {
    let store = Arc::new(FakePolicyStore::default());
    *store.policies.lock() = policies;
    let cache = Arc::new(ChannelPolicyCache::new(store));
    poll_once(cache.refresh()).expect("the fake store resolves immediately");
    let health = Arc::new(ChannelHealth::new(1, Duration::from_secs(60)));
    (
        health.clone(),
        ChannelPool::new(health).with_policies(cache),
    )
}

/// Drives a future that is known to complete without yielding.
fn poll_once<T>(future: impl std::future::Future<Output = T>) -> T {
    use std::task::{Context, Poll, Waker};
    let mut cx = Context::from_waker(Waker::noop());
    let mut future = Box::pin(future);
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("future did not complete synchronously"),
    }
}

fn policy(auth_id: &str, weight: i64, priority: i64, enabled: bool) -> ChannelPolicy {
    ChannelPolicy {
        auth_id: auth_id.to_owned(),
        weight,
        priority,
        enabled,
    }
}

#[test]
fn a_disabled_account_is_never_picked_while_another_is_available() {
    let (_, pool) = pool_with(vec![policy("a", 1, 0, false)]);
    let auths = vec![auth_record("a", "openai"), auth_record("b", "openai")];
    for _ in 0..10 {
        assert_eq!(pool.pick(&auths).expect("a pick").id, "b");
    }
}

#[test]
fn a_benched_account_is_skipped() {
    let (health, pool) = pool_with(vec![]);
    health.record_result("a", false, None);
    let auths = vec![auth_record("a", "openai"), auth_record("b", "openai")];
    for _ in 0..10 {
        assert_eq!(pool.pick(&auths).expect("a pick").id, "b");
    }
}

#[test]
fn only_the_highest_priority_tier_that_still_has_a_survivor_is_used() {
    let (health, pool) = pool_with(vec![policy("hi", 1, 10, true), policy("lo", 1, 0, true)]);
    let auths = vec![auth_record("hi", "openai"), auth_record("lo", "openai")];
    for _ in 0..10 {
        assert_eq!(pool.pick(&auths).expect("a pick").id, "hi");
    }

    // Bench the top tier and the lower one takes over.
    health.record_result("hi", false, None);
    assert_eq!(pool.pick(&auths).expect("a pick").id, "lo");
}

#[test]
fn weight_biases_selection_without_starving_the_lighter_account() {
    let (_, pool) = pool_with(vec![
        policy("heavy", 4, 0, true),
        policy("light", 1, 0, true),
    ]);
    let auths = vec![
        auth_record("heavy", "openai"),
        auth_record("light", "openai"),
    ];

    let mut heavy = 0;
    let mut light = 0;
    for _ in 0..100 {
        match pool.pick(&auths).expect("a pick").id.as_str() {
            "heavy" => heavy += 1,
            "light" => light += 1,
            other => panic!("unexpected pick {other}"),
        }
    }
    assert!(heavy > light, "weight 4 should out-draw weight 1");
    assert!(
        light > 0,
        "a weighted pool must not starve the lighter account"
    );
}

#[test]
fn an_absurd_weight_is_clamped_rather_than_blowing_up_the_candidate_list() {
    let (_, pool) = pool_with(vec![policy("a", i64::MAX, 0, true)]);
    let auths = vec![auth_record("a", "openai")];
    assert_eq!(pool.pick(&auths).expect("a pick").id, "a");
}

#[test]
fn a_fully_benched_pool_fails_open_rather_than_refusing_the_client() {
    // Trying a benched account beats answering "no auth available".
    let (health, pool) = pool_with(vec![]);
    health.record_result("a", false, None);
    health.record_result("b", false, None);
    let auths = vec![auth_record("a", "openai"), auth_record("b", "openai")];
    assert!(pool.pick(&auths).is_some());
}

#[test]
fn an_empty_pool_yields_nothing() {
    let (_, pool) = pool_with(vec![]);
    assert!(pool.pick(&[]).is_none());
}

#[test]
fn a_credential_either_kill_switch_rules_out_is_excluded_before_policy() {
    // Both switches are gw-authcore's call (`AuthRecord::is_usable`); the pool
    // must honour them rather than re-deriving the rule.
    let (_, pool) = pool_with(vec![]);
    let mut operator_disabled = auth_record("a", "openai");
    operator_disabled.disabled = true;
    let mut health_disabled = auth_record("b", "openai");
    health_disabled.unavailable = true;
    let auths = vec![
        operator_disabled,
        health_disabled,
        auth_record("c", "openai"),
    ];
    for _ in 0..5 {
        assert_eq!(pool.pick(&auths).expect("a pick").id, "c");
    }
}

#[test]
fn retrying_excludes_the_accounts_already_tried() {
    let (_, pool) = pool_with(vec![]);
    let auths = vec![
        auth_record("a", "openai"),
        auth_record("b", "openai"),
        auth_record("c", "openai"),
    ];

    let first = pool.pick(&auths).expect("a pick").id.clone();
    let second = pool
        .pick_excluding(&auths, std::slice::from_ref(&first))
        .expect("a second pick")
        .id
        .clone();
    assert_ne!(first, second, "failover must move to a different account");

    let tried = vec![first, second];
    let third = pool.pick_excluding(&auths, &tried).expect("a third pick");
    assert!(!tried.contains(&third.id));

    let all: Vec<String> = auths.iter().map(|a| a.id.clone()).collect();
    assert!(
        pool.pick_excluding(&auths, &all).is_none(),
        "an exhausted pool must report exhaustion instead of repeating",
    );
}

#[test]
fn a_sticky_account_is_preferred_while_it_stays_healthy() {
    let (_, pool) = pool_with(vec![]);
    let auths = vec![auth_record("a", "openai"), auth_record("b", "openai")];
    pool.remember(7, "gpt-4o", "b");
    let preferred = pool.preferred(7, "gpt-4o");
    assert_eq!(preferred.as_deref(), Some("b"));
    for _ in 0..8 {
        assert_eq!(
            pool.pick_sticky(&auths, preferred.as_deref(), &[])
                .expect("a pick")
                .id,
            "b",
        );
    }
}

#[test]
fn a_benched_sticky_account_falls_back_to_the_weighted_pool() {
    let (health, pool) = pool_with(vec![]);
    let auths = vec![auth_record("a", "openai"), auth_record("b", "openai")];
    pool.remember(7, "gpt-4o", "b");
    health.record_result("b", false, None);
    let preferred = pool.preferred(7, "gpt-4o");
    for _ in 0..8 {
        assert_eq!(
            pool.pick_sticky(&auths, preferred.as_deref(), &[])
                .expect("a pick")
                .id,
            "a",
            "affinity must not override health",
        );
    }
}

#[test]
fn excluding_the_sticky_account_does_not_clone_the_rest_of_the_pool() {
    // 行为钉的是「排除生效」，不是实现细节。克隆凭证表是热路径上的浪费，
    // 这条只保证排除之后还能挑到剩下的账号。
    let (_, pool) = pool_with(vec![]);
    let auths = vec![
        auth_record("a", "openai"),
        auth_record("b", "openai"),
        auth_record("c", "openai"),
    ];
    pool.remember(7, "gpt-4o", "a");
    let picked = pool
        .pick_sticky(&auths, Some("a"), &["a".to_owned()])
        .expect("a fallback pick");
    assert_ne!(picked.id, "a");
}
